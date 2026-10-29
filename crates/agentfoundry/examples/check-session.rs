use agentfoundry::{infrastructure as fs, session, update};
use std::{path::PathBuf, process::Command};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(
        args.len(),
        3,
        "old-binary target-binary empty-consumer-root"
    );
    let root = PathBuf::from(&args[2]);
    assert!(!root.exists());
    let run = |binary: &str, command: &str, extra: &[&str]| {
        let output = Command::new(binary)
            .args([command, "--directory", &args[2], "--json"])
            .args(extra)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    run(&args[0], "install", &[]);
    let output = Command::new(&args[1])
        .args(["distribution", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &envelope["data"];
    let payload: Vec<(String, Vec<u8>)> = data["payload"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let hex = item["bytes"].as_str().unwrap();
            (
                item["source"].as_str().unwrap().to_owned(),
                (0..hex.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
                    .collect(),
            )
        })
        .collect();
    let identity: update::CandidateIdentity = serde_json::from_value(serde_json::json!({"version":data["version"],"source":data["source"],"payload_digest":data["payload_digest"],"binary_digest":data["binary_digest"]})).unwrap();
    if std::env::var_os("AF_CHECK_CLEAN").is_some() {
        let prepared = update::prepare_target(
            &root,
            data["manifest"].as_str().unwrap().as_bytes(),
            &payload,
            std::fs::read(&args[1]).unwrap(),
            identity.clone(),
        )
        .unwrap();
        assert!(prepared.conflicts.is_empty());
        agentfoundry::activation::apply(&root, &prepared).unwrap();
        assert!(!agentfoundry::activation::pending(&root).unwrap());
        assert_eq!(
            fs::digest(&std::fs::read(root.join("scripts/bin/agentfoundry")).unwrap()),
            identity.binary_digest
        );
        println!("clean standalone activation passed");
        return;
    }
    let block = std::env::var_os("AF_CHECK_BLOCK").is_some();
    let target = if block {
        "AGENTS.md"
    } else {
        "docs/agentfoundry/WORKFLOW.md"
    };
    let original = std::fs::read(root.join(target)).unwrap();
    let local = if block {
        let text = String::from_utf8(original.clone()).unwrap();
        let heading = text.lines().find(|line| line.starts_with("## ")).unwrap();
        text.replacen(heading, "## LOCAL CONFLICT", 1).into_bytes()
    } else {
        let mut bytes = b"LOCAL CONFLICT\n".to_vec();
        bytes
            .extend_from_slice(&original[original.iter().position(|b| *b == b'\n').unwrap() + 1..]);
        bytes
    };
    std::fs::write(root.join(target), &local).unwrap();
    let prepare = || {
        update::prepare_target(
            &root,
            data["manifest"].as_str().unwrap().as_bytes(),
            &payload,
            std::fs::read(&args[1]).unwrap(),
            identity.clone(),
        )
        .unwrap()
    };
    let before = std::fs::read(root.join(".agentfoundry/state/manifest.json")).unwrap();
    let prepared = prepare();
    assert_eq!(prepared.conflicts.len(), 1);
    let report = session::stage(&root, &prepared).unwrap();
    let status = run(&args[1], "status", &[]);
    assert_eq!(status["data"]["resolution_pending"], true);
    assert!(status["data"]["installed"].as_bool().unwrap());
    assert!(status["data"]["running_executable"]["source"].is_string());
    assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
    assert_eq!(
        std::fs::read(root.join(".agentfoundry/state/manifest.json")).unwrap(),
        before
    );
    let blocked = Command::new(&args[1])
        .args(["update", "--continue", "--directory", &args[2], "--json"])
        .output()
        .unwrap();
    assert!(!blocked.status.success());
    let blocked: serde_json::Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_eq!(blocked["code"], "CONFLICT_PENDING");
    assert!(session::pending(&root).unwrap());
    let normal = Command::new(&args[1])
        .args(["update", "--offline", "--directory", &args[2], "--json"])
        .output()
        .unwrap();
    let normal: serde_json::Value = serde_json::from_slice(&normal.stdout).unwrap();
    assert_eq!(normal["code"], "SESSION_EXISTS");
    let resolved_path = root.join(report["conflicts"][0]["resolved"].as_str().unwrap());
    for kind in ["BASE", "LOCAL", "UPSTREAM"] {
        let path = resolved_path.with_file_name(kind);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"tampered").unwrap();
        assert!(session::resume(&root, true).is_err());
        std::fs::write(path, bytes).unwrap();
    }
    let control = root.join(".agentfoundry/runtime/resolution.json");
    let control_bytes = std::fs::read(&control).unwrap();
    let mut control_value: serde_json::Value = serde_json::from_slice(&control_bytes).unwrap();
    control_value["target"]["binary_digest"] = serde_json::json!("0".repeat(64));
    std::fs::write(&control, serde_json::to_vec(&control_value).unwrap()).unwrap();
    assert!(session::resume(&root, true).is_err());
    std::fs::write(control, control_bytes).unwrap();
    run(&args[1], "update", &["--abort"]);
    assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
    let prepared = prepare();
    session::stage(&root, &prepared).unwrap();
    let resolved = report["conflicts"][0]["resolved"].as_str().unwrap();
    let status = session::status(&root).unwrap();
    let actual = status["conflicts"][0]["resolved"].as_str().unwrap();
    assert_ne!(resolved, actual);
    let resolved_bytes = if block {
        b"<!-- AGENTFOUNDRY:BEGIN -->\nResolved by consumer\n<!-- AGENTFOUNDRY:END -->\n".as_slice()
    } else {
        b"Resolved by consumer\n".as_slice()
    };
    std::fs::write(root.join(actual), resolved_bytes).unwrap();
    run(&args[1], "update", &["--continue", "--dry-run"]);
    assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
    assert!(session::pending(&root).unwrap());
    assert_eq!(std::fs::read(root.join(actual)).unwrap(), resolved_bytes);
    let clean = root.join("AGENTS.md");
    let clean_bytes = std::fs::read(&clean).unwrap();
    std::fs::write(&clean, b"external drift\n").unwrap();
    let drift = session::resume(&root, true).unwrap_err();
    assert_eq!(drift.code, "WORKSPACE_DRIFT");
    assert_eq!(std::fs::read(&clean).unwrap(), b"external drift\n");
    std::fs::write(&clean, clean_bytes).unwrap();
    if let Some(checkpoint) = std::env::var_os("AF_CHECK_CRASH") {
        let crash_binary = std::env::var("AF_CHECK_CRASH_BINARY").unwrap();
        let interrupted = Command::new(crash_binary)
            .args(["update", "--continue", "--directory", &args[2], "--json"])
            .env("AGENTFOUNDRY_TEST_CRASH", &checkpoint)
            .output()
            .unwrap();
        assert_eq!(interrupted.status.code(), Some(86));
        assert!(session::pending(&root).unwrap());
        if checkpoint == "core-committed-durable" {
            run(&args[1], "recover", &[]);
            let binary_path = root.join("scripts/bin/agentfoundry");
            let binary_bytes = std::fs::read(&args[1]).unwrap();
            assert_ne!(
                std::fs::read(&binary_path).unwrap(),
                binary_bytes,
                "core commit must precede executable activation"
            );
            std::fs::write(&binary_path, b"external binary change").unwrap();
            let refused = Command::new(&args[1])
                .args(["update", "--continue", "--directory", &args[2], "--json"])
                .output()
                .unwrap();
            assert!(
                !refused.status.success(),
                "committed session accepted an unverified active binary"
            );
            assert!(session::pending(&root).unwrap());
            let abort = Command::new(&args[1])
                .args(["update", "--abort", "--directory", &args[2], "--json"])
                .output()
                .unwrap();
            assert_eq!(
                abort.status.code(),
                Some(8),
                "abort must retain candidate needed for activation recovery"
            );
            assert!(session::pending(&root).unwrap());
            assert_eq!(
                std::fs::read(&binary_path).unwrap(),
                b"external binary change"
            );
            let refused = Command::new(&args[1])
                .args(["recover", "--directory", &args[2], "--json"])
                .output()
                .unwrap();
            assert_eq!(refused.status.code(), Some(8));
            let refused: serde_json::Value = serde_json::from_slice(&refused.stdout).unwrap();
            assert_eq!(refused["code"], "RECOVERY_BLOCKED");
            assert_eq!(
                std::fs::read(&binary_path).unwrap(),
                b"external binary change"
            );
            std::fs::remove_file(&binary_path).unwrap();
            assert_eq!(
                session::resume(&root, true).unwrap_err().code,
                "RECOVERY_BLOCKED"
            );
            assert!(session::pending(&root).unwrap());
            let control: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join(".agentfoundry/runtime/activation.json")).unwrap(),
            )
            .unwrap();
            let candidate_path = root.join(format!(
                ".agentfoundry/backups/{}/objects/{}",
                control["id"].as_str().unwrap(),
                identity.binary_digest
            ));
            let candidate_bytes = std::fs::read(&candidate_path).unwrap();
            std::fs::remove_file(&candidate_path).unwrap();
            let refused = Command::new(&args[1])
                .args(["recover", "--directory", &args[2], "--json"])
                .output()
                .unwrap();
            assert_eq!(refused.status.code(), Some(8));
            assert!(!binary_path.exists());
            assert!(session::pending(&root).unwrap());
            std::fs::write(candidate_path, candidate_bytes).unwrap();
            let preview = run(&args[1], "recover", &["--dry-run"]);
            assert_eq!(preview["data"]["recovery"], "activation_preview");
            assert!(!binary_path.exists());
            if let Some(repair_checkpoint) = std::env::var_os("AF_CHECK_REPAIR_CRASH") {
                let interrupted = Command::new(std::env::var("AF_CHECK_CRASH_BINARY").unwrap())
                    .args(["recover", "--directory", &args[2], "--json"])
                    .env("AGENTFOUNDRY_TEST_CRASH", &repair_checkpoint)
                    .output()
                    .unwrap();
                assert_eq!(interrupted.status.code(), Some(86));
                let recovered = run(&args[1], "recover", &[]);
                if repair_checkpoint == "core-committed-durable" {
                    assert_eq!(std::fs::read(&binary_path).unwrap(), binary_bytes);
                    std::fs::remove_file(&binary_path).unwrap();
                } else {
                    assert_eq!(recovered["data"]["recovery"], "rolled_back");
                    assert!(!binary_path.exists());
                }
                assert!(session::pending(&root).unwrap());
            }
            let repaired = run(&args[1], "recover", &[]);
            assert_eq!(repaired["data"]["recovery"], "activation_repaired");
            assert_eq!(std::fs::read(&binary_path).unwrap(), binary_bytes);
            assert!(session::pending(&root).unwrap());
            let repeated = run(&args[1], "recover", &[]);
            assert_eq!(repeated["data"]["recovery"], "not_needed");
            assert_eq!(std::fs::read(&binary_path).unwrap(), binary_bytes);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o644))
                    .unwrap();
                assert_eq!(
                    session::resume(&root, true).unwrap_err().code,
                    "RECOVERY_REQUIRED"
                );
                assert!(session::pending(&root).unwrap());
                std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))
                    .unwrap();
            }
        }
    }
    run(&args[1], "update", &["--continue"]);
    assert_eq!(
        std::fs::read(root.join(target)).unwrap(),
        if block {
            agentfoundry::domain::patch_block(&local, resolved_bytes, "AGENTFOUNDRY").unwrap()
        } else {
            resolved_bytes.to_vec()
        }
    );
    assert!(!session::pending(&root).unwrap());
    assert_eq!(
        fs::digest(&std::fs::read(root.join("scripts/bin/agentfoundry")).unwrap()),
        identity.binary_digest
    );
    println!("real two-build stage/marker refusal/abort/preview/continue passed");
}
