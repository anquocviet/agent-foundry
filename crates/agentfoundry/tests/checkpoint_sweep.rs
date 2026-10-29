use agentfoundry::{
    application::{self, Operation, Plan},
    infrastructure as fs,
    transaction_store::StoredPlan,
};
#[test]
fn every_apply_prefix_recovers_and_manifest_publish_is_commit_witness() {
    for prefix in 0..=4 {
        let root =
            std::env::temp_dir().join(format!("agentfoundry-prefix-{}", agentfoundry::unique_id()));
        std::fs::create_dir(&root).unwrap();
        let plan = Plan {
            schema: "agentfoundry.plan/v1".into(),
            root: root.to_string_lossy().into(),
            root_identity: fs::root_identity(&root).unwrap(),
            snapshot: None,
            payload_digest: "a".repeat(64),
            choices: Default::default(),
            operations: vec![
                Operation {
                    target: "existing.md".into(),
                    before: Some(b"private old".to_vec()),
                    after: Some(b"updated".to_vec()),
                    mode: 0o644,
                },
                Operation {
                    target: "new.md".into(),
                    before: None,
                    after: Some(b"new".to_vec()),
                    mode: 0o644,
                },
                Operation {
                    target: "removed.md".into(),
                    before: Some(b"retired private bytes".to_vec()),
                    after: None,
                    mode: 0o644,
                },
                Operation {
                    target: ".agentfoundry/state/manifest.json".into(),
                    before: Some(b"old-state".to_vec()),
                    after: Some(b"new-state".to_vec()),
                    mode: 0o644,
                },
                Operation {
                    target: "preserved.md".into(),
                    before: Some(b"consumer preserved bytes".to_vec()),
                    after: Some(b"consumer preserved bytes".to_vec()),
                    mode: 0o644,
                },
            ],
        };
        for op in &plan.operations {
            if let Some(bytes) = &op.before {
                fs::atomic(&root, &op.target, bytes, op.mode).unwrap();
            }
        }
        #[cfg(unix)]
        let preserved_inode = {
            use std::os::unix::fs::MetadataExt;
            std::fs::metadata(root.join("preserved.md")).unwrap().ino()
        };
        let stored = StoredPlan::from_plan(&plan);
        stored.persist_images(&root, "abc", &plan).unwrap();
        let bytes = serde_json::to_vec(&stored).unwrap();
        fs::atomic(&root, ".agentfoundry/backups/abc/plan.json", &bytes, 0o600).unwrap();
        let journal = serde_json::json!({"schema":"agentfoundry.journal/v1","id":"abc","root":plan.root,"plan_hash":fs::digest(&bytes),"phase":"Applying","plan":stored});
        fs::atomic(
            &root,
            ".agentfoundry/runtime/transaction.json",
            &serde_json::to_vec(&journal).unwrap(),
            0o600,
        )
        .unwrap();
        for op in plan.operations.iter().take(prefix) {
            match &op.after {
                Some(bytes) => fs::atomic(&root, &op.target, bytes, op.mode).unwrap(),
                None => fs::remove(&root, &op.target).unwrap(),
            }
        }
        if prefix == 3 {
            std::fs::write(root.join("removed.md"), b"consumer recreation").unwrap();
            assert_eq!(
                application::recover(&root, false).unwrap_err().code,
                "RECOVERY_BLOCKED"
            );
            assert_eq!(
                std::fs::read(root.join("removed.md")).unwrap(),
                b"consumer recreation"
            );
            assert_eq!(std::fs::read(root.join("existing.md")).unwrap(), b"updated");
            assert!(
                root.join(".agentfoundry/runtime/transaction.json")
                    .is_file()
            );
            std::fs::remove_file(root.join("removed.md")).unwrap();
        }
        let result = application::recover(&root, false).unwrap();
        assert_eq!(
            result["recovery"],
            if prefix == 4 {
                "committed"
            } else {
                "rolled_back"
            }
        );
        for op in &plan.operations {
            assert_eq!(
                fs::read(&root, &op.target, 1024).unwrap(),
                if prefix == 4 {
                    op.after.clone()
                } else {
                    op.before.clone()
                }
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                std::fs::metadata(root.join("preserved.md")).unwrap().ino(),
                preserved_inode
            );
        }
        assert_eq!(
            application::recover(&root, false).unwrap()["recovery"],
            "not_needed"
        );
        assert!(root.join(".agentfoundry/backups/abc/plan.json").is_file());
        std::fs::remove_dir_all(root).unwrap();
    }
}
