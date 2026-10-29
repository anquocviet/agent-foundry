use crate::{
    Failure, Result,
    application::{self, Operation, Plan},
    domain, infrastructure as fs,
    interface::Args,
    merge::{self, Merge},
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const SOURCE_BASELINE_COMMIT: &str = "485fd4062be1c71b27c1c9e5585ac4cd2faa7e06";
pub const RELEASE_0_1_10_COMMIT: &str = "e765792b635b4d5e3e5fc0578f82f9ca5dea2681";

const HARNESS_MANIFEST: &str = ".harness-core/manifest.json";
const HARNESS_BASE_DIR: &str = ".harness-core/base";
const HARNESS_TRANSACTION: &str = ".harness-core/transaction.json";
const HARNESS_UPDATE_SESSION: &str = ".harness-core/update/session.json";
const STATE: &str = ".agentfoundry/state/manifest.json";

pub static BASELINE_FILES: &[(&str, &str)] = &[
    (
        ".agents/skills/audit-onboarding-proposal/SKILL.md",
        "a5362994d70221bfc26bb9dc814426eb52f9a728dc828421a31d0fe704c0b370",
    ),
    (
        ".agents/skills/audit-onboarding-proposal/agents/openai.yaml",
        "1f822840d05d627c43e8dcb87a8c54ef28e52c24cba72cf6950c142e360e1a81",
    ),
    (
        ".agents/skills/audit-onboarding-proposal/scripts/validate_evidence_capsule.py",
        "6d9503db59137b6df758157ab9d225c75253523c4fed9e70dc7f61c787fdea84",
    ),
    (
        ".agents/skills/encode-invariant/SKILL.md",
        "025d099757786d203462d8f2a56431c4e995b679caf0ce757bb7e31b366b8a46",
    ),
    (
        ".agents/skills/encode-invariant/agents/openai.yaml",
        "cfa2b4c5263b09f8d51dbabb575fb2597378be54f4b09205bea0a98cd562b2ac",
    ),
    (
        ".agents/skills/improve-harness/SKILL.md",
        "22c9ddae0396d0884b759f2372685ab74ea29ce40a7d876829a28088ebc8fb30",
    ),
    (
        ".agents/skills/improve-harness/agents/openai.yaml",
        "77982c7e2c70c3becb5076c2ce10eefb49ada39f53ed01da930f092fe6d263c5",
    ),
    (
        ".agents/skills/onboard-repository/SKILL.md",
        "dd63e9828c45b0e07449d1b58f41e797f1c569b7787111a4ad9797c95a2586ce",
    ),
    (
        ".agents/skills/onboard-repository/agents/openai.yaml",
        "0992cf606b2680397e6aee60db3d39c62aa3d5773acaeefc2f4f1e18103537e9",
    ),
    (
        ".agents/skills/onboard-repository/references/evidence-capsule-v1.md",
        "b9e1f9208842e725289469d0976b3e65670a30ee7f69be6d799cf4ba2a19fadb",
    ),
    (
        ".agents/skills/onboard-repository/references/evidence-capsule-v2.md",
        "fb8454f138178b8a1166467006c9fe9b8a533029b9bb78a0f9ba846c91f16642",
    ),
    (
        ".agents/skills/onboard-repository/scripts/emit_evidence_bundle.py",
        "c849badb5ad720b150caebaf5961457cfecba9127a9811652e691a68bdd34454",
    ),
    (
        ".agents/skills/onboard-repository/scripts/render_patch.py",
        "9e6b5c4b148e78171695ab964757e792d2a67b32517d08ad113df8de2da9cb38",
    ),
    (
        "AGENTS.md",
        "d034c93fcce8a4ed46d2857b92d39ad1a7d3240102ef9ddc2eb35a8eb6020dcb",
    ),
    (
        "docs/README.md",
        "3482f4d46e1a0f22d2d06b3f2ffbdf31b0443e29db1efc9f0c8844629f3ea866",
    ),
    (
        "docs/WORKFLOW.md",
        "b76b94a895adf1c105ea5535db53059008c701c76481aa0d9c54b32389945652",
    ),
    (
        "docs/decisions/README.md",
        "22943f63368fb0d8aead5e8bc2c0142eb37384287cf50100e421796af019bd4a",
    ),
    (
        "docs/patterns/encoding-invariants.md",
        "c49f7fe5c5995e11f095c70791f731874c8cb8e1cdabef36d26581aa0a25ffd5",
    ),
    (
        "docs/plans/README.md",
        "4b8203c4a199fc7ba06fec0ac3c3b779c7104660ba9374095a8c087c292a29b9",
    ),
    (
        "docs/plans/active/README.md",
        "a05d6098af9a87cb8c2423cf7f09499afb24239a65c64d7bb0d3fbbe7d2c07e2",
    ),
    (
        "docs/plans/completed/README.md",
        "204d6027628c7450727eb24a36d57ea58280e4644d02ee1551a00c781c9f4c00",
    ),
    (
        "docs/product/README.md",
        "72e7901c5d2fedb874340146125515b47ce2598d99568f9eaa3ae193b0d8920f",
    ),
    (
        "docs/templates/application-runbook.md",
        "1e87f2189066dd0469555406c9ef63c3939e53adfcb23494d2f13f0b9552a72d",
    ),
    (
        "docs/templates/decision.md",
        "e92ba7a5b7bc10ce6c213af75a191619fc305c3945cdd55d4dc6bac72dcd42c1",
    ),
    (
        "docs/templates/exec-plan.md",
        "111c40ea8ab63a03561fdb54f235358546788b3a4e1bb26f220da8db50db92b9",
    ),
    (
        "docs/templates/harness-improvement.md",
        "cbdab1aefab8a38840cc408ce512e16f63acbbcdb0acacf271599e149a5ea608",
    ),
];

pub static RELEASE_0_1_10_FILES: &[(&str, &str)] = &[
    (
        ".agents/skills/audit-onboarding-proposal/SKILL.md",
        "8aa6e5d75743f747818a72cb2376efea9971882c23ebaaee18489095cad5f53e",
    ),
    (
        ".agents/skills/audit-onboarding-proposal/agents/openai.yaml",
        "1f822840d05d627c43e8dcb87a8c54ef28e52c24cba72cf6950c142e360e1a81",
    ),
    (
        ".agents/skills/audit-onboarding-proposal/scripts/validate_evidence_capsule.py",
        "9ea918717fa72f9040e851d93c3e9783a6e0100a94d23bec0d31a96f63859cb0",
    ),
    (
        ".agents/skills/encode-invariant/SKILL.md",
        "cd2742df488a1046b049fae868094df7b2f882ff98488fd76579468c3b41e606",
    ),
    (
        ".agents/skills/encode-invariant/agents/openai.yaml",
        "480be69346c2af2d8f6f381692add09a71593dc6746b8b1cd603412e40e75f2a",
    ),
    (
        ".agents/skills/improve-harness/SKILL.md",
        "1061235593bc7f9564671d2912bccc0b45397ced9a2193d8695c8fafc4b3cfe6",
    ),
    (
        ".agents/skills/improve-harness/agents/openai.yaml",
        "77982c7e2c70c3becb5076c2ce10eefb49ada39f53ed01da930f092fe6d263c5",
    ),
    (
        ".agents/skills/onboard-repository/SKILL.md",
        "220302bec89ebe70f983fd515bdffd3e417a652c0777fe8ba8d34a0197cf6f3e",
    ),
    (
        ".agents/skills/onboard-repository/agents/openai.yaml",
        "0992cf606b2680397e6aee60db3d39c62aa3d5773acaeefc2f4f1e18103537e9",
    ),
    (
        ".agents/skills/onboard-repository/references/evidence-capsule-v1.md",
        "b9e1f9208842e725289469d0976b3e65670a30ee7f69be6d799cf4ba2a19fadb",
    ),
    (
        ".agents/skills/onboard-repository/references/evidence-capsule-v2.md",
        "e91ef78922e3a196e966a7e60ae53e4894617668887abf165b293d147fb88737",
    ),
    (
        ".agents/skills/onboard-repository/scripts/emit_evidence_bundle.py",
        "30583170ed4764b9ee3a147bc6e02ebce5095600a292688b9411825404f9e13f",
    ),
    (
        ".agents/skills/onboard-repository/scripts/render_patch.py",
        "9e6b5c4b148e78171695ab964757e792d2a67b32517d08ad113df8de2da9cb38",
    ),
    (
        "AGENTS.md",
        "745a9e87a0f3881cabbef2e2de654cb8ff844d0636600fa38505d31b4abf8d73",
    ),
    (
        "docs/README.md",
        "c94d45be05355194908c38f8fa2352731d3e6341175a9785e3aa63d55cff160f",
    ),
    (
        "docs/WORKFLOW.md",
        "2aa47d7ba7e45d0284bccf5fd9ab3261f2c1ec73b3e855eb26cfce3d676bf979",
    ),
    (
        "docs/decisions/README.md",
        "521abd46ee1291dfe13a500664f6046723c0f8360728501da434c0292ed92b2b",
    ),
    (
        "docs/patterns/encoding-invariants.md",
        "c49f7fe5c5995e11f095c70791f731874c8cb8e1cdabef36d26581aa0a25ffd5",
    ),
    (
        "docs/plans/README.md",
        "e552bbb5a2940754a9c4ab7d4582ec68dba12f6a536da6cb2d9a7a2effc31550",
    ),
    (
        "docs/plans/active/README.md",
        "aec64998a72585b2a9c4c8909c7ed775c396c4c2e39c71b0ce4ee6e392784160",
    ),
    (
        "docs/plans/completed/README.md",
        "1ba887f4fb54b7aa45e3f4f619d4b2dfa1eb63d52c09fbbd9588598862346dc7",
    ),
    (
        "docs/product/README.md",
        "463441ca9b77d8ae5cbb40fd653277496b005447f98f77d6f3bc3ca51195a65b",
    ),
    (
        "docs/templates/application-runbook.md",
        "1e87f2189066dd0469555406c9ef63c3939e53adfcb23494d2f13f0b9552a72d",
    ),
    (
        "docs/templates/decision.md",
        "e92ba7a5b7bc10ce6c213af75a191619fc305c3945cdd55d4dc6bac72dcd42c1",
    ),
    (
        "docs/templates/exec-plan.md",
        "111c40ea8ab63a03561fdb54f235358546788b3a4e1bb26f220da8db50db92b9",
    ),
    (
        "docs/templates/harness-improvement.md",
        "cbdab1aefab8a38840cc408ce512e16f63acbbcdb0acacf271599e149a5ea608",
    ),
];

pub static BASELINE_WISDOM_FILES: &[(&str, &str)] = &[
    (
        ".agents/skills/engineering-wisdom/SKILL.md",
        "049e8c08aa775e622d119217ab51c68eba90fa5b5d2655ae0ea66d1930d72c01",
    ),
    (
        ".agents/skills/engineering-wisdom/agents/openai.yaml",
        "53af5b2bc808731b555a067a4238faf342769b375bfff08c489202773ce164aa",
    ),
    (
        ".agents/skills/engineering-wisdom/references/heuristics.md",
        "316802d6fede2123ff28358107ea212394f2fd1aed22d68d69938a18fe62a86a",
    ),
    (
        ".agents/skills/engineering-wisdom/references/sources.md",
        "54ac2e92aaa171809877640808f706b3d4d7016cff192e47b504a44c8f536612",
    ),
];

pub static RELEASE_0_1_10_WISDOM_FILES: &[(&str, &str)] = &[
    (
        ".agents/skills/engineering-wisdom/SKILL.md",
        "ef5552861a03749e83688c7207601d283806bffc131c72225239c5ae32c70a4e",
    ),
    (
        ".agents/skills/engineering-wisdom/agents/openai.yaml",
        "53af5b2bc808731b555a067a4238faf342769b375bfff08c489202773ce164aa",
    ),
    (
        ".agents/skills/engineering-wisdom/references/heuristics.md",
        "316802d6fede2123ff28358107ea212394f2fd1aed22d68d69938a18fe62a86a",
    ),
    (
        ".agents/skills/engineering-wisdom/references/sources.md",
        "54ac2e92aaa171809877640808f706b3d4d7016cff192e47b504a44c8f536612",
    ),
];

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct HarnessManifest {
    schema_version: u32,
    core_version: String,
    files: Vec<HarnessManifestFile>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct HarnessManifestFile {
    path: String,
    upstream_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BaselineIdentity {
    SourceBaseline,
    Release0110,
    Unknown(String),
}

impl BaselineIdentity {
    fn as_str(&self) -> &str {
        match self {
            Self::SourceBaseline => "source-baseline",
            Self::Release0110 => "release-0.1.10",
            Self::Unknown(_) => "unknown",
        }
    }

    fn is_known(&self) -> bool {
        matches!(self, Self::SourceBaseline | Self::Release0110)
    }
}

struct DetectionReport {
    detected: bool,
    legacy_version: Option<String>,
    baseline_identity: Option<BaselineIdentity>,
    manifest: Option<HarnessManifest>,
    wisdom_detected: bool,
    claude_detected: bool,
    already_migrated: bool,
    pending_transaction: bool,
    blockers: Vec<String>,
}

fn match_baseline(manifest: &HarnessManifest) -> BaselineIdentity {
    let mut seen = BTreeSet::new();
    for f in &manifest.files {
        if domain::RelativePath::new(&f.path).is_err() {
            return BaselineIdentity::Unknown(format!(
                "invalid portable path in manifest: {}",
                f.path
            ));
        }
        if !seen.insert(&f.path) {
            return BaselineIdentity::Unknown("duplicate file path in manifest".into());
        }
    }

    let has_wisdom = manifest.files.len() == 30;
    if manifest.files.len() != 26 && !has_wisdom {
        return BaselineIdentity::Unknown(format!(
            "manifest file count {} does not match known baseline (expected 26 or 30)",
            manifest.files.len()
        ));
    }

    let manifest_map: BTreeMap<&str, &str> = manifest
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.upstream_sha256.as_str()))
        .collect();

    let check_table = |core: &[(&str, &str)], wisdom: &[(&str, &str)]| -> bool {
        for (path, hash) in core {
            if manifest_map.get(path) != Some(hash) {
                return false;
            }
        }
        if has_wisdom {
            for (path, hash) in wisdom {
                if manifest_map.get(path) != Some(hash) {
                    return false;
                }
            }
        }
        true
    };

    if check_table(BASELINE_FILES, BASELINE_WISDOM_FILES) {
        BaselineIdentity::SourceBaseline
    } else if check_table(RELEASE_0_1_10_FILES, RELEASE_0_1_10_WISDOM_FILES) {
        BaselineIdentity::Release0110
    } else {
        BaselineIdentity::Unknown(format!(
            "unverified baseline with version {}",
            manifest.core_version
        ))
    }
}

fn detect(root: &Path) -> Result<DetectionReport> {
    let canonical = root.canonicalize().map_err(Failure::from)?;
    if !canonical.is_dir() {
        return Err(Failure::new(
            "UNSAFE_PATH",
            "workspace root is not a directory",
        ));
    }

    let already_migrated = fs::read(root, STATE, domain::CONTROL_LIMIT)?.is_some();
    let mut blockers = Vec::new();
    if already_migrated {
        blockers.push("repository already has AgentFoundry installation; migration cannot overwrite existing state".into());
    }

    let manifest_bytes = fs::read(root, HARNESS_MANIFEST, domain::CONTROL_LIMIT)?;
    let Some(raw_manifest) = manifest_bytes else {
        let legacy_bin = fs::read(root, "scripts/bin/harness", 128 * 1024 * 1024)?;
        let legacy_core_dir =
            fs::safe(root, ".harness-core").is_ok() && root.join(".harness-core").exists();
        let detected = legacy_bin.is_some() || legacy_core_dir;
        let mut b = blockers;
        if detected {
            b.push("Harness core structure found but .harness-core/manifest.json is missing or inaccessible".into());
        } else {
            b.push("no Harness installation detected at workspace root".into());
        }
        return Ok(DetectionReport {
            detected,
            legacy_version: None,
            baseline_identity: None,
            manifest: None,
            wisdom_detected: false,
            claude_detected: false,
            already_migrated,
            pending_transaction: application::pending(root)?,
            blockers: b,
        });
    };

    let manifest: HarnessManifest = serde_json::from_slice(&raw_manifest).map_err(|e| {
        Failure::new(
            "INVALID_STATE",
            format!("corrupt .harness-core/manifest.json: {e}"),
        )
    })?;

    if manifest.schema_version != 1 {
        let legacy_version = manifest.core_version.clone();
        let mut b = blockers;
        b.push("unsupported Harness manifest schema version".into());
        return Ok(DetectionReport {
            detected: true,
            legacy_version: Some(legacy_version),
            baseline_identity: Some(BaselineIdentity::Unknown("unsupported_schema".into())),
            manifest: Some(manifest),
            wisdom_detected: false,
            claude_detected: false,
            already_migrated,
            pending_transaction: false,
            blockers: b,
        });
    }

    let identity = match_baseline(&manifest);

    let pending_transaction = application::pending(root)?
        || fs::read(root, HARNESS_TRANSACTION, domain::CONTROL_LIMIT)?.is_some()
        || fs::read(root, HARNESS_UPDATE_SESSION, domain::CONTROL_LIMIT)?.is_some();

    if pending_transaction {
        blockers.push(
            "pending transaction exists; complete or recover transaction before migration".into(),
        );
    }

    if !identity.is_known() {
        blockers.push(format!(
            "unknown Harness version {} or unverified baseline; migration v1 only supports verified baseline {} and release 0.1.10",
            manifest.core_version, SOURCE_BASELINE_COMMIT
        ));
    }

    let wisdom_detected = fs::read(
        root,
        ".agents/skills/engineering-wisdom/SKILL.md",
        8 * 1024 * 1024,
    )?
    .is_some()
        || manifest
            .files
            .iter()
            .any(|f| f.path == ".agents/skills/engineering-wisdom/SKILL.md");

    let claude_file = fs::read(root, "CLAUDE.md", 8 * 1024 * 1024)?;
    let claude_detected = match &claude_file {
        Some(bytes) => {
            let has_harness = bytes
                .windows(b"<!-- HARNESS:BEGIN -->".len())
                .any(|w| w == b"<!-- HARNESS:BEGIN -->")
                || bytes
                    .windows(b"<!-- HARNESS:END -->".len())
                    .any(|w| w == b"<!-- HARNESS:END -->");
            let has_agentfoundry =
                domain::block_content(bytes, "AGENTFOUNDRY:CLAUDE").is_ok_and(|opt| opt.is_some());
            has_harness || has_agentfoundry
        }
        None => false,
    };

    let legacy_version = manifest.core_version.clone();

    Ok(DetectionReport {
        detected: true,
        legacy_version: Some(legacy_version),
        baseline_identity: Some(identity),
        manifest: Some(manifest),
        wisdom_detected,
        claude_detected,
        already_migrated,
        pending_transaction,
        blockers,
    })
}

pub fn inventory(root: &Path) -> Result<serde_json::Value> {
    let report = detect(root)?;

    let dist = crate::models::Distribution::parse(super::PAYLOAD_MANIFEST.as_bytes())?;
    let mut mappings = Vec::new();
    let mut blockers = report.blockers.clone();
    let mut retained_legacy = Vec::new();

    if report.detected {
        for candidate in [
            ".harness-core",
            ".harness-backup",
            "scripts/bin/harness",
            "scripts/bin/harness.exe",
        ] {
            if fs::safe(root, candidate).is_ok() && root.join(candidate).exists() {
                retained_legacy.push(candidate.to_string());
            }
        }
    }

    if let Some(manifest) = &report.manifest {
        let manifest_map: BTreeMap<&str, &str> = manifest
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.upstream_sha256.as_str()))
            .collect();

        for entry in &dist.entries {
            if entry.addon.as_deref() == Some("engineering-wisdom") && !report.wisdom_detected {
                continue;
            }
            if entry.adapter.as_deref() == Some("claude") && !report.claude_detected {
                continue;
            }

            let legacy_source = &entry.logical_id;
            let dest = &entry.target;
            let dest_exists = fs::safe(root, dest).is_ok() && root.join(dest).exists();

            if domain::RelativePath::new(legacy_source).is_err() {
                blockers.push(format!(
                    "invalid portable path for legacy source: {legacy_source}"
                ));
                continue;
            }

            let (base_hash, locally_modified, local_hash) =
                if let Some(expected_base) = manifest_map.get(legacy_source.as_str()) {
                    let local_bytes = fs::read(root, legacy_source, 8 * 1024 * 1024)?;
                    let local_h = local_bytes.as_deref().map(fs::digest);
                    let modified = local_h.as_deref() != Some(*expected_base);

                    let base_path = format!("{HARNESS_BASE_DIR}/{legacy_source}");
                    let base_bytes = fs::read(root, &base_path, 8 * 1024 * 1024)?;
                    if let Some(base_data) = &base_bytes {
                        if &fs::digest(base_data) != expected_base {
                            blockers.push(format!(
                                "base content mismatch for legacy file: {legacy_source}"
                            ));
                        }
                    } else if report
                        .baseline_identity
                        .as_ref()
                        .is_some_and(|b| b.is_known())
                    {
                        blockers.push(format!("missing legacy base file: {base_path}"));
                    }

                    (Some(expected_base.to_string()), modified, local_h)
                } else {
                    let local_bytes = fs::read(root, legacy_source, 8 * 1024 * 1024)?;
                    let local_h = local_bytes.as_deref().map(fs::digest);
                    (None, false, local_h)
                };

            let ownership_str = match entry.ownership {
                crate::models::Ownership::ManagedFile => "managed_file",
                crate::models::Ownership::ManagedBlock => "managed_block",
                crate::models::Ownership::SeedOnce => "seed_once",
                crate::models::Ownership::ConsumerOwned => "consumer_owned",
                crate::models::Ownership::Executable => "executable",
            };

            mappings.push(serde_json::json!({
                "source": legacy_source,
                "destination": dest,
                "ownership": ownership_str,
                "locally_modified": locally_modified,
                "base_hash": base_hash,
                "local_hash": local_hash,
                "target_exists": dest_exists,
            }));
        }
    }

    let baseline_verified = report
        .baseline_identity
        .as_ref()
        .is_some_and(|b| b.is_known());

    let can_migrate = if baseline_verified && blockers.is_empty() && !report.already_migrated {
        match prepare(root) {
            Ok(_) => true,
            Err(e) => {
                blockers.push(format!("migration plan diagnostic: {}", e.message));
                false
            }
        }
    } else {
        false
    };

    Ok(serde_json::json!({
        "detected": report.detected,
        "legacy_version": report.legacy_version,
        "baseline_identity": report.baseline_identity.as_ref().map(|b| b.as_str()),
        "baseline_verified": baseline_verified,
        "known_baseline": baseline_verified,
        "can_migrate": can_migrate,
        "choices": {
            "wisdom": report.wisdom_detected,
            "claude": report.claude_detected,
        },
        "mappings": mappings,
        "retained_legacy": retained_legacy,
        "blockers": blockers,
    }))
}

pub fn replace_legacy_block(
    container: &[u8],
    new_block: &[u8],
    marker_namespace: &str,
) -> Result<Vec<u8>> {
    let begin_harness = b"<!-- HARNESS:BEGIN -->";
    let end_harness = b"<!-- HARNESS:END -->";

    let begins = container
        .windows(begin_harness.len())
        .enumerate()
        .filter_map(|(i, w)| (w == begin_harness).then_some(i))
        .collect::<Vec<_>>();
    let ends = container
        .windows(end_harness.len())
        .enumerate()
        .filter_map(|(i, w)| (w == end_harness).then_some(i))
        .collect::<Vec<_>>();

    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => {
            if domain::block_content(container, marker_namespace).is_ok_and(|opt| opt.is_some()) {
                return Ok(container.to_vec());
            }
            domain::patch_block(container, new_block, marker_namespace)
                .map_err(|e| Failure::new("INVALID_STATE", e))
        }
        ([b], [e]) if b < e => {
            domain::block_content(container, marker_namespace)
                .map_err(|e| Failure::new("INVALID_STATE", e))?;
            if container[*b..*e]
                .windows(b"<!-- AGENTFOUNDRY:".len())
                .any(|bytes| bytes == b"<!-- AGENTFOUNDRY:")
            {
                return Err(Failure::new(
                    "INVALID_STATE",
                    "AgentFoundry marker nested inside Harness block",
                ));
            }
            let mut out = Vec::with_capacity(container.len() + new_block.len());
            out.extend_from_slice(&container[..*b]);
            out.extend_from_slice(new_block.strip_suffix(b"\n").unwrap_or(new_block));
            out.extend_from_slice(&container[e + end_harness.len()..]);

            domain::block_content(&out, marker_namespace)
                .map_err(|e| Failure::new("INVALID_STATE", e))?;
            Ok(out)
        }
        _ => Err(Failure::new(
            "INVALID_STATE",
            "malformed, reversed or duplicate Harness marker pair",
        )),
    }
}

pub fn prepare(root: &Path) -> Result<Plan> {
    let report = detect(root)?;

    if report.already_migrated {
        return Err(Failure::new(
            "INVALID_STATE",
            "repository already has AgentFoundry installation; migration cannot overwrite existing state",
        ));
    }
    if !report.detected {
        return Err(Failure::new(
            "INVALID_ARGUMENT",
            "no Harness installation detected to migrate",
        ));
    }
    if report.pending_transaction {
        return Err(Failure::new(
            "RECOVERY_REQUIRED",
            "pending transaction exists; complete or recover transaction before migration",
        ));
    }
    let Some(identity) = &report.baseline_identity else {
        return Err(Failure::new(
            "MISSING_CAPABILITY",
            "unverified Harness baseline; migration requires known baseline or release 0.1.10",
        ));
    };
    if !identity.is_known() {
        return Err(Failure::new(
            "MISSING_CAPABILITY",
            format!(
                "unknown Harness version {} or unverified baseline; migration v1 only supports verified baseline {} and release 0.1.10",
                report.legacy_version.unwrap_or_default(),
                SOURCE_BASELINE_COMMIT
            ),
        ));
    }
    if !report.blockers.is_empty() {
        return Err(Failure::new("INVALID_STATE", report.blockers.join("; ")));
    }

    let manifest = report
        .manifest
        .ok_or_else(|| Failure::new("INVALID_STATE", "missing harness manifest"))?;

    let dist = crate::models::Distribution::parse(super::PAYLOAD_MANIFEST.as_bytes())?;

    for entry in &dist.entries {
        if entry.addon.as_deref() == Some("engineering-wisdom") && !report.wisdom_detected {
            continue;
        }
        if entry.adapter.as_deref() == Some("claude") && !report.claude_detected {
            continue;
        }
        if entry.ownership != crate::models::Ownership::ManagedBlock
            && fs::read(root, &entry.target, domain::CONTROL_LIMIT)?.is_some()
        {
            return Err(Failure::new(
                "INVALID_ARGUMENT",
                format!(
                    "existing target {}; migration blocked by unmanaged collision",
                    entry.target
                ),
            ));
        }
    }

    let args = Args {
        directory: root.to_string_lossy().into(),
        wisdom: report.wisdom_detected,
        claude: report.claude_detected,
        merge: true,
        refresh: true,
        ..Default::default()
    };

    let (mut plan, _warnings) = super::install_plan(root, &args)?;

    let state_idx = plan
        .operations
        .iter()
        .position(|op| op.target == STATE)
        .ok_or_else(|| {
            Failure::new(
                "INVALID_STATE",
                "install plan missing state commit operation",
            )
        })?;

    let mut state: crate::models::InstallationState =
        serde_json::from_slice(plan.operations[state_idx].after.as_ref().ok_or_else(|| {
            Failure::new("INVALID_STATE", "missing state payload in install plan")
        })?)?;

    let mut guard_ops = Vec::new();

    let manifest_bytes =
        fs::read(root, HARNESS_MANIFEST, domain::CONTROL_LIMIT)?.ok_or_else(|| {
            Failure::new(
                "INVALID_STATE",
                "could not read harness manifest for preimage guard",
            )
        })?;
    guard_ops.push(Operation {
        target: HARNESS_MANIFEST.into(),
        before: Some(manifest_bytes.clone()),
        after: Some(manifest_bytes),
        mode: super::existing_mode(root, HARNESS_MANIFEST, 0o644)? & 0o777,
    });

    for f in &manifest.files {
        let legacy_source = &f.path;
        domain::RelativePath::new(legacy_source).map_err(|e| Failure::new("INVALID_STATE", e))?;

        let base_target = format!("{HARNESS_BASE_DIR}/{legacy_source}");
        let base_bytes = fs::read(root, &base_target, 8 * 1024 * 1024)?.ok_or_else(|| {
            Failure::new(
                "INVALID_STATE",
                format!("missing legacy baseline file: {base_target}"),
            )
        })?;

        if fs::digest(&base_bytes) != f.upstream_sha256 {
            return Err(Failure::new(
                "INTEGRITY_FAILURE",
                format!("corrupt legacy baseline file: {base_target}"),
            ));
        }

        guard_ops.push(Operation {
            target: base_target,
            before: Some(base_bytes.clone()),
            after: Some(base_bytes.clone()),
            mode: super::existing_mode(
                root,
                &format!("{HARNESS_BASE_DIR}/{legacy_source}"),
                0o644,
            )? & 0o777,
        });

        let local_bytes = fs::read(root, legacy_source, 8 * 1024 * 1024)?;

        if legacy_source != "AGENTS.md" && legacy_source != "CLAUDE.md" {
            match &local_bytes {
                Some(bytes) => {
                    guard_ops.push(Operation {
                        target: legacy_source.clone(),
                        before: Some(bytes.clone()),
                        after: Some(bytes.clone()),
                        mode: super::existing_mode(root, legacy_source, 0o644)? & 0o777,
                    });
                }
                None => {
                    guard_ops.push(Operation {
                        target: legacy_source.clone(),
                        before: None,
                        after: None,
                        mode: 0o644,
                    });
                }
            }
        }

        if let Some(entry) = dist.entries.iter().find(|e| e.logical_id == *legacy_source) {
            let dest_target = &entry.target;
            let upstream_bytes = super::PAYLOAD
                .iter()
                .find(|(s, _)| *s == entry.source)
                .map(|(_, b)| *b)
                .ok_or_else(|| {
                    Failure::new("INTEGRITY_FAILURE", "embedded payload entry missing")
                })?;

            if entry.ownership == crate::models::Ownership::ManagedBlock {
                let marker_name = if dest_target == "CLAUDE.md" {
                    "AGENTFOUNDRY:CLAUDE"
                } else {
                    "AGENTFOUNDRY"
                };
                if let Some(op) = plan
                    .operations
                    .iter_mut()
                    .find(|o| o.target == *dest_target)
                {
                    let current = op.before.as_deref().unwrap_or_default();
                    let updated = replace_legacy_block(current, upstream_bytes, marker_name)?;
                    op.after = Some(updated);
                }
            } else {
                let merge_result = merge::three_way(
                    Some(&base_bytes),
                    local_bytes.as_deref(),
                    Some(upstream_bytes),
                )?;

                match merge_result {
                    Merge::Clean(Some(merged)) => {
                        if let Some(op) = plan
                            .operations
                            .iter_mut()
                            .find(|o| o.target == *dest_target)
                        {
                            op.after = Some(merged);
                        }
                    }
                    Merge::Clean(None) => {
                        plan.operations.retain(|o| o.target != *dest_target);
                        state.entries.retain(|e| e.target != *dest_target);
                    }
                    Merge::Conflict(_) => {
                        return Err(Failure::new(
                            "CONFLICT_PENDING",
                            format!(
                                "unresolved three-way merge conflict for legacy file {legacy_source}"
                            ),
                        ));
                    }
                    Merge::Structural(reason) => {
                        return Err(Failure::new(
                            "CONFLICT_PENDING",
                            format!(
                                "structural merge conflict ({reason}) for legacy file {legacy_source}"
                            ),
                        ));
                    }
                }
            }
        }
    }

    if report.claude_detected
        && !manifest.files.iter().any(|f| f.path == "CLAUDE.md")
        && let Some(entry) = dist.entries.iter().find(|e| e.target == "CLAUDE.md")
    {
        let upstream_bytes = super::PAYLOAD
            .iter()
            .find(|(s, _)| *s == entry.source)
            .map(|(_, b)| *b)
            .ok_or_else(|| Failure::new("INTEGRITY_FAILURE", "embedded claude payload missing"))?;

        if let Some(op) = plan.operations.iter_mut().find(|o| o.target == "CLAUDE.md") {
            let current = op.before.as_deref().unwrap_or_default();
            let updated = replace_legacy_block(current, upstream_bytes, "AGENTFOUNDRY:CLAUDE")?;
            op.after = Some(updated);
        }
    }

    let updated_state_bytes = serde_json::to_vec_pretty(&state)?;
    let state_idx = plan
        .operations
        .iter()
        .position(|op| op.target == STATE)
        .unwrap();

    let mut state_op = plan.operations.remove(state_idx);
    state_op.after = Some(updated_state_bytes);

    for guard_op in guard_ops {
        if !plan.operations.iter().any(|o| o.target == guard_op.target) {
            plan.operations.push(guard_op);
        }
    }

    plan.operations.push(state_op);
    plan.freeze(root)?;

    crate::application::verify(&plan, root)?;
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_legacy_block_preserves_outer_content() {
        let before = b"# Consumer Header\r\nLocal rule.\r\n<!-- HARNESS:BEGIN -->\r\nold harness\r\n<!-- HARNESS:END -->\r\nLocal footer.\r\n";
        let new_block =
            b"<!-- AGENTFOUNDRY:BEGIN -->\nnew agentfoundry\n<!-- AGENTFOUNDRY:END -->\n";
        let out = replace_legacy_block(before, new_block, "AGENTFOUNDRY").unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("# Consumer Header\r\nLocal rule.\r\n"));
        assert!(text.contains("\r\nLocal footer.\r\n"));
        assert!(
            text.contains(
                "<!-- AGENTFOUNDRY:BEGIN -->\nnew agentfoundry\n<!-- AGENTFOUNDRY:END -->"
            )
        );
        assert!(!text.contains("HARNESS"));
    }

    #[test]
    fn replace_legacy_block_rejects_malformed_markers() {
        let malformed = b"<!-- HARNESS:BEGIN -->\nunterminated\n";
        let new_block = b"<!-- AGENTFOUNDRY:BEGIN -->\nnew\n<!-- AGENTFOUNDRY:END -->\n";
        assert!(replace_legacy_block(malformed, new_block, "AGENTFOUNDRY").is_err());
        let nested = b"<!-- HARNESS:BEGIN --><!-- AGENTFOUNDRY:BEGIN -->private<!-- AGENTFOUNDRY:END --><!-- HARNESS:END -->";
        assert!(replace_legacy_block(nested, new_block, "AGENTFOUNDRY").is_err());
    }

    #[test]
    fn replace_legacy_block_idempotent_on_migrated() {
        let migrated =
            b"# Header\n\n<!-- AGENTFOUNDRY:BEGIN -->\ncurrent\n<!-- AGENTFOUNDRY:END -->\n";
        let new_block = b"<!-- AGENTFOUNDRY:BEGIN -->\nnew\n<!-- AGENTFOUNDRY:END -->\n";
        let out = replace_legacy_block(migrated, new_block, "AGENTFOUNDRY").unwrap();
        assert_eq!(out, migrated);
    }
}
