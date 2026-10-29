use agentfoundry::infrastructure;
#[test]
fn managed_paths_do_not_cross_nested_repositories_or_file_parents() {
    let root =
        std::env::temp_dir().join(format!("agentfoundry-nested-{}", agentfoundry::unique_id()));
    std::fs::create_dir_all(root.join("vendor")).unwrap();
    std::fs::write(root.join("vendor/.git"), b"gitdir: ../private-git").unwrap();
    std::fs::write(root.join("vendor/sentinel"), b"nested owner").unwrap();
    assert_eq!(
        infrastructure::atomic(&root, "vendor/sentinel", b"replacement", 0o644)
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
    assert_eq!(
        std::fs::read(root.join("vendor/sentinel")).unwrap(),
        b"nested owner"
    );
    std::fs::write(root.join("file-parent"), b"outside ownership").unwrap();
    assert_eq!(
        infrastructure::safe(&root, "file-parent/child")
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn swapped_root_symlink_does_not_retarget_transaction() {
    let base = std::env::temp_dir().join(format!(
        "agentfoundry-root-swap-{}",
        agentfoundry::unique_id()
    ));
    std::fs::create_dir_all(base.join("outside")).unwrap();
    std::fs::write(base.join("outside/sentinel"), b"outside").unwrap();
    std::os::unix::fs::symlink(base.join("outside"), base.join("root")).unwrap();
    assert_eq!(
        infrastructure::atomic(&base.join("root"), "sentinel", b"replacement", 0o644)
            .unwrap_err()
            .code,
        "UNSAFE_PATH"
    );
    assert_eq!(
        std::fs::read(base.join("outside/sentinel")).unwrap(),
        b"outside"
    );
    std::fs::remove_dir_all(base).unwrap();
}
