fn main() {
    let raw = std::env::args().skip(1).collect::<Vec<_>>();
    let json = raw.iter().any(|s| s == "--json");
    let command = raw
        .iter()
        .find(|s| !s.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| "help".into());
    let parsed = agentfoundry::interface::parse(&raw);
    let command = parsed
        .as_ref()
        .map(|a| a.command.clone())
        .unwrap_or(command);
    let result = parsed.and_then(|a| agentfoundry::interface::run(&a));
    let (value, exit) = match result {
        Ok((data, warnings, mutation)) => (
            serde_json::json!({"schema":"agentfoundry.command-result/v1","command":command,"status":"ok","code":"OK","mutation":mutation,"data":data,"warnings":warnings,"errors":[]}),
            0,
        ),
        Err(e) => {
            let exit = agentfoundry::interface::exit(&e.code);
            (
                serde_json::json!({"schema":"agentfoundry.command-result/v1","command":command,"status":"blocked","code":e.code,"mutation":e.mutation,"data":{},"warnings":[],"errors":[{"message":e.message,"next_action":"Inspect the diagnostic and docs/recovery.md; preserve backups and journal."}]}),
                exit,
            )
        }
    };
    if json {
        println!("{value}");
    } else {
        match serde_json::to_string_pretty(&value) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("output serialization failed: {e}");
                std::process::exit(1);
            }
        }
    }
    std::process::exit(exit);
}
