use agentfoundry::{
    application::{self, Operation, Plan},
    infrastructure as fs,
};
use std::path::PathBuf;
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "agentfoundry-recovery-{}",
            agentfoundry::unique_id()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn interrupted(f: &Fixture, external: bool) -> Plan {
    let plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: f.0.to_string_lossy().into(),
        root_identity: fs::root_identity(&f.0).unwrap(),
        snapshot: None,
        payload_digest: "a".repeat(64),
        choices: Default::default(),
        operations: vec![
            Operation {
                target: "owned.md".into(),
                before: Some(b"before".to_vec()),
                after: Some(b"after".to_vec()),
                mode: 0o644,
            },
            Operation {
                target: ".agentfoundry/state/manifest.json".into(),
                before: None,
                after: Some(b"commit-witness".to_vec()),
                mode: 0o644,
            },
        ],
    };
    let stored = agentfoundry::transaction_store::StoredPlan::from_plan(&plan);
    stored.persist_images(&f.0, "abc", &plan).unwrap();
    let bytes = serde_json::to_vec(&stored).unwrap();
    fs::atomic(&f.0, ".agentfoundry/backups/abc/plan.json", &bytes, 0o600).unwrap();
    let journal = serde_json::json!({"schema":"agentfoundry.journal/v1","id":"abc","root":plan.root,"plan_hash":fs::digest(&bytes),"phase":"Applying","plan":stored});
    fs::atomic(
        &f.0,
        ".agentfoundry/runtime/transaction.json",
        &serde_json::to_vec(&journal).unwrap(),
        0o600,
    )
    .unwrap();
    fs::atomic(
        &f.0,
        "owned.md",
        if external { b"external" } else { b"after" },
        0o644,
    )
    .unwrap();
    plan
}
#[test]
fn rollback_replays_idempotently() {
    let f = Fixture::new();
    interrupted(&f, false);
    application::recover(&f.0, false).unwrap();
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"before");
    application::recover(&f.0, false).unwrap();
    assert!(f.0.join(".agentfoundry/backups/abc/plan.json").is_file());
}
#[test]
fn external_edit_blocks_rollback() {
    let f = Fixture::new();
    interrupted(&f, true);
    let e = application::recover(&f.0, false).unwrap_err();
    assert_eq!(e.code, "RECOVERY_BLOCKED");
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"external");
    assert!(f.0.join(".agentfoundry/runtime/transaction.json").is_file());
}
#[test]
fn commit_witness_beats_stale_phase() {
    let f = Fixture::new();
    interrupted(&f, true);
    fs::atomic(
        &f.0,
        ".agentfoundry/state/manifest.json",
        b"commit-witness",
        0o644,
    )
    .unwrap();
    application::recover(&f.0, false).unwrap();
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"external");
}
#[test]
fn committed_phase_does_not_replace_manifest_witness() {
    let f = Fixture::new();
    interrupted(&f, false);
    let path = f.0.join(".agentfoundry/runtime/transaction.json");
    let mut journal: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    journal["phase"] = serde_json::json!("CoreCommitted");
    std::fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    let result = application::recover(&f.0, false).unwrap();
    assert_eq!(result["recovery"], "rolled_back");
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"before");
    assert!(!f.0.join(".agentfoundry/state/manifest.json").exists());
}
#[test]
fn committed_generic_transaction_preserves_later_consumer_edits() {
    let f = Fixture::new();
    let mut plan = interrupted(&f, true);
    plan.operations.pop();
    let stored = agentfoundry::transaction_store::StoredPlan::from_plan(&plan);
    let bytes = serde_json::to_vec(&stored).unwrap();
    fs::atomic(&f.0, ".agentfoundry/backups/abc/plan.json", &bytes, 0o600).unwrap();
    let journal = serde_json::json!({"schema":"agentfoundry.journal/v1","id":"abc","root":plan.root,"plan_hash":fs::digest(&bytes),"phase":"CoreCommitted","plan":stored});
    fs::atomic(
        &f.0,
        ".agentfoundry/runtime/transaction.json",
        &serde_json::to_vec(&journal).unwrap(),
        0o600,
    )
    .unwrap();
    let result = application::recover(&f.0, false).unwrap();
    assert_eq!(result["recovery"], "committed");
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"external");
    assert_eq!(
        application::recover(&f.0, false).unwrap()["recovery"],
        "not_needed"
    );
}
#[test]
fn stale_plan_rejects_before_writing() {
    let f = Fixture::new();
    let mut plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: f.0.to_string_lossy().into(),
        root_identity: fs::root_identity(&f.0).unwrap(),
        snapshot: None,
        payload_digest: "b".repeat(64),
        choices: Default::default(),
        operations: vec![Operation {
            target: "owned.md".into(),
            before: None,
            after: Some(b"replacement".to_vec()),
            mode: 0o644,
        }],
    };
    plan.freeze(&f.0).unwrap();
    std::fs::write(f.0.join("owned.md"), b"new local edit").unwrap();
    assert_eq!(
        application::apply(&f.0, &plan).unwrap_err().code,
        "WORKSPACE_DRIFT"
    );
    assert_eq!(
        std::fs::read(f.0.join("owned.md")).unwrap(),
        b"new local edit"
    );
}

#[cfg(unix)]
#[test]
fn external_permission_edit_blocks_rollback() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    interrupted(&f, false);
    std::fs::set_permissions(f.0.join("owned.md"), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        application::recover(&f.0, false).unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"after");
    assert_eq!(
        std::fs::metadata(f.0.join("owned.md"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(f.0.join(".agentfoundry/runtime/transaction.json").is_file());
}

#[test]
fn corrupt_backup_blocks_recovery_without_mutation() {
    let f = Fixture::new();
    interrupted(&f, false);
    std::fs::write(f.0.join(".agentfoundry/backups/abc/plan.json"), b"{}").unwrap();
    assert_eq!(
        application::recover(&f.0, false).unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"after");
    assert!(f.0.join(".agentfoundry/runtime/transaction.json").is_file());
    assert_eq!(
        std::fs::read(f.0.join(".agentfoundry/backups/abc/plan.json")).unwrap(),
        b"{}"
    );
}

#[test]
fn preview_refuses_external_edit_without_clearing_journal() {
    let f = Fixture::new();
    interrupted(&f, true);
    assert_eq!(
        application::recover(&f.0, true).unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"external");
    assert!(f.0.join(".agentfoundry/runtime/transaction.json").is_file());
}

#[cfg(unix)]
#[test]
fn replaced_root_blocks_recovery_and_retains_evidence() {
    let f = Fixture::new();
    interrupted(&f, false);
    let journal = ".agentfoundry/runtime/transaction.json";
    let bytes = std::fs::read(f.0.join(journal)).unwrap();
    let retained = Fixture::new();
    std::fs::remove_dir(&retained.0).unwrap();
    std::fs::rename(&f.0, &retained.0).unwrap();
    std::fs::create_dir(&f.0).unwrap();
    fs::atomic(&f.0, journal, &bytes, 0o600).unwrap();
    fs::atomic(&f.0, "owned.md", b"after", 0o644).unwrap();
    assert_eq!(
        application::recover(&f.0, false).unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    assert_eq!(std::fs::read(f.0.join(journal)).unwrap(), bytes);
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"after");
    std::fs::remove_dir_all(&f.0).unwrap();
    std::fs::rename(&retained.0, &f.0).unwrap();
    assert_eq!(
        application::recover(&f.0, false).unwrap()["recovery"],
        "rolled_back"
    );
    assert_eq!(std::fs::read(f.0.join("owned.md")).unwrap(), b"before");
}
