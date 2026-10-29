use crate::domain::{RelativePath, validate_digest};
use crate::{Failure, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum JournalPhase {
    Prepared,
    Applying,
    RollingBack,
    Recovered,
    CoreCommitted,
    Complete,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Journal {
    pub schema: String,
    pub id: String,
    pub root: String,
    pub plan_hash: String,
    pub phase: JournalPhase,
    pub plan: crate::transaction_store::StoredPlan,
}
impl Journal {
    pub fn parse(bytes: &[u8], expected_root: &str) -> Result<Self> {
        use sha2::{Digest, Sha256};
        if bytes.len() as u64 > crate::domain::CONTROL_LIMIT {
            return Err(Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "journal exceeds control limit",
            ));
        }
        let journal: Self = serde_json::from_slice(bytes)
            .map_err(|error| Failure::new("RECOVERY_BLOCKED", error.to_string()))?;
        let plan_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&journal.plan)?));
        if journal.schema != "agentfoundry.journal/v1"
            || journal.root != expected_root
            || journal.plan.root != expected_root
            || journal.plan_hash != plan_hash
            || journal.id.is_empty()
            || journal.id.len() > 128
            || !journal.id.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(Failure::new("RECOVERY_BLOCKED", "journal identity invalid"));
        }
        journal.plan.validate(expected_root)?;
        Ok(journal)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Ownership {
    ManagedFile,
    ManagedBlock,
    SeedOnce,
    ConsumerOwned,
    Executable,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub logical_id: String,
    pub target: String,
    pub ownership: Ownership,
    pub source: String,
    pub sha256: String,
    pub mode: u32,
    #[serde(default)]
    pub addon: Option<String>,
    #[serde(default)]
    pub adapter: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    pub schema: String,
    pub entries: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationState {
    pub schema: String,
    pub generation: u64,
    pub version: String,
    pub source: String,
    pub payload_digest: String,
    pub entries: Vec<Entry>,
    pub binary_digest: String,
}
impl InstallationState {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > crate::domain::CONTROL_LIMIT {
            return Err(Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "installation state exceeds control limit",
            ));
        }
        let state: Self = serde_json::from_slice(bytes)?;
        state.validate()?;
        Ok(state)
    }
    pub fn validate(&self) -> Result<()> {
        let s = self;
        if s.schema != "agentfoundry.installation/v1"
            || s.entries.len() > 10000
            || s.generation == 0
            || s.version.is_empty()
            || s.source.is_empty()
        {
            return Err(Failure::new(
                "INVALID_STATE",
                "unknown installation schema or invalid generation",
            ));
        }
        validate_digest(&s.payload_digest).map_err(|e| Failure::new("INVALID_STATE", e))?;
        validate_digest(&s.binary_digest).map_err(|e| Failure::new("INVALID_STATE", e))?;
        validate_entries(&s.entries)
    }
}
fn validate_entries(entries: &[Entry]) -> Result<()> {
    if entries.len() > 10000 {
        return Err(Failure::new("INPUT_LIMIT_EXCEEDED", "entry limit exceeded"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    for e in entries {
        validate_entry(e)?;
        if !ids.insert(&e.logical_id) {
            return Err(Failure::new("INVALID_STATE", "duplicate logical ID"));
        }
        if !seen.insert(e.target.to_lowercase()) {
            return Err(Failure::new("INVALID_STATE", "duplicate effective path"));
        }
    }
    for target in &seen {
        for (index, _) in target.match_indices('/') {
            if seen.contains(&target[..index]) {
                return Err(Failure::new(
                    "INVALID_STATE",
                    "installed file collides with another target's parent directory",
                ));
            }
        }
    }
    Ok(())
}
impl Distribution {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() as u64 > crate::domain::CONTROL_LIMIT {
            return Err(Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "distribution exceeds control limit",
            ));
        }
        let distribution: Self = serde_json::from_slice(bytes)?;
        if distribution.schema != "agentfoundry.distribution/v1" {
            return Err(Failure::new("INVALID_STATE", "unknown distribution schema"));
        }
        validate_entries(&distribution.entries)?;
        Ok(distribution)
    }
}
pub(crate) fn validate_entry(e: &Entry) -> Result<()> {
    RelativePath::new(&e.target).map_err(|e| Failure::new("UNSAFE_PATH", e))?;
    RelativePath::new(&e.source).map_err(|e| Failure::new("INVALID_STATE", e))?;
    validate_digest(&e.sha256).map_err(|e| Failure::new("INVALID_STATE", e))?;
    let target = e.target.to_ascii_lowercase();
    if e.logical_id.is_empty()
        || e.mode & !0o777 != 0
        || e.addon
            .as_deref()
            .is_some_and(|addon| addon != "engineering-wisdom")
        || e.adapter
            .as_deref()
            .is_some_and(|adapter| adapter != "claude")
        || target == ".agentfoundry"
        || target.starts_with(".agentfoundry/")
        || target == "scripts/bin"
        || target.starts_with("scripts/bin/")
        || !matches!(
            e.ownership,
            Ownership::ManagedFile | Ownership::ManagedBlock
        )
        || (e.ownership == Ownership::ManagedBlock
            && !["AGENTS.md", "CLAUDE.md"].contains(&e.target.as_str()))
    {
        return Err(Failure::new(
            "INVALID_STATE",
            "reserved payload target or unknown ownership",
        ));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Planned,
    ResolutionPending,
    Prepared,
    Applying,
    RollingBack,
    Recovered,
    RecoveryBlocked,
    CoreCommitted,
    Activating,
    ActivationPending,
    Complete,
}
impl Phase {
    pub fn transition(self, next: Self) -> Result<Self, String> {
        use Phase::*;
        if matches!(
            (self, next),
            (Planned, ResolutionPending | Prepared)
                | (ResolutionPending, Planned)
                | (Prepared, Applying)
                | (Applying, CoreCommitted | RollingBack)
                | (RollingBack, Recovered | RecoveryBlocked)
                | (CoreCommitted, Activating | Complete)
                | (Activating, Complete | ActivationPending)
                | (ActivationPending, Activating)
        ) {
            Ok(next)
        } else {
            Err(format!(
                "invalid lifecycle transition: {self:?} -> {next:?}"
            ))
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Presence {
    Absent,
    File { digest: String, mode: u32 },
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotEntry {
    pub path: RelativePath,
    pub presence: Presence,
}
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSnapshot {
    pub schema: String,
    pub root_identity: String,
    pub generation: u64,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub config_digest: Option<String>,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub state_digest: Option<String>,
    pub entries: Vec<SnapshotEntry>,
}
impl WorkspaceSnapshot {
    pub fn parse(bytes: &[u8]) -> crate::Result<Self> {
        if bytes.len() as u64 > crate::domain::CONTROL_LIMIT {
            return Err(crate::Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "snapshot exceeds control document limit",
            ));
        }
        let snapshot: Self = serde_json::from_slice(bytes)?;
        snapshot
            .validate()
            .map_err(|error| crate::Failure::new("INVALID_STATE", error))?;
        Ok(snapshot)
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "agentfoundry.snapshot/v1"
            || self.root_identity.is_empty()
            || self.entries.len() > 10000
        {
            return Err("invalid snapshot root or resource limit".into());
        }
        for hash in [&self.config_digest, &self.state_digest]
            .into_iter()
            .flatten()
        {
            validate_digest(hash)?;
        }
        for (path, fingerprint) in [
            (".agentfoundry/config.json", &self.config_digest),
            (".agentfoundry/state/manifest.json", &self.state_digest),
        ] {
            let recorded = self
                .entries
                .iter()
                .find(|entry| entry.path.as_str() == path)
                .ok_or("snapshot omits required control input")?;
            let recorded = match &recorded.presence {
                Presence::File { digest, .. } => Some(digest),
                Presence::Absent => None,
            };
            if recorded != fingerprint.as_ref() {
                return Err("snapshot control fingerprint differs from frozen entry".into());
            }
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            if !seen.insert(entry.path.as_str().to_lowercase()) {
                return Err("duplicate effective snapshot path".into());
            }
            if let Presence::File { digest, mode } = &entry.presence {
                validate_digest(digest)?;
                if mode & !0o777 != 0 {
                    return Err("unsupported file mode".into());
                }
            }
        }
        for target in &seen {
            for (index, _) in target.match_indices('/') {
                if seen.contains(&target[..index]) {
                    return Err(
                        "snapshot file collides with another path's parent directory".into(),
                    );
                }
            }
        }
        Ok(())
    }
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConflictInput {
    pub path: RelativePath,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub base: Option<String>,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub local: Option<String>,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub upstream: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionSession {
    pub schema: String,
    pub id: String,
    pub from_generation: u64,
    pub to_payload_digest: String,
    pub plan_digest: String,
    pub frozen: WorkspaceSnapshot,
    pub conflicts: Vec<ConflictInput>,
}
impl ResolutionSession {
    pub fn parse(bytes: &[u8], expected_paths: &BTreeSet<String>) -> crate::Result<Self> {
        if bytes.len() as u64 > crate::domain::CONTROL_LIMIT {
            return Err(crate::Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "resolution session exceeds control document limit",
            ));
        }
        let session: Self = serde_json::from_slice(bytes)?;
        session
            .validate(expected_paths)
            .map_err(|error| crate::Failure::new("INVALID_STATE", error))?;
        Ok(session)
    }
    pub fn validate(&self, expected_paths: &BTreeSet<String>) -> Result<(), String> {
        if self.schema != "agentfoundry.resolution/v1"
            || self.id.is_empty()
            || self.id.len() > 128
            || !self.id.bytes().all(|b| b.is_ascii_hexdigit())
            || self.from_generation != self.frozen.generation
            || self.conflicts.is_empty()
            || self.conflicts.len() > 10000
        {
            return Err("invalid resolution identity or generation".into());
        }
        validate_digest(&self.to_payload_digest)?;
        validate_digest(&self.plan_digest)?;
        self.frozen.validate()?;
        let entries = self
            .frozen
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), &entry.presence))
            .collect::<std::collections::BTreeMap<_, _>>();
        if !entries
            .keys()
            .copied()
            .eq(expected_paths.iter().map(String::as_str))
        {
            return Err("unknown, additional or missing frozen paths".into());
        }
        let mut seen = BTreeSet::new();
        for conflict in &self.conflicts {
            if !entries.contains_key(conflict.path.as_str()) || !seen.insert(conflict.path.as_str())
            {
                return Err("unknown or duplicate conflict path".into());
            }
            let local = match entries
                .get(conflict.path.as_str())
                .ok_or("conflict path absent from frozen inputs")?
            {
                Presence::Absent => None,
                Presence::File { digest, .. } => Some(digest),
            };
            if local != conflict.local.as_ref() {
                return Err("conflict local digest differs from frozen input".into());
            }
            for hash in [&conflict.base, &conflict.local, &conflict.upstream]
                .into_iter()
                .flatten()
            {
                validate_digest(hash)?;
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn session() -> ResolutionSession {
        ResolutionSession {
            schema: "agentfoundry.resolution/v1".into(),
            id: "abc".into(),
            from_generation: 2,
            to_payload_digest: "a".repeat(64),
            plan_digest: "b".repeat(64),
            frozen: WorkspaceSnapshot {
                schema: "agentfoundry.snapshot/v1".into(),
                root_identity: "fixture".into(),
                generation: 2,
                config_digest: None,
                state_digest: None,
                entries: vec![
                    SnapshotEntry {
                        path: RelativePath::new("AGENTS.md").unwrap(),
                        presence: Presence::File {
                            digest: "d".repeat(64),
                            mode: 0o644,
                        },
                    },
                    SnapshotEntry {
                        path: RelativePath::new("clean.md").unwrap(),
                        presence: Presence::Absent,
                    },
                    SnapshotEntry {
                        path: RelativePath::new(".agentfoundry/config.json").unwrap(),
                        presence: Presence::Absent,
                    },
                    SnapshotEntry {
                        path: RelativePath::new(".agentfoundry/state/manifest.json").unwrap(),
                        presence: Presence::Absent,
                    },
                ],
            },
            conflicts: vec![ConflictInput {
                path: RelativePath::new("AGENTS.md").unwrap(),
                base: Some("e".repeat(64)),
                local: Some("d".repeat(64)),
                upstream: Some("0".repeat(64)),
            }],
        }
    }
    #[test]
    fn session_requires_exact_full_input_set() {
        let mut s = session();
        let expected = BTreeSet::from([
            "AGENTS.md".into(),
            "clean.md".into(),
            ".agentfoundry/config.json".into(),
            ".agentfoundry/state/manifest.json".into(),
        ]);
        s.validate(&expected).unwrap();
        s.frozen.entries.pop();
        assert!(s.validate(&expected).is_err());
        let mut collision = session();
        collision.frozen.entries[1].path = RelativePath::new("agents.md/child").unwrap();
        assert!(collision.frozen.validate().is_err());
    }
    #[test]
    fn duplicate_conflict_and_unknown_schema_fail() {
        let mut s = session();
        let expected = BTreeSet::from([
            "AGENTS.md".into(),
            "clean.md".into(),
            ".agentfoundry/config.json".into(),
            ".agentfoundry/state/manifest.json".into(),
        ]);
        s.conflicts.push(s.conflicts[0].clone());
        assert!(s.validate(&expected).is_err());
        s.conflicts.pop();
        s.schema = "agentfoundry.resolution/v2".into();
        assert!(s.validate(&expected).is_err());
    }
    #[test]
    fn committed_core_cannot_roll_back() {
        assert!(Phase::CoreCommitted.transition(Phase::RollingBack).is_err());
        assert_eq!(
            Phase::CoreCommitted
                .transition(Phase::Activating)
                .unwrap()
                .transition(Phase::ActivationPending)
                .unwrap()
                .transition(Phase::Activating)
                .unwrap()
                .transition(Phase::Complete)
                .unwrap(),
            Phase::Complete
        );
    }
    #[test]
    fn session_wire_rejects_local_digest_detached_from_frozen_input() {
        let mut session = session();
        let expected = BTreeSet::from([
            "AGENTS.md".into(),
            "clean.md".into(),
            ".agentfoundry/config.json".into(),
            ".agentfoundry/state/manifest.json".into(),
        ]);
        let wire = serde_json::to_vec(&session).unwrap();
        ResolutionSession::parse(&wire, &expected).unwrap();
        for field in ["base", "local", "upstream"] {
            let mut bad = serde_json::to_value(&session).unwrap();
            bad["conflicts"][0].as_object_mut().unwrap().remove(field);
            assert!(
                ResolutionSession::parse(&serde_json::to_vec(&bad).unwrap(), &expected).is_err()
            );
        }
        session.conflicts[0].local = Some("f".repeat(64));
        assert!(session.validate(&expected).is_err());
    }
    #[test]
    fn installation_parser_rejects_ambiguous_and_unbounded_inputs() {
        let valid = serde_json::json!({"schema":"agentfoundry.installation/v1","generation":1,"version":"0.1.0-dev.1","source":"fixture","payload_digest":"a".repeat(64),"binary_digest":"b".repeat(64),"entries":[]});
        InstallationState::parse(&serde_json::to_vec(&valid).unwrap()).unwrap();
        for (field, value) in [
            ("generation", serde_json::json!(0)),
            ("schema", serde_json::json!("agentfoundry.installation/v99")),
            ("binary_digest", serde_json::json!("invalid")),
            ("version", serde_json::json!("")),
            ("source", serde_json::json!("")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(InstallationState::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        let oversized = vec![b' '; crate::domain::CONTROL_LIMIT as usize + 1];
        assert_eq!(
            InstallationState::parse(&oversized).err().unwrap().code,
            "INPUT_LIMIT_EXCEEDED"
        );
    }
    #[test]
    fn distribution_parser_rejects_ambiguous_paths_and_unknown_contracts() {
        let entry = serde_json::json!({"logical_id":"one","target":"docs/a","source":"payload/a","ownership":"managed_file","mode":420,"sha256":"a".repeat(64)});
        let valid =
            serde_json::json!({"schema":"agentfoundry.distribution/v1","entries":[entry.clone()]});
        Distribution::parse(&serde_json::to_vec(&valid).unwrap()).unwrap();
        for target in ["docs/a", "DOCS/A", "docs/a/child", "../outside"] {
            let mut invalid = valid.clone();
            let mut second = entry.clone();
            second["logical_id"] = serde_json::json!("two");
            second["target"] = serde_json::json!(target);
            invalid["entries"].as_array_mut().unwrap().push(second);
            assert!(Distribution::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        for (field, value) in [
            ("schema", serde_json::json!("agentfoundry.distribution/v99")),
            ("adapters", serde_json::json!([])),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            assert!(Distribution::parse(&serde_json::to_vec(&invalid).unwrap()).is_err());
        }
        assert_eq!(
            Distribution::parse(&vec![b' '; crate::domain::CONTROL_LIMIT as usize + 1])
                .err()
                .unwrap()
                .code,
            "INPUT_LIMIT_EXCEEDED"
        );
    }
    #[test]
    fn journal_parser_binds_root_phase_and_plan_identity() {
        use sha2::{Digest, Sha256};
        let plan = crate::transaction_store::StoredPlan {
            schema: "agentfoundry.stored-plan/v1".into(),
            root: "/fixture".into(),
            root_identity: Some("fixture".into()),
            payload_digest: "a".repeat(64),
            choices: Default::default(),
            operations: vec![],
        };
        let hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&plan).unwrap()));
        let valid = serde_json::json!({"schema":"agentfoundry.journal/v1","id":"abc","root":"/fixture","plan_hash":hash,"phase":"Prepared","plan":plan});
        Journal::parse(&serde_json::to_vec(&valid).unwrap(), "/fixture").unwrap();
        for (field, value) in [
            ("id", "../outside"),
            ("root", "/foreign"),
            ("plan_hash", "invalid"),
            ("phase", "Unknown"),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = serde_json::json!(value);
            assert_eq!(
                Journal::parse(&serde_json::to_vec(&invalid).unwrap(), "/fixture")
                    .err()
                    .unwrap()
                    .code,
                "RECOVERY_BLOCKED"
            );
        }
        let mut detached = valid.clone();
        detached["plan"]["root"] = serde_json::json!("/foreign");
        let detached_plan: crate::transaction_store::StoredPlan =
            serde_json::from_value(detached["plan"].clone()).unwrap();
        detached["plan_hash"] = serde_json::json!(format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&detached_plan).unwrap())
        ));
        assert_eq!(
            Journal::parse(&serde_json::to_vec(&detached).unwrap(), "/fixture")
                .err()
                .unwrap()
                .code,
            "RECOVERY_BLOCKED"
        );
        for field in ["schema", "payload_digest"] {
            let mut invalid = valid.clone();
            invalid["plan"][field] = serde_json::json!("invalid");
            let plan: crate::transaction_store::StoredPlan =
                serde_json::from_value(invalid["plan"].clone()).unwrap();
            invalid["plan_hash"] = serde_json::json!(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&plan).unwrap())
            ));
            assert_eq!(
                Journal::parse(&serde_json::to_vec(&invalid).unwrap(), "/fixture")
                    .err()
                    .unwrap()
                    .code,
                "RECOVERY_BLOCKED"
            );
        }
        for target in [
            ".agentfoundry/runtime/transaction.json".to_string(),
            ".AgentFoundry/config.json".into(),
            format!(".agentfoundry/state/objects/{}", "a".repeat(64)),
        ] {
            let mut invalid = valid.clone();
            let mut plan = plan.clone();
            plan.operations
                .push(crate::transaction_store::StoredOperation {
                    target,
                    before: None,
                    after: Some("b".repeat(64)),
                    mode: 0o600,
                });
            invalid["plan_hash"] = serde_json::json!(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&plan).unwrap())
            ));
            invalid["plan"] = serde_json::to_value(plan).unwrap();
            assert_eq!(
                Journal::parse(&serde_json::to_vec(&invalid).unwrap(), "/fixture")
                    .err()
                    .unwrap()
                    .code,
                "RECOVERY_BLOCKED"
            );
        }
        assert_eq!(
            Journal::parse(
                &vec![b' '; crate::domain::CONTROL_LIMIT as usize + 1],
                "/fixture"
            )
            .err()
            .unwrap()
            .code,
            "INPUT_LIMIT_EXCEEDED"
        );
    }
}
