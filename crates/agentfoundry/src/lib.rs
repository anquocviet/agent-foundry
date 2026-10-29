pub mod activation;
pub mod application;
pub mod candidate;
pub mod config;
pub mod domain;
pub mod infrastructure;
pub mod interface;
pub mod merge;
pub mod migration;
pub mod models;
pub mod payload_identity;
pub mod process;
pub mod release;
pub mod release_client;
pub mod session;
pub mod transaction_store;
pub mod update;
use infrastructure as fs;
use models::{Distribution, InstallationState};
use std::path::{Path, PathBuf};
include!(concat!(env!("OUT_DIR"), "/payload.rs"));
pub type Result<T, E = Failure> = std::result::Result<T, E>;
#[derive(Debug)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub mutation: String,
}
impl Failure {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            mutation: "none".into(),
        }
    }
}
impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self::new("IO_ERROR", e.to_string())
    }
}
impl From<serde_json::Error> for Failure {
    fn from(e: serde_json::Error) -> Self {
        Self::new("INVALID_STATE", e.to_string())
    }
}
pub fn unique_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{:x}{:x}{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}
const STATE: &str = ".agentfoundry/state/manifest.json";
fn load_state(root: &Path) -> Result<Option<InstallationState>> {
    let Some(bytes) = fs::read(root, STATE, fs::CONTROL_LIMIT)? else {
        return Ok(None);
    };
    let s = InstallationState::parse(&bytes)?;
    for e in &s.entries {
        let b = fs::read(
            root,
            &format!(".agentfoundry/state/objects/{}", e.sha256),
            8 * 1024 * 1024,
        )?
        .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "baseline object missing"))?;
        if fs::digest(&b) != e.sha256 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "baseline object digest mismatch",
            ));
        }
    }
    Ok(Some(s))
}
pub fn root(path: &str) -> Result<PathBuf> {
    let path = PathBuf::from(path);
    if path.exists() {
        return Ok(path.canonicalize()?);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| Failure::new("UNSAFE_PATH", "invalid root"))?;
    Ok(parent.canonicalize()?.join(name))
}
fn operation(
    root: &Path,
    target: &str,
    after: Vec<u8>,
    mode: u32,
) -> Result<application::Operation> {
    let limit = if target.starts_with("scripts/bin/") {
        128 * 1024 * 1024
    } else if matches!(target, STATE | ".agentfoundry/config.json") {
        fs::CONTROL_LIMIT as usize
    } else {
        8 * 1024 * 1024
    };
    if after.len() > limit {
        return Err(Failure::new(
            "INPUT_LIMIT_EXCEEDED",
            format!("planned output exceeds {limit} bytes: {target}"),
        ));
    }
    let before = fs::read(root, target, limit as u64)?;
    let mode = existing_mode(root, target, mode)?;
    Ok(application::Operation {
        target: target.into(),
        before,
        after: Some(after),
        mode,
    })
}
fn existing_mode(root: &Path, target: &str, default: u32) -> Result<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::symlink_metadata(fs::safe(root, target)?) {
            Ok(m) => Ok(m.permissions().mode() & 0o777),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(default),
            Err(e) => Err(e.into()),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (root, target);
        Ok(default)
    }
}
pub fn install_plan(
    root: &Path,
    args: &interface::Args,
) -> Result<(application::Plan, Vec<String>)> {
    if application::pending(root)? {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "run recover before planning",
        ));
    }
    let dist = Distribution::parse(PAYLOAD_MANIFEST.as_bytes())?;
    let state_preimage = fs::read(root, STATE, fs::CONTROL_LIMIT)?;
    let installed = load_state(root)?;
    let old_config = fs::read(root, ".agentfoundry/config.json", fs::CONTROL_LIMIT)?;
    if installed.is_none() && old_config.is_some() {
        return Err(Failure::new(
            "INVALID_STATE",
            "installation config exists without manifest; preserve state and recover instead of adopting again",
        ));
    }
    let mut selected = match &old_config {
        Some(bytes) => config::Config::parse(bytes)?,
        None => config::Config::selected(false, false),
    };
    if args.claude && !selected.adapters.iter().any(|a| a == "claude") {
        selected.adapters.push("claude".into());
    }
    if args.wisdom && !selected.addons.iter().any(|a| a == "engineering-wisdom") {
        selected.addons.push("engineering-wisdom".into());
    }
    if installed
        .as_ref()
        .is_some_and(|s| s.payload_digest != PAYLOAD_DIGEST)
    {
        return Err(Failure::new(
            "INVALID_STATE",
            "installed distribution differs; use update",
        ));
    }
    if fs::read(root, STATE, fs::CONTROL_LIMIT)? != state_preimage {
        return Err(Failure::new(
            "WORKSPACE_DRIFT",
            "installation changed while reading state",
        ));
    }
    let mut ops = Vec::new();
    let mut tracked = Vec::new();
    let mut warnings = Vec::new();
    for e in dist.entries {
        if e.addon.is_some()
            && !selected.addons.iter().any(|a| a == "engineering-wisdom")
            && !installed
                .as_ref()
                .is_some_and(|s| s.entries.iter().any(|i| i.logical_id == e.logical_id))
        {
            continue;
        }
        if e.adapter.is_some()
            && !selected.adapters.iter().any(|a| a == "claude")
            && !installed
                .as_ref()
                .is_some_and(|s| s.entries.iter().any(|i| i.logical_id == e.logical_id))
        {
            continue;
        }
        let bytes = PAYLOAD
            .iter()
            .find(|(s, _)| *s == e.source)
            .map(|(_, b)| *b)
            .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "embedded entry missing"))?;
        if fs::digest(bytes) != e.sha256 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "embedded digest mismatch",
            ));
        }
        let before = fs::read(root, &e.target, 8 * 1024 * 1024)?;
        let known = installed.as_ref().is_some_and(|s| {
            s.entries
                .iter()
                .any(|i| i.logical_id == e.logical_id && i.target == e.target)
        });
        let mut after = bytes.to_vec();
        if e.ownership == models::Ownership::ManagedBlock {
            let name = if e.target == "CLAUDE.md" {
                "AGENTFOUNDRY:CLAUDE"
            } else {
                "AGENTFOUNDRY"
            };
            after = domain::patch_block(before.as_deref().unwrap_or_default(), bytes, name)
                .map_err(|e| Failure::new("INVALID_STATE", e))?;
        }
        if known
            && !args.replace
            && !(args.refresh && e.ownership == models::Ownership::ManagedBlock)
        {
            after = before.clone().ok_or_else(|| {
                Failure::new(
                    "INVALID_STATE",
                    format!("missing managed target: {}", e.target),
                )
            })?;
        } else if before.is_some()
            && before.as_ref() != Some(&after)
            && !known
            && !args.replace
            && !(args.refresh && e.ownership == models::Ownership::ManagedBlock)
        {
            if args.merge {
                warnings.push(format!("SKIPPED_CONSUMER_OWNED: {}", e.target));
                if e.target == "AGENTS.md" {
                    warnings.push("ENTRYPOINT_NOT_INTEGRATED: use --refresh-agent-shim".into());
                }
                ops.push(application::Operation {
                    target: e.target.clone(),
                    before: before.clone(),
                    after: before,
                    mode: existing_mode(root, &e.target, e.mode)?,
                });
                continue;
            }
            return Err(Failure::new(
                "INVALID_ARGUMENT",
                format!("existing target {}; choose --merge or --override", e.target),
            ));
        }
        let mut payload_operation = operation(root, &e.target, after, e.mode)?;
        payload_operation.before = before;
        ops.push(payload_operation);
        let object = format!(".agentfoundry/state/objects/{}", e.sha256);
        if !ops.iter().any(|op| op.target == object) {
            ops.push(operation(root, &object, bytes.to_vec(), 0o644)?);
        }
        tracked.push(e);
    }
    let binary = std::fs::read(std::env::current_exe()?)?;
    let binary_digest = fs::digest(&binary);
    let bin = if cfg!(windows) {
        "scripts/bin/agentfoundry.exe"
    } else {
        "scripts/bin/agentfoundry"
    };
    ops.push(operation(root, bin, binary, 0o755)?);
    let config_preimage = old_config.clone();
    let config = match old_config {
        Some(bytes) if config::Config::parse(&bytes)? == selected => bytes,
        _ => serde_json::to_vec_pretty(&selected)?,
    };
    let mut config_operation = operation(root, ".agentfoundry/config.json", config, 0o644)?;
    config_operation.before = config_preimage;
    ops.push(config_operation);
    let internal_preimage = fs::read(root, ".agentfoundry/.gitignore", 8 * 1024 * 1024)?;
    let internal_ignore = internal_preimage.as_deref().unwrap_or_default();
    let mut internal_after = internal_ignore.to_vec();
    for rule in [b"runtime/".as_slice(), b"backups/".as_slice()] {
        if !internal_ignore
            .split(|b| *b == b'\n')
            .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == rule)
        {
            if !internal_after.is_empty() && !internal_after.ends_with(b"\n") {
                internal_after.push(b'\n');
            }
            internal_after.extend_from_slice(rule);
            internal_after.push(b'\n');
        }
    }
    let mut internal_operation =
        operation(root, ".agentfoundry/.gitignore", internal_after, 0o644)?;
    internal_operation.before = internal_preimage;
    ops.push(internal_operation);
    let ignore_preimage = fs::read(root, ".gitignore", 8 * 1024 * 1024)?;
    let ignore = ignore_preimage.as_deref().unwrap_or_default();
    let mut ignore_after = ignore.to_vec();
    if !ignore
        .split(|b| *b == b'\n')
        .any(|l| l == b"/scripts/bin/agentfoundry*")
    {
        if !ignore_after.is_empty() && !ignore_after.ends_with(b"\n") {
            ignore_after.push(b'\n');
        }
        ignore_after.extend_from_slice(b"/scripts/bin/agentfoundry*\n");
    }
    let mut ignore_operation = operation(root, ".gitignore", ignore_after, 0o644)?;
    ignore_operation.before = ignore_preimage;
    ops.push(ignore_operation);
    let mut state = InstallationState {
        schema: "agentfoundry.installation/v1".into(),
        generation: installed.as_ref().map(|s| s.generation).unwrap_or(1),
        version: env!("CARGO_PKG_VERSION").into(),
        source: env!("AF_SOURCE_ID").into(),
        payload_digest: PAYLOAD_DIGEST.into(),
        entries: tracked,
        binary_digest,
    };
    let proposed_state_bytes = serde_json::to_vec_pretty(&state)?;
    let state_changed = state_preimage.as_ref() != Some(&proposed_state_bytes);
    if installed.is_some() && (state_changed || ops.iter().any(|op| op.before != op.after)) {
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| Failure::new("INVALID_STATE", "installation generation overflow"))?;
    }
    let mut state_operation = operation(root, STATE, serde_json::to_vec_pretty(&state)?, 0o644)?;
    state_operation.before = state_preimage;
    ops.push(state_operation);
    let mut plan = application::Plan {
        schema: "agentfoundry.plan/v1".into(),
        root: root.to_string_lossy().into(),
        root_identity: fs::root_identity(root)?,
        snapshot: None,
        payload_digest: PAYLOAD_DIGEST.into(),
        operations: ops,
        choices: application::PlanChoices {
            merge: args.merge,
            replace: args.replace,
            refresh: args.refresh,
            wisdom: args.wisdom,
            claude: args.claude,
        },
    };
    plan.freeze(root)?;
    Ok((plan, warnings))
}
pub fn diagnostics(root: &Path, doctor: bool) -> Result<serde_json::Value> {
    if application::pending(root)? {
        if !doctor {
            let preview = application::recover(root, true);
            return Ok(match preview {
                Ok(outcome) => {
                    serde_json::json!({"transaction_pending":true,"recovery_preview":outcome,"next_action":"Run recover; status never performs recovery","latest":"Unknown"})
                }
                Err(error) => {
                    serde_json::json!({"transaction_pending":true,"recovery_error":{"code":error.code,"message":error.message},"next_action":"Preserve journal/backups and inspect recover diagnostics","latest":"Unknown"})
                }
            });
        }
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "journal pending; run recover",
        ));
    }
    if crate::activation::pending(root)? {
        if doctor {
            return Err(Failure::new(
                "RECOVERY_REQUIRED",
                "activation pending; run recover",
            ));
        }
        return Ok(
            serde_json::json!({"activation_pending":true,"next_action":"Run recover; unknown executable bytes are preserved","latest":"Unknown"}),
        );
    }
    let resolution = if crate::session::pending(root)? {
        Some(crate::session::status(root)?)
    } else {
        None
    };
    let Some(s) = load_state(root)? else {
        if fs::read(root, ".agentfoundry/config.json", fs::CONTROL_LIMIT)?.is_some() {
            return Err(Failure::new(
                "INVALID_STATE",
                "installation config exists but manifest is missing; preserve control state",
            ));
        }
        return Ok(serde_json::json!({"installed":false,"checks":[],"latest":"Unknown"}));
    };
    let config_bytes = fs::read(root, ".agentfoundry/config.json", fs::CONTROL_LIMIT)?
        .ok_or_else(|| Failure::new("INVALID_STATE", "installed config missing"))?;
    let selected = config::Config::parse(&config_bytes)?;
    let mut checks = Vec::new();
    if doctor {
        for (capability, programs) in [
            ("git", &["git"][..]),
            ("python", &["python3", "python"][..]),
        ] {
            let mut observation = None;
            for program in programs {
                let mut command = std::process::Command::new(program);
                if capability == "python" {
                    command.args(["-I", "-B"]);
                }
                command.arg("--version");
                match crate::process::capture(
                    &mut command,
                    65536,
                    std::time::Duration::from_secs(5),
                ) {
                    Ok(output) if output.status.success() => {
                        let bytes = if output.stdout.is_empty() {
                            output.stderr
                        } else {
                            output.stdout
                        };
                        observation = Some(
                            serde_json::json!({"capability":capability,"result":"Unknown","observed_version":String::from_utf8_lossy(&bytes).trim(),"reason":"executable responds to version probe; minimum-version and workflow capability not yet verified"}),
                        );
                        break;
                    }
                    Err(error) if error.code == "MISSING_CAPABILITY" => continue,
                    _ => {
                        observation = Some(
                            serde_json::json!({"capability":capability,"result":"Unknown","reason":"version probe failed or exceeded output/lifetime limit"}),
                        );
                    }
                }
            }
            checks.push(observation.unwrap_or_else(|| serde_json::json!({"capability":capability,"result":"Unknown","reason":"executable unavailable; Git merge/evidence and Python evidence workflows require their respective dependency; offline maintenance remains available"})));
        }
    }
    let mut local = Vec::new();
    let mut missing = Vec::new();
    for e in &s.entries {
        match fs::read(root, &e.target, 8 * 1024 * 1024)? {
            None => missing.push(e.target.clone()),
            Some(b) => {
                if e.ownership == models::Ownership::ManagedFile && fs::digest(&b) != e.sha256 {
                    local.push(e.target.clone());
                }
                if e.ownership == models::Ownership::ManagedBlock {
                    let name = if e.target == "CLAUDE.md" {
                        "AGENTFOUNDRY:CLAUDE"
                    } else {
                        "AGENTFOUNDRY"
                    };
                    let block = domain::block_content(&b, name)
                        .map_err(|error| Failure::new("INVALID_STATE", error))?
                        .ok_or_else(|| {
                            Failure::new(
                                "INVALID_STATE",
                                format!("managed marker block missing: {}", e.target),
                            )
                        })?;
                    let baseline = fs::read(
                        root,
                        &format!(".agentfoundry/state/objects/{}", e.sha256),
                        8 * 1024 * 1024,
                    )?
                    .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "block baseline missing"))?;
                    if block != baseline.strip_suffix(b"\n").unwrap_or(&baseline) {
                        local.push(e.target.clone());
                    }
                }
            }
        }
    }
    let bin = if cfg!(windows) {
        "scripts/bin/agentfoundry.exe"
    } else {
        "scripts/bin/agentfoundry"
    };
    let binary_ok =
        fs::read(root, bin, 128 * 1024 * 1024)?.is_some_and(|b| fs::digest(&b) == s.binary_digest);
    #[cfg(unix)]
    let binary_executable = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(fs::safe(root, bin)?)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    };
    #[cfg(not(unix))]
    let binary_executable = binary_ok;
    if doctor && (!missing.is_empty() || !binary_ok || !binary_executable) {
        return Err(Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "missing managed paths: {missing:?}; binary identity valid: {binary_ok}; executable permission: {binary_executable}"
            ),
        ));
    }
    let entrypoint_integrated = s.entries.iter().any(|entry| {
        entry.target == "AGENTS.md" && entry.ownership == models::Ownership::ManagedBlock
    });
    Ok(
        serde_json::json!({"installed":true,"version":s.version,"source":s.source,"payload_digest":s.payload_digest,"generation":s.generation,"adapters":selected.adapters,"addons":selected.addons,"checks":checks,"running_executable":{"version":env!("CARGO_PKG_VERSION"),"source":env!("AF_SOURCE_ID"),"payload_digest":PAYLOAD_DIGEST},"local_modifications":local,"missing_paths":missing,"binary_identity_valid":binary_ok,"binary_executable":binary_executable,"entrypoint_integrated":entrypoint_integrated,"warnings":if entrypoint_integrated {vec![]} else {vec!["ENTRYPOINT_NOT_INTEGRATED: use install --refresh-agent-shim"]},"latest":"Unknown","platform_verification":"Linux development only"}),
    ).map(|mut report: serde_json::Value| {
        report["resolution_pending"] = serde_json::json!(resolution.is_some());
        report["session"] = serde_json::json!(resolution);
        report
    })
}
pub fn inspect(root: &Path) -> Result<serde_json::Value> {
    let mut sources = Vec::new();
    for name in [
        "AGENTS.md",
        "README.md",
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "Makefile",
        "docs/README.md",
        "docs/ARCHITECTURE.md",
    ] {
        if let Some(b) = fs::read(root, name, 8 * 1024 * 1024)? {
            let mut row = serde_json::json!({"path":name,"classification":"Observed","sha256":fs::digest(&b)});
            if name == "package.json" {
                let v: serde_json::Value = serde_json::from_slice(&b)?;
                if !v.is_object()
                    || v.get("scripts").is_some_and(|scripts| {
                        !scripts.is_object()
                            || scripts.as_object().is_some_and(|commands| {
                                commands.values().any(|value| !value.is_string())
                            })
                    })
                {
                    return Err(Failure::new(
                        "INVALID_STATE",
                        "package.json must be an object with string-valued scripts",
                    ));
                }
                row["declared_commands"] =
                    v.get("scripts").cloned().unwrap_or(serde_json::json!({}));
            }
            sources.push(row);
        }
    }
    Ok(
        serde_json::json!({"sources":sources,"business_policy":"Unknown","commands_executed":false,"network":false}),
    )
}
