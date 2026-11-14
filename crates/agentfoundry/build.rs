use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf};
#[path = "src/domain.rs"]
pub mod domain;
#[path = "src/payload_identity.rs"]
mod payload_identity;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?)
        .join("../..")
        .canonicalize()?;
    let raw = fs::read(root.join("payload/manifest.json"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&raw)?;
    if manifest.as_object().is_none_or(|object| {
        object
            .keys()
            .any(|key| !["schema", "entries"].contains(&key.as_str()))
    }) {
        return Err("unknown distribution manifest field".into());
    }
    let entries = manifest["entries"].as_array().ok_or("entries missing")?;
    if manifest["schema"].as_str() != Some("agentfoundry.distribution/v1") {
        return Err("unknown distribution schema".into());
    }
    let payload_digest =
        payload_identity::canonical_digest(entries).map_err(std::io::Error::other)?;
    let mut generated = String::from("pub static PAYLOAD: &[(&str, &[u8])] = &[\n");
    for e in entries {
        let source = e["source"].as_str().ok_or("source missing")?;
        domain::RelativePath::new(source).map_err(std::io::Error::other)?;
        let path = root.join(source).canonicalize()?;
        if !path.starts_with(&root) {
            return Err("payload source escapes source tree".into());
        }
        let bytes = fs::read(&path)?;
        let hash = format!("{:x}", Sha256::digest(&bytes));
        if e["sha256"].as_str() != Some(&hash) {
            return Err(format!("payload digest mismatch: {source}").into());
        }
        println!("cargo:rerun-if-changed={}", path.display());
        generated.push_str(&format!("({source:?}, include_bytes!({:?})),\n", path));
    }
    generated.push_str("];\n");
    generated.push_str(&format!(
        "pub const PAYLOAD_MANIFEST: &str = {:?};\npub const PAYLOAD_DIGEST: &str = {:?};\n",
        String::from_utf8(raw.clone())?,
        payload_digest
    ));
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("payload.rs"),
        generated,
    )?;
    println!(
        "cargo:rerun-if-changed={}",
        root.join("payload/manifest.json").display()
    );
    let mut source = Sha256::new();
    for name in [
        "Cargo.toml",
        "Cargo.lock",
        "rust-toolchain.toml",
        "crates/agentfoundry/Cargo.toml",
        "crates/agentfoundry/build.rs",
        "crates/agentfoundry/src/main.rs",
        "crates/agentfoundry/src/lib.rs",
        "crates/agentfoundry/src/domain.rs",
        "crates/agentfoundry/src/activation.rs",
        "crates/agentfoundry/src/application.rs",
        "crates/agentfoundry/src/infrastructure.rs",
        "crates/agentfoundry/src/interface.rs",
        "crates/agentfoundry/src/merge.rs",
        "crates/agentfoundry/src/migration.rs",
        "crates/agentfoundry/src/candidate.rs",
        "crates/agentfoundry/src/config.rs",
        "crates/agentfoundry/src/models.rs",
        "crates/agentfoundry/src/release.rs",
        "crates/agentfoundry/src/release_client.rs",
        "crates/agentfoundry/src/session.rs",
        "crates/agentfoundry/src/transaction_store.rs",
        "crates/agentfoundry/src/update.rs",
        "crates/agentfoundry/src/payload_identity.rs",
        "crates/agentfoundry/src/process.rs",
        "payload/manifest.json",
    ] {
        let path = root.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        source.update(name.as_bytes());
        source.update([0]);
        source.update(fs::read(path)?);
    }
    let fault_feature = env::var_os("CARGO_FEATURE_FAULT_INJECTION").is_some();
    source.update(if fault_feature {
        b"fault-injection=1".as_slice()
    } else {
        b"fault-injection=0".as_slice()
    });
    println!("cargo:rustc-env=AF_FAULT_INJECTION={fault_feature}");
    println!("cargo:rerun-if-env-changed=AF_STABLE_BUILD");
    if env::var_os("AF_STABLE_BUILD").is_some() {
        if fault_feature {
            return Err("stable builds cannot enable fault injection".into());
        }
        let version = env::var("CARGO_PKG_VERSION")?;
        let parts: Vec<_> = version.split('.').collect();
        if parts.len() != 3
            || parts.iter().any(|p| {
                p.is_empty()
                    || !p.bytes().all(|b| b.is_ascii_digit())
                    || (p.len() > 1 && p.starts_with('0'))
                    || p.parse::<u64>().is_err()
            })
        {
            return Err("stable build requires exact stable semver".into());
        }
        let git = |args: &[&str]| -> Result<String, Box<dyn std::error::Error>> {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&root)
                .output()?;
            if !out.status.success() {
                return Err("git verification failed".into());
            }
            Ok(String::from_utf8(out.stdout)?.trim().to_owned())
        };
        let commit = git(&["rev-parse", "HEAD"])?;
        if commit.len() != 40
            || !commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !git(&["status", "--porcelain", "--untracked-files=all"])?.is_empty()
            || git(&["rev-parse", &format!("refs/tags/v{version}^{{commit}}")])? != commit
        {
            return Err("stable build requires clean checkout at matching release tag".into());
        }
        println!("cargo:rustc-env=AF_SOURCE_ID={commit}");
        println!("cargo:rustc-env=AF_DISTRIBUTION=stable");
    } else {
        println!(
            "cargo:rustc-env=AF_SOURCE_ID=development-sha256:{:x}",
            source.finalize()
        );
        println!("cargo:rustc-env=AF_DISTRIBUTION=development");
    }
    Ok(())
}
