use agentfoundry::{
    application::{self, Operation, Plan},
    infrastructure as fs,
    transaction_store::StoredPlan,
};
#[test]
fn large_images_stay_out_of_control_json_and_tamper_blocks_recovery() {
    let root =
        std::env::temp_dir().join(format!("agentfoundry-large-{}", agentfoundry::unique_id()));
    std::fs::create_dir(&root).unwrap();
    let bytes = vec![b'x'; 9 * 1024 * 1024];
    let mut plan = Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: root.to_string_lossy().into(),
        root_identity: fs::root_identity(&root).unwrap(),
        snapshot: None,
        payload_digest: "a".repeat(64),
        choices: Default::default(),
        operations: vec![Operation {
            target: "scripts/bin/large.bin".into(),
            before: None,
            after: Some(bytes),
            mode: 0o644,
        }],
    };
    plan.freeze(&root).unwrap();
    application::apply(&root, &plan).unwrap();
    assert_eq!(
        std::fs::metadata(root.join("scripts/bin/large.bin"))
            .unwrap()
            .len(),
        9 * 1024 * 1024
    );
    let stored = StoredPlan::from_plan(&plan);
    assert!(serde_json::to_vec(&stored).unwrap().len() < 1024);
    stored.persist_images(&root, "abc", &plan).unwrap();
    for field in ["payload_digest", "target", "mode", "after"] {
        let mut invalid = stored.clone();
        match field {
            "payload_digest" => invalid.payload_digest = "invalid".into(),
            "target" => invalid.operations[0].target = "../outside".into(),
            "mode" => invalid.operations[0].mode = 0o7777,
            _ => invalid.operations[0].after = Some("invalid".into()),
        }
        assert_eq!(
            invalid.hydrate(&root, "abc").unwrap_err().code,
            "RECOVERY_BLOCKED"
        );
    }
    for target in [
        "scripts/bin/large.bin",
        "SCRIPTS/BIN/LARGE.BIN",
        "scripts/bin/large.bin/child",
    ] {
        let mut invalid = stored.clone();
        let mut duplicate = invalid.operations[0].clone();
        duplicate.target = target.into();
        invalid.operations.push(duplicate);
        assert_eq!(
            invalid.hydrate(&root, "abc").unwrap_err().code,
            "RECOVERY_BLOCKED"
        );
    }
    let mut invalid = stored.clone();
    invalid.choices.merge = true;
    invalid.choices.replace = true;
    assert_eq!(
        invalid.hydrate(&root, "abc").unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    let mut invalid = stored.clone();
    let mut commit = invalid.operations[0].clone();
    commit.target = ".agentfoundry/state/manifest.json".into();
    invalid.operations.insert(0, commit);
    assert_eq!(
        invalid.hydrate(&root, "abc").unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    let hash = stored.operations[0].after.as_ref().unwrap();
    fs::atomic(
        &root,
        &format!(".agentfoundry/backups/abc/objects/{hash}"),
        b"tampered",
        0o600,
    )
    .unwrap();
    assert_eq!(
        stored.hydrate(&root, "abc").unwrap_err().code,
        "RECOVERY_BLOCKED"
    );
    assert_eq!(
        std::fs::metadata(root.join("scripts/bin/large.bin"))
            .unwrap()
            .len(),
        9 * 1024 * 1024
    );
    std::fs::remove_dir_all(root).unwrap();
}
