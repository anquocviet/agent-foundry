use crate::{
    Failure, Result,
    application::{Operation, Plan, PlanChoices},
    infrastructure as fs,
    merge::Merge,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PreparedUpdate {
    pub plan: Plan,
    pub conflicts: Vec<TextConflict>,
    pub target: CandidateIdentity,
    pub release: Option<crate::release::ReleaseMetadata>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextConflict {
    pub target: String,
    pub base: Vec<u8>,
    pub local: Vec<u8>,
    pub upstream: Vec<u8>,
    pub merged: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateIdentity {
    pub version: String,
    pub source: String,
    pub payload_digest: String,
    pub binary_digest: String,
}

pub fn prepare(root: &Path) -> Result<Plan> {
    if crate::application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "recover interrupted transaction before update",
        ));
    }
    let state_preimage = fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "repository is not installed"))?;
    let state = crate::load_state(root)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "repository is not installed"))?;
    if fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?.as_ref() != Some(&state_preimage) {
        return Err(Failure::new(
            "WORKSPACE_DRIFT",
            "state changed during planning",
        ));
    }
    let config = fs::read(root, ".agentfoundry/config.json", fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "installed config missing"))?;
    crate::config::Config::parse(&config)?;
    // Candidate acquisition/activation must verify a complete release before crossing identity.
    if state.payload_digest != crate::PAYLOAD_DIGEST
        || state.source != env!("AF_SOURCE_ID")
        || state.version != env!("CARGO_PKG_VERSION")
    {
        return Err(Failure::new(
            "MISSING_CAPABILITY",
            "offline embedded update requires exact installed build identity; verified release candidate handoff is not implemented",
        ));
    }
    let binary_path = crate::binary_path();
    let mut paths = std::collections::BTreeSet::from([
        crate::STATE.to_string(),
        ".agentfoundry/config.json".into(),
        binary_path.into(),
    ]);
    for entry in &state.entries {
        paths.insert(entry.target.clone());
        paths.insert(format!(".agentfoundry/state/objects/{}", entry.sha256));
    }
    let paths = paths.iter().map(String::as_str).collect::<Vec<_>>();
    let frozen = fs::snapshot(root, state.generation, &paths)?;
    let mut operations = Vec::new();
    for entry in &state.entries {
        let base = fs::read(
            root,
            &format!(".agentfoundry/state/objects/{}", entry.sha256),
            8 * 1024 * 1024,
        )?
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "baseline missing"))?;
        let local = fs::read(root, &entry.target, 8 * 1024 * 1024)?;
        let incoming = crate::PAYLOAD
            .iter()
            .find(|(source, _)| *source == entry.source)
            .map(|(_, bytes)| *bytes)
            .ok_or_else(|| {
                Failure::new(
                    "INTEGRITY_FAILURE",
                    "installed logical entry absent from embedded distribution",
                )
            })?;
        if fs::digest(incoming) != entry.sha256 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "installed entry identity differs from embedded payload",
            ));
        }
        let after = if local.is_none() {
            return Err(Failure::new(
                "CONFLICT_PENDING",
                format!(
                    "missing-managed structural conflict: {}; restore deliberately or replan migration",
                    entry.target
                ),
            ));
        } else if entry.ownership == crate::models::Ownership::ManagedBlock {
            let name = if entry.target == "CLAUDE.md" {
                "AGENTFOUNDRY:CLAUDE"
            } else {
                "AGENTFOUNDRY"
            };
            if crate::domain::block_content(local.as_deref().unwrap_or_default(), name)
                .map_err(|e| Failure::new("INVALID_STATE", e))?
                .is_none()
            {
                return Err(Failure::new(
                    "CONFLICT_PENDING",
                    format!(
                        "missing-managed-block structural conflict: {}; restore deliberately or refresh the agent shim",
                        entry.target
                    ),
                ));
            }
            local.clone()
        } else {
            match crate::merge::three_way(Some(&base), local.as_deref(), Some(incoming))? {
                Merge::Clean(bytes) => bytes,
                Merge::Conflict(_) => {
                    return Err(Failure::new(
                        "CONFLICT_PENDING",
                        format!("text conflict: {}", entry.target),
                    ));
                }
                Merge::Structural(kind) => {
                    return Err(Failure::new(
                        "CONFLICT_PENDING",
                        format!("{kind}: {}", entry.target),
                    ));
                }
            }
        };
        operations.push(Operation {
            target: entry.target.clone(),
            mode: crate::existing_mode(root, &entry.target, entry.mode)?,
            before: local,
            after,
        });
        let object = format!(".agentfoundry/state/objects/{}", entry.sha256);
        if !operations.iter().any(|o| o.target == object) {
            operations.push(Operation {
                target: object,
                mode: 0o644,
                before: Some(base.clone()),
                after: Some(base),
            });
        }
    }
    operations.push(Operation {
        target: ".agentfoundry/config.json".into(),
        before: Some(config.clone()),
        after: Some(config),
        mode: crate::existing_mode(root, ".agentfoundry/config.json", 0o644)?,
    });
    let state_bytes = state_preimage;
    operations.push(Operation {
        target: crate::STATE.into(),
        before: Some(state_bytes.clone()),
        after: Some(state_bytes),
        mode: crate::existing_mode(root, crate::STATE, 0o644)?,
    });
    let binary = fs::read(root, binary_path, 128 * 1024 * 1024)?.ok_or_else(|| {
        Failure::new(
            "RECOVERY_REQUIRED",
            "repository executable missing; verified activation repair required",
        )
    })?;
    if fs::digest(&binary) != state.binary_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "repository executable does not match installed identity",
        ));
    }
    operations.push(Operation {
        target: binary_path.into(),
        before: Some(binary.clone()),
        after: Some(binary),
        mode: crate::existing_mode(root, binary_path, 0o755)?,
    });
    fs::verify_snapshot(root, &frozen)?;
    let plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: root.to_string_lossy().into(),
        root_identity: fs::root_identity(root)?,
        snapshot: Some(frozen),
        payload_digest: crate::PAYLOAD_DIGEST.into(),
        choices: PlanChoices::default(),
        operations,
    };
    crate::application::verify(&plan, root)?;
    Ok(plan)
}

pub fn prepare_target(
    root: &Path,
    manifest: &[u8],
    payload: &[(String, Vec<u8>)],
    binary: Vec<u8>,
    identity: CandidateIdentity,
) -> Result<PreparedUpdate> {
    if crate::application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "recover interrupted transaction before update",
        ));
    }
    let state_preimage = fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "repository is not installed"))?;
    let state = crate::load_state(root)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "repository is not installed"))?;
    if fs::read(root, crate::STATE, fs::CONTROL_LIMIT)?.as_ref() != Some(&state_preimage) {
        return Err(Failure::new(
            "WORKSPACE_DRIFT",
            "state changed during planning",
        ));
    }
    let config_bytes = fs::read(root, ".agentfoundry/config.json", fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "installed config missing"))?;
    let config = crate::config::Config::parse(&config_bytes)?;

    if binary.len() > 128 * 1024 * 1024 {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "candidate binary exceeds 128MiB limit",
        ));
    }
    if fs::digest(&binary) != identity.binary_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "candidate binary digest does not match identity",
        ));
    }

    let dist = crate::models::Distribution::parse(manifest)?;
    let manifest_val: serde_json::Value = serde_json::from_slice(manifest)?;
    let entries_arr = manifest_val["entries"]
        .as_array()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "manifest entries missing"))?;
    let canonical_digest = crate::payload_identity::canonical_digest(entries_arr)
        .map_err(|e| Failure::new("INTEGRITY_FAILURE", e.to_string()))?;
    if canonical_digest != identity.payload_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "manifest canonical payload digest does not match identity",
        ));
    }

    let mut target_entries = Vec::new();
    for e in &dist.entries {
        if e.addon.is_some()
            && !config.addons.iter().any(|a| a == "engineering-wisdom")
            && !state.entries.iter().any(|i| i.logical_id == e.logical_id)
        {
            continue;
        }
        if e.adapter.is_some()
            && !config.adapters.iter().any(|a| a == "claude")
            && !state.entries.iter().any(|i| i.logical_id == e.logical_id)
        {
            continue;
        }
        target_entries.push(e.clone());
    }

    for e in &target_entries {
        let bytes = payload
            .iter()
            .find(|(s, _)| s == &e.source)
            .map(|(_, b)| b.as_slice())
            .ok_or_else(|| {
                Failure::new(
                    "INTEGRITY_FAILURE",
                    format!("target distribution source missing: {}", e.source),
                )
            })?;
        if fs::digest(bytes) != e.sha256 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                format!("target distribution entry digest mismatch: {}", e.source),
            ));
        }
    }

    for entry in &target_entries {
        if state
            .entries
            .iter()
            .any(|old| old.logical_id == entry.logical_id && old.target != entry.target)
        {
            return Err(Failure::new(
                "CONFLICT_PENDING",
                format!("undeclared path migration: {}", entry.logical_id),
            ));
        }
    }
    let mut installed_map: BTreeMap<String, crate::models::Entry> = BTreeMap::new();
    for entry in &state.entries {
        installed_map.insert(entry.target.clone(), entry.clone());
    }

    let mut target_map: BTreeMap<String, crate::models::Entry> = BTreeMap::new();
    for entry in &target_entries {
        target_map.insert(entry.target.clone(), entry.clone());
    }

    let mut all_targets: BTreeSet<String> = BTreeSet::new();
    for t in installed_map.keys() {
        all_targets.insert(t.clone());
    }
    for t in target_map.keys() {
        all_targets.insert(t.clone());
    }

    let mut operations = Vec::new();
    let mut conflicts = Vec::new();
    let mut new_tracked_entries = Vec::new();

    for target in &all_targets {
        let old_entry_opt = installed_map.get(target);
        let new_entry_opt = target_map.get(target);
        let local = fs::read(root, target, 8 * 1024 * 1024)?;

        match (old_entry_opt, new_entry_opt) {
            (Some(old_entry), Some(new_entry)) => {
                let base = fs::read(
                    root,
                    &format!(".agentfoundry/state/objects/{}", old_entry.sha256),
                    8 * 1024 * 1024,
                )?
                .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "baseline missing"))?;

                let Some(local_bytes) = local.clone() else {
                    return Err(Failure::new(
                        "CONFLICT_PENDING",
                        format!(
                            "missing-managed structural conflict: {}; restore deliberately or replan migration",
                            target
                        ),
                    ));
                };

                let incoming = payload
                    .iter()
                    .find(|(s, _)| s == &new_entry.source)
                    .map(|(_, b)| b.clone())
                    .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "incoming payload missing"))?;

                if old_entry.logical_id != new_entry.logical_id
                    || old_entry.ownership != new_entry.ownership
                {
                    return Err(Failure::new(
                        "CONFLICT_PENDING",
                        format!("structural ownership or identity change: {target}"),
                    ));
                }
                if new_entry.ownership == crate::models::Ownership::ManagedBlock {
                    let name = if target == "CLAUDE.md" {
                        "AGENTFOUNDRY:CLAUDE"
                    } else {
                        "AGENTFOUNDRY"
                    };
                    if crate::domain::block_content(&local_bytes, name)
                        .map_err(|e| Failure::new("INVALID_STATE", e))?
                        .is_none()
                    {
                        return Err(Failure::new(
                            "CONFLICT_PENDING",
                            format!(
                                "missing-managed-block structural conflict: {}; restore deliberately or refresh the agent shim",
                                target
                            ),
                        ));
                    }
                    let local_block = crate::domain::block_content(&local_bytes, name)
                        .map_err(|e| Failure::new("INVALID_STATE", e))?
                        .unwrap();
                    let mut local_block = local_block.to_vec();
                    if base.ends_with(b"\n") {
                        local_block.push(b'\n');
                    }
                    let merged = match crate::merge::three_way(
                        Some(&base),
                        Some(&local_block),
                        Some(&incoming),
                    )? {
                        Merge::Clean(Some(bytes)) => bytes,
                        Merge::Conflict(merged) => {
                            conflicts.push(TextConflict {
                                target: target.clone(),
                                base: base.clone(),
                                local: local_block,
                                upstream: incoming.clone(),
                                merged,
                            });
                            incoming.clone()
                        }
                        _ => {
                            return Err(Failure::new(
                                "CONFLICT_PENDING",
                                format!("structural managed block conflict: {target}"),
                            ));
                        }
                    };
                    let patched = crate::domain::patch_block(&local_bytes, &merged, name)
                        .map_err(|e| Failure::new("INVALID_STATE", e))?;
                    operations.push(Operation {
                        target: target.clone(),
                        mode: crate::existing_mode(root, target, new_entry.mode)?,
                        before: Some(local_bytes),
                        after: Some(patched),
                    });
                    new_tracked_entries.push(new_entry.clone());
                } else {
                    match crate::merge::three_way(Some(&base), Some(&local_bytes), Some(&incoming))?
                    {
                        Merge::Clean(bytes) => {
                            operations.push(Operation {
                                target: target.clone(),
                                mode: crate::existing_mode(root, target, new_entry.mode)?,
                                before: Some(local_bytes),
                                after: bytes,
                            });
                            new_tracked_entries.push(new_entry.clone());
                        }
                        Merge::Conflict(merged) => {
                            conflicts.push(TextConflict {
                                target: target.clone(),
                                base: base.clone(),
                                local: local_bytes.clone(),
                                upstream: incoming.clone(),
                                merged: merged.clone(),
                            });
                            operations.push(Operation {
                                target: target.clone(),
                                mode: crate::existing_mode(root, target, new_entry.mode)?,
                                before: Some(local_bytes),
                                after: Some(merged),
                            });
                            new_tracked_entries.push(new_entry.clone());
                        }
                        Merge::Structural(kind) => {
                            return Err(Failure::new(
                                "CONFLICT_PENDING",
                                format!("{kind}: {target}"),
                            ));
                        }
                    }
                }
            }

            (Some(old_entry), None) => {
                let base = fs::read(
                    root,
                    &format!(".agentfoundry/state/objects/{}", old_entry.sha256),
                    8 * 1024 * 1024,
                )?
                .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "baseline missing"))?;

                match &local {
                    None => operations.push(Operation {
                        target: target.clone(),
                        before: None,
                        after: None,
                        mode: old_entry.mode,
                    }),
                    Some(local_bytes) if local_bytes == &base => {
                        operations.push(Operation {
                            target: target.clone(),
                            mode: crate::existing_mode(root, target, old_entry.mode)?,
                            before: local,
                            after: None,
                        });
                    }
                    Some(_) => {
                        return Err(Failure::new(
                            "CONFLICT_PENDING",
                            format!("modified-removed structural conflict: {target}"),
                        ));
                    }
                }
            }

            (None, Some(new_entry)) => {
                let incoming = payload
                    .iter()
                    .find(|(s, _)| s == &new_entry.source)
                    .map(|(_, b)| b.clone())
                    .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "incoming payload missing"))?;

                match &local {
                    None => {
                        operations.push(Operation {
                            target: target.clone(),
                            mode: new_entry.mode,
                            before: None,
                            after: Some(incoming),
                        });
                        new_tracked_entries.push(new_entry.clone());
                    }
                    Some(local_bytes) if local_bytes == &incoming => {
                        operations.push(Operation {
                            target: target.clone(),
                            mode: crate::existing_mode(root, target, new_entry.mode)?,
                            before: local,
                            after: Some(incoming),
                        });
                        new_tracked_entries.push(new_entry.clone());
                    }
                    Some(_) => {
                        return Err(Failure::new(
                            "CONFLICT_PENDING",
                            format!("unmanaged-collision structural conflict: {target}"),
                        ));
                    }
                }
            }

            (None, None) => unreachable!(),
        }
    }
    for entry in &state.entries {
        let target = format!(".agentfoundry/state/objects/{}", entry.sha256);
        if !operations.iter().any(|op| op.target == target) {
            let bytes = fs::read(root, &target, 8 * 1024 * 1024)?
                .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "installed baseline missing"))?;
            if fs::digest(&bytes) != entry.sha256 {
                return Err(Failure::new(
                    "INTEGRITY_FAILURE",
                    "installed baseline changed during planning",
                ));
            }
            operations.push(Operation {
                mode: crate::existing_mode(root, &target, 0o644)?,
                target,
                before: Some(bytes.clone()),
                after: Some(bytes),
            });
        }
    }

    let mut seen_objects = BTreeSet::new();
    for entry in &new_tracked_entries {
        if operations
            .iter()
            .any(|op| op.target == format!(".agentfoundry/state/objects/{}", entry.sha256))
        {
            continue;
        }
        let object_path = format!(".agentfoundry/state/objects/{}", entry.sha256);
        if !seen_objects.insert(object_path.clone()) {
            continue;
        }
        let incoming = payload
            .iter()
            .find(|(s, _)| s == &entry.source)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "payload missing"))?;
        let existing_obj = fs::read(root, &object_path, 8 * 1024 * 1024)?;
        let before = existing_obj.clone();
        let after = existing_obj.or(Some(incoming));
        operations.push(Operation {
            target: object_path,
            mode: 0o644,
            before,
            after,
        });
    }

    operations.push(Operation {
        target: ".agentfoundry/config.json".into(),
        before: Some(config_bytes.clone()),
        after: Some(config_bytes),
        mode: crate::existing_mode(root, ".agentfoundry/config.json", 0o644)?,
    });

    let binary_path = crate::binary_path();
    let local_binary = fs::read(root, binary_path, 128 * 1024 * 1024)?.ok_or_else(|| {
        Failure::new(
            "RECOVERY_REQUIRED",
            "repository executable missing; verified activation repair required",
        )
    })?;
    if fs::digest(&local_binary) != state.binary_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "repository executable does not match installed identity",
        ));
    }
    let binary_mode = crate::existing_mode(root, binary_path, 0o755)?;
    operations.push(Operation {
        target: binary_path.into(),
        before: Some(local_binary),
        after: Some(binary),
        mode: binary_mode,
    });

    let new_state = crate::models::InstallationState {
        schema: "agentfoundry.installation/v1".into(),
        generation: state
            .generation
            .checked_add(1)
            .ok_or_else(|| Failure::new("INVALID_STATE", "generation exhausted"))?,
        version: identity.version.clone(),
        source: identity.source.clone(),
        payload_digest: identity.payload_digest.clone(),
        entries: new_tracked_entries,
        binary_digest: identity.binary_digest.clone(),
    };
    let new_state_bytes = serde_json::to_vec_pretty(&new_state)?;
    let state_mode = crate::existing_mode(root, crate::STATE, 0o644)?;
    operations.push(Operation {
        target: crate::STATE.into(),
        before: Some(state_preimage),
        after: Some(new_state_bytes),
        mode: state_mode,
    });

    let mut plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: root.to_string_lossy().into(),
        root_identity: fs::root_identity(root)?,
        snapshot: None,
        payload_digest: identity.payload_digest.clone(),
        choices: PlanChoices::default(),
        operations,
    };
    plan.freeze(root)?;
    crate::application::verify(&plan, root)?;

    Ok(PreparedUpdate {
        plan,
        conflicts,
        target: identity,
        release: None,
    })
}
