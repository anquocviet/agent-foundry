#[cfg(unix)]
#[test]
fn permission_change_invalidates_frozen_plan() {
    use agentfoundry::application::{self, Operation, Plan};
    use std::os::unix::fs::PermissionsExt;
    let root =
        std::env::temp_dir().join(format!("agentfoundry-mode-{}", agentfoundry::unique_id()));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("private.md");
    std::fs::write(&path, b"private input").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let mut plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: root.to_string_lossy().into(),
        root_identity: agentfoundry::infrastructure::root_identity(&root).unwrap(),
        snapshot: None,
        payload_digest: "a".repeat(64),
        choices: Default::default(),
        operations: vec![Operation {
            target: "private.md".into(),
            before: Some(b"private input".to_vec()),
            after: Some(b"next".to_vec()),
            mode: 0o600,
        }],
    };
    plan.freeze(&root).unwrap();
    application::verify(&plan, &root).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        application::apply(&root, &plan).unwrap_err().code,
        "WORKSPACE_DRIFT"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"private input");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    std::fs::remove_dir_all(root).unwrap();
}
