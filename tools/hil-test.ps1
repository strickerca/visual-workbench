param(
    [ValidateSet('app', 'rust', 'pen', 'pen-owner', 'win-pen', 'transport', 'video-pc', 'video-android', 'video-tiles')][string]$Mode = 'app',
    [ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
    [switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$WinPenScenario = 'normal',
    [ValidatePattern('^[0-9a-f]{32}$')][string]$VideoRunId
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $PSScriptRoot 'process.psm1') -Force
Import-Module (Join-Path $PSScriptRoot 'android-device.psm1') -Force

try {
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
    } else {
        $apk = Join-Path $projectRoot 'apps\android\build\outputs\apk\debug\android-debug.apk'
        if (-not (Test-Path -LiteralPath $apk -PathType Leaf)) { throw 'Build the Android APK with build-android before app HIL' }
        Invoke-VwAdb -Device $device -Arguments @('install', '-r', $apk) -Phase 'hil-apk-install' | Out-Null
        Start-VwAndroidStarter -Device $device -PhasePrefix 'hil-app'
        Write-Host 'HIL app install/launch/foreground: PASS.'
        $previousSerial = [Environment]::GetEnvironmentVariable('ANDROID_SERIAL', 'Process')
        try {
            [Environment]::SetEnvironmentVariable('ANDROID_SERIAL', $device.Serial, 'Process')
            $testStart = Get-Date
            $instrumentation = Invoke-VwProcess -FilePath (Join-Path $projectRoot 'apps\gradlew.bat') -ArgumentList @(':android:connectedDebugAndroidTest', '--console=plain', '--no-daemon', '--no-parallel', '--no-configuration-cache', '--rerun-tasks') -WorkingDirectory (Join-Path $projectRoot 'apps') -Phase 'hil-android-instrumentation' -TimeoutSeconds $TimeoutSeconds -RedactValues @($device.Serial, $projectRoot, $env:USERPROFILE)
            if ($instrumentation.ExitCode -ne 0) { throw "Android instrumentation failed (exit $($instrumentation.ExitCode))" }
            $reportRoot = Join-Path $projectRoot 'apps\android\build\outputs\androidTest-results\connected'
            $testCount = 0
            $reports = @(Get-ChildItem -LiteralPath $reportRoot -Filter 'TEST-*.xml' -Recurse -File -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTime -ge $testStart.AddSeconds(-2) })
            foreach ($report in $reports) {
                [xml]$result = Get-Content -Raw -LiteralPath $report.FullName
                foreach ($suite in $result.SelectNodes('//testsuite')) {
                    if ([int]$suite.failures -ne 0 -or [int]$suite.errors -ne 0) { throw 'Fresh instrumentation report contains failures or errors' }
                    $tests = [int]$suite.GetAttribute('tests')
                    $skipped = if ($suite.HasAttribute('skipped')) { [int]$suite.GetAttribute('skipped') } else { @($suite.SelectNodes('./testcase/skipped')).Count }
                    if ($skipped -lt 0 -or $skipped -gt $tests) { throw 'Fresh instrumentation report contains inconsistent skipped-test counts' }
                    $testCount += $tests - $skipped
                }
            }
            if ($testCount -eq 0) { throw 'Instrumentation produced no actual passed tests or no fresh result report; acceptance remains pending' }
            Write-Host "HIL Android instrumentation: PASS ($testCount actual passed tests). S23 Ultra/S Pen and performance acceptance remain pending."
            # AGP's connected-test runner removes its installed application after testing.
            # Restore the current built APK so the authorized startup run leaves the app open.
            Invoke-VwAdb -Device $device -Arguments @('install', '-r', $apk) -Phase 'hil-apk-restore-after-tests' | Out-Null
            Start-VwAndroidStarter -Device $device -PhasePrefix 'hil-app-restored'
            Write-Host 'HIL starter restored and foreground: PASS.'
        } finally {
            [Environment]::SetEnvironmentVariable('ANDROID_SERIAL', $previousSerial, 'Process')
        }
    }
    exit 0
} catch {
    Write-Error $_
    exit 1
}
