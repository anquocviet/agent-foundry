use crate::{Failure, Result};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: String,
    pub adapters: Vec<String>,
    pub addons: Vec<String>,
    pub paths: Paths,
    pub updates: Updates,
    pub telemetry: Telemetry,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Paths {
    pub docs: String,
    pub skills: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    pub channel: String,
    pub automatic: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Telemetry {
    pub enabled: bool,
}
impl Config {
    pub fn selected(claude: bool, wisdom: bool) -> Self {
        let mut adapters = vec!["agents-md".into()];
        if claude {
            adapters.push("claude".into());
        }
        Self {
            schema: "agentfoundry.config/v1".into(),
            adapters,
            addons: if wisdom {
                vec!["engineering-wisdom".into()]
            } else {
                vec![]
            },
            paths: Paths {
                docs: "docs/agentfoundry".into(),
                skills: ".agents/skills".into(),
            },
            updates: Updates {
                channel: "stable".into(),
                automatic: false,
            },
            telemetry: Telemetry { enabled: false },
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(Failure::new(
                "INPUT_LIMIT_EXCEEDED",
                "config exceeds control limit",
            ));
        }
        let config: Self = serde_json::from_slice(bytes)?;
        config.validate()?;
        Ok(config)
    }
    pub fn validate(&self) -> Result<()> {
        let unique = |items: &[String]| {
            let mut seen = std::collections::BTreeSet::new();
            items.iter().all(|i| seen.insert(i))
        };
        if self.schema != "agentfoundry.config/v1"
            || !unique(&self.adapters)
            || !unique(&self.addons)
            || !self.adapters.iter().any(|s| s == "agents-md")
            || self
                .adapters
                .iter()
                .any(|s| !["agents-md", "claude"].contains(&s.as_str()))
            || self.addons.iter().any(|s| s != "engineering-wisdom")
            || self.updates.channel != "stable"
            || self.updates.automatic
            || self.telemetry.enabled
        {
            return Err(Failure::new(
                "INVALID_STATE",
                "unknown config schema, selection or unsupported automatic/network policy",
            ));
        }
        // ponytail: custom target mapping awaits complete collision/migration planning; fail closed meanwhile.
        if self.paths.docs != "docs/agentfoundry" || self.paths.skills != ".agents/skills" {
            return Err(Failure::new(
                "MISSING_CAPABILITY",
                "custom payload paths require path-migration planner",
            ));
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_and_policy_rejection() {
        let config = Config::selected(true, true);
        let bytes = serde_json::to_vec(&config).unwrap();
        assert_eq!(Config::parse(&bytes).unwrap(), config);
        let mut value = serde_json::to_value(config).unwrap();
        value["schema"] = serde_json::json!("agentfoundry.config/v99");
        assert!(Config::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value["schema"] = serde_json::json!("agentfoundry.config/v1");
        value["updates"]["endpoint"] = serde_json::json!("https://untrusted.example");
        assert!(Config::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}
