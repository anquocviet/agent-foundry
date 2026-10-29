use serde_json::Value;
use sha2::{Digest, Sha256};
pub fn canonical_digest(entries: &[Value]) -> Result<String, String> {
    if entries.len() > 10000 {
        return Err("distribution entry limit exceeded".into());
    }
    let mut rows = Vec::with_capacity(entries.len());
    let mut ids = std::collections::BTreeSet::new();
    let mut targets = std::collections::BTreeSet::new();
    for e in entries {
        let object = e
            .as_object()
            .ok_or("distribution entry must be an object")?;
        if object.keys().any(|key| {
            ![
                "logical_id",
                "target",
                "source",
                "ownership",
                "sha256",
                "mode",
                "addon",
                "adapter",
            ]
            .contains(&key.as_str())
        }) {
            return Err("unknown distribution entry field".into());
        }
        let id = e["logical_id"].as_str().ok_or("missing logical ID")?;
        if id.is_empty() || !ids.insert(id) {
            return Err("duplicate logical ID".into());
        }
        let target = e["target"].as_str().ok_or("missing target")?;
        crate::domain::RelativePath::new(target)?;
        let effective = target.to_lowercase();
        if effective == ".agentfoundry"
            || effective.starts_with(".agentfoundry/")
            || effective == "scripts/bin"
            || effective.starts_with("scripts/bin/")
        {
            return Err("payload target uses reserved control/executable namespace".into());
        }
        let ownership = e["ownership"].as_str().ok_or("missing ownership")?;
        let hash = e["sha256"].as_str().ok_or("missing content hash")?;
        if ownership == "managed_block" && !["AGENTS.md", "CLAUDE.md"].contains(&target) {
            return Err("unsupported managed block container".into());
        }
        let mode = e["mode"].as_u64().ok_or("missing file mode")?;
        for (key, allowed) in [("addon", "engineering-wisdom"), ("adapter", "claude")] {
            if e.get(key)
                .is_some_and(|value| !value.is_null() && value.as_str() != Some(allowed))
            {
                return Err(format!("invalid distribution {key} selector"));
            }
        }
        if target.is_empty()
            || !targets.insert(target.to_lowercase())
            || mode > 0o777
            || !["managed_file", "managed_block"].contains(&ownership)
            || hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("invalid or duplicate target, ownership, mode or content hash".into());
        }
        rows.push((
            id,
            target,
            ownership,
            mode,
            hash,
            e.get("addon").and_then(Value::as_str),
            e.get("adapter").and_then(Value::as_str),
        ));
    }
    for target in &targets {
        for (index, _) in target.match_indices('/') {
            if targets.contains(&target[..index]) {
                return Err("payload file collides with another target's parent directory".into());
            }
        }
    }
    rows.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let bytes = serde_json::to_vec(&("agentfoundry.payload-canonical/v1", rows))
        .map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn order_and_cosmetics_do_not_change_identity_but_bytes_and_modes_do() {
        let a = serde_json::json!({"logical_id":"é","target":"docs/é.md","ownership":"managed_file","mode":420,"sha256":"a".repeat(64),"source":"build-only"});
        let mut b = a.clone();
        b["logical_id"] = serde_json::json!("a");
        b["target"] = serde_json::json!("docs/a.md");
        let digest = canonical_digest(&[a.clone(), b.clone()]).unwrap();
        assert_eq!(digest, canonical_digest(&[b.clone(), a.clone()]).unwrap());
        b["source"] = serde_json::json!("different-build-path");
        assert_eq!(digest, canonical_digest(&[a.clone(), b.clone()]).unwrap());
        b["mode"] = serde_json::json!(384);
        assert_ne!(digest, canonical_digest(&[a.clone(), b]).unwrap());
        assert!(canonical_digest(&[a.clone(), a]).is_err());
    }
}

#[cfg(test)]
#[test]
fn rejects_ambiguous_targets_and_invalid_metadata() {
    let entry = serde_json::json!({"logical_id":"one","target":"docs/A.md","ownership":"managed_file","mode":420,"sha256":"a".repeat(64)});
    let mut duplicate = entry.clone();
    duplicate["logical_id"] = serde_json::json!("two");
    duplicate["target"] = serde_json::json!("docs/a.md");
    assert!(canonical_digest(&[entry.clone(), duplicate]).is_err());
    for (key, value) in [
        ("mode", serde_json::json!(4095)),
        ("ownership", serde_json::json!("unknown")),
        ("sha256", serde_json::json!("G".repeat(64))),
        ("logical_id", serde_json::json!("")),
        ("addon", serde_json::json!(true)),
        ("adapter", serde_json::json!("unknown")),
        ("target", serde_json::json!("../outside")),
        ("target", serde_json::json!("C:/outside")),
        ("target", serde_json::json!(".AgentFoundry/config.json")),
        ("target", serde_json::json!("scripts/bin/agentfoundry")),
        ("ownership", serde_json::json!("managed_block")),
    ] {
        let mut invalid = entry.clone();
        invalid[key] = value;
        assert!(canonical_digest(&[invalid]).is_err(), "{key}");
    }
}

#[cfg(test)]
#[test]
fn rejects_file_parent_collisions_even_with_intervening_names() {
    let entries = ["docs/a", "docs/a-b", "docs/a/child"].iter().enumerate().map(|(index, target)| serde_json::json!({"logical_id":index.to_string(),"target":target,"ownership":"managed_file","mode":420,"sha256":"a".repeat(64)})).collect::<Vec<_>>();
    assert!(canonical_digest(&entries).is_err());
}

#[cfg(test)]
#[test]
fn rejects_unknown_distribution_fields() {
    let entry = serde_json::json!({"logical_id":"one","target":"docs/a.md","source":"payload/a.md","ownership":"managed_file","mode":420,"sha256":"a".repeat(64),"adapters":["claude"]});
    assert!(canonical_digest(&[entry]).is_err());
}
