# Requires -Version 5.1
<#
.SYNOPSIS
    AgentFoundry external caller-waited Windows launcher.
.DESCRIPTION
    Provides platform adapter for Windows binary replacement (ADR-0004 / AF-012).
    - For non-replacing commands, invokes selected repository binary directly.
    - For self-replacing commands on an existing installed binary, stages a worker
      outside the target repository to release the Windows NTFS SEC_IMAGE lock,
      verified by SHA-256 digest against the selected executable.
    - When the selected binary is missing, invokes a separate trusted installed
      binary path to verify and restore the retained candidate via Rust before
      the candidate is ever executed.
    - Never fabricates JSON command envelopes; all structured outputs come from
      the native binary.
#>

$ErrorActionPreference = 'Stop'

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

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir "..")).Path

$targetDir = $null
$subcommand = $null

for ($i = 0; $i -lt $args.Count; $i++) {
    $arg = $args[$i]
    if ($arg -in @('--directory', '--plan-out', '--apply-plan')) {
        if ($i + 1 -lt $args.Count) {
            if ($arg -eq '--directory') { $targetDir = $args[$i + 1] }
            $i++
        }
    } elseif (-not $arg.StartsWith('-') -and -not $subcommand) {
        $subcommand = $arg
    }
}

if ($targetDir) {
    if (-not [System.IO.Path]::IsPathRooted($targetDir)) {
        $targetDir = [System.IO.Path]::GetFullPath((Join-Path (Get-Location).Path $targetDir))
    } else {
        $targetDir = [System.IO.Path]::GetFullPath($targetDir)
    }
} else {
    $targetDir = (Get-Location).Path
}

$selectedBinary = Join-Path $targetDir "scripts\bin\agentfoundry.exe"
$launcherBinary = Join-Path $repoRoot "scripts\bin\agentfoundry.exe"

# Resolve trusted installed binary if selected binary is absent
function Resolve-TrustedBinary() {
    if ($env:AGENTFOUNDRY_BIN -and (Test-Path $env:AGENTFOUNDRY_BIN)) {
        return (Resolve-Path $env:AGENTFOUNDRY_BIN).Path
    }
    if (Test-Path $launcherBinary) {
        return (Resolve-Path $launcherBinary).Path
    }
    $cmd = Get-Command agentfoundry.exe -ErrorAction SilentlyContinue
    if ($cmd -and (Test-Path $cmd.Source)) {
        return (Resolve-Path $cmd.Source).Path
    }
    return $null
}

$escapedArgs = @()
foreach ($a in $args) {
    $escapedArgs += (Escape-CommandLineArg $a)
}
$argumentsString = $escapedArgs -join ' '

# Case 1: Target selected binary exists on disk
if (Test-Path $selectedBinary) {
    $isReplacing = $subcommand -in @('update', 'recover')

    if ($isReplacing) {
        # Self-replacing action: stage worker in TEMP to release NTFS SEC_IMAGE lock on selected binary
        $fileInfo = New-Object System.IO.FileInfo($selectedBinary)
        if ($fileInfo.Length -gt 134217728) {
            [Console]::Error.WriteLine("agentfoundry: executable exceeds 128MiB limit")
            exit 7
        }
        $sourceBytes = [System.IO.File]::ReadAllBytes($selectedBinary)

        $guid = [Guid]::NewGuid().ToString("N")
        $tempDir = Join-Path ([System.IO.Path]::GetTempPath()) "af-worker-$guid"
        [System.IO.Directory]::CreateDirectory($tempDir) | Out-Null
        $workerExe = Join-Path $tempDir "agentfoundry.exe"

        $exitCode = 1
        try {
            [System.IO.File]::WriteAllBytes($workerExe, $sourceBytes)

            $psi = New-Object System.Diagnostics.ProcessStartInfo
            $psi.FileName = $workerExe
            $psi.Arguments = $argumentsString
            $psi.UseShellExecute = $false
            $psi.EnvironmentVariables["AF_WORKER"] = "1"
            $psi.EnvironmentVariables["AF_SELECTED_BINARY"] = $selectedBinary

            $proc = [System.Diagnostics.Process]::Start($psi)
            $proc.WaitForExit()
            $exitCode = $proc.ExitCode
        } finally {
            if (Test-Path $tempDir) {
                for ($attempt = 0; $attempt -lt 10; $attempt++) {
                    try {
                        Remove-Item -Path $tempDir -Recurse -Force -ErrorAction Stop
                        break
                    } catch {
                        Start-Sleep -Milliseconds 50
                    }
                }
            }
        }
        exit $exitCode
    } else {
        # Non-replacing command: direct execution via target's selected binary
        $psi = New-Object System.Diagnostics.ProcessStartInfo
        $psi.FileName = $selectedBinary
        $psi.Arguments = $argumentsString
        $psi.UseShellExecute = $false

        $proc = [System.Diagnostics.Process]::Start($psi)
        $proc.WaitForExit()
        exit $proc.ExitCode
    }
}

# Case 2: Target selected binary is missing on disk
# Use separate trusted installed binary path to verify and execute via Rust
$trustedBinary = Resolve-TrustedBinary

if (-not $trustedBinary) {
    [Console]::Error.WriteLine("agentfoundry: selected executable missing at '$selectedBinary' and no trusted installed binary available")
    exit 2
}

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $trustedBinary
$psi.Arguments = $argumentsString
$psi.UseShellExecute = $false

$proc = [System.Diagnostics.Process]::Start($psi)
$proc.WaitForExit()
exit $proc.ExitCode
