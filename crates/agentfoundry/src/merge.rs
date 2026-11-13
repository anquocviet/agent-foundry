use crate::{Failure, Result};
#[derive(Debug, PartialEq, Eq)]
pub enum Merge {
    Clean(Option<Vec<u8>>),
    Conflict(Vec<u8>),
    Structural(&'static str),
}
pub fn three_way(
    base: Option<&[u8]>,
    local: Option<&[u8]>,
    upstream: Option<&[u8]>,
) -> Result<Merge> {
    if base.is_some() && local.is_none() && upstream.is_some() {
        return Ok(Merge::Structural("missing_managed"));
    }
    if local == base {
        return Ok(Merge::Clean(upstream.map(Vec::from)));
    }
    if upstream == base || local == upstream {
        return Ok(Merge::Clean(local.map(Vec::from)));
    }
    match (base, local, upstream) {
        (None, Some(_), Some(_)) => Ok(Merge::Structural("unmanaged_collision")),
        (Some(_), Some(_), None) => Ok(Merge::Structural("modified_removed")),
        (Some(b), Some(l), Some(u)) => {
            if [b, l, u]
                .iter()
                .any(|v| v.contains(&0) || std::str::from_utf8(v).is_err())
            {
                return Ok(Merge::Structural("binary_divergence"));
            }
            let dir =
                std::env::temp_dir().join(format!("agentfoundry-merge-{}", crate::unique_id()));
            let builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                let mut builder = builder;
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
                builder.create(&dir)?;
            }
            #[cfg(not(unix))]
            builder.create(&dir)?;
            struct Cleanup(std::path::PathBuf);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let _cleanup = Cleanup(dir.clone());
            for (name, bytes) in [("BASE", b), ("LOCAL", l), ("UPSTREAM", u)] {
                std::fs::write(dir.join(name), bytes)?;
            }
            let mut command = std::process::Command::new("git");
            command
                .env("GIT_OPTIONAL_LOCKS", "0")
                .args([
                    "merge-file",
                    "-p",
                    "--diff3",
                    "-L",
                    "LOCAL",
                    "-L",
                    "BASE",
                    "-L",
                    "UPSTREAM",
                ])
                .arg(dir.join("LOCAL"))
                .arg(dir.join("BASE"))
                .arg(dir.join("UPSTREAM"));
            let output = crate::process::capture(
                &mut command,
                32 * 1024 * 1024,
                std::time::Duration::from_secs(30),
            )?;
            match output.status.code() {
                Some(0) => Ok(Merge::Clean(Some(output.stdout))),
                Some(1..=127) => Ok(Merge::Conflict(output.stdout)),
                _ => Err(Failure::new("IO_ERROR", "git merge-file process failed")),
            }
        }
        _ => Err(Failure::new("INVALID_STATE", "unhandled merge state")),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_boundaries() {
        assert_eq!(
            three_way(Some(b"b"), Some(b"b"), Some(b"u")).unwrap(),
            Merge::Clean(Some(b"u".to_vec()))
        );
        assert_eq!(
            three_way(Some(b"b"), Some(b"l"), Some(b"b")).unwrap(),
            Merge::Clean(Some(b"l".to_vec()))
        );
        assert_eq!(
            three_way(Some(b"b"), Some(b"l"), None).unwrap(),
            Merge::Structural("modified_removed")
        );
        assert_eq!(
            three_way(None, Some(b"l"), Some(b"u")).unwrap(),
            Merge::Structural("unmanaged_collision")
        );
        assert_eq!(
            three_way(Some(b"b"), None, Some(b"u")).unwrap(),
            Merge::Structural("missing_managed")
        );
        assert_eq!(
            three_way(Some(b"b"), None, Some(b"b")).unwrap(),
            Merge::Structural("missing_managed")
        );
    }
    #[test]
    fn nonoverlapping_edits_preserve_both_sides() {
        let base = b"first\n1\n2\n3\n4\n5\n6\nlast\n";
        let local = b"local\n1\n2\n3\n4\n5\n6\nlast\n";
        let upstream = b"first\n1\n2\n3\n4\n5\n6\nupstream\n";
        assert_eq!(
            three_way(Some(base), Some(local), Some(upstream)).unwrap(),
            Merge::Clean(Some(b"local\n1\n2\n3\n4\n5\n6\nupstream\n".to_vec()))
        );
        assert_eq!(
            three_way(Some(b"a\0"), Some(b"b\0"), Some(b"c\0")).unwrap(),
            Merge::Structural("binary_divergence")
        );
    }
    #[test]
    fn multiple_git_conflicts_are_not_process_errors() {
        let base = b"a\n1\n2\n3\n4\n5\n6\n7\nz\n";
        let local = b"local\n1\n2\n3\n4\n5\n6\n7\nlast-local\n";
        let incoming = b"upstream\n1\n2\n3\n4\n5\n6\n7\nlast-upstream\n";
        match three_way(Some(base), Some(local), Some(incoming)).unwrap() {
            Merge::Conflict(b) => {
                assert_eq!(String::from_utf8(b).unwrap().matches("<<<<<<<").count(), 2)
            }
            other => panic!("{other:?}"),
        }
    }
}
