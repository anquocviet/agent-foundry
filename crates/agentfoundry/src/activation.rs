use crate::{
    Failure, Result,
    application::{Operation, Plan},
    infrastructure as fs,
    update::PreparedUpdate,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
const RECORD: &str = ".agentfoundry/runtime/activation.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: String,
    id: String,
    root_identity: String,
    before_state: String,
    after_state: String,
    before_binary: String,
    target: crate::update::CandidateIdentity,
    release: Option<crate::release::ReleaseMetadata>,
    mode: u32,
}
fn binary_path() -> &'static str {
    if cfg!(windows) {
        "scripts/bin/agentfoundry.exe"
    } else {
        "scripts/bin/agentfoundry"
    }
}
pub fn pending(root: &Path) -> Result<bool> {
    Ok(fs::read(root, RECORD, fs::CONTROL_LIMIT)?.is_some())
}
fn image(id: &str, hash: &str) -> String {
    format!(".agentfoundry/backups/{id}/objects/{hash}")
}
fn load(root: &Path) -> Result<(Record, Vec<u8>)> {
    let bytes = fs::read(root, RECORD, fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "activation record missing"))?;
    let r: Record = serde_json::from_slice(&bytes)?;
    if r.schema != "agentfoundry.activation/v1"
        || r.id.is_empty()
        || r.id.len() > 128
        || !r.id.bytes().all(|b| b.is_ascii_hexdigit())
        || Some(r.root_identity.clone()) != fs::root_identity(root)?
        || r.mode & !0o777 != 0
    {
        return Err(Failure::new(
            "RECOVERY_BLOCKED",
            "invalid activation record or root identity",
        ));
    }
    for hash in [
        &r.before_state,
        &r.after_state,
        &r.before_binary,
        &r.target.binary_digest,
        &r.target.payload_digest,
    ] {
        crate::domain::validate_digest(hash).map_err(|e| Failure::new("RECOVERY_BLOCKED", e))?;
    }
    let candidate = fs::read(
        root,
        &image(&r.id, &r.target.binary_digest),
        128 * 1024 * 1024,
    )?
    .ok_or_else(|| Failure::new("RECOVERY_BLOCKED", "retained candidate missing"))?;
    if fs::digest(&candidate) != r.target.binary_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "retained candidate corrupted",
        ));
    }
    if let Some(metadata) = &r.release {
        metadata
            .verify_candidate(crate::candidate::current_platform()?, &candidate)
            .map_err(|e| Failure::new("INTEGRITY_FAILURE", e))?;
        if metadata.version != r.target.version
            || metadata.source_commit != r.target.source
            || metadata.payload_digest != r.target.payload_digest
        {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "retained release identity mismatch",
            ));
        }
    } else if !r.target.source.starts_with("development-sha256:") {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "release evidence missing",
        ));
    }
    Ok((r, candidate))
}
pub fn apply(root: &Path, prepared: &PreparedUpdate) -> Result<()> {
    let _lock = fs::Lock::acquire(root)?;
    apply_locked(root, prepared)
}

pub(crate) fn apply_locked(root: &Path, prepared: &PreparedUpdate) -> Result<()> {
    if pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "activation pending; run recover",
        ));
    }
    crate::application::recover(root, false)?;
    crate::application::verify(&prepared.plan, root)?;
    let mut core = prepared.plan.clone();
    let binary = core
        .operations
        .iter_mut()
        .find(|op| op.target == binary_path())
        .ok_or_else(|| Failure::new("INVALID_STATE", "candidate operation missing"))?;
    let before_binary = fs::digest(
        binary
            .before
            .as_ref()
            .ok_or_else(|| Failure::new("INVALID_STATE", "installed executable missing"))?,
    );
    let candidate = binary
        .after
        .take()
        .ok_or_else(|| Failure::new("INVALID_STATE", "candidate missing"))?;
    if fs::digest(&candidate) != prepared.target.binary_digest {
        return Err(Failure::new("INTEGRITY_FAILURE", "candidate mismatch"));
    }
    binary.after = binary.before.clone();
    let mode = binary.mode;
    let state = core
        .operations
        .iter()
        .find(|op| op.target == crate::STATE)
        .ok_or_else(|| Failure::new("INVALID_STATE", "state operation missing"))?;
    let before_state = fs::digest(
        state
            .before
            .as_ref()
            .ok_or_else(|| Failure::new("INVALID_STATE", "prior state missing"))?,
    );
    let after_state = fs::digest(
        state
            .after
            .as_ref()
            .ok_or_else(|| Failure::new("INVALID_STATE", "target state missing"))?,
    );
    let id = crate::unique_id();
    fs::atomic(
        root,
        &image(&id, &prepared.target.binary_digest),
        &candidate,
        0o600,
    )?;
    let record = Record {
        schema: "agentfoundry.activation/v1".into(),
        id,
        root_identity: core
            .root_identity
            .clone()
            .ok_or_else(|| Failure::new("INVALID_STATE", "root identity missing"))?,
        before_state,
        after_state,
        before_binary,
        target: prepared.target.clone(),
        release: prepared.release.clone(),
        mode,
    };
    fs::atomic(root, RECORD, &serde_json::to_vec(&record)?, 0o600)?;
    crate::application::apply_locked(root, &core)?;
    recover_locked(root, false).map_err(|mut error| {
        if error.mutation == "none" {
            error.mutation = "applied".into();
        }
        error
    })?;
    Ok(())
}
pub fn recover(root: &Path, dry: bool) -> Result<serde_json::Value> {
    let _lock = if dry {
        None
    } else {
        Some(fs::Lock::acquire(root)?)
    };
    recover_locked(root, dry)
}
pub(crate) fn recover_locked(root: &Path, dry: bool) -> Result<serde_json::Value> {
    if crate::application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "recover transaction before activation",
        ));
    }
    let (record, candidate) = load(root)?;
    let state = fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("RECOVERY_BLOCKED", "installed state missing"))?;
    let hash = fs::digest(&state);
    if hash == record.before_state {
        if !dry {
            fs::remove(root, RECORD)?;
        }
        return Ok(serde_json::json!({"recovery":"activation_cancelled","dry_run":dry}));
    }
    if hash != record.after_state {
        return Err(Failure::new(
            "RECOVERY_BLOCKED",
            "installed state differs from activation witness",
        ));
    }
    let installed = crate::models::InstallationState::parse(&state)?;
    if installed.binary_digest != record.target.binary_digest
        || installed.version != record.target.version
        || installed.source != record.target.source
        || installed.payload_digest != record.target.payload_digest
    {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "committed identity differs from activation",
        ));
    }
    let active = fs::read(root, binary_path(), 128 * 1024 * 1024)?;
    if let Some(bytes) = &active {
        let digest = fs::digest(bytes);
        if digest != record.before_binary && digest != record.target.binary_digest {
            return Err(Failure::new(
                "RECOVERY_BLOCKED",
                "unknown executable bytes retained",
            ));
        }
    }
    #[cfg(unix)]
    if active.is_some() && crate::existing_mode(root, binary_path(), 0)? != record.mode {
        return Err(Failure::new(
            "RECOVERY_BLOCKED",
            "executable permission changed; retain activation evidence",
        ));
    }
    let already = active
        .as_ref()
        .is_some_and(|b| fs::digest(b) == record.target.binary_digest);
    let mut applied = false;
    if !already {
        let mut plan = Plan {
            schema: "agentfoundry.plan/v1".into(),
            root: root.to_string_lossy().into(),
            root_identity: Some(record.root_identity.clone()),
            snapshot: None,
            payload_digest: record.target.payload_digest.clone(),
            choices: Default::default(),
            operations: vec![
                Operation {
                    target: binary_path().into(),
                    before: active,
                    after: Some(candidate),
                    mode: record.mode,
                },
                Operation {
                    target: crate::STATE.into(),
                    before: Some(state.clone()),
                    after: Some(state),
                    mode: crate::existing_mode(root, crate::STATE, 0o644)?,
                },
            ],
        };
        plan.freeze(root)?;
        crate::application::verify(&plan, root)?;
        if dry {
            return Ok(serde_json::json!({"recovery":"activation_preview"}));
        }
        crate::application::apply_locked(root, &plan)?;
        applied = true;
    }
    if dry {
        return Ok(serde_json::json!({"recovery":"activation_verified"}));
    }
    (|| -> Result<serde_json::Value> {
        let mut cmd = std::process::Command::new(fs::safe(root, binary_path())?);
        cmd.args(["version", "--json"]);
        let output =
            crate::process::capture(&mut cmd, 64 * 1024, std::time::Duration::from_secs(10))?;
        let data: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        if !output.status.success()
            || data["data"]["version"] != record.target.version
            || data["data"]["source"] != record.target.source
            || data["data"]["payload_digest"] != record.target.payload_digest
        {
            return Err(Failure::new(
                "RECOVERY_BLOCKED",
                "activated executable identity mismatch",
            ));
        }
        fs::remove(root, RECORD)?;
        Ok(serde_json::json!({"recovery":"activation_repaired"}))
    })()
    .map_err(|mut error| {
        if applied && error.mutation == "none" {
            error.mutation = "applied".into();
        }
        error
    })
}
