use crate::models::{Journal, JournalPhase};
use crate::{Failure, Result, infrastructure as fs};
use serde::{Deserialize, Serialize};
use std::path::Path;
const JOURNAL: &str = ".agentfoundry/runtime/transaction.json";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub target: String,
    #[serde(with = "hex_bytes")]
    pub before: Option<Vec<u8>>,
    #[serde(with = "hex_bytes")]
    pub after: Option<Vec<u8>>,
    pub mode: u32,
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(
        bytes: &Option<Vec<u8>>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match bytes {
            None => serializer.serialize_none(),
            Some(bytes) => {
                use std::fmt::Write;
                let mut hex = String::with_capacity(bytes.len() * 2);
                for byte in bytes {
                    write!(&mut hex, "{byte:02x}").map_err(serde::ser::Error::custom)?;
                }
                serializer.serialize_some(&hex)
            }
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Option<Vec<u8>>, D::Error> {
        let value = Option::<String>::deserialize(deserializer)?;
        value
            .map(|hex| {
                if hex.len() % 2 != 0
                    || hex.len() > 256 * 1024 * 1024
                    || !hex.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err(serde::de::Error::custom("invalid byte image"));
                }
                hex.as_bytes()
                    .chunks_exact(2)
                    .map(|pair| {
                        let digit = |b: u8| {
                            if b.is_ascii_digit() {
                                b - b'0'
                            } else {
                                b.to_ascii_lowercase() - b'a' + 10
                            }
                        };
                        Ok((digit(pair[0]) << 4) | digit(pair[1]))
                    })
                    .collect()
            })
            .transpose()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub schema: String,
    pub root: String,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub root_identity: Option<String>,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub snapshot: Option<crate::models::WorkspaceSnapshot>,
    pub payload_digest: String,
    pub operations: Vec<Operation>,
    pub choices: PlanChoices,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanChoices {
    pub merge: bool,
    pub replace: bool,
    pub refresh: bool,
    pub wisdom: bool,
    pub claude: bool,
}
impl Plan {
    pub fn freeze(&mut self, root: &Path) -> Result<()> {
        self.snapshot = if self.root_identity.is_some() {
            let generation = self
                .operations
                .iter()
                .find(|op| op.target == crate::STATE)
                .and_then(|op| op.before.as_deref())
                .map(crate::models::InstallationState::parse)
                .transpose()?
                .map_or(0, |state| state.generation);
            let paths = self
                .operations
                .iter()
                .map(|op| op.target.as_str())
                .collect::<Vec<_>>();
            Some(fs::snapshot(root, generation, &paths)?)
        } else {
            None
        };
        Ok(())
    }
    pub fn semantic_id(&self) -> Result<String> {
        let operations = self
            .operations
            .iter()
            .map(|operation| {
                (
                    &operation.target,
                    operation.before.as_deref().map(fs::digest),
                    operation.after.as_deref().map(fs::digest),
                    operation.mode,
                )
            })
            .collect::<Vec<_>>();
        let canonical = serde_json::to_vec(&(
            "agentfoundry.plan-canonical/v1",
            &self.schema,
            &self.root,
            &self.root_identity,
            &self.snapshot,
            &self.payload_digest,
            &self.choices,
            operations,
        ))?;
        Ok(fs::digest(&canonical))
    }
}
fn validate(plan: &Plan, root: &Path) -> Result<()> {
    if plan.schema != "agentfoundry.plan/v1"
        || plan.root != root.to_string_lossy()
        || plan.operations.len() > 10000
        || (plan.choices.merge && plan.choices.replace)
    {
        return Err(Failure::new(
            "INVALID_STATE",
            "invalid plan schema, root binding or resource limit",
        ));
    }
    crate::domain::validate_digest(&plan.payload_digest)
        .map_err(|e| Failure::new("INVALID_STATE", e))?;
    if plan
        .operations
        .iter()
        .position(|op| op.target == ".agentfoundry/state/manifest.json" && op.before != op.after)
        .is_some_and(|index| {
            plan.operations[index + 1..]
                .iter()
                .any(|op| op.before != op.after)
        })
    {
        return Err(Failure::new(
            "INVALID_STATE",
            "changed operations cannot follow manifest commit",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for op in &plan.operations {
        crate::domain::validate_transaction_target(&op.target)
            .map_err(|error| Failure::new("INVALID_STATE", error))?;
        fs::safe(root, &op.target)?;
        let target = op.target.to_ascii_lowercase();
        if target == ".agentfoundry" || target.starts_with(".agentfoundry/") {
            let object = op.target.strip_prefix(".agentfoundry/state/objects/");
            if object.is_some_and(|hash| {
                op.after
                    .as_ref()
                    .is_none_or(|bytes| fs::digest(bytes) != hash)
                    || op
                        .before
                        .as_ref()
                        .is_some_and(|bytes| fs::digest(bytes) != hash)
            }) {
                return Err(Failure::new(
                    "INTEGRITY_FAILURE",
                    "immutable baseline object content mismatch",
                ));
            }
        }
        if !seen.insert(op.target.to_lowercase())
            || op.target.starts_with(".agentfoundry/runtime/")
            || op.target.starts_with(".agentfoundry/backups/")
            || op.mode & !0o777 != 0
        {
            return Err(Failure::new(
                "INVALID_STATE",
                format!(
                    "duplicate, reserved target or unsupported mode: {}",
                    op.target
                ),
            ));
        }
    }
    for target in &seen {
        for (index, _) in target.match_indices('/') {
            if seen.contains(&target[..index]) {
                return Err(Failure::new(
                    "INVALID_STATE",
                    "transaction file collides with another target's parent directory",
                ));
            }
        }
    }
    Ok(())
}
pub fn pending(root: &Path) -> Result<bool> {
    Ok(fs::read(root, JOURNAL, fs::CONTROL_LIMIT)?.is_some())
}
pub fn verify(plan: &Plan, root: &Path) -> Result<()> {
    match (&plan.root_identity, &plan.snapshot) {
        (Some(identity), Some(snapshot)) if identity == &snapshot.root_identity => {
            let mut expected = plan
                .operations
                .iter()
                .map(|op| op.target.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            expected.extend([crate::STATE, ".agentfoundry/config.json"]);
            let actual = snapshot
                .entries
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if actual != expected {
                return Err(Failure::new(
                    "INVALID_STATE",
                    "frozen plan path set differs from operations",
                ));
            }
            let inputs = snapshot
                .entries
                .iter()
                .map(|entry| (entry.path.as_str(), &entry.presence))
                .collect::<std::collections::BTreeMap<_, _>>();
            for operation in &plan.operations {
                let presence = inputs.get(operation.target.as_str()).ok_or_else(|| {
                    Failure::new("INVALID_STATE", "plan input missing from snapshot")
                })?;
                let matches = match (*presence, &operation.before) {
                    (crate::models::Presence::Absent, None) => true,
                    (crate::models::Presence::File { digest, mode }, Some(bytes)) => {
                        digest == &fs::digest(bytes) && *mode == operation.mode
                    }
                    _ => false,
                };
                if !matches {
                    return Err(Failure::new(
                        "INVALID_STATE",
                        "snapshot differs from plan preimage",
                    ));
                }
            }
            fs::verify_snapshot(root, snapshot)?;
        }
        (None, None) => {}
        _ => {
            return Err(Failure::new(
                "INVALID_STATE",
                "plan snapshot/root binding missing or inconsistent",
            ));
        }
    }
    validate(plan, root)?;
    if fs::root_identity(root)? != plan.root_identity {
        return Err(Failure::new("WORKSPACE_DRIFT", "root identity changed"));
    }
    for op in &plan.operations {
        if fs::read(root, &op.target, 128 * 1024 * 1024)? != op.before {
            return Err(Failure::new(
                "WORKSPACE_DRIFT",
                format!("input changed: {}", op.target),
            ));
        }
        verify_mode(root, op)?;
    }
    Ok(())
}
fn verify_mode(root: &Path, op: &Operation) -> Result<()> {
    #[cfg(unix)]
    if op.before.is_some() {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::symlink_metadata(fs::safe(root, &op.target)?)?
            .permissions()
            .mode()
            & 0o7777;
        if mode != op.mode {
            return Err(Failure::new(
                "WORKSPACE_DRIFT",
                format!("mode changed: {}", op.target),
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = (root, op);
    Ok(())
}
fn put(root: &Path, op: &Operation, after: bool) -> Result<()> {
    match if after { &op.after } else { &op.before } {
        Some(bytes) => fs::atomic(root, &op.target, bytes, op.mode),
        None => fs::remove(root, &op.target),
    }
}
pub fn recover(root: &Path, dry: bool) -> Result<serde_json::Value> {
    let Some(raw) = fs::read(root, JOURNAL, fs::CONTROL_LIMIT)? else {
        return Ok(serde_json::json!({"recovery":"not_needed"}));
    };
    let mut j = Journal::parse(&raw, &root.to_string_lossy())?;
    if fs::root_identity(root)? != j.plan.root_identity {
        return Err(Failure::new(
            "RECOVERY_BLOCKED",
            "transaction root identity changed",
        ));
    }
    let backup = format!(".agentfoundry/backups/{}/plan.json", j.id);
    if fs::read(root, &backup, fs::CONTROL_LIMIT)? != Some(serde_json::to_vec(&j.plan)?) {
        return Err(Failure::new(
            "RECOVERY_BLOCKED",
            "backup missing or corrupt; preserve journal",
        ));
    }
    let plan = j.plan.hydrate(root, &j.id)?;
    validate(&plan, root)?;
    let witness = plan
        .operations
        .iter()
        .find(|o| o.target == ".agentfoundry/state/manifest.json");
    let committed = match witness {
        Some(o) => {
            fs::read(root, &o.target, fs::CONTROL_LIMIT)? == o.after
                && (o.before != o.after
                    || matches!(
                        j.phase,
                        JournalPhase::CoreCommitted | JournalPhase::Complete
                    ))
        }
        None => matches!(
            j.phase,
            JournalPhase::CoreCommitted | JournalPhase::Complete
        ),
    };
    let mut mutation_attempted = false;
    let outcome = (|| {
        if committed {
            for op in &plan.operations {
                if op.target.starts_with(".agentfoundry/state/objects/")
                    && fs::read(root, &op.target, fs::CONTROL_LIMIT)? != op.after
                {
                    return Err(Failure::new(
                        "RECOVERY_BLOCKED",
                        "committed baseline object is corrupt",
                    ));
                }
            }
            if dry {
                return Ok(serde_json::json!({"recovery":"finish_commit"}));
            }
        } else {
            let verify_rollback_mode = |op: &Operation, present: bool| -> Result<()> {
                if present {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let mode = std::fs::symlink_metadata(fs::safe(root, &op.target)?)?
                            .permissions()
                            .mode()
                            & 0o7777;
                        if mode != op.mode {
                            return Err(Failure::new(
                                "RECOVERY_BLOCKED",
                                format!("external permission edit retained: {}", op.target),
                            ));
                        }
                    }
                    #[cfg(not(unix))]
                    let _ = op;
                }
                Ok(())
            };
            for op in plan.operations.iter().rev() {
                let current = fs::read(root, &op.target, 128 * 1024 * 1024)?;
                if current != op.before && current != op.after {
                    return Err(Failure::new(
                        "RECOVERY_BLOCKED",
                        format!("external edit retained: {}; backup {backup}", op.target),
                    ));
                }
                verify_rollback_mode(op, current.is_some())?;
            }
            if dry {
                return Ok(serde_json::json!({"recovery":"rollback"}));
            }
            j.phase = JournalPhase::RollingBack;
            mutation_attempted = true;
            fs::atomic(root, JOURNAL, &serde_json::to_vec(&j)?, 0o600)?;
            #[cfg(feature = "fault-injection")]
            crash_checkpoint("rollback-durable");
            for op in plan.operations.iter().rev() {
                let current = fs::read(root, &op.target, 128 * 1024 * 1024)?;
                if current != op.before && current != op.after {
                    return Err(Failure::new(
                        "RECOVERY_BLOCKED",
                        format!("external edit during rollback retained: {}", op.target),
                    ));
                }
                verify_rollback_mode(op, current.is_some())?;
                if op.before != op.after && current == op.after {
                    mutation_attempted = true;
                    put(root, op, false)?;
                    #[cfg(feature = "fault-injection")]
                    if std::env::var("AGENTFOUNDRY_TEST_RECOVERY_ERROR").as_deref()
                        == Ok(op.target.as_str())
                    {
                        return Err(Failure::new(
                            "IO_ERROR",
                            "injected failure after rollback mutation",
                        ));
                    }
                    #[cfg(feature = "fault-injection")]
                    crash_checkpoint(&format!("rollback:{}", op.target));
                }
            }
            j.phase = JournalPhase::Recovered;
            fs::atomic(root, JOURNAL, &serde_json::to_vec(&j)?, 0o600)?;
            #[cfg(feature = "fault-injection")]
            crash_checkpoint("recovered-durable");
        }
        mutation_attempted = true;
        fs::remove(root, JOURNAL)?;
        Ok(serde_json::json!({"recovery":if committed {"committed"} else {"rolled_back"}}))
    })();
    outcome.map_err(|mut error: Failure| {
        if mutation_attempted {
            error.mutation = "partial_recovery_required".into();
        }
        error
    })
}
#[cfg(feature = "fault-injection")]
fn crash_checkpoint(name: &str) {
    if std::env::var("AGENTFOUNDRY_TEST_CRASH").as_deref() == Ok(name) {
        std::process::exit(86);
    }
}
fn verify_applied(root: &Path, operations: &[Operation]) -> Result<()> {
    for operation in operations {
        if fs::read(root, &operation.target, 128 * 1024 * 1024)? != operation.after {
            return Err(Failure::new(
                "WORKSPACE_DRIFT",
                format!("changed before commit: {}", operation.target),
            ));
        }
        if operation.after.is_some()
            && crate::existing_mode(root, &operation.target, operation.mode)? != operation.mode
        {
            return Err(Failure::new(
                "WORKSPACE_DRIFT",
                format!("mode changed before commit: {}", operation.target),
            ));
        }
    }
    Ok(())
}

pub fn apply(root: &Path, plan: &Plan) -> Result<()> {
    if fs::root_identity(root)? != plan.root_identity {
        return Err(Failure::new("WORKSPACE_DRIFT", "root identity changed"));
    }
    let _lock = fs::Lock::acquire(root)?;
    apply_locked(root, plan)
}

pub(crate) fn apply_locked(root: &Path, plan: &Plan) -> Result<()> {
    recover(root, false)?;
    let mut bound_plan;
    let plan = if plan.root_identity.is_none() {
        bound_plan = plan.clone();
        bound_plan.root_identity = fs::root_identity(root)?;
        bound_plan.freeze(root)?;
        &bound_plan
    } else {
        plan
    };
    verify(plan, root)?;
    let id = crate::unique_id();
    let stored = crate::transaction_store::StoredPlan::from_plan(plan);
    let plan_bytes = serde_json::to_vec(&stored)?;
    if plan_bytes.len() as u64 > fs::CONTROL_LIMIT {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "transaction plan exceeds control limit",
        ));
    }
    stored.persist_images(root, &id, plan)?;
    let backup = format!(".agentfoundry/backups/{id}/plan.json");
    fs::atomic(root, &backup, &plan_bytes, 0o600)?;
    if fs::read(root, &backup, fs::CONTROL_LIMIT)? != Some(plan_bytes.clone()) {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "backup verification failed",
        ));
    }
    let mut j = Journal {
        schema: "agentfoundry.journal/v1".into(),
        id,
        root: plan.root.clone(),
        plan_hash: fs::digest(&plan_bytes),
        phase: JournalPhase::Prepared,
        plan: stored,
    };
    #[cfg(feature = "fault-injection")]
    crash_checkpoint("before-journal");
    fs::atomic(root, JOURNAL, &serde_json::to_vec(&j)?, 0o600)?;
    #[cfg(feature = "fault-injection")]
    crash_checkpoint("prepared-durable");
    j.phase = JournalPhase::Applying;
    fs::atomic(root, JOURNAL, &serde_json::to_vec(&j)?, 0o600)?;
    #[cfg(feature = "fault-injection")]
    crash_checkpoint("journal-durable");
    let result = (|| {
        for (index, op) in plan.operations.iter().enumerate() {
            if op.target == crate::STATE && op.before != op.after {
                verify_applied(root, &plan.operations[..index])?;
                verify_applied(root, &plan.operations[index + 1..])?;
            }
            if fs::read(root, &op.target, 128 * 1024 * 1024)? != op.before {
                return Err(Failure::new(
                    "WORKSPACE_DRIFT",
                    format!("changed during apply: {}", op.target),
                ));
            }
            verify_mode(root, op)?;
            if op.before != op.after {
                put(root, op, true)?;
                #[cfg(feature = "fault-injection")]
                crash_checkpoint(&format!("applied:{}", op.target));
            }
        }
        if !plan
            .operations
            .iter()
            .any(|op| op.target == crate::STATE && op.before != op.after)
        {
            verify_applied(root, &plan.operations)?;
        }
        j.phase = JournalPhase::CoreCommitted;
        fs::atomic(root, JOURNAL, &serde_json::to_vec(&j)?, 0o600)?;
        #[cfg(feature = "fault-injection")]
        crash_checkpoint("core-committed-durable");
        #[cfg(feature = "fault-injection")]
        crash_checkpoint("before-complete");
        j.phase = JournalPhase::Complete;
        fs::atomic(root, JOURNAL, &serde_json::to_vec(&j)?, 0o600)?;
        fs::remove(root, JOURNAL)
    })();
    if let Err(mut error) = result {
        match recover(root, false) {
            Ok(outcome) => {
                error.mutation = if outcome["recovery"] == "committed" {
                    "applied"
                } else {
                    "rolled_back"
                }
                .into()
            }
            Err(e) => {
                error.code = "RECOVERY_BLOCKED".into();
                error.message = format!("{}; {}", error.message, e.message);
                error.mutation = "partial_recovery_required".into();
            }
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod commit_checks {
    use super::*;

    #[test]
    fn commit_guard_preserves_external_edits_and_recreated_deletions() {
        let root = std::env::temp_dir().join(format!("agentfoundry-commit-{}", crate::unique_id()));
        std::fs::create_dir(&root).unwrap();
        let operation = Operation {
            target: "owned.md".into(),
            before: Some(b"before".to_vec()),
            after: Some(b"after".to_vec()),
            mode: 0o600,
        };
        fs::atomic(&root, "owned.md", b"after", 0o600).unwrap();
        verify_applied(&root, std::slice::from_ref(&operation)).unwrap();
        fs::atomic(&root, "owned.md", b"editor", 0o600).unwrap();
        assert_eq!(
            verify_applied(&root, std::slice::from_ref(&operation))
                .unwrap_err()
                .code,
            "WORKSPACE_DRIFT"
        );
        assert_eq!(std::fs::read(root.join("owned.md")).unwrap(), b"editor");
        let deletion = Operation {
            after: None,
            ..operation
        };
        assert_eq!(
            verify_applied(&root, &[deletion]).unwrap_err().code,
            "WORKSPACE_DRIFT"
        );
        assert_eq!(std::fs::read(root.join("owned.md")).unwrap(), b"editor");
        std::fs::remove_dir_all(root).unwrap();
    }
}
