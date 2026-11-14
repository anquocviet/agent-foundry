#!/usr/bin/env bash
#
# scripts/install.sh - Verified bootstrap installer for AgentFoundry (macOS / Linux)
#
# Requirements & Dependencies:
#   - bash (>= 3.2, macOS default bash supported)
#   - curl (for bounded HTTPS network transport)
#   - jq (for strict JSON schema & release metadata validation)
#   - sha256sum or shasum (for SHA-256 candidate binary and sidecar verification)
#
# Architecture & Security (AF-012 / ADR-0004):
#   - Pinned repository and release endpoint (anquocviet/agent-foundry).
#   - Bounded network I/O with strict HTTPS-only transport, standard port 443,
#     no userinfo, no path traversal, and domain-restricted redirects (max 5).
#   - Strict validation of release metadata, exact 11-asset inventory, platform support,
#     binary size, SHA-256 digest, and sidecar checksum.
#   - Pre-install candidate execution check: invokes '--version --json' to verify
#     metadata version, source commit, payload digest, stable distribution,
#     and fault_injection=false.
#   - Safe failure without target mutations: all staging occurs in an isolated
#     ephemeral temporary directory cleaned up on exit.
#   - Preserves console streams and native binary exit code when forwarding to 'install'.
#
set -euo pipefail

# --- Pinned Release Constants ---
GITHUB_REPOSITORY="anquocviet/agent-foundry"
LATEST_RELEASE_API_URL="https://api.github.com/repos/anquocviet/agent-foundry/releases/latest"
METADATA_ASSET_NAME="agentfoundry-release.json"

MAX_CONTROL_BYTES=16777216    # 16 MiB for API and release metadata JSON
MAX_BINARY_BYTES=134217728    # 128 MiB for candidate executables
MAX_CHECKSUM_BYTES=65536      # 64 KiB for checksum sidecar files
MAX_REDIRECTS=5

# --- Exit Code Constants (matching agentfoundry core exits) ---
EXIT_OK=0
EXIT_IO_ERROR=1
EXIT_INVALID_ARGUMENT=2
EXIT_INTEGRITY_FAILURE=5
EXIT_MISSING_CAPABILITY=6
EXIT_RELEASE_UNAVAILABLE=7

# --- Dependency Check ---
check_dependencies() {
    local missing=()
    for tool in curl jq; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            missing+=("$tool")
        fi
    done

    if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
        missing+=("sha256sum or shasum")
    fi

    if [ ${#missing[@]} -gt 0 ]; then
        echo "MISSING_CAPABILITY: Required dependency missing: ${missing[*]}" >&2
        echo "Please install curl, jq, and sha256sum/shasum before running install.sh" >&2
        exit "$EXIT_MISSING_CAPABILITY"
    fi
}

# --- Platform Detection ---
detect_platform() {
    local os
    os="$(uname -s)"
    local arch
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            case "$arch" in
                arm64|aarch64)
                    echo "aarch64-apple-darwin"
                    ;;
                x86_64)
                    echo "x86_64-apple-darwin"
                    ;;
                *)
                    echo "MISSING_CAPABILITY: unsupported macOS architecture '$arch'" >&2
                    exit "$EXIT_MISSING_CAPABILITY"
                    ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64|amd64)
                    echo "x86_64-unknown-linux-gnu"
                    ;;
                aarch64|arm64)
                    echo "aarch64-unknown-linux-gnu"
                    ;;
                *)
                    echo "MISSING_CAPABILITY: unsupported Linux architecture '$arch'" >&2
                    exit "$EXIT_MISSING_CAPABILITY"
                    ;;
            esac
            ;;
        *)
            echo "MISSING_CAPABILITY: unsupported operating system '$os'; use agentfoundry.ps1 on Windows" >&2
            exit "$EXIT_MISSING_CAPABILITY"
            ;;
    esac
}

# --- Digest & Size Helpers ---
compute_sha256() {
    local target="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$target" | awk '{print $1}'
    else
        shasum -a 256 "$target" | awk '{print $1}'
    fi
}

get_file_size() {
    local target="$1"
    wc -c < "$target" | tr -d ' \t\r\n'
}

# --- Strict URL Validation (mirrors crates/agentfoundry/src/release_client.rs) ---
validate_url() {
    local url="$1"
    local context="$2" # "api" or "download"

    case "$url" in
        https://*) ;;
        *)
            echo "NETWORK_ERROR: unsupported non-HTTPS scheme in URL: $url" >&2
            return 1
            ;;
    esac

    local rest="${url#https://}"
    local authority="${rest%%/*}"
    local path
    if [[ "$rest" == *"/"* ]]; then
        path="/${rest#*/}"
    else
        path="/"
    fi

    if [[ "$authority" == *"@"* ]]; then
        echo "NETWORK_ERROR: userinfo is forbidden in URL: $url" >&2
        return 1
    fi

    local host=""
    local port=""
    if [[ "$authority" =~ ^([^:]+):([0-9]+)$ ]]; then
        host="${BASH_REMATCH[1]}"
        port="${BASH_REMATCH[2]}"
        if [ "$port" != "443" ]; then
            echo "NETWORK_ERROR: nonstandard HTTPS port $port in URL: $url" >&2
            return 1
        fi
    elif [[ "$authority" =~ ^[^:]+$ ]]; then
        host="$authority"
    else
        echo "NETWORK_ERROR: malformed authority in URL: $url" >&2
        return 1
    fi

    if [ -z "$host" ]; then
        echo "NETWORK_ERROR: URL missing host: $url" >&2
        return 1
    fi

    local host_lower
    host_lower="$(echo "$host" | tr '[:upper:]' '[:lower:]')"

    # Check for path traversal segments (. or ..)
    local old_ifs="$IFS"
    IFS='/'
    # shellcheck disable=SC2206
    local segments=($path)
    IFS="$old_ifs"
    for seg in "${segments[@]}"; do
        if [ "$seg" = "." ] || [ "$seg" = ".." ]; then
            echo "NETWORK_ERROR: path traversal detected in URL: $url" >&2
            return 1
        fi
    done

    # Reject % in path unless host is release-assets.githubusercontent.com
    if [ "$host_lower" != "release-assets.githubusercontent.com" ] && [[ "$path" == *"%"* ]]; then
        echo "NETWORK_ERROR: path traversal detected in URL: $url" >&2
        return 1
    fi

    local path_lower
    path_lower="$(echo "$path" | tr '[:upper:]' '[:lower:]')"

    case "$context" in
        api)
            if [ "$host_lower" != "api.github.com" ]; then
                echo "NETWORK_ERROR: API URL host must be api.github.com: $url" >&2
                return 1
            fi
            if [[ "$path_lower" != "/repos/${GITHUB_REPOSITORY}/"* ]]; then
                echo "NETWORK_ERROR: API URL must be within ${GITHUB_REPOSITORY}: $url" >&2
                return 1
            fi
            ;;
        download)
            if [ "$host_lower" = "github.com" ]; then
                if [[ "$path_lower" != "/${GITHUB_REPOSITORY}/releases/download/"* ]]; then
                    echo "NETWORK_ERROR: download URL on github.com must be within ${GITHUB_REPOSITORY}/releases/download: $url" >&2
                    return 1
                fi
            elif [ "$host_lower" = "release-assets.githubusercontent.com" ]; then
                : # Permitted signed redirect target
            else
                echo "NETWORK_ERROR: download URL host not permitted ($host_lower): $url" >&2
                return 1
            fi
            ;;
        *)
            echo "INTERNAL_ERROR: unknown URL context: $context" >&2
            return 1
            ;;
    esac

    return 0
}

# --- Bounded Network Fetch with Redirect Validation ---
fetch_bounded() {
    local initial_url="$1"
    local context="$2"      # "api" or "download"
    local accept="$3"
    local max_bytes="$4"
    local output_file="$5"

    local current_url="$initial_url"
    local redirect_count=0

    while :; do
        if ! validate_url "$current_url" "$context"; then
            return "$EXIT_INTEGRITY_FAILURE"
        fi

        local headers_file="$tmpdir/headers" body_file="$tmpdir/body"

        local http_code
        local curl_status body_size
        set +e
        curl -sS -D "$headers_file" --proto =https --connect-timeout 15 --max-time 60 \
            -H "User-Agent: agentfoundry" -H "Accept: $accept" \
            "$current_url" 2>/dev/null | head -c "$((max_bytes + 1))" > "$body_file"
        curl_status=${PIPESTATUS[0]}
        set -e
        body_size="$(get_file_size "$body_file")"
        if [ "$body_size" -gt "$max_bytes" ]; then
            rm -f "$headers_file" "$body_file"
            echo "INPUT_LIMIT_EXCEEDED: response body exceeded byte limit" >&2
            return "$EXIT_IO_ERROR"
        fi
        if [ "$curl_status" -ne 0 ]; then
            rm -f "$headers_file" "$body_file"
            echo "NETWORK_ERROR: HTTP request failed for $current_url" >&2
            return "$EXIT_IO_ERROR"
        fi
        http_code="$(awk '/^HTTP\// {code=$2} END {print code}' "$headers_file")"

        if [ "$http_code" = "404" ]; then
            rm -f "$headers_file" "$body_file"
            echo "RELEASE_UNAVAILABLE: release unavailable (HTTP 404): $current_url" >&2
            return "$EXIT_RELEASE_UNAVAILABLE"
        fi

        # Redirection codes: 301, 302, 303, 307, 308
        if [[ "$http_code" =~ ^30[12378]$ ]]; then
            local location
            location="$(awk 'tolower($1) == "location:" { sub(/\r$/, "", $2); print $2 }' "$headers_file" | tail -n 1)"
            rm -f "$headers_file" "$body_file"

            if [ -z "$location" ]; then
                echo "NETWORK_ERROR: redirect from $current_url missing Location header" >&2
                return "$EXIT_IO_ERROR"
            fi

            local next_url=""
            if [[ "$location" == "https://"* ]]; then
                next_url="$location"
            elif [[ "$location" == "/"* ]]; then
                local rest="${current_url#https://}"
                local host="${rest%%/*}"
                next_url="https://${host}${location}"
            else
                echo "NETWORK_ERROR: unsupported redirect target: $location" >&2
                return "$EXIT_IO_ERROR"
            fi

            redirect_count=$((redirect_count + 1))
            if [ "$redirect_count" -gt "$MAX_REDIRECTS" ]; then
                echo "NETWORK_ERROR: exceeded maximum redirect limit ($MAX_REDIRECTS) for $initial_url" >&2
                return "$EXIT_IO_ERROR"
            fi

            current_url="$next_url"
            continue
        fi

        # Success codes: 200..299
        if [[ "$http_code" =~ ^2[0-9][0-9]$ ]]; then
            rm -f "$headers_file"
            local body_size
            body_size="$(get_file_size "$body_file")"
            if [ "$body_size" -gt "$max_bytes" ]; then
                rm -f "$body_file"
                echo "INPUT_LIMIT_EXCEEDED: response body from $current_url exceeded byte limit ($body_size > $max_bytes)" >&2
                return "$EXIT_IO_ERROR"
            fi
            mv "$body_file" "$output_file"
            return 0
        fi

        rm -f "$headers_file" "$body_file"
        echo "NETWORK_ERROR: HTTP status $http_code for $current_url" >&2
        return "$EXIT_IO_ERROR"
    done
}

# --- Main Entry Point ---
main() {
    check_dependencies
    local platform
    platform="$(detect_platform)"

    tmpdir="$(mktemp -d 2>/dev/null || mktemp -d -t 'agentfoundry-install')"
    # Trap ensures safe cleanup of candidate bytes on any exit; no target writes outside core
    trap 'rm -rf "$tmpdir"' EXIT INT TERM HUP

    # 1. Fetch Latest Release API Object
    local release_json="$tmpdir/release.json"
    fetch_bounded "$LATEST_RELEASE_API_URL" "api" "application/vnd.github+json" "$MAX_CONTROL_BYTES" "$release_json"

    # 2. Validate API Release DTO
    local tag_name
    tag_name="$(jq -r '.tag_name // empty' "$release_json")"
    if ! jq -e '.draft == false and .prerelease == false and (.assets | type == "array")' "$release_json" >/dev/null; then
        echo "RELEASE_UNAVAILABLE: invalid, draft or prerelease release" >&2
        exit "$EXIT_RELEASE_UNAVAILABLE"
    fi

    if [[ ! "$tag_name" =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
        echo "INTEGRITY_FAILURE: release tag must be stable vMAJOR.MINOR.PATCH, got '$tag_name'" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    local api_asset_count
    api_asset_count="$(jq -r '.assets | length' "$release_json")"
    if [ "$api_asset_count" -ne 11 ]; then
        echo "INTEGRITY_FAILURE: API assets must contain exactly 11 items, found $api_asset_count" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    local api_asset_names
    api_asset_names="$(jq -r '.assets[].name' "$release_json" | sort)"
    local unique_api_names
    unique_api_names="$(echo "$api_asset_names" | uniq)"
    if [ "$api_asset_names" != "$unique_api_names" ]; then
        echo "INTEGRITY_FAILURE: duplicate asset name in GitHub release" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # 3. Download Release Metadata
    local metadata_url="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag_name}/${METADATA_ASSET_NAME}"
    local metadata_json="$tmpdir/metadata.json"
    fetch_bounded "$metadata_url" "download" "application/json, */*" "$MAX_CONTROL_BYTES" "$metadata_json"

    # 4. Validate Release Metadata Schema & Values (schemas/release.json & release.rs)
    local meta_valid
    meta_valid="$(jq -r --arg tag "$tag_name" '
        (
            .schema == "agentfoundry.release/v1" and
            (.version | test("^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$")) and
            .tag == ("v" + .version) and
            .tag == $tag and
            (.source_commit | test("^[0-9a-f]{40}$")) and
            (.payload_digest | test("^[0-9a-f]{64}$")) and
            .state_schema_min == 1 and
            .state_schema_max >= 1 and
            .handoff_protocol == 1 and
            (.assets | type == "array") and
            (.assets | length == 5)
        )
    ' "$metadata_json")"

    if [ "$meta_valid" != "true" ]; then
        echo "INTEGRITY_FAILURE: release metadata schema or identity verification failed" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # Verify exact 5 platforms in assets
    local platforms_json
    platforms_json="$(jq -r '[.assets[].platform] | sort | join(",")' "$metadata_json")"
    local expected_platforms="aarch64-apple-darwin,aarch64-unknown-linux-gnu,x86_64-apple-darwin,x86_64-pc-windows-msvc,x86_64-unknown-linux-gnu"
    if [ "$platforms_json" != "$expected_platforms" ]; then
        echo "INTEGRITY_FAILURE: release requires exact five-platform binary inventory" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # Validate asset details (size, sha256 pattern, checksum_name binding)
    local assets_detail_valid
    assets_detail_valid="$(jq -r '
        all(.assets[];
            (.size | type == "number" and floor == . and . > 0 and . <= 134217728) and
            (.sha256 | test("^[0-9a-f]{64}$")) and
            (.name | test("^[A-Za-z0-9][A-Za-z0-9._-]*$")) and
            (.checksum_name == (.name + ".sha256"))
        )
    ' "$metadata_json")"

    if [ "$assets_detail_valid" != "true" ]; then
        echo "INTEGRITY_FAILURE: release assets have invalid size, sha256, or naming format" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # Validate full inventory equality (5 binaries + 5 checksum sidecars + metadata = 11 items)
    local expected_inventory
    expected_inventory="$(jq -r '
        [.assets[].name, .assets[].checksum_name, "agentfoundry-release.json"] | sort | .[]
    ' "$metadata_json")"

    if [ "$api_asset_names" != "$expected_inventory" ]; then
        echo "INTEGRITY_FAILURE: API assets do not match expected release inventory (5 binaries + 5 checksums + agentfoundry-release.json)" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # 5. Locate Asset for Current Target Platform
    local asset_name asset_size asset_sha256 asset_checksum_name
    asset_name="$(jq -r --arg p "$platform" '.assets[] | select(.platform == $p) | .name' "$metadata_json")"
    asset_size="$(jq -r --arg p "$platform" '.assets[] | select(.platform == $p) | .size' "$metadata_json")"
    asset_sha256="$(jq -r --arg p "$platform" '.assets[] | select(.platform == $p) | .sha256' "$metadata_json")"
    asset_checksum_name="$(jq -r --arg p "$platform" '.assets[] | select(.platform == $p) | .checksum_name' "$metadata_json")"

    if [ -z "$asset_name" ] || [ -z "$asset_sha256" ] || [ -z "$asset_size" ]; then
        echo "MISSING_CAPABILITY: unsupported candidate platform: $platform" >&2
        exit "$EXIT_MISSING_CAPABILITY"
    fi

    # 6. Download Binary and Checksum Sidecar
    local binary_url="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag_name}/${asset_name}"
    local candidate_binary="$tmpdir/$asset_name"
    fetch_bounded "$binary_url" "download" "application/octet-stream, */*" "$MAX_BINARY_BYTES" "$candidate_binary"

    local checksum_url="https://github.com/${GITHUB_REPOSITORY}/releases/download/${tag_name}/${asset_checksum_name}"
    local candidate_checksum="$tmpdir/$asset_checksum_name"
    fetch_bounded "$checksum_url" "download" "text/plain, */*" "$MAX_CHECKSUM_BYTES" "$candidate_checksum"

    # 7. Validate Candidate Size and SHA-256
    local actual_size
    actual_size="$(get_file_size "$candidate_binary")"
    if [ "$actual_size" -ne "$asset_size" ]; then
        echo "INTEGRITY_FAILURE: candidate binary size mismatch: expected $asset_size, got $actual_size" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    local actual_sha256
    actual_sha256="$(compute_sha256 "$candidate_binary")"
    if [ "$actual_sha256" != "$asset_sha256" ]; then
        echo "INTEGRITY_FAILURE: candidate binary digest mismatch: expected $asset_sha256, got $actual_sha256" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # Validate Checksum Sidecar Content
    local sidecar_matched="false"
    while IFS= read -r line || [ -n "$line" ]; do
        # Strip trailing carriage return and trim
        line="$(echo "$line" | sed 's/\r$//' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
        [ -z "$line" ] && continue

        # Parse: <digest> <filename> or <digest> *<filename>
        local sidecar_digest sidecar_filename
        sidecar_digest="$(echo "$line" | awk '{print $1}')"
        sidecar_filename="$(echo "$line" | awk '{print $2}' | sed 's/^\*//')"

        if [ "$sidecar_digest" = "$asset_sha256" ]; then
            if [ -n "$sidecar_filename" ] && [ "$sidecar_filename" != "$asset_name" ]; then
                echo "INTEGRITY_FAILURE: checksum sidecar filename mismatch: expected $asset_name, got $sidecar_filename" >&2
                exit "$EXIT_INTEGRITY_FAILURE"
            fi
            sidecar_matched="true"
            break
        fi
    done < "$candidate_checksum"

    if [ "$sidecar_matched" != "true" ]; then
        echo "INTEGRITY_FAILURE: checksum sidecar does not contain expected sha256 for asset" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # 8. Candidate Execution Policy Verification (AF-012 / ADR-0004)
    # Check binary --version --json identifies exact metadata version/source_commit/payload_digest,
    # stable distribution, and fault_injection=false before install.
    chmod +x "$candidate_binary"

    local meta_version meta_source meta_payload
    meta_version="$(jq -r '.version' "$metadata_json")"
    meta_source="$(jq -r '.source_commit' "$metadata_json")"
    meta_payload="$(jq -r '.payload_digest' "$metadata_json")"

    local version_output
    if ! version_output="$(env -u AF_WORKER -u AF_SELECTED_BINARY AF_INVOCATION_DEPTH=1 AF_NO_RECURSIVE=1 "$candidate_binary" --version --json 2>/dev/null)"; then
        echo "INTEGRITY_FAILURE: candidate binary failed to execute --version --json" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    local candidate_check
    candidate_check="$(echo "$version_output" | jq -r \
        --arg exp_v "$meta_version" \
        --arg exp_s "$meta_source" \
        --arg exp_p "$meta_payload" '
        (if .schema == "agentfoundry.command-result/v1" then
            (if .status == "ok" then .data else null end)
        else
            .
        end) as $d |
        if $d and
           $d.version == $exp_v and
           $d.source == $exp_s and
           $d.payload_digest == $exp_p and
           $d.distribution == "stable" and
           $d.fault_injection == false then
            "ok"
        else
            "mismatch"
        end
    ')"

    if [ "$candidate_check" != "ok" ]; then
        echo "INTEGRITY_FAILURE: candidate binary identity, distribution, or fault injection mismatch with release metadata" >&2
        exit "$EXIT_INTEGRITY_FAILURE"
    fi

    # 9. Invoke Core Install Forwarding All Arguments
    # Preserves console streams and native exit status; no target writes outside core
    local exit_status=0
    "$candidate_binary" install "$@" || exit_status=$?

    exit "$exit_status"
}

main "$@"
