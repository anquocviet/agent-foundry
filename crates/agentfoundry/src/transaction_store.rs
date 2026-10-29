use crate::{
    Failure, Result,
    application::{Operation, Plan, PlanChoices},
    infrastructure as fs,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredOperation {
    pub target: String,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub before: Option<String>,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub after: Option<String>,
    pub mode: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredPlan {
    pub schema: String,
    pub root: String,
    #[serde(deserialize_with = "serde::Deserialize::deserialize")]
    pub root_identity: Option<String>,
    pub payload_digest: String,
    pub choices: PlanChoices,
    pub operations: Vec<StoredOperation>,
}
impl StoredPlan {
    pub fn from_plan(plan: &Plan) -> Self {
        Self {
            schema: "agentfoundry.stored-plan/v1".into(),
            root: plan.root.clone(),
            root_identity: plan.root_identity.clone(),
            payload_digest: plan.payload_digest.clone(),
            choices: plan.choices.clone(),
            operations: plan
                .operations
                .iter()
                .map(|o| StoredOperation {
                    target: o.target.clone(),
                    before: o.before.as_ref().map(|b| fs::digest(b)),
                    after: o.after.as_ref().map(|b| fs::digest(b)),
                    mode: o.mode,
                })
                .collect(),
        }
    }
    pub fn persist_images(&self, root: &Path, id: &str, plan: &Plan) -> Result<()> {
        validate_id(id)?;
        if serde_json::to_vec(self)? != serde_json::to_vec(&Self::from_plan(plan))? {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                "stored plan and staged input set differ",
            ));
        }
        for (stored, op) in self.operations.iter().zip(&plan.operations) {
            for (hash, bytes) in [(&stored.before, &op.before), (&stored.after, &op.after)] {
                if let (Some(hash), Some(bytes)) = (hash, bytes) {
                    if fs::digest(bytes) != *hash {
                        return Err(Failure::new("INTEGRITY_FAILURE", "staged image changed"));
                    }
                    let path = format!(".agentfoundry/backups/{id}/objects/{hash}");
                    match fs::read(root, &path, 128 * 1024 * 1024)? {
                        Some(existing) if fs::digest(&existing) != *hash => {
                            return Err(Failure::new(
                                "INTEGRITY_FAILURE",
                                "backup object collision",
                            ));
                        }
                        Some(_) => {}
                        None => fs::atomic(root, &path, bytes, 0o600)?,
                    }
                    if fs::read(root, &path, 128 * 1024 * 1024)?
                        .is_none_or(|b| fs::digest(&b) != *hash)
                    {
                        return Err(Failure::new(
                            "INTEGRITY_FAILURE",
                            "backup image verification failed",
                        ));
                    }
                }
            }
        }
        Ok(())
    }
    pub(crate) fn validate(&self, expected_root: &str) -> Result<()> {
        if self.schema != "agentfoundry.stored-plan/v1"
            || self.operations.len() > 10000
            || self.root != expected_root
            || self
                .root_identity
                .as_ref()
                .is_none_or(|identity| identity.is_empty())
            || (self.choices.merge && self.choices.replace)
        {
            return Err(Failure::new(
                "RECOVERY_BLOCKED",
                "invalid stored plan schema/root/resource bound",
            ));
        }
        crate::domain::validate_digest(&self.payload_digest)
            .map_err(|error| Failure::new("RECOVERY_BLOCKED", error))?;
        if self
            .operations
            .iter()
            .position(|operation| {
                operation.target == ".agentfoundry/state/manifest.json"
                    && operation.before != operation.after
            })
            .is_some_and(|index| {
                self.operations[index + 1..]
                    .iter()
                    .any(|operation| operation.before != operation.after)
            })
        {
            return Err(Failure::new(
                "RECOVERY_BLOCKED",
                "changed operations follow stored manifest commit",
            ));
        }
        let mut targets = std::collections::BTreeSet::new();
        for operation in &self.operations {
            crate::domain::validate_transaction_target(&operation.target)
                .map_err(|error| Failure::new("RECOVERY_BLOCKED", error))?;
            if let Some(hash) = operation
                .target
                .strip_prefix(".agentfoundry/state/objects/")
                && (operation.after.as_deref() != Some(hash)
                    || operation
                        .before
                        .as_deref()
                        .is_some_and(|before| before != hash))
            {
                return Err(Failure::new(
                    "RECOVERY_BLOCKED",
                    "immutable baseline object content mismatch",
                ));
            }
            if !targets.insert(operation.target.to_lowercase()) {
                return Err(Failure::new("RECOVERY_BLOCKED", "duplicate stored target"));
            }
            if operation.mode & !0o777 != 0 {
                return Err(Failure::new(
                    "RECOVERY_BLOCKED",
                    "unsupported stored file mode",
                ));
            }
            for hash in [&operation.before, &operation.after].into_iter().flatten() {
                crate::domain::validate_digest(hash)
                    .map_err(|error| Failure::new("RECOVERY_BLOCKED", error))?;
            }
        }
        for target in &targets {
            for (index, _) in target.match_indices('/') {
                if targets.contains(&target[..index]) {
                    return Err(Failure::new(
                        "RECOVERY_BLOCKED",
                        "stored target collides with parent",
                    ));
                }
            }
        }
        Ok(())
    }
    pub fn hydrate(&self, root: &Path, id: &str) -> Result<Plan> {
        validate_id(id)?;
        self.validate(&root.to_string_lossy())?;
        let image = |hash: &Option<String>| -> Result<Option<Vec<u8>>> {
            hash.as_ref()
                .map(|hash| {
                    let path = format!(".agentfoundry/backups/{id}/objects/{hash}");
                    let bytes = fs::read(root, &path, 128 * 1024 * 1024)?
                        .ok_or_else(|| Failure::new("RECOVERY_BLOCKED", "backup object missing"))?;
                    if fs::digest(&bytes) != *hash {
                        return Err(Failure::new("RECOVERY_BLOCKED", "backup object corrupt"));
                    }
                    Ok(bytes)
                })
                .transpose()
        };
        let operations = self
            .operations
            .iter()
            .map(|o| {
                Ok(Operation {
                    target: o.target.clone(),
                    before: image(&o.before)?,
                    after: image(&o.after)?,
                    mode: o.mode,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Plan {
            schema: "agentfoundry.plan/v1".into(),
            root: self.root.clone(),
            root_identity: self.root_identity.clone(),
            snapshot: None,
            payload_digest: self.payload_digest.clone(),
            choices: self.choices.clone(),
            operations,
        })
    }
}
fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        Err(Failure::new("RECOVERY_BLOCKED", "unsafe transaction ID"))
    } else {
        Ok(())
    }
}
