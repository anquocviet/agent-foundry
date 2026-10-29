use agentfoundry::{
    application::{self, Operation, Plan},
    infrastructure,
};
#[test]
fn untrusted_internal_plan_cannot_write_arbitrary_state_or_corrupt_objects() {
    let root = std::env::temp_dir().join(format!(
        "agentfoundry-allowlist-{}",
        agentfoundry::unique_id()
    ));
    std::fs::create_dir(&root).unwrap();
    let sentinel = root.join("consumer.txt");
    std::fs::write(&sentinel, b"private").unwrap();
    for target in [
        ".agentfoundry/unowned.txt".to_string(),
        ".AgentFoundry/runtime/evil".into(),
        format!(".agentfoundry/state/objects/{}", "a".repeat(64)),
    ] {
        let plan = Plan {
            schema: "agentfoundry.plan/v1".into(),
            root: root.to_string_lossy().into(),
            root_identity: infrastructure::root_identity(&root).unwrap(),
            snapshot: None,
            payload_digest: "b".repeat(64),
            choices: Default::default(),
            operations: vec![Operation {
                target: target.clone(),
                before: None,
                after: Some(b"not-the-digest".to_vec()),
                mode: 0o644,
            }],
        };
        assert!(application::verify(&plan, &root).is_err());
        assert!(!root.join(target).exists());
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"private");
    }
    let bytes = b"valid immutable object";
    let mut plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: root.to_string_lossy().into(),
        root_identity: infrastructure::root_identity(&root).unwrap(),
        snapshot: None,
        payload_digest: "b".repeat(64),
        choices: Default::default(),
        operations: vec![Operation {
            target: format!(
                ".agentfoundry/state/objects/{}",
                infrastructure::digest(bytes)
            ),
            before: None,
            after: Some(bytes.to_vec()),
            mode: 0o644,
        }],
    };
    plan.freeze(&root).unwrap();
    application::verify(&plan, &root).unwrap();
    let id = plan.semantic_id().unwrap();
    let mut changed = plan.clone();
    changed.operations[0].mode = 0o600;
    assert_ne!(id, changed.semantic_id().unwrap());
    changed = plan.clone();
    changed.operations[0].after = Some(b"other bytes".to_vec());
    assert_ne!(id, changed.semantic_id().unwrap());
    changed = plan.clone();
    changed.choices.replace = true;
    assert_ne!(id, changed.semantic_id().unwrap());
    changed.choices.merge = true;
    assert_eq!(
        application::verify(&changed, &root).unwrap_err().code,
        "INVALID_STATE"
    );
    changed = plan.clone();
    changed.root.push_str("-different");
    assert_ne!(id, changed.semantic_id().unwrap());
    let mut collision = plan.clone();
    collision.operations = ["docs/a", "docs/a-b", "DOCS/A/child"]
        .iter()
        .map(|target| Operation {
            target: (*target).into(),
            before: None,
            after: Some(b"content".to_vec()),
            mode: 0o644,
        })
        .collect();
    assert_eq!(
        application::verify(&collision, &root).unwrap_err().code,
        "INVALID_STATE"
    );
    assert!(!root.join("docs").exists());
    std::fs::remove_dir_all(root).unwrap();
}
