[CmdletBinding()]
param(
    [ValidateSet("RC1", "RC2", "MANAGED", "PHASE6", "PILOT")]
    [string]$Stage = "RC1",
    [ValidateSet("Idle", "Running", "Storm", "StartupFailure")]
    [string]$Scenario = "Idle",
    [string]$AgentKey,
    [string]$SourceDatabase,
    [string]$SourceCodexHome,
    [string]$CodexExecutable = "codex",
    [ValidateRange(30, 600)]
    [int]$TimeoutSeconds = 180,
    [string]$ResultPath,
    [string]$PrebuiltHelperPath,
    [string]$PrebuiltTestPath,
    [ValidateSet("PAIR", "OFF", "ON")]
    [string]$PilotRuns = "PAIR",
    [ValidateSet("SMALL", "QUALIFIED")]
    [string]$PilotVariant = "SMALL"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
if ([string]::IsNullOrWhiteSpace($SourceDatabase)) {
    $casUserDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
    $SourceDatabase = Join-Path $casUserDirectory ".codex-agent-switch\com.codexagentswitch.desktop\cas.db"
}
if ([string]::IsNullOrWhiteSpace($SourceCodexHome)) {
    $SourceCodexHome = Join-Path $env:USERPROFILE ".codex"
}

$runId = [Guid]::NewGuid().ToString("N")
$stageSlug = if ($Stage -eq "PHASE6") {
    "phase12-$($Scenario.ToLowerInvariant())"
} else {
    $Stage.ToLowerInvariant()
}
$e2eRoot = Join-Path ([IO.Path]::GetTempPath()) "cas-$stageSlug-$runId"
if ([string]::IsNullOrWhiteSpace($ResultPath)) {
    $resultDirectory = Join-Path ([IO.Path]::GetTempPath()) "cas-$stageSlug-results"
    $ResultPath = Join-Path $resultDirectory "$runId.json"
}
$benchmarkRoot = Join-Path (Split-Path -Parent $ResultPath) "$runId-runs"
$benchmarkTemplate = Join-Path $repoRoot "benchmarks\efficiency-fixture"
$helperPath = if ([string]::IsNullOrWhiteSpace($PrebuiltHelperPath)) {
    Join-Path $repoRoot "src-tauri\target\debug\cas-helper.exe"
} else {
    $PrebuiltHelperPath
}
$manifestPath = Join-Path $repoRoot "src-tauri\Cargo.toml"

if ($Stage -ne "PHASE6" -and -not (Test-Path -LiteralPath $SourceDatabase -PathType Leaf)) {
    throw "CAS database not found: $SourceDatabase"
}
$requiresNativeRuntime = $Stage -ne "PHASE6" -or $Scenario -in @("Idle", "Running")
if ($requiresNativeRuntime -and -not (Test-Path -LiteralPath $SourceCodexHome -PathType Container)) {
    throw "CODEX_HOME not found: $SourceCodexHome"
}

$environmentNames = @(
    "CAS_DATABASE_PATH",
    "CAS_E2E_AGENT_KEY",
    "CAS_E2E_BENCHMARK_ROOT",
    "CAS_E2E_BENCHMARK_TEMPLATE",
    "CAS_E2E_PILOT_RUNS",
    "CAS_E2E_PILOT_VARIANT",
    "CAS_E2E_CODEX_EXECUTABLE",
    "CAS_E2E_HELPER_PATH",
    "CAS_E2E_RESULT_PATH",
    "CAS_E2E_ROOT",
    "CAS_E2E_SOURCE_CODEX_HOME",
    "CAS_E2E_SOURCE_DATABASE_PATH",
    "CAS_E2E_TIMEOUT_SECONDS",
    "CODEX_HOME"
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
    $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

try {
    if ($requiresNativeRuntime) {
        [Environment]::SetEnvironmentVariable("CODEX_HOME", $SourceCodexHome, "Process")
        $savedErrorActionPreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = "Continue"
            $loginStatus = & $CodexExecutable login status 2>&1
            $loginExitCode = $LASTEXITCODE
        }
        finally {
            $ErrorActionPreference = $savedErrorActionPreference
        }
        $loginMessage = ($loginStatus | Out-String).Trim()
        $loginAvailable = $loginExitCode -eq 0 `
            -and $loginMessage -match "(?i)Logged in using" `
            -and $loginMessage -notmatch "(?i)Not logged in"
        if (-not $loginAvailable) {
            throw ("Codex native login is unavailable: {0}. Run 'codex login' first." -f $loginMessage)
        }
    }

    if ($Stage -ne "PHASE6" -and [string]::IsNullOrWhiteSpace($PrebuiltHelperPath)) {
        Write-Host "[$Stage] Build current cas-helper..."
        & cargo build --quiet --manifest-path $manifestPath -p cas-helper
        if ($LASTEXITCODE -ne 0) {
            throw "cas-helper build failed."
        }
    }
    elseif ($Stage -ne "PHASE6" -and -not (Test-Path -LiteralPath $helperPath -PathType Leaf)) {
        throw "Prebuilt cas-helper not found: $helperPath"
    }

    [Environment]::SetEnvironmentVariable("CAS_DATABASE_PATH", (Join-Path $e2eRoot "cas-data\cas.db"), "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_AGENT_KEY", $AgentKey, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_BENCHMARK_ROOT", $benchmarkRoot, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_BENCHMARK_TEMPLATE", $benchmarkTemplate, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_PILOT_RUNS", $PilotRuns, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_PILOT_VARIANT", $PilotVariant, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_CODEX_EXECUTABLE", $CodexExecutable, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_HELPER_PATH", $helperPath, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_RESULT_PATH", $ResultPath, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_ROOT", $e2eRoot, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_SOURCE_CODEX_HOME", $SourceCodexHome, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_SOURCE_DATABASE_PATH", $SourceDatabase, "Process")
    [Environment]::SetEnvironmentVariable("CAS_E2E_TIMEOUT_SECONDS", $TimeoutSeconds.ToString(), "Process")

    if ($Stage -eq "PHASE6") {
        switch ($Scenario) {
            "Running" {
                $testName = "runtime_bridge::rc_e2e::managed_session_phase12_running_disconnect_is_not_replayed"
                Write-Host "[PHASE12/RUNNING] Interrupt active Turn -> resume same Primary -> verify no replay..."
            }
            "Storm" {
                $testName = "runtime_bridge::rc_e2e::managed_session_phase12_recovery_storm_stops_at_ceiling"
                Write-Host "[PHASE12/STORM] Fail recovery repeatedly -> stop at retry ceiling..."
            }
            "StartupFailure" {
                $testName = "runtime_bridge::rc_e2e::managed_session_phase12_startup_failure_is_terminal"
                Write-Host "[PHASE12/STARTUP] Reject missing executable -> retain terminal FAILED state..."
            }
            default {
                $testName = "runtime_bridge::rc_e2e::managed_session_phase6_idle_disconnect_recovers_same_primary"
                Write-Host "[PHASE12/IDLE] Kill App Server -> resume same Primary -> explicit stop..."
            }
        }
    }
    elseif ($Stage -eq "MANAGED") {
        $testName = "runtime_bridge::rc_e2e::managed_worker_spawn_reuse_receipt_review"
        Write-Host "[MANAGED] Run real Managed Worker SPAWN -> REUSE -> Receipt -> Review..."
    }
    elseif ($Stage -eq "RC2") {
        $testName = "runtime_bridge::rc_e2e::managed_session_rc2_scheduling_matrix"
        Write-Host "[RC2] Run real CAS2 Job/Receipt/Review SPAWN -> REUSE, then compatibility scheduling matrix..."
    }
    elseif ($Stage -eq "PILOT") {
        $testName = "runtime_bridge::rc_e2e::managed_session_efficiency_pair_off01_on01"
        Write-Host "[PILOT] Run $PilotVariant OFF-01 -> ON-01 pair without global AGENTS fallback..."
    }
    else {
        $testName = "runtime_bridge::rc_e2e::managed_session_rc1_spawn_bind_idle_reuse"
        Write-Host "[RC1] Run real CAS2 TaskPacket -> Job -> SPAWN/REUSE -> Receipt -> Review..."
    }
    if ([string]::IsNullOrWhiteSpace($PrebuiltTestPath)) {
        & cargo test --quiet --manifest-path $manifestPath --lib `
            $testName `
            -- --ignored --exact --nocapture
    }
    else {
        if (-not (Test-Path -LiteralPath $PrebuiltTestPath -PathType Leaf)) {
            throw "Prebuilt test binary not found: $PrebuiltTestPath"
        }
        & $PrebuiltTestPath $testName --ignored --exact --nocapture
    }
    $testExitCode = $LASTEXITCODE

    if (Test-Path -LiteralPath $ResultPath -PathType Leaf) {
        Get-Content -Raw -Encoding utf8 -LiteralPath $ResultPath
        Write-Host "[$Stage/$Scenario] Evidence: $ResultPath"
    }
    if ($testExitCode -ne 0) {
        throw "$Stage real E2E failed with exit code $testExitCode."
    }
}
finally {
    foreach ($name in $environmentNames) {
        [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], "Process")
    }
}
