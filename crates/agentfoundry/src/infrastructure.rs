use crate::{Failure, Result, domain::RelativePath};
use fs2::FileExt;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub use crate::domain::CONTROL_LIMIT;
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn safe(root: &Path, relative: &str) -> Result<PathBuf> {
    RelativePath::new(relative).map_err(|e| Failure::new("UNSAFE_PATH", e))?;
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(Failure::new(
                "UNSAFE_PATH",
                "workspace root changed kind or became a symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let mut current = root.to_path_buf();
    let mut parts = relative.split('/').peekable();
    while let Some(part) = parts.next() {
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(m) => {
                if m.file_type().is_symlink() || !(m.is_file() || m.is_dir()) {
                    return Err(Failure::new(
                        "UNSAFE_PATH",
                        format!("unsupported path kind: {relative}"),
                    ));
                }
                if parts.peek().is_some() && !m.is_dir() {
                    return Err(Failure::new(
                        "UNSAFE_PATH",
                        format!("parent is not a directory: {relative}"),
                    ));
                }
                if m.is_dir() && parts.peek().is_some() {
                    match fs::symlink_metadata(current.join(".git")) {
                        Ok(_) => {
                            return Err(Failure::new(
                                "UNSAFE_PATH",
                                format!(
                                    "managed path crosses nested repository/submodule: {relative}"
                                ),
                            ));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    if m.is_file() && m.nlink() > 1 {
                        return Err(Failure::new(
                            "UNSAFE_PATH",
                            format!("hard-linked target: {relative}"),
                        ));
                    }
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(Failure::new("UNSAFE_PATH", "reparse point"));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(current)
}
pub fn read(root: &Path, relative: &str, limit: u64) -> Result<Option<Vec<u8>>> {
    let path = safe(root, relative)?;
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
        Ok(m) => {
            if !m.is_file() {
                return Err(Failure::new(
                    "UNSAFE_PATH",
                    format!("not a regular file: {relative}"),
                ));
            }
            if m.len() > limit {
                return Err(Failure::new("INPUT_LIMIT_EXCEEDED", relative));
            }
            let mut bytes = Vec::new();
            File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
            if bytes.len() as u64 > limit {
                return Err(Failure::new("INPUT_LIMIT_EXCEEDED", relative));
            }
            Ok(Some(bytes))
        }
    }
}

pub fn root_identity(root: &Path) -> Result<Option<String>> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Failure::new("UNSAFE_PATH", "root must be a directory"));
    }
    let identity = root.canonicalize()?.to_string_lossy().into_owned();
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}:{identity}", metadata.dev(), metadata.ino())
    };
    // ponytail: non-Unix identity is path-only; add native file IDs with the Windows adapter.
    Ok(Some(identity))
}

pub fn snapshot(
    root: &Path,
    generation: u64,
    paths: &[&str],
) -> Result<crate::models::WorkspaceSnapshot> {
    use crate::models::{Presence, SnapshotEntry, WorkspaceSnapshot};
    if paths.len() > 10000 {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "snapshot path limit exceeded",
        ));
    }
    let root_identity = root_identity(root)?
        .ok_or_else(|| Failure::new("WORKSPACE_DRIFT", "snapshot root disappeared"))?;
    safe(root, ".agentfoundry/state/manifest.json")?;
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        RelativePath::new(path).map_err(|error| Failure::new("UNSAFE_PATH", error))?;
        if !seen.insert(path.to_lowercase()) {
            return Err(Failure::new(
                "INVALID_STATE",
                "duplicate effective snapshot path",
            ));
        }
    }
    let mut paths = paths.to_vec();
    for control in [".agentfoundry/config.json", crate::STATE] {
        if !paths.contains(&control) {
            paths.push(control);
        }
    }
    if paths.len() > 10000 {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "snapshot path limit exceeded",
        ));
    }
    let mut entries = Vec::with_capacity(paths.len());
    let mut config_digest = None;
    let mut state_digest = None;
    for path in &paths {
        let presence = match read(
            root,
            path,
            if path.starts_with("scripts/bin/") {
                128 * 1024 * 1024
            } else if matches!(*path, crate::STATE | ".agentfoundry/config.json") {
                CONTROL_LIMIT
            } else {
                8 * 1024 * 1024
            },
        )? {
            None => Presence::Absent,
            Some(bytes) => {
                let digest = digest(&bytes);
                if *path == ".agentfoundry/config.json" {
                    crate::config::Config::parse(&bytes)?;
                    config_digest = Some(digest.clone());
                }
                if *path == crate::STATE {
                    let state = crate::models::InstallationState::parse(&bytes)?;
                    if state.generation != generation {
                        return Err(Failure::new(
                            "WORKSPACE_DRIFT",
                            "manifest schema/generation differs from snapshot request",
                        ));
                    }
                    state_digest = Some(digest.clone());
                }
                let default_mode = if *path == crate::binary_path() {
                    0o755
                } else {
                    0o644
                };
                Presence::File {
                    digest,
                    mode: crate::existing_mode(root, path, default_mode)?,
                }
            }
        };
        entries.push(SnapshotEntry {
            path: RelativePath::new(path).map_err(|error| Failure::new("UNSAFE_PATH", error))?,
            presence,
        });
    }
    let snapshot = WorkspaceSnapshot {
        schema: "agentfoundry.snapshot/v1".into(),
        root_identity,
        generation,
        config_digest,
        state_digest,
        entries,
    };
    snapshot
        .validate()
        .map_err(|error| Failure::new("INVALID_STATE", error))?;
    Ok(snapshot)
}

pub fn verify_snapshot(root: &Path, frozen: &crate::models::WorkspaceSnapshot) -> Result<()> {
    frozen
        .validate()
        .map_err(|error| Failure::new("INVALID_STATE", error))?;
    let paths = frozen
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    let current = snapshot(root, frozen.generation, &paths)?;
    if current.root_identity != frozen.root_identity
        || current.config_digest != frozen.config_digest
        || current.state_digest != frozen.state_digest
        || current.entries != frozen.entries
    {
        return Err(Failure::new(
            "WORKSPACE_DRIFT",
            "frozen workspace inputs changed",
        ));
    }
    Ok(())
}
pub fn atomic(root: &Path, relative: &str, bytes: &[u8], mode: u32) -> Result<()> {
    #[cfg(not(unix))]
    let _ = mode;
    let path = safe(root, relative)?;
    let parent = path
        .parent()
        .ok_or_else(|| Failure::new("UNSAFE_PATH", "missing parent"))?;
    fs::create_dir_all(parent)?;
    safe(root, relative)?;
    let temp = parent.join(format!(
        ".agentfoundry-{}-{}.tmp",
        std::process::id(),
        crate::unique_id()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(mode);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode))?;
    }
    drop(file);
    safe(root, relative)?;
    if let Err(e) = fs::rename(&temp, &path) {
        let _ = fs::remove_file(&temp);
        return Err(e.into());
    }
    sync_dir(parent)?;
    Ok(())
}
pub fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
pub fn remove(root: &Path, relative: &str) -> Result<()> {
    let path = safe(root, relative)?;
    match fs::remove_file(&path) {
        Ok(()) => {
            if let Some(p) = path.parent() {
                sync_dir(p)?;
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub struct Lock {
    file: File,
}
impl Lock {
    pub fn acquire(root: &Path) -> Result<Self> {
        let path = safe(root, ".agentfoundry/runtime/operation.lock")?;
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| Failure::new("UNSAFE_PATH", "lock parent"))?,
        )?;
        safe(root, ".agentfoundry/runtime/operation.lock")?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        file.try_lock_exclusive().map_err(|_| {
            Failure::new("LOCK_BUSY", "another operation holds the repository lock")
        })?;
        Ok(Self { file })
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn read_plan(path: &Path) -> Result<crate::application::Plan> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_file() {
        return Err(Failure::new("INVALID_STATE", "unsafe plan file"));
    }
    if meta.len() > CONTROL_LIMIT {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "saved plan exceeds control limit",
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(CONTROL_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CONTROL_LIMIT {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "saved plan exceeds control limit",
        ));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_plan(root: &Path, out: &Path, plan: &crate::application::Plan) -> Result<()> {
    let parent = out
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    if parent.starts_with(root) {
        return Err(Failure::new(
            "OUTPUT_INSIDE_TARGET",
            "plan output must be outside target",
        ));
    }
    let bytes = serde_json::to_vec_pretty(plan)?;
    if bytes.len() as u64 > CONTROL_LIMIT {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            "saved plan exceeds control limit; no output created",
        ));
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(out)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}
