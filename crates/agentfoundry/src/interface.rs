use crate::{Failure, Result};
#[derive(Clone, Default)]
pub struct Args {
    pub command: String,
    pub directory: String,
    pub json: bool,
    pub dry: bool,
    pub merge: bool,
    pub replace: bool,
    pub refresh: bool,
    pub wisdom: bool,
    pub claude: bool,
    pub plan_out: Option<String>,
    pub apply_plan: Option<String>,
    pub offline: bool,
    pub apply: bool,
    pub check_updates: bool,
    pub resume: bool,
    pub abort: bool,
}
pub fn parse(raw: &[String]) -> Result<Args> {
    let mut a = Args {
        directory: ".".into(),
        ..Default::default()
    };
    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--directory" | "--plan-out" | "--apply-plan" => {
                let flag = &raw[i];
                i += 1;
                let v = raw
                    .get(i)
                    .ok_or_else(|| {
                        Failure::new("INVALID_ARGUMENT", format!("value missing for {flag}"))
                    })?
                    .clone();
                match flag.as_str() {
                    "--directory" => a.directory = v,
                    "--plan-out" => a.plan_out = Some(v),
                    _ => a.apply_plan = Some(v),
                }
            }
            "--json" => a.json = true,
            "--dry-run" => a.dry = true,
            "--apply" => a.apply = true,
            "--offline" => a.offline = true,
            "--check-updates" => a.check_updates = true,
            "--continue" => a.resume = true,
            "--abort" => a.abort = true,
            "--merge" => a.merge = true,
            "--override" | "--force" => a.replace = true,
            "--refresh-agent-shim" => a.refresh = true,
            "--with-engineering-wisdom" => a.wisdom = true,
            "--claude" => a.claude = true,
            "--yes" | "--no-color" => {}
            "--help" | "-h" => a.command = "help".into(),
            "--version" | "-V" => a.command = "version".into(),
            "harness" if a.command == "migrate" => a.command = "migrate harness".into(),
            s if !s.starts_with('-') && a.command.is_empty() => a.command = s.into(),
            s => {
                return Err(Failure::new(
                    "INVALID_ARGUMENT",
                    format!("unknown argument: {s}"),
                ));
            }
        }
        i += 1;
    }
    if a.command.is_empty() {
        a.command = "help".into();
    }
    if a.merge && a.replace {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "--merge conflicts with --override/--force",
        ));
    }
    if a.plan_out.is_some() && !a.dry {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "--plan-out requires --dry-run",
        ));
    }
    if a.apply_plan.is_some()
        && (a.plan_out.is_some() || a.merge || a.replace || a.refresh || a.wisdom || a.claude)
    {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "apply-plan cannot change frozen choices",
        ));
    }
    if a.apply && (a.command != "migrate harness" || a.dry || a.apply_plan.is_some()) {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "--apply requires migrate harness without --dry-run/--apply-plan",
        ));
    }
    if a.check_updates && (a.command != "status" || a.offline) {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "--check-updates requires status without --offline",
        ));
    }
    if (a.resume || a.abort)
        && (a.command != "update"
            || (a.resume && a.abort)
            || a.merge
            || a.replace
            || a.refresh
            || a.wisdom
            || a.claude
            || a.apply_plan.is_some()
            || a.plan_out.is_some()
            || (a.abort && a.dry))
    {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "--continue/--abort require update without conflicting policy flags; abort cannot be dry-run",
        ));
    }
    Ok(a)
}
pub fn run(a: &Args) -> Result<(serde_json::Value, Vec<String>, String)> {
    if a.command == "distribution" {
        return Ok((crate::candidate::export()?, vec![], "none".into()));
    }
    if a.command == "version" {
        return Ok((
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"source":env!("AF_SOURCE_ID"),"payload_digest":crate::PAYLOAD_DIGEST,"distribution":"development","fault_injection":env!("AF_FAULT_INJECTION") == "true"}),
            vec![],
            "none".into(),
        ));
    }
    if a.command == "help" {
        return Ok((
            serde_json::json!({"usage":"agentfoundry inspect|install|update|status|doctor|recover|migrate harness --directory PATH [--json]","status":"--check-updates: explicitly observe pinned GitHub release metadata without installing", "install":"--dry-run --merge|--override --refresh-agent-shim --claude --with-engineering-wisdom --plan-out OUTSIDE_TARGET --apply-plan FILE","update":"--offline [--dry-run] [--plan-out OUTSIDE_TARGET | --apply-plan FILE]: validate exact installed embedded build and preserve local edits; cross-release upgrade unavailable","migration":"migrate harness: read-only inventory by default; --dry-run previews; --apply explicitly migrates; --plan-out requires --dry-run; --apply-plan reauthorizes frozen outputs","mutation":"install/update/recover/migration apply use journaled transactions; inspect/status/doctor/migration inventory offline read-only","exits":{"0":"OK","1":"IO_ERROR","2":"INVALID_ARGUMENT","3":"CONFLICT_PENDING","4":"WORKSPACE_DRIFT","5":"UNSAFE_PATH/INVALID_STATE/INTEGRITY_FAILURE","6":"MISSING_CAPABILITY","7":"RELEASE_UNAVAILABLE","8":"RECOVERY_REQUIRED/BLOCKED","9":"LOCK_BUSY"},"recovery":"docs/recovery.md","development_limits":"cross-release update/conflict sessions/self-update/release bootstrap not implemented"}),
            vec![],
            "none".into(),
        ));
    }
    let mut recovery_mutation = "none";
    let result = (|| {
        let root = crate::root(&a.directory)?;
        let data = match a.command.as_str() {
            "inspect" => crate::inspect(&root)?,
            "status" => {
                let mut status = crate::diagnostics(&root, false)?;
                if a.check_updates {
                    status["release_observation"] = crate::release_client::check()?;
                }
                status
            }
            "doctor" => crate::diagnostics(&root, true)?,
            "recover" => {
                #[cfg(windows)]
                if crate::candidate::is_worker_mode() {
                    crate::candidate::verify_worker_authorization(&root)?;
                }
                if !crate::application::pending(&root)? && crate::activation::pending(&root)? {
                    let outcome = crate::activation::recover(&root, a.dry)?;
                    let mutation = if !a.dry { "applied" } else { "none" };
                    return Ok((outcome, vec![], mutation.into()));
                }
                if !crate::application::pending(&root)? && crate::session::pending(&root)? {
                    let outcome = crate::session::recover_activation(&root, a.dry)?;
                    let mutation = if outcome["recovery"] == "activation_repaired" {
                        "applied"
                    } else {
                        "none"
                    };
                    return Ok((outcome, vec![], mutation.into()));
                }
                if a.dry || !crate::application::pending(&root)? {
                    crate::application::recover(&root, true)?
                } else {
                    let _lock = crate::infrastructure::Lock::acquire(&root)?;
                    let outcome = crate::application::recover(&root, false)?;
                    let mutation = if outcome["recovery"] == "rolled_back" {
                        "rolled_back"
                    } else {
                        "applied"
                    };
                    return Ok((outcome, vec![], mutation.into()));
                }
            }
            "install" | "init" => {
                if crate::activation::pending(&root)? {
                    return Err(Failure::new(
                        "RECOVERY_REQUIRED",
                        "activation pending; run recover",
                    ));
                }
                if !a.dry && crate::application::pending(&root)? {
                    let _lock = crate::infrastructure::Lock::acquire(&root)?;
                    let outcome = crate::application::recover(&root, false)?;
                    recovery_mutation = if outcome["recovery"] == "rolled_back" {
                        "rolled_back"
                    } else {
                        "applied"
                    };
                }
                let (plan, warnings) = if let Some(path) = &a.apply_plan {
                    let plan = crate::infrastructure::read_plan(std::path::Path::new(path))?;
                    crate::application::verify(&plan, &root)?;
                    let c = &plan.choices;
                    let (authorized, _) = crate::install_plan(
                        &root,
                        &Args {
                            merge: c.merge,
                            replace: c.replace,
                            refresh: c.refresh,
                            wisdom: c.wisdom,
                            claude: c.claude,
                            ..Default::default()
                        },
                    )?;
                    for op in &plan.operations {
                        if !authorized.operations.iter().any(|p| {
                            p.target == op.target && p.after == op.after && p.mode == op.mode
                        }) {
                            return Err(Failure::new(
                                "INVALID_STATE",
                                "plan contains unauthorized target/content",
                            ));
                        }
                    }
                    if plan.operations.len() != authorized.operations.len() {
                        return Err(Failure::new(
                            "INVALID_STATE",
                            "plan target set differs from selected distribution",
                        ));
                    }
                    (plan, vec![])
                } else {
                    match crate::install_plan(&root, a) {
                        Err(error)
                            if error.code == "INVALID_ARGUMENT"
                                && error.message.starts_with("existing target ")
                                && !a.json =>
                        {
                            use std::io::{IsTerminal, Write};
                            if !std::io::stdin().is_terminal() {
                                return Err(error);
                            }
                            eprint!(
                                "Existing managed targets differ. [m]erge (preserve), [o]verride selected targets with backup, [s]top: "
                            );
                            std::io::stderr().flush()?;
                            let mut choice = String::new();
                            std::io::stdin().read_line(&mut choice)?;
                            let mut selected = a.clone();
                            match choice.trim().to_ascii_lowercase().as_str() {
                                "m" | "merge" => selected.merge = true,
                                "o" | "override" => selected.replace = true,
                                _ => {
                                    return Err(Failure::new(
                                        "INVALID_ARGUMENT",
                                        "installation stopped; no mutation performed",
                                    ));
                                }
                            }
                            crate::install_plan(&root, &selected)?
                        }
                        result => result?,
                    }
                };
                crate::application::verify(&plan, &root)?;
                let plan_id = plan.semantic_id()?;
                let changes=plan.operations.iter().filter(|o|o.before!=o.after).map(|o|serde_json::json!({"target":o.target,"operation":if o.before.is_none(){"create"}else{"replace"},"before_hash":o.before.as_ref().map(|b|crate::infrastructure::digest(b)),"after_hash":o.after.as_ref().map(|b|crate::infrastructure::digest(b))})).collect::<Vec<_>>();
                if let Some(path) = &a.plan_out {
                    crate::infrastructure::write_plan(&root, std::path::Path::new(path), &plan)?;
                }
                let changed = !changes.is_empty();
                if !a.dry && changed {
                    crate::application::apply(&root, &plan)?;
                }
                return Ok((
                    serde_json::json!({"changes":changes,"payload_digest":plan.payload_digest,"plan_id":plan_id,"development_distribution":true,"no_op":!changed}),
                    warnings,
                    if !a.dry && changed { "applied" } else { "none" }.into(),
                ));
            }
            "update" => {
                if a.abort {
                    return Ok((crate::session::abort(&root)?, vec![], "staged_only".into()));
                }
                if a.resume {
                    return Ok((
                        crate::session::resume(&root, a.dry)?,
                        vec![],
                        if a.dry { "none" } else { "applied" }.into(),
                    ));
                }
                if crate::activation::pending(&root)? {
                    return Err(Failure::new(
                        "RECOVERY_REQUIRED",
                        "activation pending; run recover",
                    ));
                }
                if crate::session::pending(&root)? {
                    return Err(Failure::new(
                        "SESSION_EXISTS",
                        "pending resolution retained; use update --continue or --abort",
                    ));
                }
                if !a.offline {
                    if a.apply_plan.is_some()
                        || a.plan_out.is_some()
                        || a.replace
                        || a.merge
                        || a.refresh
                        || a.wisdom
                        || a.claude
                    {
                        return Err(Failure::new(
                            "INVALID_ARGUMENT",
                            "online update does not accept install policy or saved-plan flags",
                        ));
                    }
                    if !a.dry && crate::application::pending(&root)? {
                        let _lock = crate::infrastructure::Lock::acquire(&root)?;
                        let outcome = crate::application::recover(&root, false)?;
                        recovery_mutation = if outcome["recovery"] == "rolled_back" {
                            "rolled_back"
                        } else {
                            "applied"
                        };
                    }
                    let prepared = crate::candidate::prepare(&root)?;
                    if !a.dry {
                        #[cfg(windows)]
                        {
                            if crate::candidate::is_worker_mode() {
                                crate::candidate::verify_worker_authorization(&root)?;
                            } else {
                                return Err(Failure::new(
                                    "MISSING_CAPABILITY",
                                    "Windows candidate binary replacement requires platform adapter; candidate activation is not supported",
                                ));
                            }
                        }
                        #[cfg(not(windows))]
                        {
                            let selected = crate::binary_path();
                            if std::env::current_exe()?.canonicalize()?
                                != crate::infrastructure::safe(&root, selected)?.canonicalize()?
                            {
                                return Err(Failure::new(
                                    "INVALID_ARGUMENT",
                                    "online update activation requires invocation from the selected repository executable",
                                ));
                            }
                        }
                    }
                    if !prepared.conflicts.is_empty() {
                        if a.dry {
                            return Err(Failure::new(
                                "CONFLICT_PENDING",
                                format!(
                                    "text conflicts (preview only): {}",
                                    prepared
                                        .conflicts
                                        .iter()
                                        .map(|c| c.target.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                ),
                            ));
                        }
                        let staged = crate::session::stage(&root, &prepared)?;
                        let mut error = Failure::new(
                            "CONFLICT_PENDING",
                            format!("resolution staged: {staged}"),
                        );
                        error.mutation = "staged_only".into();
                        return Err(error);
                    }
                    let id = prepared.plan.semantic_id()?;
                    if !a.dry {
                        crate::activation::apply(&root, &prepared)?;
                    }
                    return Ok((
                        serde_json::json!({"plan_id":id,"target":prepared.target,"dry_run":a.dry}),
                        vec![],
                        if a.dry { "none" } else { "applied" }.into(),
                    ));
                }
                if a.replace || a.merge || a.refresh || a.wisdom || a.claude {
                    return Err(Failure::new(
                        "INVALID_ARGUMENT",
                        "offline update does not accept install policy flags",
                    ));
                }
                if !a.dry && crate::application::pending(&root)? {
                    let _lock = crate::infrastructure::Lock::acquire(&root)?;
                    let outcome = crate::application::recover(&root, false)?;
                    recovery_mutation = if outcome["recovery"] == "rolled_back" {
                        "rolled_back"
                    } else {
                        "applied"
                    };
                }
                let plan = if let Some(path) = &a.apply_plan {
                    let plan = crate::infrastructure::read_plan(std::path::Path::new(path))?;
                    crate::application::verify(&plan, &root)?;
                    let authorized = crate::update::prepare(&root)?;
                    if plan != authorized {
                        return Err(Failure::new(
                            "INVALID_STATE",
                            "saved update plan differs from authorized embedded update",
                        ));
                    }
                    plan
                } else {
                    crate::update::prepare(&root)?
                };
                crate::application::verify(&plan, &root)?;
                let plan_id = plan.semantic_id()?;
                if let Some(path) = &a.plan_out {
                    crate::infrastructure::write_plan(&root, std::path::Path::new(path), &plan)?;
                }
                let changed = plan.operations.iter().any(|o| o.before != o.after);
                if changed && !a.dry {
                    crate::application::apply(&root, &plan)?;
                }
                return Ok((
                    serde_json::json!({"no_op":!changed,"payload_digest":plan.payload_digest,"plan_id":plan_id,"offline":true,"release_checked":false}),
                    vec![],
                    if changed && !a.dry { "applied" } else { "none" }.into(),
                ));
            }
            "migrate harness" => {
                if crate::activation::pending(&root)? && (a.apply || a.apply_plan.is_some()) {
                    return Err(Failure::new(
                        "RECOVERY_REQUIRED",
                        "activation pending; run recover",
                    ));
                }
                if a.merge || a.replace || a.refresh || a.wisdom || a.claude || a.offline {
                    return Err(Failure::new(
                        "INVALID_ARGUMENT",
                        "migration derives selections from verified Harness state; install/update flags are not accepted",
                    ));
                }
                if !a.dry && !a.apply && a.apply_plan.is_none() {
                    return Ok((crate::migration::inventory(&root)?, vec![], "none".into()));
                }
                if !a.dry && crate::application::pending(&root)? {
                    let _lock = crate::infrastructure::Lock::acquire(&root)?;
                    let outcome = crate::application::recover(&root, false)?;
                    recovery_mutation = if outcome["recovery"] == "rolled_back" {
                        "rolled_back"
                    } else {
                        "applied"
                    };
                }
                let saved = a
                    .apply_plan
                    .as_ref()
                    .map(|path| crate::infrastructure::read_plan(std::path::Path::new(path)))
                    .transpose()?;
                if let Some(saved) = &saved {
                    crate::application::verify(saved, &root)?;
                }
                let plan = crate::migration::prepare(&root)?;
                if let Some(saved) = &saved
                    && saved != &plan
                {
                    return Err(Failure::new(
                        "INVALID_STATE",
                        "saved migration plan differs from authorized migration",
                    ));
                }
                crate::application::verify(&plan, &root)?;
                if let Some(path) = &a.plan_out {
                    crate::infrastructure::write_plan(&root, std::path::Path::new(path), &plan)?;
                }
                let changes = plan.operations.iter().filter(|op| op.before != op.after).map(|op| serde_json::json!({"target":op.target,"before_hash":op.before.as_ref().map(|b|crate::infrastructure::digest(b)),"after_hash":op.after.as_ref().map(|b|crate::infrastructure::digest(b))})).collect::<Vec<_>>();
                let mutate = !a.dry && (a.apply || a.apply_plan.is_some()) && !changes.is_empty();
                if mutate {
                    crate::application::apply(&root, &plan)?;
                }
                return Ok((
                    serde_json::json!({"changes":changes,"plan_id":plan.semantic_id()?,"legacy_retained":true}),
                    vec![],
                    if mutate { "applied" } else { "none" }.into(),
                ));
            }
            _ => return Err(Failure::new("INVALID_ARGUMENT", "unknown command")),
        };
        Ok((data, vec![], "none".into()))
    })();
    result
        .map(
            |(data, warnings, mutation): (serde_json::Value, Vec<String>, String)| {
                (
                    data,
                    warnings,
                    if mutation == "none" {
                        recovery_mutation.into()
                    } else {
                        mutation
                    },
                )
            },
        )
        .map_err(|mut error: Failure| {
            if error.mutation == "none" {
                error.mutation = recovery_mutation.into();
            }
            error
        })
}
pub fn exit(code: &str) -> i32 {
    match code {
        "INVALID_ARGUMENT" => 2,
        "CONFLICT_PENDING" => 3,
        "WORKSPACE_DRIFT" => 4,
        "UNSAFE_PATH"
        | "INVALID_STATE"
        | "INTEGRITY_FAILURE"
        | "INPUT_LIMIT_EXCEEDED"
        | "OUTPUT_INSIDE_TARGET" => 5,
        "MISSING_CAPABILITY" => 6,
        "NETWORK_ERROR" | "RELEASE_UNAVAILABLE" => 7,
        "RECOVERY_REQUIRED" | "RECOVERY_BLOCKED" => 8,
        "LOCK_BUSY" => 9,
        _ => 1,
    }
}
