param([ValidateRange(30, 3600)][int]$TimeoutSeconds = 300)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
$check = Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/stroke-spike/build_receipt.py','check') -WorkingDirectory $projectRoot -Phase 'stroke-build-binding' -TimeoutSeconds 30
if ($check.ExitCode -ne 0) { throw 'Run build.ps1 build-stroke first; no device action performed' }
$device = Get-VwAndroidDevice -Root $projectRoot
$apiResult = Invoke-VwAdb -Device $device -Arguments @('shell','getprop','ro.build.version.sdk') -Phase 'stroke-device-api' -TimeoutSeconds 15 -Capture
$releaseResult = Invoke-VwAdb -Device $device -Arguments @('shell','getprop','ro.build.version.release') -Phase 'stroke-device-release' -TimeoutSeconds 15 -Capture
$androidApi = ($apiResult.Lines -join '').Trim()
$androidRelease = ($releaseResult.Lines -join '').Trim()
if ($androidApi -notmatch '^\d{2,3}$' -or $androidRelease -notmatch '^\d{1,3}(\.\d{1,3}){0,2}$') { throw 'Invalid Android version response' }
$runId = [guid]::NewGuid().ToString('N')
$runDirectory = Join-Path $projectRoot ('.local/stroke-hil-' + $runId)
New-Item -ItemType Directory -Path $runDirectory | Out-Null
$remoteBinary = "/data/local/tmp/visualworkbench-stroke-$runId"
$package = 'com.visualworkbench.strokespike'
$apkRoot = Join-Path $PSScriptRoot 'android/build/outputs/apk'
$success = $false
try {
    Invoke-VwAdb -Device $device -Arguments @('push', (Join-Path $projectRoot 'target/aarch64-linux-android/release/vw-stroke-spike'), $remoteBinary) -Phase 'stroke-native-push' -Capture | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('shell','chmod','700',$remoteBinary) -Phase 'stroke-native-mode' -Capture | Out-Null
    $goldens = Invoke-VwAdb -Device $device -Arguments @('shell',$remoteBinary,'goldens') -Phase 'stroke-native-goldens' -TimeoutSeconds $TimeoutSeconds -Capture
    $goldenText = $goldens.Lines -join "`n"
    $actual = $goldenText | ConvertFrom-Json
    $expected = Get-Content -Raw -LiteralPath (Join-Path $projectRoot 'core/crates/vw-ink/fixtures/goldens.json') | ConvertFrom-Json
    if ($actual.algorithm_version -ne $expected.algorithm_version -or @($actual.fixtures).Count -ne 16 -or @($expected.fixtures).Count -ne 16) { throw 'Stroke fixture count/version mismatch' }
    foreach ($fixture in $expected.fixtures) {
        $match = @($actual.fixtures | Where-Object { $_.name -ceq $fixture.name })
        if ($match.Count -ne 1 -or $match[0].geometry_hash -cne $fixture.geometry_hash -or $match[0].vertices -ne $fixture.vertices -or $match[0].samples -ne $fixture.samples) { throw 'Cross-platform stroke golden mismatch' }
    }
    [IO.File]::WriteAllText((Join-Path $runDirectory 'native-goldens.json'), $goldenText, [Text.UTF8Encoding]::new($false))
    $benchmark = Invoke-VwAdb -Device $device -Arguments @('shell',$remoteBinary,'benchmark','100') -Phase 'stroke-native-benchmark' -TimeoutSeconds $TimeoutSeconds -Capture
    # The runner emits bounded progress on stderr; retain only the JSON object.
    $benchmarkText = @($benchmark.Lines | Where-Object { $_ -notmatch '^stroke benchmark progress ' }) -join "`n"
    $benchmarkJson = $benchmarkText | ConvertFrom-Json
    if ($benchmarkJson.samples_measured -ne 12800 -or $benchmarkJson.repetitions -ne 100 -or $benchmarkJson.p95_ns -lt 0) { throw 'Incomplete stroke benchmark' }
    [IO.File]::WriteAllText((Join-Path $runDirectory 'native-benchmark.json'), $benchmarkText, [Text.UTF8Encoding]::new($false))
    Invoke-VwAdb -Device $device -Arguments @('install','-r',(Join-Path $apkRoot 'debug/stroke-spike-debug.apk')) -Phase 'stroke-install' -Capture | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('install','-r',(Join-Path $apkRoot 'androidTest/debug/stroke-spike-debug-androidTest.apk')) -Phase 'stroke-test-install' -Capture | Out-Null
    $ready = $false
    for ($attempt = 1; $attempt -le 2; $attempt++) {
        $launch = Invoke-VwAdb -Device $device -Arguments @('shell','am','start','-W','-n',"$package/.StrokeActivity") -Phase 'stroke-launch' -TimeoutSeconds 15 -Capture
        if (($launch.Lines -join "`n") -notmatch '(?m)^Status:\s*ok\s*$') { throw 'Stroke activity launch failed' }
        $focus = Invoke-VwAdb -Device $device -Arguments @('shell','dumpsys','activity','activities') -Phase 'stroke-focus' -TimeoutSeconds 15 -Capture
        $ready = @($focus.Lines | Where-Object { $_ -match '(?:mResumedActivity|topResumedActivity).*com\.visualworkbench\.strokespike/' }).Count -gt 0
        if ($ready) { break }
        Write-Host "Shared phone changed foreground; bounded relaunch $attempt/2."
    }
    if (-not $ready) { throw 'INCONCLUSIVE: comparison app could not retain foreground' }
    $instrumentation = Invoke-VwAdb -Device $device -Arguments @('shell','am','instrument','-w','-r','-e','runId',$runId,"$package.test/androidx.test.runner.AndroidJUnitRunner") -Phase 'stroke-instrumentation' -TimeoutSeconds $TimeoutSeconds -Capture
    $output = $instrumentation.Lines -join "`n"
    [IO.File]::WriteAllText((Join-Path $runDirectory 'instrumentation-private.txt'), $output, [Text.UTF8Encoding]::new($false))
    $ok = [regex]::Match($output, 'OK \(([1-9][0-9]*) tests?\)')
    if (-not $ok.Success -or [int]$ok.Groups[1].Value -ne 3 -or $output -match 'FAILURES|INSTRUMENTATION_FAILED|INSTRUMENTATION_ABORTED') {
        $instrumentation.Lines | Where-Object { $_ -match 'AssertionError|IllegalArgumentException|IllegalStateException|INCONCLUSIVE|FAILURES' } | ForEach-Object { Write-Host $_ }
        throw 'Stroke instrumentation failed; source-bound native receipts retained'
    }
    $reportResult = Invoke-VwAdb -Device $device -Arguments @('exec-out','run-as',$package,'cat','files/stroke-report.json') -Phase 'stroke-report' -TimeoutSeconds 15 -Capture
    $reportText = $reportResult.Lines -join "`n"
    $report = $reportText | ConvertFrom-Json
    if ($report.run_id -cne $runId -or -not $report.complete -or @($report.results).Count -ne 3 -or @($report.results | Where-Object { -not $_.passed }).Count -gt 0) { throw 'Fresh stroke report incomplete' }
    [IO.File]::WriteAllText((Join-Path $runDirectory 'stroke-report.json'), $reportText, [Text.UTF8Encoding]::new($false))
    $buildBinding = Get-Content -Raw -LiteralPath (Join-Path $projectRoot '.local/stroke-build.json') | ConvertFrom-Json
    $receipt = [ordered]@{schema=1;run_id=$runId;device_model=$device.Model;android_api=[int]$androidApi;android_release=$androidRelease;hardware_pen_acceptance=$false;synthetic_goldens=16;instrumentation_tests=3;cpu_append_p95_ns=$benchmarkJson.p95_ns;cpu_append_target_ns=20000;cpu_append_target_met=$benchmarkJson.meets_target;verification_image_files=0;source_binding=$buildBinding}
    [IO.File]::WriteAllText((Join-Path $runDirectory 'receipt.json'), ($receipt | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $success = $true
    Write-Host "Stroke software HIL: PASS (16 cross-platform goldens, 3 instrumentation tests); CPU append p95=$($benchmarkJson.p95_ns) ns."
    Write-Host "Private text receipts: $runDirectory. S23 pen/display/owner acceptance deferred."
} finally {
    if (-not $success) {
        try {
            $failureReport = Invoke-VwAdb -Device $device -Arguments @('exec-out','run-as',$package,'cat','files/stroke-report.json') -Phase 'stroke-failure-report' -TimeoutSeconds 15 -Capture
            $failureText = $failureReport.Lines -join "`n"
            $failureJson = $failureText | ConvertFrom-Json
            if ($failureJson.run_id -ceq $runId) {
                [IO.File]::WriteAllText((Join-Path $runDirectory 'stroke-report-failed.json'), $failureText, [Text.UTF8Encoding]::new($false))
            }
        } catch { Write-Host 'No fresh instrumentation report was available for this failed attempt.' }
    }
    # Exact unique task-owned file and packages; other device apps are preserved.
    Invoke-VwAdb -Device $device -Arguments @('shell','rm','-f',$remoteBinary) -Phase 'stroke-native-cleanup' -TimeoutSeconds 15 -Capture | Out-Null
    foreach ($ownedPackage in @("$package.test", $package)) {
        Invoke-VwAdb -Device $device -Arguments @('shell','am','force-stop',$ownedPackage) -Phase 'stroke-app-cleanup' -TimeoutSeconds 15 -Capture | Out-Null
    }
    if (-not $success) { Write-Host "Stroke run incomplete; retained text receipts: $runDirectory" }
}
