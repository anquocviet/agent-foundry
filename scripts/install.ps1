# Requires -Version 5.1
<#
.SYNOPSIS
    AgentFoundry bootstrap installer for Windows PowerShell 5.1.
.DESCRIPTION
    AF-012 verified bootstrap:
    - Observes pinned anquocviet/agent-foundry release metadata.
    - Enforces bounded redirects (max 5) and strict HTTPS destination boundaries.
    - Validates release identity, 11-item asset inventory, and exact platform asset.
    - Downloads the native x86_64-pc-windows-msvc executable to a temporary location outside the target.
    - Verifies file size, SHA-256 digest, and sidecar checksum.
    - Executes candidate probe (--version --json) verifying metadata version, source commit,
      payload digest, stable distribution, and no fault injection before execution.
    - Invokes install forwarding all arguments with preserved console and exit status.
    - Makes zero filesystem writes outside core installation commands and cleans up temporary files safely.
#>

$ErrorActionPreference = 'Stop'

# Ensure TLS 1.2 is active for GitHub HTTPS endpoints on Windows PowerShell 5.1
[System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor [System.Net.SecurityProtocolType]::Tls12

$GITHUB_REPOSITORY = "anquocviet/agent-foundry"
$LATEST_RELEASE_API_URL = "https://api.github.com/repos/anquocviet/agent-foundry/releases/latest"
$METADATA_ASSET_NAME = "agentfoundry-release.json"
$MAX_METADATA_BYTES = 16 * 1024 * 1024 # 16 MiB
$MAX_SIDECAR_BYTES = 64 * 1024          # 64 KiB
$MAX_BINARY_BYTES = 128 * 1024 * 1024   # 128 MiB
$MAX_REDIRECTS = 5
$TARGET_PLATFORM = "x86_64-pc-windows-msvc"

$PLATFORMS = @(
    'aarch64-apple-darwin',
    'x86_64-apple-darwin',
    'x86_64-unknown-linux-gnu',
    'aarch64-unknown-linux-gnu',
    'x86_64-pc-windows-msvc'
)

function Escape-CommandLineArg([string]$arg) {
    if ([string]::IsNullOrEmpty($arg)) {
        return '""'
    }
    if ($arg -match '[\s"]') {
        $clean = $arg -replace '(\\*)(")', '$1$1\"'
        $clean = $clean -replace '(\\+)$', '$1$1'
        return "`"$clean`""
    }
    return $arg
}

function Validate-Url([string]$urlStr, [string]$context) {
    try {
        $uri = [System.Uri]::new($urlStr, [System.UriKind]::Absolute)
    } catch {
        throw "malformed URL '$urlStr': $_"
    }

    if ($uri.Scheme -cne "https") {
        throw "unsupported non-HTTPS scheme in URL: $urlStr"
    }

    if (-not [string]::IsNullOrEmpty($uri.UserInfo) -or $uri.Authority.Contains("@")) {
        throw "userinfo is forbidden in URL: $urlStr"
    }

    if ($uri.Port -ne 443) {
        throw "nonstandard HTTPS port $($uri.Port) in URL: $urlStr"
    }

    $hostLower = $uri.Host.ToLowerInvariant()
    $path = $uri.AbsolutePath

    # Check path traversal on segments
    $segments = $path.Split('/')
    foreach ($seg in $segments) {
        if ($seg -eq '.' -or $seg -eq '..') {
            throw "path traversal detected in URL: $urlStr"
        }
    }

    # Path traversal check on raw path string for %-encoding
    if ($hostLower -ne "release-assets.githubusercontent.com") {
        $pathOnly = $urlStr
        $qIdx = $pathOnly.IndexOf('?')
        if ($qIdx -ge 0) {
            $pathOnly = $pathOnly.Substring(0, $qIdx)
        }
        if ($pathOnly.Contains('%')) {
            throw "path traversal detected in URL: $urlStr"
        }
    }

    $pathLower = $path.ToLowerInvariant()

    if ($context -eq "Api") {
        if ($hostLower -ne "api.github.com") {
            throw "API URL host must be api.github.com: $urlStr"
        }
        if (-not $pathLower.StartsWith("/repos/anquocviet/agent-foundry/")) {
            throw "API URL must be within anquocviet/agent-foundry: $urlStr"
        }
    } elseif ($context -eq "Download") {
        if ($hostLower -eq "github.com") {
            if (-not $pathLower.StartsWith("/anquocviet/agent-foundry/releases/download/")) {
                throw "download URL on github.com must be within anquocviet/agent-foundry/releases/download: $urlStr"
            }
        } elseif ($hostLower -eq "release-assets.githubusercontent.com") {
            # Allowed for signed redirects
        } else {
            throw "download URL host not permitted ($hostLower): $urlStr"
        }
    } else {
        throw "unknown URL context: $context"
    }
}

function Invoke-BoundedGet([string]$initialUrl, [string]$context, [string]$accept, [long]$maxBytes, [string]$outFilePath = $null) {
    $currentUrl = $initialUrl
    Validate-Url $currentUrl $context

    for ($i = 0; $i -le $MAX_REDIRECTS; $i++) {
        $request = [System.Net.HttpWebRequest]::Create($currentUrl)
        $request.Method = "GET"
        $request.UserAgent = "agentfoundry"
        $request.Accept = $accept
        $request.AllowAutoRedirect = $false
        $request.Timeout = 30000        # 30 seconds
        $request.ReadWriteTimeout = 60000

        $response = $null
        try {
            $response = [System.Net.HttpWebResponse]$request.GetResponse()
        } catch [System.Net.WebException] {
            $webEx = $_.Exception
            if ($webEx.Response -ne $null) {
                $response = [System.Net.HttpWebResponse]$webEx.Response
            } else {
                throw "HTTP request failed for $currentUrl`: $($webEx.Message)"
            }
        }

        $statusCode = [int]$response.StatusCode

        if ($statusCode -eq 404) {
            $response.Close()
            throw "release unavailable (HTTP 404): $currentUrl"
        }

        # Check for HTTP redirect statuses
        if ($statusCode -ge 300 -and $statusCode -le 399) {
            $location = $response.Headers["Location"]
            $response.Close()
            if ([string]::IsNullOrEmpty($location)) {
                throw "redirect from $currentUrl missing Location header"
            }

            $nextUrl = $null
            if ($location.StartsWith("/")) {
                $currentUri = [System.Uri]::new($currentUrl)
                $nextUrl = "https://$($currentUri.Host)$location"
            } elseif ($location.StartsWith("https://")) {
                $nextUrl = $location
            } else {
                throw "unsupported redirect target: $location"
            }

            Validate-Url $nextUrl $context
            $currentUrl = $nextUrl
            continue
        }

        if ($statusCode -lt 200 -or $statusCode -ge 300) {
            $response.Close()
            throw "HTTP status $statusCode for $currentUrl"
        }

        # Successful response body reading
        try {
            $responseStream = $response.GetResponseStream()
            if ($outFilePath) {
                $fileStream = [System.IO.File]::Create($outFilePath)
                try {
                    $buffer = New-Object byte[] 65536
                    $totalBytes = 0
                    while (($bytesRead = $responseStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        $totalBytes += $bytesRead
                        if ($totalBytes -gt $maxBytes) {
                            throw "response body from $currentUrl exceeded byte limit"
                        }
                        $fileStream.Write($buffer, 0, $bytesRead)
                    }
                } finally {
                    $fileStream.Close()
                }
                return $null
            } else {
                $memStream = New-Object System.IO.MemoryStream
                try {
                    $buffer = New-Object byte[] 65536
                    $totalBytes = 0
                    while (($bytesRead = $responseStream.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        $totalBytes += $bytesRead
                        if ($totalBytes -gt $maxBytes) {
                            throw "response body from $currentUrl exceeded byte limit"
                        }
                        $memStream.Write($buffer, 0, $bytesRead)
                    }
                    $bytes = $memStream.ToArray()
                    return [System.Text.Encoding]::UTF8.GetString($bytes)
                } finally {
                    $memStream.Close()
                }
            }
        } finally {
            $response.Close()
        }
    }

    throw "exceeded maximum redirect limit ($MAX_REDIRECTS) for $initialUrl"
}

function Validate-Semver([string]$version) {
    $parts = $version.Split('.')
    if ($parts.Count -ne 3) {
        return $false
    }
    foreach ($part in $parts) {
        if ([string]::IsNullOrEmpty($part)) {
            return $false
        }
        if ($part -notmatch '^[0-9]+$') {
            return $false
        }
        if ($part.Length -gt 1 -and $part.StartsWith('0')) {
            return $false
        }
        $val = [uint64]0
        if (-not [uint64]::TryParse($part, [ref]$val)) {
            return $false
        }
    }
    return $true
}

function Validate-AssetInventory($releaseDto, $metadata) {
    $apiNames = New-Object 'System.Collections.Generic.HashSet[string]'
    foreach ($asset in $releaseDto.assets) {
        $name = [string]$asset.name
        if (-not $apiNames.Add($name)) {
            throw "duplicate asset name in GitHub release: $name"
        }
    }

    if ($releaseDto.assets.Count -ne 11) {
        throw "API assets must contain exactly 11 items, found $($releaseDto.assets.Count)"
    }

    $expectedNames = New-Object 'System.Collections.Generic.HashSet[string]'
    foreach ($asset in $metadata.assets) {
        $expectedNames.Add([string]$asset.name) | Out-Null
        $expectedNames.Add([string]$asset.checksum_name) | Out-Null
    }
    $expectedNames.Add($METADATA_ASSET_NAME) | Out-Null

    if ($expectedNames.Count -ne 11) {
        throw "metadata assets and sidecars must produce 11 distinct names"
    }

    if (-not $apiNames.SetEquals($expectedNames)) {
        throw "API assets do not match expected release inventory (5 binaries + 5 checksums + agentfoundry-release.json)"
    }
}

function Validate-Metadata($metadata) {
    if ($metadata.schema -ne "agentfoundry.release/v1") {
        throw "release schema mismatch: $($metadata.schema)"
    }
    if (-not (Validate-Semver $metadata.version)) {
        throw "release version must be exact stable semver: $($metadata.version)"
    }
    if ($metadata.tag -ne "v$($metadata.version)") {
        throw "release tag mismatch: $($metadata.tag) vs v$($metadata.version)"
    }
    if ($metadata.source_commit -notmatch '^[0-9a-f]{40}$') {
        throw "release source commit identity mismatch: $($metadata.source_commit)"
    }
    if ($metadata.payload_digest -notmatch '^[0-9a-f]{64}$') {
        throw "release payload digest identity mismatch: $($metadata.payload_digest)"
    }
    if ($metadata.state_schema_min -ne 1 -or $metadata.state_schema_max -lt 1 -or $metadata.handoff_protocol -ne 1) {
        throw "candidate cannot read installed state/handoff schema"
    }
    if (-not $metadata.assets -or $metadata.assets.Count -ne 5) {
        throw "release requires exact five-platform binary inventory"
    }

    $platformsSeen = New-Object 'System.Collections.Generic.HashSet[string]'
    $namesSeen = New-Object 'System.Collections.Generic.HashSet[string]'

    foreach ($asset in $metadata.assets) {
        $p = [string]$asset.platform
        if ($PLATFORMS -notcontains $p -or (-not $platformsSeen.Add($p))) {
            throw "invalid platform, duplicate or oversized release asset: $p"
        }
        $size = [long]$asset.size
        if ($size -le 0 -or $size -gt $MAX_BINARY_BYTES) {
            throw "invalid platform, duplicate or oversized release asset: size $size"
        }
        $sha = [string]$asset.sha256
        if ($sha -notmatch '^[0-9a-f]{64}$') {
            throw "invalid asset sha256: $sha"
        }
        $name = [string]$asset.name
        $checksumName = [string]$asset.checksum_name
        if ($name -cnotmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$' -or (-not $namesSeen.Add($name))) {
            throw "asset names must be distinct portable basenames: $name"
        }
        if ($checksumName.Contains('/') -or $checksumName.Contains('\') -or (-not $namesSeen.Add($checksumName))) {
            throw "asset names must be distinct portable basenames: $checksumName"
        }
        if ($checksumName -ne "$name.sha256") {
            throw "checksum sidecar must bind exact binary basename: $checksumName"
        }
    }
}

function Get-FileSha256([string]$filePath) {
    $stream = [System.IO.File]::OpenRead($filePath)
    try {
        $sha = [System.Security.Cryptography.SHA256]::Create()
        try {
            $hashBytes = $sha.ComputeHash($stream)
            $builder = New-Object System.Text.StringBuilder
            foreach ($b in $hashBytes) {
                $builder.Append($b.ToString("x2")) | Out-Null
            }
            return $builder.ToString()
        } finally {
            $sha.Dispose()
        }
    } finally {
        $stream.Close()
    }
}

function Verify-ChecksumSidecar([string]$checksumStr, $asset) {
    $matched = $false
    $lines = $checksumStr -split "[\r\n]+"
    foreach ($line in $lines) {
        $trimmed = $line.Trim()
        if ([string]::IsNullOrEmpty($trimmed)) {
            continue
        }
        $parts = $trimmed -split '\s+'
        if ($parts.Count -gt 0) {
            $digest = $parts[0].ToLowerInvariant()
            if ($digest -notmatch '^[0-9a-f]{64}$') {
                throw "invalid sidecar digest: $digest"
            }
            if ($digest -eq $asset.sha256) {
                if ($parts.Count -gt 1) {
                    $filename = $parts[1].TrimStart('*')
                    if ($filename -ne $asset.name) {
                        throw "checksum sidecar filename mismatch: expected $($asset.name), got $filename"
                    }
                }
                $matched = $true
                break
            }
        }
    }
    if (-not $matched) {
        throw "checksum sidecar does not contain expected sha256 for asset"
    }
}

function Verify-CandidateBinary([string]$binaryPath, $metadata) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $binaryPath
    $psi.Arguments = "--version --json"
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true

    $proc = [System.Diagnostics.Process]::Start($psi)
    $stdout = $proc.StandardOutput.ReadToEnd()
    $stderr = $proc.StandardError.ReadToEnd()
    $proc.WaitForExit()

    if ($proc.ExitCode -ne 0) {
        throw "candidate binary --version --json probe failed with exit code $($proc.ExitCode): $stderr"
    }

    try {
        $json = $stdout | ConvertFrom-Json
    } catch {
        throw "failed to parse candidate binary --version --json output: $_"
    }

    $data = $json
    if ($json.schema -eq "agentfoundry.command-result/v1") {
        if ($json.status -ne "ok" -or $json.code -ne "OK") {
            throw "candidate binary returned non-ok status: $($json.status) ($($json.code))"
        }
        $data = $json.data
    }

    if ($data.version -ne $metadata.version) {
        throw "candidate binary version mismatch: expected $($metadata.version), got $($data.version)"
    }
    if ($data.source -ne $metadata.source_commit) {
        throw "candidate binary source identity mismatch: expected $($metadata.source_commit), got $($data.source)"
    }
    if ($data.payload_digest -ne $metadata.payload_digest) {
        throw "candidate binary payload_digest mismatch: expected $($metadata.payload_digest), got $($data.payload_digest)"
    }
    if ($data.distribution -ne "stable") {
        throw "candidate binary distribution mismatch: expected 'stable', got '$($data.distribution)'"
    }
    if ($data.fault_injection -ne $false) {
        throw "candidate binary fault_injection mismatch: expected false, got $($data.fault_injection)"
    }
}

# Platform Architecture Validation
$arch = $env:PROCESSOR_ARCHITECTURE
if ($env:PROCESSOR_ARCHITEW6432) {
    $arch = $env:PROCESSOR_ARCHITEW6432
}
if ($arch -ine "AMD64" -and $arch -ine "x86_64") {
    [Console]::Error.WriteLine("agentfoundry install: unsupported architecture '$arch' for Windows AgentFoundry installation. Only x86_64 (AMD64) is supported.")
    exit 6
}

$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ("af-install-" + [System.Guid]::NewGuid().ToString("N"))
[System.IO.Directory]::CreateDirectory($tempDir) | Out-Null

$exitCode = 1

try {
    # 1. Fetch latest release DTO from GitHub API
    $apiJsonStr = Invoke-BoundedGet $LATEST_RELEASE_API_URL "Api" "application/vnd.github+json" $MAX_METADATA_BYTES
    $releaseDto = $apiJsonStr | ConvertFrom-Json

    if ($releaseDto.draft -isnot [bool] -or $releaseDto.prerelease -isnot [bool] -or $releaseDto.draft -or $releaseDto.prerelease) {
        throw "invalid, draft or prerelease release"
    }

    $tag = [string]$releaseDto.tag_name
    if (-not $tag.StartsWith("v")) {
        throw "release tag must be stable vMAJOR.MINOR.PATCH"
    }
    $versionFromTag = $tag.Substring(1)
    if (-not (Validate-Semver $versionFromTag)) {
        throw "release tag must be stable vMAJOR.MINOR.PATCH"
    }

    # 2. Fetch and validate agentfoundry-release.json metadata
    $metadataUrl = "https://github.com/$GITHUB_REPOSITORY/releases/download/$tag/$METADATA_ASSET_NAME"
    $metadataJsonStr = Invoke-BoundedGet $metadataUrl "Download" "application/json, */*" $MAX_METADATA_BYTES
    $metadata = $metadataJsonStr | ConvertFrom-Json

    Validate-Metadata $metadata

    if ($metadata.tag -ne $releaseDto.tag_name) {
        throw "release metadata tag '$($metadata.tag)' does not match GitHub release tag '$($releaseDto.tag_name)'"
    }

    # 3. Validate asset inventory matches exact 11 items
    Validate-AssetInventory $releaseDto $metadata

    # 4. Locate Windows platform asset
    $selectedAsset = $null
    foreach ($a in $metadata.assets) {
        if ($a.platform -eq $TARGET_PLATFORM) {
            $selectedAsset = $a
            break
        }
    }
    if (-not $selectedAsset) {
        throw "unsupported candidate platform: $TARGET_PLATFORM"
    }

    # 5. Download binary candidate
    $binaryUrl = "https://github.com/$GITHUB_REPOSITORY/releases/download/$tag/$($selectedAsset.name)"
    $binaryPath = Join-Path $tempDir $selectedAsset.name
    Invoke-BoundedGet $binaryUrl "Download" "application/octet-stream, */*" $MAX_BINARY_BYTES $binaryPath

    $downloadedSize = (Get-Item $binaryPath).Length
    if ($downloadedSize -ne [long]$selectedAsset.size) {
        throw "candidate binary size mismatch: expected $($selectedAsset.size), got $downloadedSize"
    }

    $computedDigest = Get-FileSha256 $binaryPath
    if ($computedDigest -ne $selectedAsset.sha256) {
        throw "candidate binary digest mismatch: expected $($selectedAsset.sha256), got $computedDigest"
    }

    # 6. Download and verify checksum sidecar
    $checksumUrl = "https://github.com/$GITHUB_REPOSITORY/releases/download/$tag/$($selectedAsset.checksum_name)"
    $sidecarContent = Invoke-BoundedGet $checksumUrl "Download" "text/plain, */*" $MAX_SIDECAR_BYTES
    Verify-ChecksumSidecar $sidecarContent $selectedAsset

    # 7. Candidate binary probe: verify version, source commit, payload digest, stable distribution, no fault injection
    Verify-CandidateBinary $binaryPath $metadata

    # 8. Prepare arguments forwarding for install subcommand
    $forwarded = @('install') + $args

    $escapedArgs = @()
    foreach ($a in $forwarded) {
        $escapedArgs += (Escape-CommandLineArg $a)
    }
    $argumentsString = $escapedArgs -join ' '

    # 9. Invoke native install preserving console and exit status
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $binaryPath
    $psi.Arguments = $argumentsString
    $psi.UseShellExecute = $false

    $proc = [System.Diagnostics.Process]::Start($psi)
    $proc.WaitForExit()
    $exitCode = $proc.ExitCode
    $proc.Dispose()

} catch {
    $msg = $_.Exception.Message
    if (-not $msg) {
        $msg = "$_"
    }
    [Console]::Error.WriteLine("agentfoundry install: $msg")
    $exitCode = 1
} finally {
    if ($tempDir -and (Test-Path $tempDir)) {
        try {
            Remove-Item -Recurse -Force $tempDir -ErrorAction SilentlyContinue
        } catch {
            # Best-effort cleanup
        }
    }
}

exit $exitCode
