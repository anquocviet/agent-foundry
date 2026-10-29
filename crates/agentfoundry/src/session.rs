use crate::{Failure, Result, application::Plan, infrastructure as fs, update::PreparedUpdate};
use serde::{Deserialize, Serialize};
use std::path::Path;
const ACTIVE: &str = ".agentfoundry/runtime/resolution.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Conflict {
    target: String,
    base: String,
    local: String,
    upstream: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Session {
    schema: String,
    id: String,
    plan_digest: String,
    frozen: crate::models::WorkspaceSnapshot,
    target: crate::update::CandidateIdentity,
    release: Option<crate::release::ReleaseMetadata>,
    conflicts: Vec<Conflict>,
}
fn artifact(id: &str, name: &str) -> String {
    format!(".agentfoundry/runtime/resolutions/{id}/{name}")
}
pub fn pending(root: &Path) -> Result<bool> {
    Ok(fs::read(root, ACTIVE, fs::CONTROL_LIMIT)?.is_some())
}
fn load(root: &Path) -> Result<(Session, Plan)> {
    let raw = fs::read(root, ACTIVE, fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "no pending resolution session"))?;
    let session: Session = serde_json::from_slice(&raw)?;
    if session.schema != "agentfoundry.pending-resolution/v1"
        || session.id.is_empty()
        || session.id.len() > 128
        || !session.id.bytes().all(|b| b.is_ascii_hexdigit())
        || session.conflicts.is_empty()
        || session.conflicts.len() > 10000
    {
        return Err(Failure::new("INVALID_STATE", "invalid resolution identity"));
    }
    let bytes = fs::read(root, &artifact(&session.id, "plan.json"), fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "session plan missing"))?;
    if fs::digest(&bytes) != session.plan_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "session plan digest mismatch",
        ));
    }
    let stored: crate::transaction_store::StoredPlan = serde_json::from_slice(&bytes)?;
    if stored.root_identity != fs::root_identity(root)? {
        return Err(Failure::new(
            "WORKSPACE_DRIFT",
            "session root identity changed",
        ));
    }
    let mut plan = stored.hydrate(root, &session.id)?;
    plan.snapshot = Some(session.frozen.clone());
    if plan.payload_digest != session.target.payload_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "session target differs from plan",
        ));
    }
    let state_bytes = plan
        .operations
        .iter()
        .find(|op| op.target == crate::STATE)
        .and_then(|op| op.after.as_deref())
        .ok_or_else(|| Failure::new("INVALID_STATE", "session has no state commit"))?;
    let state = crate::models::InstallationState::parse(state_bytes)?;
    if state.version != session.target.version
        || state.source != session.target.source
        || state.binary_digest != session.target.binary_digest
        || state.payload_digest != session.target.payload_digest
    {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "session target identity differs from pinned state",
        ));
    }
    let binary = plan
        .operations
        .iter()
        .find(|op| op.target.starts_with("scripts/bin/agentfoundry"))
        .and_then(|op| op.after.as_deref())
        .ok_or_else(|| Failure::new("INVALID_STATE", "session binary missing"))?;
    if fs::digest(binary) != session.target.binary_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "pinned candidate digest differs",
        ));
    }
    if let Some(metadata) = &session.release {
        metadata
            .verify_candidate(crate::candidate::current_platform()?, binary)
            .map_err(|e| Failure::new("INTEGRITY_FAILURE", e))?;
        if metadata.version != session.target.version
            || metadata.source_commit != session.target.source
            || metadata.payload_digest != session.target.payload_digest
        {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "retained release identity mismatch",
            ));
        }
    } else if !session.target.source.starts_with("development-sha256:") {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "stable session release evidence missing",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (index, conflict) in session.conflicts.iter().enumerate() {
        crate::domain::RelativePath::new(&conflict.target)
            .map_err(|e| Failure::new("INVALID_STATE", e))?;
        if !seen.insert(&conflict.target)
            || !plan
                .operations
                .iter()
                .any(|op| op.target == conflict.target)
        {
            return Err(Failure::new(
                "INVALID_STATE",
                "unknown or duplicate conflict",
            ));
        }
        for (kind, hash) in [
            ("BASE", &conflict.base),
            ("LOCAL", &conflict.local),
            ("UPSTREAM", &conflict.upstream),
        ] {
            crate::domain::validate_digest(hash).map_err(|e| Failure::new("INVALID_STATE", e))?;
            let bytes = fs::read(
                root,
                &artifact(&session.id, &format!("{index}/{kind}")),
                8 * 1024 * 1024,
            )?
            .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "conflict input missing"))?;
            if fs::digest(&bytes) != *hash {
                return Err(Failure::new("INTEGRITY_FAILURE", "conflict input changed"));
            }
            if kind == "LOCAL"
                && !plan.operations.iter().any(|op| {
                    if op.target != conflict.target {
                        return false;
                    }
                    let Some(before) = op.before.as_deref() else {
                        return false;
                    };
                    if let Some(entry) = state.entries.iter().find(|e| {
                        e.target == conflict.target
                            && e.ownership == crate::models::Ownership::ManagedBlock
                    }) {
                        let name = if entry.target == "CLAUDE.md" {
                            "AGENTFOUNDRY:CLAUDE"
                        } else {
                            "AGENTFOUNDRY"
                        };
                        let Ok(Some(block)) = crate::domain::block_content(before, name) else {
                            return false;
                        };
                        block == bytes.strip_suffix(b"\n").unwrap_or(&bytes)
                    } else {
                        before == bytes.as_slice()
                    }
                })
            {
                return Err(Failure::new(
                    "INTEGRITY_FAILURE",
                    "conflict LOCAL differs from frozen plan",
                ));
            }
            if kind == "UPSTREAM"
                && !state
                    .entries
                    .iter()
                    .any(|entry| entry.target == conflict.target && entry.sha256 == *hash)
            {
                return Err(Failure::new(
                    "INTEGRITY_FAILURE",
                    "conflict UPSTREAM differs from target baseline",
                ));
            }
            if kind == "BASE" {
                let previous = plan
                    .operations
                    .iter()
                    .find(|op| op.target == crate::STATE)
                    .and_then(|op| op.before.as_deref())
                    .ok_or_else(|| Failure::new("INVALID_STATE", "session prior state missing"))?;
                let previous = crate::models::InstallationState::parse(previous)?;
                if !previous
                    .entries
                    .iter()
                    .any(|entry| entry.target == conflict.target && entry.sha256 == *hash)
                {
                    return Err(Failure::new(
                        "INTEGRITY_FAILURE",
                        "conflict BASE differs from installed provenance",
                    ));
                }
            }
        }
    }
    Ok((session, plan))
}
pub fn status(root: &Path) -> Result<serde_json::Value> {
    let (session, _) = load(root)?;
    Ok(
        serde_json::json!({"id":session.id,"target":session.target,"conflicts":session.conflicts.iter().enumerate().map(|(index,c)| serde_json::json!({"target":c.target,"resolved":artifact(&session.id,&format!("{index}/RESOLVED"))})).collect::<Vec<_>>(),"next_action":"Edit RESOLVED; update --continue [--dry-run], or update --abort"}),
    )
}
pub fn stage(root: &Path, prepared: &PreparedUpdate) -> Result<serde_json::Value> {
    let _lock = fs::Lock::acquire(root)?;
    if pending(root)? {
        return Err(Failure::new(
            "SESSION_EXISTS",
            "pending resolution retained; use --continue or --abort",
        ));
    }
    crate::application::verify(&prepared.plan, root)?;
    let id = crate::unique_id();
    let stored = crate::transaction_store::StoredPlan::from_plan(&prepared.plan);
    let plan_bytes = serde_json::to_vec(&stored)?;
    if plan_bytes.len() as u64 > fs::CONTROL_LIMIT {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "session plan exceeds control limit",
        ));
    }
    let frozen = prepared
        .plan
        .snapshot
        .clone()
        .ok_or_else(|| Failure::new("INVALID_STATE", "session frozen input set missing"))?;
    stored.persist_images(root, &id, &prepared.plan)?;
    let mut conflicts = Vec::new();
    for (index, c) in prepared.conflicts.iter().enumerate() {
        for (kind, bytes) in [
            ("BASE", &c.base),
            ("LOCAL", &c.local),
            ("UPSTREAM", &c.upstream),
        ] {
            fs::atomic(
                root,
                &artifact(&id, &format!("{index}/{kind}")),
                bytes,
                0o600,
            )?;
        }
        let nonce = format!("AGENTFOUNDRY-RESOLUTION-{id}");
        let text = std::str::from_utf8(&c.merged)
            .map_err(|_| Failure::new("INVALID_STATE", "non-text resolution"))?;
        let mut resolved = String::new();
        for line in text.split_inclusive('\n') {
            if ["<<<<<<<", "|||||||", "=======", ">>>>>>>"]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            {
                resolved.push_str(&nonce);
                resolved.push(' ');
            }
            resolved.push_str(line);
        }
        fs::atomic(
            root,
            &artifact(&id, &format!("{index}/RESOLVED")),
            resolved.as_bytes(),
            0o600,
        )?;
        conflicts.push(Conflict {
            target: c.target.clone(),
            base: fs::digest(&c.base),
            local: fs::digest(&c.local),
            upstream: fs::digest(&c.upstream),
        });
    }
    if conflicts.is_empty() {
        return Err(Failure::new(
            "INVALID_STATE",
            "cannot stage empty conflict set",
        ));
    }
    fs::atomic(root, &artifact(&id, "plan.json"), &plan_bytes, 0o600)?;
    let session = Session {
        schema: "agentfoundry.pending-resolution/v1".into(),
        id,
        plan_digest: fs::digest(&plan_bytes),
        frozen,
        target: prepared.target.clone(),
        release: prepared.release.clone(),
        conflicts,
    };
    fs::atomic(root, ACTIVE, &serde_json::to_vec(&session)?, 0o600)?;
    status(root)
}
fn clear(root: &Path, session: &Session) -> Result<()> {
    // Remove only declared regular artifacts; never recurse through user-controlled directories.
    for name in std::iter::once("plan.json".to_string()).chain(
        (0..session.conflicts.len()).flat_map(|index| {
            ["BASE", "LOCAL", "UPSTREAM", "RESOLVED"].map(move |kind| format!("{index}/{kind}"))
        }),
    ) {
        let path = fs::safe(root, &artifact(&session.id, &name))?;
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
            return Err(Failure::new(
                "UNSAFE_PATH",
                "session cleanup contains non-file artifact",
            ));
        }
    }
    fs::remove(root, ACTIVE)?;
    fs::remove(root, &artifact(&session.id, "plan.json"))?;
    for index in 0..session.conflicts.len() {
        for kind in ["BASE", "LOCAL", "UPSTREAM", "RESOLVED"] {
            fs::remove(root, &artifact(&session.id, &format!("{index}/{kind}")))?;
        }
    }
    Ok(())
}
pub fn abort(root: &Path) -> Result<serde_json::Value> {
    let _lock = fs::Lock::acquire(root)?;
    if crate::activation::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "activation pending; recover before abort",
        ));
    }
    if crate::application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "recover transaction before aborting retained session",
        ));
    }
    let (session, plan) = load(root)?;
    let committed = plan
        .operations
        .iter()
        .find(|op| op.target == crate::STATE)
        .and_then(|op| op.after.as_ref());
    if committed.is_some() && fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?.as_ref() == committed
    {
        let binary = if cfg!(windows) {
            "scripts/bin/agentfoundry.exe"
        } else {
            "scripts/bin/agentfoundry"
        };
        let active = fs::read(root, binary, 128 * 1024 * 1024)?;
        if active
            .as_ref()
            .is_none_or(|bytes| fs::digest(bytes) != session.target.binary_digest)
        {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "committed activation requires retained candidate; recover before abort",
            ));
        }
        #[cfg(unix)]
        if crate::existing_mode(root, binary, 0)? & 0o111 == 0 {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "committed executable lacks execute permission; retain candidate",
            ));
        }
    }
    clear(root, &session)?;
    Ok(serde_json::json!({"aborted":true,"workspace_preserved":true}))
}
pub fn recover_activation(root: &Path, dry: bool) -> Result<serde_json::Value> {
    let _lock = if dry {
        None
    } else {
        Some(fs::Lock::acquire(root)?)
    };
    if crate::application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "recover transaction before activation repair",
        ));
    }
    let (session, retained) = load(root)?;
    let state = retained
        .operations
        .iter()
        .find(|op| op.target == crate::STATE)
        .and_then(|op| op.after.as_ref())
        .ok_or_else(|| Failure::new("INVALID_STATE", "retained state missing"))?;
    if fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?.as_ref() != Some(state) {
        return Ok(serde_json::json!({"recovery":"not_needed","activation":"core_not_committed"}));
    }
    crate::load_state(root)?;
    let target = if cfg!(windows) {
        "scripts/bin/agentfoundry.exe"
    } else {
        "scripts/bin/agentfoundry"
    };
    if let Some(active) = fs::read(root, target, 128 * 1024 * 1024)? {
        if fs::digest(&active) != session.target.binary_digest {
            return Err(Failure::new(
                "RECOVERY_BLOCKED",
                "active executable contains unknown bytes; preserve it before explicit repair",
            ));
        }
        #[cfg(unix)]
        if crate::existing_mode(root, target, 0)? & 0o111 == 0 {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "active executable lacks execute permission; retain evidence and restore permission explicitly",
            ));
        }
        return Ok(
            serde_json::json!({"recovery":"not_needed","activation":"binary_present","session_retained":true}),
        );
    }
    let state = state.clone();
    let candidate = retained
        .operations
        .iter()
        .find(|op| op.target == target)
        .and_then(|op| op.after.clone())
        .ok_or_else(|| Failure::new("RECOVERY_BLOCKED", "pinned executable image missing"))?;
    let mut plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: retained.root,
        root_identity: retained.root_identity,
        snapshot: None,
        payload_digest: session.target.payload_digest.clone(),
        choices: Default::default(),
        operations: vec![
            crate::application::Operation {
                target: target.into(),
                before: None,
                after: Some(candidate),
                mode: 0o755,
            },
            crate::application::Operation {
                target: crate::STATE.into(),
                before: Some(state.clone()),
                after: Some(state.clone()),
                mode: crate::existing_mode(root, crate::STATE, 0o644)?,
            },
        ],
    };
    plan.freeze(root)?;
    crate::application::verify(&plan, root)?;
    if dry {
        return Ok(
            serde_json::json!({"recovery":"activation_preview","target":target,"binary_digest":session.target.binary_digest,"session_retained":true}),
        );
    }
    crate::application::apply_locked(root, &plan)?;
    let mut command = std::process::Command::new(fs::safe(root, target)?);
    command.args(["version", "--json"]);
    let result = (|| -> Result<serde_json::Value> {
        let output =
            crate::process::capture(&mut command, 64 * 1024, std::time::Duration::from_secs(10))?;
        if !output.status.success() {
            return Err(Failure::new(
                "RECOVERY_BLOCKED",
                "repaired executable identity query failed",
            ));
        }
        Ok(serde_json::from_slice(&output.stdout)?)
    })()
    .map_err(|mut error| {
        error.mutation = "applied".into();
        error
    })?;
    if result["data"]["version"] != session.target.version
        || result["data"]["source"] != session.target.source
        || result["data"]["payload_digest"] != session.target.payload_digest
    {
        let mut error = Failure::new(
            "RECOVERY_BLOCKED",
            "repaired executable build identity mismatch",
        );
        error.mutation = "applied".into();
        return Err(error);
    }
    Ok(
        serde_json::json!({"recovery":"activation_repaired","binary_digest":session.target.binary_digest,"session_retained":true}),
    )
}
pub fn resume(root: &Path, dry: bool) -> Result<serde_json::Value> {
    let _lock = if dry {
        None
    } else {
        Some(fs::Lock::acquire(root)?)
    };
    if crate::application::pending(root)? {
        if dry {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "recover transaction before continue preview",
            ));
        }
        crate::application::recover(root, false)?;
    }
    if crate::activation::pending(root)? {
        crate::activation::recover_locked(root, dry)?;
    }
    let (session, mut plan) = load(root)?;
    let committed = plan
        .operations
        .iter()
        .find(|op| op.target == crate::STATE)
        .and_then(|op| op.after.as_ref());
    if committed.is_some() && fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?.as_ref() == committed
    {
        let binary_path = if cfg!(windows) {
            "scripts/bin/agentfoundry.exe"
        } else {
            "scripts/bin/agentfoundry"
        };
        let active = fs::read(root, binary_path, 128 * 1024 * 1024)?;
        if active
            .as_ref()
            .is_none_or(|bytes| fs::digest(bytes) != session.target.binary_digest)
        {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "committed core has no verified active executable; preserve session and restore the pinned candidate",
            ));
        }
        #[cfg(unix)]
        if crate::existing_mode(root, binary_path, 0)? & 0o111 == 0 {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "committed executable lacks execute permission; preserve session",
            ));
        }
        if !dry {
            crate::load_state(root)?;
            clear(root, &session)?;
        }
        return Ok(
            serde_json::json!({"continued":!dry,"already_committed":true,"session_retained":dry}),
        );
    }
    crate::application::verify(&plan, root)?;
    let next_state = crate::models::InstallationState::parse(
        plan.operations
            .iter()
            .find(|op| op.target == crate::STATE)
            .and_then(|op| op.after.as_deref())
            .ok_or_else(|| Failure::new("INVALID_STATE", "state commit missing"))?,
    )?;
    for (index, c) in session.conflicts.iter().enumerate() {
        let bytes = fs::read(
            root,
            &artifact(&session.id, &format!("{index}/RESOLVED")),
            8 * 1024 * 1024,
        )?
        .ok_or_else(|| Failure::new("INVALID_STATE", "RESOLVED missing"))?;
        let nonce = format!("AGENTFOUNDRY-RESOLUTION-{}", session.id);
        if bytes
            .windows(nonce.len())
            .any(|window| window == nonce.as_bytes())
        {
            return Err(Failure::new(
                "CONFLICT_PENDING",
                "RESOLVED still contains session conflict markers",
            ));
        }
        std::str::from_utf8(&bytes)
            .map_err(|_| Failure::new("INVALID_STATE", "RESOLVED must be UTF-8 text"))?;
        let op = plan
            .operations
            .iter_mut()
            .find(|op| op.target == c.target)
            .ok_or_else(|| Failure::new("INVALID_STATE", "conflict target not planned"))?;
        let state = next_state
            .entries
            .iter()
            .find(|entry| {
                entry.target == c.target
                    && entry.ownership == crate::models::Ownership::ManagedBlock
            })
            .map(|entry| {
                if entry.target == "CLAUDE.md" {
                    "AGENTFOUNDRY:CLAUDE"
                } else {
                    "AGENTFOUNDRY"
                }
            });
        op.after = Some(if let Some(name) = state {
            crate::domain::patch_block(
                op.before
                    .as_deref()
                    .ok_or_else(|| Failure::new("INVALID_STATE", "block container missing"))?,
                &bytes,
                name,
            )
            .map_err(|e| Failure::new("INVALID_STATE", e))?
        } else {
            bytes
        });
    }
    let plan_id = plan.semantic_id()?;
    if !dry {
        crate::activation::apply_locked(
            root,
            &PreparedUpdate {
                plan,
                conflicts: Vec::new(),
                target: session.target.clone(),
                release: session.release.clone(),
            },
        )?;
        clear(root, &session)?;
    }
    Ok(serde_json::json!({"plan_id":plan_id,"continued":!dry,"session_retained":dry}))
}
