use crate::{
    Failure, Result, infrastructure as fs,
    release::ReleaseMetadata,
    update::{self, CandidateIdentity},
};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn export() -> Result<serde_json::Value> {
    let exe = std::env::current_exe()?;
    let meta = std::fs::metadata(&exe)?;
    if meta.len() > 128 * 1024 * 1024 {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "current executable exceeds 128MiB limit",
        ));
    }
    let binary = std::fs::read(&exe)?;
    let binary_digest = fs::digest(&binary);

    let mut total_payload_bytes = 0usize;
    let mut payload_entries = Vec::new();
    for (source, bytes) in crate::PAYLOAD {
        total_payload_bytes += bytes.len();
        if total_payload_bytes > 16 * 1024 * 1024 {
            return Err(Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "exported payload exceeds 16MiB limit",
            ));
        }
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in *bytes {
            use std::fmt::Write;
            let _ = write!(&mut hex, "{b:02x}");
        }
        payload_entries.push(serde_json::json!({
            "source": *source,
            "bytes": hex,
        }));
    }

    Ok(serde_json::json!({
        "schema": "agentfoundry.exported-distribution/v1",
        "version": env!("CARGO_PKG_VERSION"),
        "source": env!("AF_SOURCE_ID"),
        "payload_digest": crate::PAYLOAD_DIGEST,
        "binary_digest": binary_digest,
        "manifest": crate::PAYLOAD_MANIFEST,
        "payload": payload_entries,
    }))
}

pub fn current_platform() -> Result<&'static str> {
    #[cfg(all(target_arch = "aarch64", target_os = "macos"))]
    return Ok("aarch64-apple-darwin");
    #[cfg(all(target_arch = "x86_64", target_os = "macos"))]
    return Ok("x86_64-apple-darwin");
    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
    return Ok("x86_64-unknown-linux-gnu");
    #[cfg(all(target_arch = "aarch64", target_os = "linux"))]
    return Ok("aarch64-unknown-linux-gnu");
    #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
    return Ok("x86_64-pc-windows-msvc");

    #[allow(unreachable_code)]
    Err(Failure::new(
        "MISSING_CAPABILITY",
        "unsupported current platform for release update",
    ))
}

struct TempCandidate {
    dir: PathBuf,
    binary_path: PathBuf,
}

impl Drop for TempCandidate {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn parse_semver(version: &str) -> std::result::Result<[u64; 3], String> {
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err("release version must be exact stable semver".into());
    }
    let mut out = [0; 3];
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty()
            || !part.bytes().all(|byte| byte.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err("version components must be non-empty ASCII decimal".into());
        }
        out[index] = part.parse::<u64>().map_err(|e| e.to_string())?;
    }
    Ok(out)
}

type ExportedCandidate = (Vec<u8>, Vec<(String, Vec<u8>)>, CandidateIdentity);
fn query_candidate(
    candidate_path: &Path,
    metadata: &ReleaseMetadata,
    asset_sha256: &str,
) -> Result<ExportedCandidate> {
    let mut cmd = std::process::Command::new(candidate_path);
    cmd.arg("distribution").arg("--json");
    cmd.env("AF_INVOCATION_DEPTH", "1");
    cmd.env("AF_NO_RECURSIVE", "1");

    let output = crate::process::capture(&mut cmd, 32 * 1024 * 1024, Duration::from_secs(30))?;
    if !output.status.success() {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "candidate distribution query failed with non-zero exit status",
        ));
    }

    let raw: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let data =
        if raw.get("schema").and_then(|s| s.as_str()) == Some("agentfoundry.command-result/v1") {
            if raw.get("status").and_then(|s| s.as_str()) != Some("ok") {
                return Err(Failure::new(
                    "INTEGRITY_FAILURE",
                    "candidate distribution returned non-ok status",
                ));
            }
            raw.get("data").ok_or_else(|| {
                Failure::new("INTEGRITY_FAILURE", "candidate command-result missing data")
            })?
        } else {
            &raw
        };

    let version = data["version"]
        .as_str()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "candidate version missing"))?;
    if version != metadata.version {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "candidate version mismatch: expected {}, got {}",
                metadata.version, version
            ),
        ));
    }

    let source = data["source"]
        .as_str()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "candidate source missing"))?;
    if source != metadata.source_commit {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "candidate source identity mismatch: expected {}, got {}",
                metadata.source_commit, source
            ),
        ));
    }

    let payload_digest = data["payload_digest"]
        .as_str()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "candidate payload_digest missing"))?;
    if payload_digest != metadata.payload_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "candidate payload_digest mismatch: expected {}, got {}",
                metadata.payload_digest, payload_digest
            ),
        ));
    }

    let binary_digest = data["binary_digest"]
        .as_str()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "candidate binary_digest missing"))?;
    if binary_digest != asset_sha256 {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "candidate binary_digest mismatch: expected {}, got {}",
                asset_sha256, binary_digest
            ),
        ));
    }

    let manifest_bytes = match &data["manifest"] {
        serde_json::Value::String(s) => s.as_bytes().to_vec(),
        other => serde_json::to_vec(other)?,
    };

    let dist = crate::models::Distribution::parse(&manifest_bytes)?;
    let manifest_val: serde_json::Value = serde_json::from_slice(&manifest_bytes)?;
    let entries_arr = manifest_val["entries"]
        .as_array()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "manifest entries missing"))?;
    let canonical_digest = crate::payload_identity::canonical_digest(entries_arr)
        .map_err(|e| Failure::new("INTEGRITY_FAILURE", e.to_string()))?;
    if canonical_digest != metadata.payload_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "manifest canonical payload digest does not match release metadata",
        ));
    }

    let payload_arr = data["payload"]
        .as_array()
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "candidate payload array missing"))?;
    let mut total_payload_bytes = 0usize;
    let mut payload_map = std::collections::BTreeMap::new();
    for item in payload_arr {
        let src = item["source"]
            .as_str()
            .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "payload entry source missing"))?;
        let hex = item["bytes"]
            .as_str()
            .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "payload entry bytes missing"))?;
        if !hex.is_ascii() || hex.len() % 2 != 0 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "invalid hex length in payload entry",
            ));
        }
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<std::result::Result<Vec<u8>, _>>()
            .map_err(|_| Failure::new("INTEGRITY_FAILURE", "invalid hex digit in payload entry"))?;
        total_payload_bytes += bytes.len();
        if total_payload_bytes > 16 * 1024 * 1024 {
            return Err(Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "candidate payload exceeds 16MiB limit",
            ));
        }
        if payload_map.insert(src.to_string(), bytes).is_some() {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "duplicate payload source",
            ));
        }
    }

    let mut payload_entries = Vec::new();
    for entry in &dist.entries {
        let bytes = payload_map.get(&entry.source).ok_or_else(|| {
            Failure::new(
                "INTEGRITY_FAILURE",
                format!("entry source {} missing in payload", entry.source),
            )
        })?;
        if fs::digest(bytes) != entry.sha256 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                format!("digest mismatch for payload entry {}", entry.source),
            ));
        }
        payload_entries.push((entry.source.clone(), bytes.clone()));
    }

    let identity = CandidateIdentity {
        version: metadata.version.clone(),
        source: metadata.source_commit.clone(),
        payload_digest: metadata.payload_digest.clone(),
        binary_digest: asset_sha256.to_string(),
    };

    Ok((manifest_bytes, payload_entries, identity))
}

pub fn prepare(root: &Path) -> Result<update::PreparedUpdate> {
    if cfg!(windows) {
        return Err(Failure::new(
            "MISSING_CAPABILITY",
            "Windows candidate binary replacement requires platform adapter; candidate activation is not supported",
        ));
    }

    if std::env::var_os("AF_NO_RECURSIVE").is_some() {
        return Err(Failure::new(
            "INVALID_STATE",
            "recursive candidate invocation detected",
        ));
    }

    if crate::application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "recover interrupted transaction before update",
        ));
    }
    if crate::session::pending(root)? {
        return Err(Failure::new(
            "SESSION_EXISTS",
            "pending resolution retained; use update --continue or --abort",
        ));
    }

    let state = crate::load_state(root)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "repository is not installed"))?;

    let metadata = crate::release_client::resolve()?;

    let next_v =
        parse_semver(&metadata.version).map_err(|e| Failure::new("INTEGRITY_FAILURE", e))?;
    let cur_v = parse_semver(&state.version).map_err(|e| Failure::new("INVALID_STATE", e))?;
    if next_v < cur_v {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "release downgrade rejected",
        ));
    }
    if next_v == cur_v
        && (metadata.source_commit != state.source
            || metadata.payload_digest != state.payload_digest)
    {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "immutable same-version release identity violation",
        ));
    }

    let platform = current_platform()?;
    let (asset, binary_bytes) = crate::release_client::download_candidate(&metadata, platform)?;

    if next_v == cur_v && asset.sha256 != state.binary_digest {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "immutable same-version release identity violation",
        ));
    }

    let temp_dir =
        std::env::temp_dir().join(format!("agentfoundry-candidate-{}", crate::unique_id()));
    if temp_dir.starts_with(root) {
        return Err(Failure::new(
            "UNSAFE_PATH",
            "temp directory is inside target repository",
        ));
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&temp_dir)?;

    let bin_name = if cfg!(windows) {
        "agentfoundry.exe"
    } else {
        "agentfoundry"
    };
    let binary_path = temp_dir.join(bin_name);
    std::fs::write(&binary_path, &binary_bytes)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary_path, std::fs::Permissions::from_mode(0o755))?;
    }

    let temp_candidate = TempCandidate {
        dir: temp_dir,
        binary_path,
    };

    let written = std::fs::read(&temp_candidate.binary_path)?;
    if written.len() != binary_bytes.len() || fs::digest(&written) != asset.sha256 {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            "written candidate binary failed verification",
        ));
    }

    let (manifest_bytes, payload_entries, identity) =
        query_candidate(&temp_candidate.binary_path, &metadata, &asset.sha256)?;

    let mut prepared = update::prepare_target(
        root,
        &manifest_bytes,
        &payload_entries,
        binary_bytes,
        identity,
    )?;
    prepared.release = Some(metadata);
    Ok(prepared)
}
