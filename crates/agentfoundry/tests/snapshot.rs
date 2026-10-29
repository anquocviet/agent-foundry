use agentfoundry::infrastructure::{snapshot, verify_snapshot};

#[test]
fn frozen_inputs_detect_preserved_file_absence_config_and_mode_drift() {
    let root = std::env::temp_dir().join(format!(
        "agentfoundry-snapshot-{}",
        agentfoundry::unique_id()
    ));
    std::fs::create_dir_all(root.join(".agentfoundry")).unwrap();
    std::fs::write(root.join("clean.md"), b"preserved\r\n").unwrap();
    let fresh = snapshot(&root, 0, &["clean.md"]).unwrap();
    assert!(fresh.config_digest.is_none() && fresh.state_digest.is_none());
    let config = serde_json::to_vec(&agentfoundry::config::Config::selected(false, false)).unwrap();
    std::fs::write(root.join(".agentfoundry/config.json"), &config).unwrap();
    assert_eq!(
        verify_snapshot(&root, &fresh).unwrap_err().code,
        "WORKSPACE_DRIFT"
    );
    std::fs::create_dir_all(root.join(".agentfoundry/state")).unwrap();
    let manifest = serde_json::to_vec(&serde_json::json!({"schema":"agentfoundry.installation/v1","generation":2,"version":"0.1.0-dev.1","source":"fixture","payload_digest":"a".repeat(64),"binary_digest":"b".repeat(64),"entries":[]})).unwrap();
    std::fs::write(root.join(".agentfoundry/state/manifest.json"), &manifest).unwrap();
    let paths = [
        "clean.md",
        "absent.md",
        ".agentfoundry/config.json",
        ".agentfoundry/state/manifest.json",
    ];
    let frozen = snapshot(&root, 2, &paths).unwrap();
    verify_snapshot(&root, &frozen).unwrap();
    let managed_only = snapshot(&root, 2, &["clean.md", "absent.md"]).unwrap();
    assert_eq!(managed_only.state_digest, frozen.state_digest);
    assert_eq!(managed_only.config_digest, frozen.config_digest);
    for control in [
        ".agentfoundry/config.json",
        ".agentfoundry/state/manifest.json",
    ] {
        let mut omitted = fresh.clone();
        omitted
            .entries
            .retain(|entry| entry.path.as_str() != control);
        assert!(omitted.validate().is_err());
    }
    let wire = serde_json::to_value(&frozen).unwrap();
    let restored =
        agentfoundry::models::WorkspaceSnapshot::parse(&serde_json::to_vec(&wire).unwrap())
            .unwrap();
    verify_snapshot(&root, &restored).unwrap();
    for field in ["config_digest", "state_digest"] {
        let mut bad = wire.clone();
        bad.as_object_mut().unwrap().remove(field);
        assert!(
            agentfoundry::models::WorkspaceSnapshot::parse(&serde_json::to_vec(&bad).unwrap())
                .is_err()
        );
    }
    let mut bad = wire.clone();
    bad["entries"][0]["path"] = serde_json::json!("../outside");
    assert!(serde_json::from_value::<agentfoundry::models::WorkspaceSnapshot>(bad).is_err());
    let mut bad = wire.clone();
    bad["entries"][0]["presence"]["kind"] = serde_json::json!("symlink");
    assert!(serde_json::from_value::<agentfoundry::models::WorkspaceSnapshot>(bad).is_err());
    let mut bad = wire.clone();
    bad["state_digest"] = serde_json::json!("a".repeat(64));
    assert!(
        agentfoundry::models::WorkspaceSnapshot::parse(&serde_json::to_vec(&bad).unwrap()).is_err()
    );
    let mut bad = wire;
    bad["schema"] = serde_json::json!("agentfoundry.snapshot/v99");
    let bad: agentfoundry::models::WorkspaceSnapshot = serde_json::from_value(bad).unwrap();
    assert!(bad.validate().is_err());
    assert_eq!(
        frozen.state_digest,
        Some(agentfoundry::infrastructure::digest(&manifest))
    );
    let mut changed: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    changed["generation"] = serde_json::json!(3);
    std::fs::write(
        root.join(".agentfoundry/state/manifest.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        verify_snapshot(&root, &frozen).unwrap_err().code,
        "WORKSPACE_DRIFT"
    );
    changed["generation"] = serde_json::json!(2);
    changed["binary_digest"] = serde_json::json!("corrupt");
    std::fs::write(
        root.join(".agentfoundry/state/manifest.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert_eq!(
        snapshot(&root, 2, &paths).unwrap_err().code,
        "INVALID_STATE"
    );
    std::fs::write(root.join(".agentfoundry/state/manifest.json"), &manifest).unwrap();
    std::fs::write(root.join(".agentfoundry/config.json"), b"{}").unwrap();
    assert!(snapshot(&root, 2, &paths).is_err());
    assert!(snapshot(&root, 2, &["clean.md"]).is_err());
    std::fs::write(root.join(".agentfoundry/config.json"), &config).unwrap();
    let mut changed_config = config.clone();
    changed_config.push(b' ');
    for (path, bytes, restore) in [
        (
            "clean.md",
            b"editor".as_slice(),
            Some(b"preserved\r\n".as_slice()),
        ),
        ("absent.md", b"new".as_slice(), None),
        (
            ".agentfoundry/config.json",
            changed_config.as_slice(),
            Some(config.as_slice()),
        ),
    ] {
        std::fs::write(root.join(path), bytes).unwrap();
        assert_eq!(
            verify_snapshot(&root, &frozen).unwrap_err().code,
            "WORKSPACE_DRIFT"
        );
        assert_eq!(
            verify_snapshot(&root, &managed_only).unwrap_err().code,
            "WORKSPACE_DRIFT"
        );
        match restore {
            Some(bytes) => std::fs::write(root.join(path), bytes).unwrap(),
            None => std::fs::remove_file(root.join(path)).unwrap(),
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = root.join("clean.md");
        let old = std::fs::metadata(&path).unwrap().permissions().mode();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(old ^ 0o100)).unwrap();
        assert_eq!(
            verify_snapshot(&root, &frozen).unwrap_err().code,
            "WORKSPACE_DRIFT"
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(old)).unwrap();
        let moved = root.with_extension("original");
        std::fs::rename(&root, &moved).unwrap();
        std::fs::create_dir(&root).unwrap();
        for name in ["clean.md", ".agentfoundry"] {
            std::fs::rename(moved.join(name), root.join(name)).unwrap();
        }
        assert_eq!(
            verify_snapshot(&root, &frozen).unwrap_err().code,
            "WORKSPACE_DRIFT"
        );
        std::fs::remove_dir(moved).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}
