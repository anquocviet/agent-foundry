use crate::domain::validate_digest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PLATFORMS: [&str; 5] = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
];
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub platform: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub checksum_name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseMetadata {
    pub schema: String,
    pub version: String,
    pub tag: String,
    pub source_commit: String,
    pub payload_digest: String,
    pub state_schema_min: u32,
    pub state_schema_max: u32,
    pub handoff_protocol: u32,
    pub assets: Vec<Asset>,
}
fn stable_version(version: &str) -> Result<[u64; 3], String> {
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err("release version must be exact stable semver".into());
    }
    let mut out = [0; 3];
    for (index, part) in parts.into_iter().enumerate() {
        if part.is_empty()
            || !part.bytes().all(|b| b.is_ascii_digit())
            || (part.len() > 1 && part.starts_with('0'))
        {
            return Err("noncanonical stable semver".into());
        }
        out[index] = part.parse().map_err(|_| "semver component overflow")?;
    }
    Ok(out)
}
impl ReleaseMetadata {
    pub fn parse(bytes: &[u8]) -> crate::Result<Self> {
        if bytes.len() as u64 > crate::domain::CONTROL_LIMIT {
            return Err(crate::Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "release metadata exceeds control limit",
            ));
        }
        let metadata: Self = serde_json::from_slice(bytes)?;
        metadata
            .validate()
            .map_err(|error| crate::Failure::new("INTEGRITY_FAILURE", error))?;
        Ok(metadata)
    }
    pub fn validate(&self) -> Result<(), String> {
        stable_version(&self.version)?;
        if self.schema != "agentfoundry.release/v1"
            || self.tag != format!("v{}", self.version)
            || self.source_commit.len() != 40
            || !self
                .source_commit
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("release schema/tag/source identity mismatch".into());
        }
        validate_digest(&self.payload_digest)?;
        if self.state_schema_min > 1
            || self.state_schema_max < 1
            || self.state_schema_min == 0
            || self.state_schema_max < self.state_schema_min
            || self.handoff_protocol != 1
        {
            return Err("candidate cannot read installed state/handoff schema".into());
        }
        if self.assets.len() != PLATFORMS.len() {
            return Err("release requires exact five-platform binary inventory".into());
        }
        let mut platforms = BTreeSet::new();
        let mut names = BTreeSet::new();
        for asset in &self.assets {
            if !PLATFORMS.contains(&asset.platform.as_str())
                || !platforms.insert(asset.platform.as_str())
                || asset.size == 0
                || asset.size > 128 * 1024 * 1024
            {
                return Err("invalid platform, duplicate or oversized release asset".into());
            }
            validate_digest(&asset.sha256)?;
            for name in [&asset.name, &asset.checksum_name] {
                crate::domain::RelativePath::new(name)?;
                if name.contains('/') || !names.insert(name.as_str()) {
                    return Err("asset names must be distinct portable basenames".into());
                }
            }
            if asset.checksum_name != format!("{}.sha256", asset.name) {
                return Err("checksum sidecar must bind exact binary basename".into());
            }
        }
        Ok(())
    }
    pub fn validate_upgrade(&self, installed: &Self) -> Result<(), String> {
        self.validate()?;
        installed.validate()?;
        let next = stable_version(&self.version)?;
        let current = stable_version(&installed.version)?;
        if next < current {
            return Err("release downgrade rejected".into());
        }
        if next == current
            && (self.source_commit != installed.source_commit
                || self.payload_digest != installed.payload_digest
                || self.assets.iter().any(|asset| {
                    !installed.assets.iter().any(|old| {
                        old.platform == asset.platform
                            && old.name == asset.name
                            && old.size == asset.size
                            && old.sha256 == asset.sha256
                    })
                }))
        {
            return Err("immutable same-version release identity violation".into());
        }
        Ok(())
    }
    pub fn verify_candidate(&self, platform: &str, bytes: &[u8]) -> Result<(), String> {
        self.validate()?;
        let asset = self
            .assets
            .iter()
            .find(|asset| asset.platform == platform)
            .ok_or("unsupported candidate platform")?;
        if bytes.len() as u64 != asset.size || crate::infrastructure::digest(bytes) != asset.sha256
        {
            return Err("candidate byte digest/size mismatch; do not execute".into());
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> ReleaseMetadata {
        ReleaseMetadata {
            schema: "agentfoundry.release/v1".into(),
            version: "1.0.0".into(),
            tag: "v1.0.0".into(),
            source_commit: "a".repeat(40),
            payload_digest: "b".repeat(64),
            state_schema_min: 1,
            state_schema_max: 1,
            handoff_protocol: 1,
            assets: PLATFORMS
                .iter()
                .map(|p| Asset {
                    platform: (*p).into(),
                    name: format!("agentfoundry-{p}"),
                    size: 4,
                    sha256: crate::infrastructure::digest(b"real"),
                    checksum_name: format!("agentfoundry-{p}.sha256"),
                })
                .collect(),
        }
    }
    #[test]
    fn verify_before_execute_and_exact_inventory() {
        let mut r = release();
        r.verify_candidate(PLATFORMS[0], b"real").unwrap();
        assert!(r.verify_candidate(PLATFORMS[0], b"fake").is_err());
        ReleaseMetadata::parse(&serde_json::to_vec(&r).unwrap()).unwrap();
        let mut bad = serde_json::to_value(&r).unwrap();
        bad["assets"][0]["name"] = serde_json::json!("../candidate");
        assert!(ReleaseMetadata::parse(&serde_json::to_vec(&bad).unwrap()).is_err());
        r.assets.pop();
        assert!(r.validate().is_err());
    }
    #[test]
    fn immutable_identity_and_downgrade_rejected() {
        let installed = release();
        let mut next = release();
        next.source_commit = "c".repeat(40);
        assert!(next.validate_upgrade(&installed).is_err());
        next.version = "0.9.0".into();
        next.tag = "v0.9.0".into();
        assert!(next.validate_upgrade(&installed).is_err());
        next.version = "1.0.1".into();
        next.tag = "v1.0.1".into();
        next.validate_upgrade(&installed).unwrap();
    }
    #[test]
    fn path_and_schema_mismatch_fail_closed() {
        let mut r = release();
        r.assets[0].name = "../candidate".into();
        assert!(r.validate().is_err());
        let mut r = release();
        r.state_schema_min = 2;
        assert!(r.validate().is_err());
    }
}
