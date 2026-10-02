param(
    [ValidateSet('pen', 'pen-owner')][string]$Mode = 'pen',
    [ValidateRange(30, 3600)][int]$TimeoutSeconds = 300
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
try {
    if ($Mode -eq 'pen-owner') {
        $validation = Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/pen-trace/trace_tool.py', 'acceptance', 'fixtures/traces') -WorkingDirectory $projectRoot -Phase 'pen-owner-fixtures' -TimeoutSeconds 30
        if ($validation.ExitCode -ne 0) { throw 'Owner traces are incomplete; no device actions performed' }
    }
    $build = Invoke-VwProcess -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'build.ps1', 'build-pen-probe', '-TimeoutSeconds', "$TimeoutSeconds") -WorkingDirectory $projectRoot -Phase 'pen-hil-build' -TimeoutSeconds ($TimeoutSeconds * 4 + 120) -ParentExitGraceSeconds 20
    if ($build.ExitCode -ne 0) { throw 'Pen probe build failed' }
    $device = Get-VwAndroidDevice -Root $projectRoot
    $apkRoot = Join-Path $PSScriptRoot 'probe-android/build/outputs/apk'
    Invoke-VwAdb -Device $device -Arguments @('install', '-r', (Join-Path $apkRoot 'debug/pen-probe-debug.apk')) -Phase 'pen-install' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('install', '-r', (Join-Path $apkRoot 'androidTest/debug/pen-probe-debug-androidTest.apk')) -Phase 'pen-test-install' | Out-Null
    $ready = $false
    for ($attempt = 1; $attempt -le 2; $attempt++) {
        $launch = Invoke-VwAdb -Device $device -Arguments @('shell', 'am', 'start', '-W', '-n', 'com.visualworkbench.penprobe/.ProbeActivity') -Phase 'pen-launch' -TimeoutSeconds 15 -Capture
        if (($launch.Lines -join "`n") -notmatch '(?m)^Status:\s*ok\s*$') { throw 'Pen activity launch did not report success' }
        $focus = Invoke-VwAdb -Device $device -Arguments @('shell', 'dumpsys', 'activity', 'activities') -Phase 'pen-focus' -TimeoutSeconds 15 -Capture
        $ready = @($focus.Lines | Where-Object { $_ -match '(?:mResumedActivity|topResumedActivity).*com\.visualworkbench\.penprobe/' }).Count -gt 0
        if ($ready) { break }
        Write-Host "Pen probe lost foreground on shared phone, attempt $attempt/2; preserving the other app."
    }
    if (-not $ready) { throw 'INCONCLUSIVE: probe could not retain foreground' }
    $traceMode = if ($Mode -eq 'pen-owner') { 'owner' } else { 'synthetic' }
    # Remove only our two stale report files so a prior result cannot satisfy this run.
    Invoke-VwAdb -Device $device -Arguments @('shell', 'run-as', 'com.visualworkbench.penprobe', 'rm', '-f', 'files/replay-report.json', 'files/last-replay-metrics.json', 'files/last-replay.json', 'files/last-replay-source.json') -Phase 'pen-stale-metrics' -Capture | Out-Null
    $result = $null
    $runDirectory = Join-Path $projectRoot ('.local/pen-hil-' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $runDirectory | Out-Null
    try {
        $result = Invoke-VwAdb -Device $device -Arguments @('shell', 'am', 'instrument', '-w', '-r', '-e', 'traceMode', $traceMode, 'com.visualworkbench.penprobe.test/androidx.test.runner.AndroidJUnitRunner') -Phase 'pen-replay' -TimeoutSeconds $TimeoutSeconds -Capture
    } finally {
        foreach ($name in @('replay-report.json', 'last-replay-metrics.json', 'last-replay.json', 'last-replay-source.json')) {
            try {
                $data = Invoke-VwAdb -Device $device -Arguments @('exec-out', 'run-as', 'com.visualworkbench.penprobe', 'cat', "files/$name") -Phase 'pen-collect-metrics' -TimeoutSeconds 15 -Capture
                $text = $data.Lines -join "`n"
                $null = $text | ConvertFrom-Json
                [IO.File]::WriteAllText((Join-Path $runDirectory $name), $text, [Text.UTF8Encoding]::new($false))
            } catch { Write-Host "No fresh $name available; evidence remains incomplete." }
        }
        # Stop only our instrumentation/probe processes, including on adb timeout.
        Invoke-VwAdb -Device $device -Arguments @('shell', 'am', 'force-stop', 'com.visualworkbench.penprobe.test') -Phase 'pen-test-cleanup' -TimeoutSeconds 15 -Capture | Out-Null
        Invoke-VwAdb -Device $device -Arguments @('shell', 'am', 'force-stop', 'com.visualworkbench.penprobe') -Phase 'pen-probe-cleanup' -TimeoutSeconds 15 -Capture | Out-Null
    }
    $output = $result.Lines -join "`n"
    if ($output -notmatch 'OK \(([1-9][0-9]*) tests?\)' -or $output -match 'FAILURES|INSTRUMENTATION_FAILED|INSTRUMENTATION_ABORTED') {
        $safeFailures = @($result.Lines | Where-Object { $_ -match 'AssertionError|IllegalArgumentException|IllegalStateException|INCONCLUSIVE|FAILURES|INSTRUMENTATION_FAILED' })
        $safeFailures | ForEach-Object { Write-Host $_ }
        throw "Pen instrumentation failed; private metrics: $runDirectory"
    }
    $tests = [int]$Matches[1]
    $report = Get-Content -LiteralPath (Join-Path $runDirectory 'replay-report.json') -Raw | ConvertFrom-Json
    if (-not $report.complete -or @($report.results).Count -eq 0 -or @($report.results | Where-Object { -not $_.passed }).Count -gt 0) { throw 'Replay metrics incomplete or failed' }
    Write-Host "Pen HIL: PASS ($tests tests; $traceMode traces). Hardware capability evidence: false."
    Write-Host "Metrics: $runDirectory"
    $report.results | Select-Object name,expected_samples,received_samples,expected_keys,received_keys,max_timestamp_deviation_ns,p95_timestamp_deviation_ns | ConvertTo-Json
    exit 0
} catch { Write-Error $_; exit 1 }
