pub const CONTROL_LIMIT: u64 = 16 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "String")]
pub struct RelativePath(String);
impl RelativePath {
    pub fn new(value: &str) -> Result<Self, String> {
        if value.is_empty()
            || value.contains(['\\', ':', '\0', '<', '>', '"', '|', '?', '*'])
            || value.len() > 4096
        {
            return Err("invalid portable relative path".into());
        }
        for c in value.split('/') {
            let stem = c.split('.').next().unwrap_or("").to_ascii_uppercase();
            if c.is_empty()
                || c == "."
                || c == ".."
                || c.ends_with(['.', ' '])
                || c.chars().any(|c| c.is_control())
                || c.eq_ignore_ascii_case(".git")
                || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9'))
            {
                return Err(format!("unsafe path component: {c}"));
            }
        }
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for RelativePath {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}
pub fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Ok(())
    } else {
        Err("expected lowercase SHA-256 digest".into())
    }
}
pub fn validate_transaction_target(target: &str) -> Result<(), String> {
    RelativePath::new(target)?;
    let internal = target
        .split('/')
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case(".agentfoundry"));
    if internal
        && ![
            ".agentfoundry/config.json",
            ".agentfoundry/.gitignore",
            ".agentfoundry/state/manifest.json",
        ]
        .contains(&target)
        && target
            .strip_prefix(".agentfoundry/state/objects/")
            .is_none_or(|hash| validate_digest(hash).is_err())
    {
        return Err("transaction targets undeclared internal path".into());
    }
    Ok(())
}
fn validate_marker_layout(container: &[u8]) -> Result<(), String> {
    let mut markers = Vec::new();
    for name in ["AGENTFOUNDRY", "AGENTFOUNDRY:CLAUDE"] {
        for (kind, begin) in [("BEGIN", true), ("END", false)] {
            let marker = format!("<!-- {name}:{kind} -->");
            for (position, bytes) in container.windows(marker.len()).enumerate() {
                if bytes == marker.as_bytes() {
                    if markers.len() == 4 {
                        return Err("duplicate marker block".into());
                    }
                    markers.push((position, name, begin));
                }
            }
        }
    }
    markers.sort_unstable_by_key(|marker| marker.0);
    let mut open = None;
    let mut seen = std::collections::BTreeSet::new();
    for (_, name, begin) in markers {
        if begin {
            if open.is_some() || !seen.insert(name) {
                return Err("nested or duplicate marker block".into());
            }
            open = Some(name);
        } else if open.take() != Some(name) {
            return Err("reversed or mismatched marker block".into());
        }
    }
    if open.is_some() {
        return Err("unterminated marker block".into());
    }
    Ok(())
}
pub fn block_content<'a>(container: &'a [u8], name: &str) -> Result<Option<&'a [u8]>, String> {
    if !["AGENTFOUNDRY", "AGENTFOUNDRY:CLAUDE"].contains(&name) {
        return Err("unknown marker namespace".into());
    }
    validate_marker_layout(container)?;
    let begin = format!("<!-- {name}:BEGIN -->");
    let end = format!("<!-- {name}:END -->");
    let starts = container
        .windows(begin.len())
        .enumerate()
        .filter_map(|(i, w)| (w == begin.as_bytes()).then_some(i))
        .collect::<Vec<_>>();
    let ends = container
        .windows(end.len())
        .enumerate()
        .filter_map(|(i, w)| (w == end.as_bytes()).then_some(i))
        .collect::<Vec<_>>();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([a], [b]) if a < b => Ok(Some(&container[*a..*b + end.len()])),
        _ => Err("malformed, duplicate or reversed marker pair".into()),
    }
}
pub fn patch_block(container: &[u8], block: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let begin = format!("<!-- {name}:BEGIN -->");
    let end = format!("<!-- {name}:END -->");
    if !["AGENTFOUNDRY", "AGENTFOUNDRY:CLAUDE"].contains(&name) {
        return Err("unknown marker namespace".into());
    }
    validate_marker_layout(container)?;
    validate_marker_layout(block)?;
    let incoming_starts = block
        .windows(begin.len())
        .filter(|w| *w == begin.as_bytes())
        .count();
    let incoming_ends = block
        .windows(end.len())
        .filter(|w| *w == end.as_bytes())
        .count();
    if incoming_starts != 1
        || incoming_ends != 1
        || !block.starts_with(begin.as_bytes())
        || !block
            .strip_suffix(b"\n")
            .unwrap_or(block)
            .ends_with(end.as_bytes())
    {
        return Err("invalid distribution marker block".into());
    }
    let positions = |needle: &[u8]| {
        container
            .windows(needle.len())
            .enumerate()
            .filter_map(|(i, w)| (w == needle).then_some(i))
            .collect::<Vec<_>>()
    };
    let starts = positions(begin.as_bytes());
    let ends = positions(end.as_bytes());
    let mut out = Vec::new();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => {
            out.extend_from_slice(container);
            if !out.is_empty() && !out.ends_with(b"\n") {
                out.push(b'\n');
            }
            out.extend_from_slice(block);
        }
        ([a], [b]) if a < b => {
            out.extend_from_slice(&container[..*a]);
            out.extend_from_slice(block.strip_suffix(b"\n").unwrap_or(block));
            out.extend_from_slice(&container[b + end.len()..]);
        }
        _ => return Err("malformed, duplicate or reversed marker pair".into()),
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_paths() {
        for p in [
            "../x", "/x", "a//b", "C:/x", "a\\b", "NUL.txt", "a/.git/x", "com1", "x.", "x ", "a?b",
            "a*b", "a|b", "a<b", "a>b", "a\"b",
        ] {
            assert!(RelativePath::new(p).is_err(), "{p}");
        }
        assert!(RelativePath::new("docs/đường dẫn.md").is_ok());
    }
    #[test]
    fn marker_preserves_outer_bytes() {
        let old = b"private\r\n<!-- AGENTFOUNDRY:BEGIN -->old<!-- AGENTFOUNDRY:END -->tail";
        let block = b"<!-- AGENTFOUNDRY:BEGIN -->new<!-- AGENTFOUNDRY:END -->\n";
        assert_eq!(
            patch_block(old, block, "AGENTFOUNDRY").unwrap(),
            b"private\r\n<!-- AGENTFOUNDRY:BEGIN -->new<!-- AGENTFOUNDRY:END -->tail"
        );
        assert!(patch_block(b"<!-- AGENTFOUNDRY:END -->", block, "AGENTFOUNDRY").is_err());
        assert!(patch_block(b"", b"missing markers", "AGENTFOUNDRY").is_err());
        let reversed = b"<!-- AGENTFOUNDRY:END --><!-- AGENTFOUNDRY:BEGIN -->";
        assert!(patch_block(reversed, block, "AGENTFOUNDRY").is_err());
        let duplicate = [old.as_slice(), old.as_slice()].concat();
        assert!(patch_block(&duplicate, block, "AGENTFOUNDRY").is_err());
    }
    #[test]
    fn nested_marker_types_do_not_authorize_replacement() {
        let nested = b"<!-- AGENTFOUNDRY:BEGIN --><!-- AGENTFOUNDRY:CLAUDE:BEGIN -->private<!-- AGENTFOUNDRY:CLAUDE:END --><!-- AGENTFOUNDRY:END -->";
        let block = b"<!-- AGENTFOUNDRY:BEGIN -->new<!-- AGENTFOUNDRY:END -->";
        assert!(block_content(nested, "AGENTFOUNDRY").is_err());
        assert!(patch_block(nested, block, "AGENTFOUNDRY").is_err());
        assert!(patch_block(b"", nested, "AGENTFOUNDRY").is_err());
    }
}
