use crate::release::ReleaseMetadata;
use std::collections::BTreeSet;
use std::io::Read;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const GITHUB_REPOSITORY: &str = "anquocviet/agent-foundry";
pub const LATEST_RELEASE_API_URL: &str =
    "https://api.github.com/repos/anquocviet/agent-foundry/releases/latest";
pub const METADATA_ASSET_NAME: &str = "agentfoundry-release.json";
pub const MAX_BODY_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
pub const MAX_REDIRECTS: usize = 5;

#[derive(Debug, serde::Deserialize, Clone)]
pub(crate) struct GithubReleaseDto {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<GithubAssetDto>,
}

#[derive(Debug, serde::Deserialize, Clone)]
pub(crate) struct GithubAssetDto {
    pub name: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum UrlContext {
    Api,
    Download,
}

pub(crate) fn validate_url(url_str: &str, context: UrlContext) -> crate::Result<()> {
    let uri: ureq::http::Uri = url_str.parse().map_err(|e| {
        crate::Failure::new("NETWORK_ERROR", format!("malformed URL '{url_str}': {e}"))
    })?;

    if uri.scheme_str() != Some("https") {
        return Err(crate::Failure::new(
            "NETWORK_ERROR",
            format!("unsupported non-HTTPS scheme in URL: {url_str}"),
        ));
    }

    let authority = uri.authority().ok_or_else(|| {
        crate::Failure::new("NETWORK_ERROR", format!("URL missing authority: {url_str}"))
    })?;

    if authority.as_str().contains('@') {
        return Err(crate::Failure::new(
            "NETWORK_ERROR",
            format!("userinfo is forbidden in URL: {url_str}"),
        ));
    }

    if let Some(port) = authority.port_u16()
        && port != 443
    {
        return Err(crate::Failure::new(
            "NETWORK_ERROR",
            format!("nonstandard HTTPS port {port} in URL: {url_str}"),
        ));
    }

    let host = uri.host().ok_or_else(|| {
        crate::Failure::new("NETWORK_ERROR", format!("URL missing host: {url_str}"))
    })?;
    let host_lower = host.to_ascii_lowercase();

    let path = uri.path();
    if path.split('/').any(|segment| matches!(segment, "." | ".."))
        || (host_lower != "release-assets.githubusercontent.com" && path.contains('%'))
    {
        return Err(crate::Failure::new(
            "NETWORK_ERROR",
            format!("path traversal detected in URL: {url_str}"),
        ));
    }
    let path_lower = path.to_ascii_lowercase();

    match context {
        UrlContext::Api => {
            if host_lower != "api.github.com" {
                return Err(crate::Failure::new(
                    "NETWORK_ERROR",
                    format!("API URL host must be api.github.com: {url_str}"),
                ));
            }
            if !path_lower.starts_with("/repos/anquocviet/agent-foundry/") {
                return Err(crate::Failure::new(
                    "NETWORK_ERROR",
                    format!("API URL must be within anquocviet/agent-foundry: {url_str}"),
                ));
            }
        }
        UrlContext::Download => {
            if host_lower == "github.com" {
                if !path_lower.starts_with("/anquocviet/agent-foundry/releases/download/") {
                    return Err(crate::Failure::new(
                        "NETWORK_ERROR",
                        format!(
                            "download URL on github.com must be within anquocviet/agent-foundry/releases/download: {url_str}"
                        ),
                    ));
                }
            } else if host_lower == "release-assets.githubusercontent.com" {
                // Allowed for signed redirects
            } else {
                return Err(crate::Failure::new(
                    "NETWORK_ERROR",
                    format!("download URL host not permitted ({host_lower}): {url_str}"),
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn validate_asset_inventory(
    release_dto: &GithubReleaseDto,
    metadata: &ReleaseMetadata,
) -> crate::Result<()> {
    let mut api_names = BTreeSet::new();
    for asset in &release_dto.assets {
        if !api_names.insert(&asset.name) {
            return Err(crate::Failure::new(
                "INTEGRITY_FAILURE",
                format!("duplicate asset name in GitHub release: {}", asset.name),
            ));
        }
    }

    if release_dto.assets.len() != 11 {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "API assets must contain exactly 11 items, found {}",
                release_dto.assets.len()
            ),
        ));
    }

    let mut expected_names = BTreeSet::new();
    for asset in &metadata.assets {
        expected_names.insert(&asset.name);
        expected_names.insert(&asset.checksum_name);
    }
    let metadata_name = METADATA_ASSET_NAME.to_string();
    expected_names.insert(&metadata_name);

    if expected_names.len() != 11 {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            "metadata assets and sidecars must produce 11 distinct names",
        ));
    }

    if api_names != expected_names {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            "API assets do not match expected release inventory (5 binaries + 5 checksums + agentfoundry-release.json)",
        ));
    }

    Ok(())
}

pub(crate) fn get_with_redirects_bounded(
    agent: &ureq::Agent,
    initial_url: &str,
    context: UrlContext,
    accept: &str,
    deadline: Instant,
    max_body_bytes: usize,
) -> crate::Result<Vec<u8>> {
    let mut current_url = initial_url.to_string();
    validate_url(&current_url, context)?;

    for _ in 0..=MAX_REDIRECTS {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| {
                crate::Failure::new("NETWORK_ERROR", "release observation deadline exceeded")
            })?;
        let mut response = match agent
            .get(&current_url)
            .header("User-Agent", "agentfoundry")
            .header("Accept", accept)
            .config()
            .timeout_global(Some(remaining))
            .build()
            .call()
        {
            Ok(res) => res,
            Err(e) => {
                return Err(crate::Failure::new(
                    "NETWORK_ERROR",
                    format!("HTTP request failed for {current_url}: {e}"),
                ));
            }
        };

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(crate::Failure::new(
                "RELEASE_UNAVAILABLE",
                format!("release unavailable (HTTP 404): {current_url}"),
            ));
        }

        if status.is_redirection() {
            let location = response
                .headers()
                .get("location")
                .ok_or_else(|| {
                    crate::Failure::new(
                        "NETWORK_ERROR",
                        format!("redirect from {current_url} missing Location header"),
                    )
                })?
                .to_str()
                .map_err(|e| {
                    crate::Failure::new(
                        "NETWORK_ERROR",
                        format!("invalid Location header in redirect from {current_url}: {e}"),
                    )
                })?;

            let next_url = if location.starts_with('/') {
                let current_uri: ureq::http::Uri = current_url.parse().map_err(|e| {
                    crate::Failure::new(
                        "NETWORK_ERROR",
                        format!("failed to parse current URL {current_url}: {e}"),
                    )
                })?;
                let host = current_uri.host().ok_or_else(|| {
                    crate::Failure::new(
                        "NETWORK_ERROR",
                        format!("current URL {current_url} has no host"),
                    )
                })?;
                format!("https://{host}{location}")
            } else if location.starts_with("https://") {
                location.to_string()
            } else {
                return Err(crate::Failure::new(
                    "NETWORK_ERROR",
                    format!("unsupported redirect target: {location}"),
                ));
            };

            validate_url(&next_url, context)?;
            current_url = next_url;
            continue;
        }

        if !status.is_success() {
            return Err(crate::Failure::new(
                "NETWORK_ERROR",
                format!("HTTP status {} for {current_url}", status.as_u16()),
            ));
        }

        let mut body = Vec::new();
        let mut reader = response
            .body_mut()
            .as_reader()
            .take((max_body_bytes + 1) as u64);
        reader.read_to_end(&mut body).map_err(|e| {
            crate::Failure::new(
                "NETWORK_ERROR",
                format!("failed to read response body from {current_url}: {e}"),
            )
        })?;

        if body.len() > max_body_bytes {
            return Err(crate::Failure::new(
                "NETWORK_ERROR",
                format!("response body from {current_url} exceeded byte limit"),
            ));
        }

        return Ok(body);
    }

    Err(crate::Failure::new(
        "NETWORK_ERROR",
        format!("exceeded maximum redirect limit ({MAX_REDIRECTS}) for {initial_url}"),
    ))
}

fn get_with_redirects(
    agent: &ureq::Agent,
    initial_url: &str,
    context: UrlContext,
    accept: &str,
    deadline: Instant,
) -> crate::Result<Vec<u8>> {
    get_with_redirects_bounded(
        agent,
        initial_url,
        context,
        accept,
        deadline,
        MAX_BODY_BYTES,
    )
}

pub fn resolve() -> crate::Result<ReleaseMetadata> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .https_only(true)
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = agent_config.into();

    let api_bytes = get_with_redirects(
        &agent,
        LATEST_RELEASE_API_URL,
        UrlContext::Api,
        "application/vnd.github+json",
        deadline,
    )?;

    let release_dto: GithubReleaseDto = serde_json::from_slice(&api_bytes).map_err(|e| {
        crate::Failure::new(
            "NETWORK_ERROR",
            format!("failed to parse GitHub release API response: {e}"),
        )
    })?;

    if release_dto.draft || release_dto.prerelease {
        return Err(crate::Failure::new(
            "RELEASE_UNAVAILABLE",
            "latest release is marked draft or prerelease",
        ));
    }

    let version = release_dto.tag_name.strip_prefix('v').ok_or_else(|| {
        crate::Failure::new(
            "INTEGRITY_FAILURE",
            "release tag must be stable vMAJOR.MINOR.PATCH",
        )
    })?;
    let parts = version.split('.').collect::<Vec<_>>();
    if parts.len() != 3
        || parts.iter().any(|part| {
            part.is_empty()
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || (part.len() > 1 && part.starts_with('0'))
                || part.parse::<u64>().is_err()
        })
    {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            "release tag must be stable vMAJOR.MINOR.PATCH",
        ));
    }
    let metadata_url = format!(
        "https://github.com/{GITHUB_REPOSITORY}/releases/download/{}/{METADATA_ASSET_NAME}",
        release_dto.tag_name
    );

    let metadata_bytes = get_with_redirects(
        &agent,
        &metadata_url,
        UrlContext::Download,
        "application/json, */*",
        deadline,
    )?;

    let metadata = ReleaseMetadata::parse(&metadata_bytes)?;

    if metadata.tag != release_dto.tag_name {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "release metadata tag '{}' does not match GitHub release tag '{}'",
                metadata.tag, release_dto.tag_name
            ),
        ));
    }

    validate_asset_inventory(&release_dto, &metadata)?;

    Ok(metadata)
}

pub fn download_candidate(
    metadata: &ReleaseMetadata,
    platform: &str,
) -> crate::Result<(crate::release::Asset, Vec<u8>)> {
    metadata
        .validate()
        .map_err(|e| crate::Failure::new("INTEGRITY_FAILURE", e))?;

    let asset = metadata
        .assets
        .iter()
        .find(|a| a.platform == platform)
        .cloned()
        .ok_or_else(|| {
            crate::Failure::new(
                "MISSING_CAPABILITY",
                format!("unsupported candidate platform: {platform}"),
            )
        })?;

    let deadline = Instant::now() + Duration::from_secs(60);
    let agent_config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .max_redirects(0)
        .https_only(true)
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = agent_config.into();

    let binary_url = format!(
        "https://github.com/{GITHUB_REPOSITORY}/releases/download/{}/{}",
        metadata.tag, asset.name
    );
    let binary_bytes = get_with_redirects_bounded(
        &agent,
        &binary_url,
        UrlContext::Download,
        "application/octet-stream, */*",
        deadline,
        128 * 1024 * 1024,
    )?;

    if binary_bytes.len() as u64 != asset.size {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "candidate binary size mismatch: expected {}, got {}",
                asset.size,
                binary_bytes.len()
            ),
        ));
    }
    let binary_digest = crate::infrastructure::digest(&binary_bytes);
    if binary_digest != asset.sha256 {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            format!(
                "candidate binary digest mismatch: expected {}, got {}",
                asset.sha256, binary_digest
            ),
        ));
    }

    let checksum_url = format!(
        "https://github.com/{GITHUB_REPOSITORY}/releases/download/{}/{}",
        metadata.tag, asset.checksum_name
    );
    let checksum_bytes = get_with_redirects_bounded(
        &agent,
        &checksum_url,
        UrlContext::Download,
        "text/plain, */*",
        deadline,
        64 * 1024,
    )?;

    let checksum_str = std::str::from_utf8(&checksum_bytes).map_err(|e| {
        crate::Failure::new(
            "INTEGRITY_FAILURE",
            format!("checksum sidecar is not UTF-8: {e}"),
        )
    })?;

    let mut matched_checksum = false;
    for line in checksum_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parts = trimmed.split_whitespace().collect::<Vec<_>>();
        if !parts.is_empty() {
            let digest = parts[0];
            crate::domain::validate_digest(digest).map_err(|e| {
                crate::Failure::new("INTEGRITY_FAILURE", format!("invalid sidecar digest: {e}"))
            })?;
            if digest == asset.sha256 {
                if parts.len() > 1 {
                    let filename = parts[1].trim_start_matches('*');
                    if filename != asset.name {
                        return Err(crate::Failure::new(
                            "INTEGRITY_FAILURE",
                            format!(
                                "checksum sidecar filename mismatch: expected {}, got {}",
                                asset.name, filename
                            ),
                        ));
                    }
                }
                matched_checksum = true;
                break;
            }
        }
    }

    if !matched_checksum {
        return Err(crate::Failure::new(
            "INTEGRITY_FAILURE",
            "checksum sidecar does not contain expected sha256 for asset",
        ));
    }

    metadata
        .verify_candidate(platform, &binary_bytes)
        .map_err(|e| crate::Failure::new("INTEGRITY_FAILURE", e))?;

    Ok((asset, binary_bytes))
}

pub fn check() -> crate::Result<serde_json::Value> {
    let metadata = resolve()?;
    let observed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| crate::Failure::new("INTERNAL_ERROR", format!("system clock error: {e}")))?
        .as_secs();

    let metadata_value = serde_json::to_value(&metadata)?;

    Ok(serde_json::json!({
        "observed_at": observed_at,
        "repository": GITHUB_REPOSITORY,
        "metadata": metadata_value,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::{Asset, PLATFORMS, ReleaseMetadata};

    fn sample_metadata() -> ReleaseMetadata {
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
                    size: 1024,
                    sha256: "c".repeat(64),
                    checksum_name: format!("agentfoundry-{p}.sha256"),
                })
                .collect(),
        }
    }

    fn sample_dto(tag: &str, draft: bool, prerelease: bool) -> GithubReleaseDto {
        let mut assets = Vec::new();
        for p in PLATFORMS {
            assets.push(GithubAssetDto {
                name: format!("agentfoundry-{p}"),
            });
            assets.push(GithubAssetDto {
                name: format!("agentfoundry-{p}.sha256"),
            });
        }
        assets.push(GithubAssetDto {
            name: METADATA_ASSET_NAME.into(),
        });
        GithubReleaseDto {
            tag_name: tag.into(),
            draft,
            prerelease,
            assets,
        }
    }

    #[test]
    fn url_scheme_and_userinfo_and_port_boundaries() {
        assert!(validate_url(LATEST_RELEASE_API_URL, UrlContext::Api).is_ok());

        assert!(
            validate_url(
                "https://api.github.com:443/repos/anquocviet/agent-foundry/releases/latest",
                UrlContext::Api
            )
            .is_ok()
        );

        let err = validate_url(
            "http://api.github.com/repos/anquocviet/agent-foundry/releases/latest",
            UrlContext::Api,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");

        let err = validate_url(
            "https://user:pass@api.github.com/repos/anquocviet/agent-foundry/releases/latest",
            UrlContext::Api,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");

        let err = validate_url(
            "https://api.github.com:8080/repos/anquocviet/agent-foundry/releases/latest",
            UrlContext::Api,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");

        let err = validate_url(
            "https://api.github.com/repos/anquocviet/agent-foundry/../evil",
            UrlContext::Api,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");
        for path in ["%2e%2e/evil", "./latest", "%2fother"] {
            assert!(
                validate_url(
                    &format!("https://api.github.com/repos/anquocviet/agent-foundry/{path}"),
                    UrlContext::Api
                )
                .is_err()
            );
        }
    }

    #[test]
    fn url_host_and_repo_isolation() {
        let err = validate_url(
            "https://api.github.com/repos/evil-user/agent-foundry/releases/latest",
            UrlContext::Api,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");

        assert!(validate_url(
            "https://github.com/anquocviet/agent-foundry/releases/download/v1.0.0/agentfoundry-release.json",
            UrlContext::Download,
        )
        .is_ok());

        let err = validate_url(
            "https://github.com/evil-user/agent-foundry/releases/download/v1.0.0/agentfoundry-release.json",
            UrlContext::Download,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");

        assert!(
            validate_url(
                "https://release-assets.githubusercontent.com/12345/asset.json",
                UrlContext::Download,
            )
            .is_ok()
        );

        let err = validate_url("https://evil.com/asset.json", UrlContext::Download).unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");

        let err = validate_url(
            "https://release-assets.githubusercontent.com/12345/asset.json",
            UrlContext::Api,
        )
        .unwrap_err();
        assert_eq!(err.code, "NETWORK_ERROR");
    }

    #[test]
    fn asset_inventory_exact_11_and_duplicates() {
        let meta = sample_metadata();
        let dto = sample_dto("v1.0.0", false, false);
        assert_eq!(dto.assets.len(), 11);
        assert!(validate_asset_inventory(&dto, &meta).is_ok());

        let mut dup_dto = dto.clone();
        dup_dto.assets[1] = dup_dto.assets[0].clone();
        let err = validate_asset_inventory(&dup_dto, &meta).unwrap_err();
        assert_eq!(err.code, "INTEGRITY_FAILURE");

        let mut short_dto = dto.clone();
        short_dto.assets.pop();
        let err = validate_asset_inventory(&short_dto, &meta).unwrap_err();
        assert_eq!(err.code, "INTEGRITY_FAILURE");

        let mut mismatch_dto = dto;
        mismatch_dto.assets[10].name = "unexpected-file.txt".into();
        let err = validate_asset_inventory(&mismatch_dto, &meta).unwrap_err();
        assert_eq!(err.code, "INTEGRITY_FAILURE");
    }
}
