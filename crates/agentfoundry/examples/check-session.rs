use agentfoundry::{infrastructure as fs, session, update};
use std::{path::PathBuf, process::Command};
fn snapshot(root: &std::path::Path) -> std::collections::BTreeMap<PathBuf, (String, u32)> {
    let mut files = std::collections::BTreeMap::new();
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let metadata = entry.metadata().unwrap();
            if metadata.is_dir() {
                directories.push(entry.path());
            } else {
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode()
                };
                #[cfg(not(unix))]
                let mode = u32::from(metadata.permissions().readonly());
                files.insert(
                    entry.path().strip_prefix(root).unwrap().to_path_buf(),
                    (fs::digest(&std::fs::read(entry.path()).unwrap()), mode),
                );
            }
        }
    }
    files
}
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
    let root = root.canonicalize().unwrap();
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
            fs::digest(&std::fs::read(root.join(agentfoundry::binary_path())).unwrap()),
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
        format!(
            "Consumer prefix\n{}",
            text.replacen(heading, "## LOCAL CONFLICT", 1)
        )
        .into_bytes()
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
    let absent_object = prepared
        .plan
        .operations
        .iter()
        .find(|op| {
            op.target.starts_with(".agentfoundry/state/objects/")
                && op.before.is_none()
                && op.after.is_some()
        })
        .expect("changed incoming baseline must have an absent object")
        .target
        .clone();
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
    assert_eq!(blocked["code"], "CONFLICT_PENDING", "{blocked}");
    assert!(session::pending(&root).unwrap());
    let normal = Command::new(&args[1])
        .args(["update", "--offline", "--directory", &args[2], "--json"])
        .output()
        .unwrap();
    let normal: serde_json::Value = serde_json::from_slice(&normal.stdout).unwrap();
    assert_eq!(normal["code"], "SESSION_EXISTS");
    let refused = |expected: &str| {
        for extra in [vec!["--continue", "--dry-run"], vec!["--continue"]] {
            let before = snapshot(&root);
            let output = Command::new(&args[1])
                .args(["update", "--directory", &args[2], "--json"])
                .args(extra)
                .output()
                .unwrap();
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(!output.status.success(), "{result}");
            assert_eq!(result["code"], expected, "{result}");
            assert_eq!(result["mutation"], "none", "{result}");
            assert_eq!(
                snapshot(&root),
                before,
                "refusal changed consumer or evidence"
            );
        }
    };
    let resolved_path = root.join(report["conflicts"][0]["resolved"].as_str().unwrap());
    refused("CONFLICT_PENDING");
    let candidate_path = root.join(format!(
        ".agentfoundry/backups/{}/objects/{}",
        report["id"].as_str().unwrap(),
        identity.binary_digest
    ));
    let candidate_bytes = std::fs::read(&candidate_path).unwrap();
    std::fs::write(&candidate_path, b"tampered candidate").unwrap();
    refused("RECOVERY_BLOCKED");
    std::fs::write(&candidate_path, candidate_bytes).unwrap();
    for kind in ["BASE", "LOCAL", "UPSTREAM"] {
        let path = resolved_path.with_file_name(kind);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, b"tampered").unwrap();
        refused("INTEGRITY_FAILURE");
        std::fs::write(path, bytes).unwrap();
    }
    let control = root.join(".agentfoundry/runtime/resolution.json");
    let control_bytes = std::fs::read(&control).unwrap();
    let mut control_value: serde_json::Value = serde_json::from_slice(&control_bytes).unwrap();
    control_value["target"]["binary_digest"] = serde_json::json!("0".repeat(64));
    std::fs::write(&control, serde_json::to_vec(&control_value).unwrap()).unwrap();
    refused("INTEGRITY_FAILURE");
    std::fs::write(&control, control_bytes).unwrap();
    let plan_path = resolved_path
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("plan.json");
    let plan_bytes = std::fs::read(&plan_path).unwrap();
    std::fs::write(&plan_path, b"{}").unwrap();
    refused("INTEGRITY_FAILURE");
    std::fs::write(plan_path, plan_bytes).unwrap();
    let before_abort = snapshot(&root);
    run(&args[1], "update", &["--abort"]);
    assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
    let retained = |files: std::collections::BTreeMap<PathBuf, (String, u32)>| {
        files
            .into_iter()
            .filter(|(path, _)| {
                path != std::path::Path::new(".agentfoundry/runtime/resolution.json")
                    && !path.starts_with(".agentfoundry/runtime/resolutions")
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    assert_eq!(retained(snapshot(&root)), retained(before_abort));
    // Exercise stable session evidence without claiming release authenticity: the
    // synthetic metadata is locally constructed, not a published release.
    let mut stable = prepare();
    stable.target.source = "a".repeat(40);
    let state_op = stable
        .plan
        .operations
        .iter_mut()
        .find(|op| op.target == ".agentfoundry/state/manifest.json")
        .unwrap();
    let mut next: serde_json::Value =
        serde_json::from_slice(state_op.after.as_ref().unwrap()).unwrap();
    next["source"] = serde_json::json!(stable.target.source);
    state_op.after = Some(serde_json::to_vec_pretty(&next).unwrap());
    let candidate = std::fs::read(&args[1]).unwrap();
    stable.release = Some(agentfoundry::release::ReleaseMetadata {
        schema: "agentfoundry.release/v1".into(),
        version: stable.target.version.clone(),
        tag: format!("v{}", stable.target.version),
        source_commit: stable.target.source.clone(),
        payload_digest: stable.target.payload_digest.clone(),
        state_schema_min: 1,
        state_schema_max: 1,
        handoff_protocol: 1,
        assets: agentfoundry::release::PLATFORMS
            .iter()
            .map(|platform| {
                let name = format!("agentfoundry-{platform}");
                agentfoundry::release::Asset {
                    platform: (*platform).into(),
                    checksum_name: format!("{name}.sha256"),
                    name,
                    size: candidate.len() as u64,
                    sha256: fs::digest(&candidate),
                }
            })
            .collect(),
    });
    session::stage(&root, &stable).unwrap();
    session::status(&root).unwrap();
    let stable_control = std::fs::read(&control).unwrap();
    for change in ["missing", "source", "payload", "asset", "protocol"] {
        let mut value: serde_json::Value = serde_json::from_slice(&stable_control).unwrap();
        match change {
            "missing" => value["release"] = serde_json::Value::Null,
            "source" => value["release"]["source_commit"] = serde_json::json!("b".repeat(40)),
            "payload" => value["release"]["payload_digest"] = serde_json::json!("0".repeat(64)),
            "asset" => {
                let platform = agentfoundry::candidate::current_platform().unwrap();
                let asset = value["release"]["assets"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|asset| asset["platform"] == platform)
                    .unwrap();
                asset["sha256"] = serde_json::json!("0".repeat(64));
            }
            _ => value["release"]["handoff_protocol"] = serde_json::json!(2),
        }
        std::fs::write(&control, serde_json::to_vec(&value).unwrap()).unwrap();
        for dry in [true, false] {
            let before = snapshot(&root);
            let error = session::resume(&root, dry).unwrap_err();
            assert_eq!(error.code, "INTEGRITY_FAILURE", "{change}");
            assert_eq!(error.mutation, "none", "{change}");
            assert_eq!(snapshot(&root), before, "{change} changed evidence");
        }
    }
    std::fs::write(&control, stable_control).unwrap();
    session::abort(&root).unwrap();
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
    std::fs::write(root.join(actual), [0xff]).unwrap();
    refused("INVALID_STATE");
    std::fs::remove_file(root.join(actual)).unwrap();
    refused("INVALID_STATE");
    std::fs::write(root.join(actual), resolved_bytes).unwrap();
    if block {
        let mut oversized = b"<!-- AGENTFOUNDRY:BEGIN -->\n".to_vec();
        let end = b"\n<!-- AGENTFOUNDRY:END -->\n";
        oversized.resize(8 * 1024 * 1024 - end.len(), b'x');
        oversized.extend_from_slice(end);
        assert_eq!(oversized.len(), 8 * 1024 * 1024);
        std::fs::write(root.join(actual), oversized).unwrap();
        refused("INPUT_LIMIT_EXCEEDED");
        std::fs::write(root.join(actual), resolved_bytes).unwrap();
    }
    let control_bytes = std::fs::read(&control).unwrap();
    for change in ["extra", "missing", "duplicate"] {
        let mut value: serde_json::Value = serde_json::from_slice(&control_bytes).unwrap();
        let entries = value["frozen"]["entries"].as_array_mut().unwrap();
        match change {
            "extra" => entries
                .push(serde_json::json!({"path":"unknown-input", "presence":{"kind":"absent"}})),
            "missing" => {
                entries.pop();
            }
            _ => entries.push(entries[0].clone()),
        }
        std::fs::write(&control, serde_json::to_vec(&value).unwrap()).unwrap();
        refused("INVALID_STATE");
        std::fs::write(&control, &control_bytes).unwrap();
    }
    for path in [".agentfoundry/config.json", "docs/agentfoundry/README.md"] {
        let path = root.join(path);
        let bytes = std::fs::read(&path).unwrap();
        let mut changed = bytes.clone();
        changed.push(b' ');
        std::fs::write(&path, changed).unwrap();
        refused("WORKSPACE_DRIFT");
        std::fs::write(path, bytes).unwrap();
    }
    let absent_path = root.join(&absent_object);
    assert!(!absent_path.exists());
    std::fs::write(&absent_path, b"unexpected object").unwrap();
    refused("WORKSPACE_DRIFT");
    std::fs::remove_file(absent_path).unwrap();
    let preview_before = snapshot(&root);
    run(&args[1], "update", &["--continue", "--dry-run"]);
    assert_eq!(
        snapshot(&root),
        preview_before,
        "preview changed consumer or evidence"
    );
    assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
    assert!(session::pending(&root).unwrap());
    assert_eq!(std::fs::read(root.join(actual)).unwrap(), resolved_bytes);
    let clean = root.join("AGENTS.md");
    let clean_bytes = std::fs::read(&clean).unwrap();
    std::fs::write(&clean, b"external drift\n").unwrap();
    refused("WORKSPACE_DRIFT");
    assert_eq!(std::fs::read(&clean).unwrap(), b"external drift\n");
    std::fs::write(&clean, clean_bytes).unwrap();
    if let Some(fault_binary) = std::env::var_os("AF_CHECK_UPDATE_RACE") {
        let ready = root.join(".agentfoundry/runtime/test-session-lock-ready");
        let release = root.join(".agentfoundry/runtime/test-session-lock-release");
        let mut first = Command::new(&fault_binary)
            .args(["update", "--continue", "--directory", &args[2], "--json"])
            .env("AGENTFOUNDRY_TEST_HOLD_SESSION_LOCK", "1")
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !ready.exists() {
            assert!(
                first.try_wait().unwrap().is_none(),
                "first update exited before lock barrier"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "session lock barrier timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let second = Command::new(&fault_binary)
            .args(["update", "--continue", "--directory", &args[2], "--json"])
            .output()
            .unwrap();
        let blocked: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
        assert_eq!(second.status.code(), Some(9), "{blocked}");
        assert_eq!(blocked["code"], "LOCK_BUSY");
        assert_eq!(blocked["mutation"], "none");
        assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
        assert_eq!(
            std::fs::read(root.join(".agentfoundry/state/manifest.json")).unwrap(),
            before
        );
        assert!(session::pending(&root).unwrap());
        std::fs::write(&release, b"release").unwrap();
        let first = first.wait_with_output().unwrap();
        assert!(
            first.status.success(),
            "{}",
            String::from_utf8_lossy(&first.stdout)
        );
        assert!(!session::pending(&root).unwrap());
        assert_eq!(
            std::fs::read(root.join(target)).unwrap(),
            if block {
                agentfoundry::domain::patch_block(&local, resolved_bytes, "AGENTFOUNDRY").unwrap()
            } else {
                resolved_bytes.to_vec()
            }
        );
        assert_eq!(
            fs::digest(&std::fs::read(root.join(agentfoundry::binary_path())).unwrap()),
            identity.binary_digest
        );
        std::fs::remove_file(ready).unwrap();
        std::fs::remove_file(release).unwrap();
        println!("overlapping update continuation passed");
        return;
    }
    if let Some(checkpoint) = std::env::var_os("AF_CHECK_CRASH") {
        let crash_binary = std::env::var("AF_CHECK_CRASH_BINARY").unwrap();
        let interrupted = Command::new(crash_binary)
            .args(["update", "--continue", "--directory", &args[2], "--json"])
            .env("AGENTFOUNDRY_TEST_CRASH", &checkpoint)
            .output()
            .unwrap();
        assert_eq!(interrupted.status.code(), Some(86));
        assert!(session::pending(&root).unwrap());
        if std::env::var_os("AF_CHECK_LIST").is_some() {
            let journal: serde_json::Value = serde_json::from_slice(
                &std::fs::read(root.join(".agentfoundry/runtime/transaction.json")).unwrap(),
            )
            .unwrap();
            for op in journal["plan"]["operations"].as_array().unwrap() {
                if op["before"] != op["after"] {
                    println!("TARGET {}", op["target"].as_str().unwrap());
                }
            }
            return;
        }
        if let Some(rollback) = std::env::var_os("AF_CHECK_ROLLBACK") {
            let interrupted = Command::new(std::env::var("AF_CHECK_CRASH_BINARY").unwrap())
                .args(["recover", "--directory", &args[2], "--json"])
                .env("AGENTFOUNDRY_TEST_CRASH", rollback)
                .output()
                .unwrap();
            assert_eq!(interrupted.status.code(), Some(86));
            assert!(session::pending(&root).unwrap());
            let recovered = run(&args[1], "recover", &[]);
            assert_eq!(recovered["data"]["recovery"], "rolled_back");
            assert_eq!(std::fs::read(root.join(target)).unwrap(), local);
            assert_eq!(
                std::fs::read(root.join(".agentfoundry/state/manifest.json")).unwrap(),
                before
            );
        }
        if checkpoint == "core-committed-durable" {
            run(&args[1], "recover", &[]);
            let binary_path = root.join(agentfoundry::binary_path());
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
                "RECOVERY_REQUIRED"
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
        fs::digest(&std::fs::read(root.join(agentfoundry::binary_path())).unwrap()),
        identity.binary_digest
    );
    println!("real two-build stage/marker refusal/abort/preview/continue passed");
}
