param(
    [ValidateSet('app', 'rust', 'stroke', 'pen', 'pen-owner', 'win-pen', 'transport', 'pairing', 'shared-ffi', 'video-pc', 'video-android', 'video-tiles', 'image-pc', 'image-android', 'vdd')][string]$Mode = 'app',
    [ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
    [switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$WinPenScenario = 'normal',
    [ValidatePattern('^[0-9a-f]{32}$')][string]$VideoRunId,
    [ValidateSet('inventory','normal','watchdog')][string]$VddScenario='inventory'
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $PSScriptRoot 'process.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'android-device.psm1') -Force

try {
    if ($Mode -eq 'app') {
        if ($TimeoutSeconds -lt 60 -or $TimeoutSeconds -gt 3600) { throw 'App HIL phase timeout must be between 60 and 3600 seconds' }
        $run = Invoke-VwProcess -FilePath 'pwsh.exe' -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/app-test/run_android.ps1',
            '-ExpectedModel', 'IN2019', '-TimeoutSeconds', "$TimeoutSeconds", '-Execute'
        ) -WorkingDirectory $projectRoot -Phase 'hil-app-isolated' -TimeoutSeconds ($TimeoutSeconds * 2 + 1800) -ParentExitGraceSeconds 20 -RedactValues @($projectRoot, $env:USERPROFILE)
        if ($run.ExitCode -ne 0) { throw 'Isolated app HIL assertions, APK binding or owned cleanup failed' }
        exit 0
    }
    if ($Mode -eq 'shared-ffi') {
        if ($TimeoutSeconds -lt 60 -or $TimeoutSeconds -gt 3600) { throw 'Shared FFI phase timeout must be between 60 and 3600 seconds' }
        # Cold configuration on this host has measured 130 seconds; keep task
        # inventory separate from test execution with a realistic upper bound.
        $inventorySeconds = 300
        # The child inventories actual AGP tasks before its three bounded
        # Gradle phases. Reserve the nine 120s parser/packaging checks, device
        # selection and process overhead separately, plus owned cleanup time.
        $runnerBudget = $TimeoutSeconds * 3 + $inventorySeconds + 1800 + 300
        $run = Invoke-VwProcess -FilePath 'pwsh.exe' -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
            'tools/ffi-test/run_shared_android.ps1', '-ExpectedModel', 'IN2019',
            '-TimeoutSeconds', "$TimeoutSeconds", '-InventoryTimeoutSeconds', "$inventorySeconds"
        ) -WorkingDirectory $projectRoot -Phase 'hil-shared-ffi' -TimeoutSeconds $runnerBudget -ParentExitGraceSeconds 20 -RedactValues @($projectRoot, $env:USERPROFILE)
        if ($run.ExitCode -ne 0) { throw 'Shared FFI assertions, provenance, packaging or owned cleanup failed' }
        exit 0
    }
    if ($Mode -eq 'pairing') {
        $run = Invoke-VwProcess -FilePath 'pwsh.exe' -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/pair-cli/run_adb.ps1', '-TimeoutSeconds', '180') -WorkingDirectory $projectRoot -Phase 'hil-pairing-adb' -TimeoutSeconds 480
        if ($run.ExitCode -ne 0) { throw 'Pairing HIL or its owned cleanup failed' }
        exit 0
    }
    if ($Mode -eq 'stroke') {
        & (Join-Path $PSScriptRoot 'stroke-spike/hil-stroke.ps1') -TimeoutSeconds $TimeoutSeconds
        exit 0
    }
    if ($Mode -eq 'image-pc') {
        & (Join-Path $PSScriptRoot 'bench/image/run_pc.ps1')
        exit 0
    }
    if ($Mode -eq 'image-android') {
        & (Join-Path $PSScriptRoot 'bench/image-android/run_android.ps1')
        exit 0
    }
    if ($Mode -eq 'vdd') {
        & (Join-Path $PSScriptRoot 'vdd-probe/run.ps1') -Scenario $VddScenario -OwnerReady:$OwnerReady
        exit 0
    }
    if ($Mode -eq 'video-pc') {
        & (Join-Path $PSScriptRoot 'bench/video-pc/run_pc.ps1') -Profile full
        exit 0
    }
    if ($Mode -eq 'video-android') {
        if (-not $VideoRunId) { throw 'Decoder HIL requires the native VideoRunId' }
        & (Join-Path $PSScriptRoot 'bench/video-android/run_android.ps1') -RunId $VideoRunId
        exit 0
    }
    if ($Mode -eq 'transport') {
        & (Join-Path $PSScriptRoot 'bench/transport/run_adb.ps1') -Profile full -TimeoutSeconds $TimeoutSeconds
        exit 0
    }
    if ($Mode -eq 'video-tiles') {
        if (-not $VideoRunId) { throw 'Tile HIL requires the native VideoRunId' }
        & (Join-Path $PSScriptRoot 'bench/video-android/run_tiles.ps1') -RunId $VideoRunId -Profile full
        exit 0
    }
    if ($Mode -eq 'win-pen') {
        if (-not $OwnerReady) { throw 'Reserve the desktop with the owner before Windows input HIL' }
        & (Join-Path $PSScriptRoot 'pen-inject\hil-injection.ps1') -OwnerReady -Scenario $WinPenScenario -TimeoutSeconds $TimeoutSeconds
        # The PowerShell runner throws on failure; it does not set a native exit code.
        exit 0
    }
    . (Join-Path $PSScriptRoot 'enter-dev.ps1')
    if ($Mode -in @('pen', 'pen-owner')) {
        & (Join-Path $PSScriptRoot 'pen-trace\hil-pen.ps1') -Mode $Mode -TimeoutSeconds $TimeoutSeconds
        exit $LASTEXITCODE
    }
    $device = Get-VwAndroidDevice -Root $projectRoot
    if ($Mode -eq 'rust') {
        if (-not $Crate) { throw 'Specify -Crate for a Rust HIL test' }
        $passedTests = Invoke-VwAndroidCargoTest -Device $device -Crate $Crate -TimeoutSeconds $TimeoutSeconds
        Write-Host "HIL Rust: PASS ($passedTests actual tests through the configured Cargo target runner); crate $Crate"

    }
    exit 0
} catch {
    Write-Error $_
    exit 1
}
