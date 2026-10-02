param(
    [ValidateSet('smoke','full')][string]$Profile = 'full',
    [string]$ExpectedModel = 'SM-S918U',
    [ValidateRange(30, 1500)][int]$TimeoutSeconds = 1250
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
$projectRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/process.psm1')
$buildCheck = Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/transport/build_receipt.py','check') -WorkingDirectory $projectRoot -Phase 'transport-build-binding' -TimeoutSeconds 30
if ($buildCheck.ExitCode -ne 0) { throw 'Transport gated-build binding failed' }
$device = Get-VwAndroidDevice -Root $projectRoot -ExpectedModel $ExpectedModel
$id = [guid]::NewGuid().ToString('N')
$root = Join-Path $projectRoot ('.local/transport-adb-' + $id)
[IO.Directory]::CreateDirectory($root) | Out-Null
$hostExe = Join-Path $projectRoot 'target/release/transport-bench.exe'
$phoneExe = Join-Path $projectRoot 'target/aarch64-linux-android/release/transport-bench'
if (-not (Test-Path -LiteralPath $hostExe) -or -not (Test-Path -LiteralPath $phoneExe)) { throw 'Run build.ps1 build-transport first' }
$remote = '/data/local/tmp/vw-transport-' + $id
$server = $null
$mapping = $null
$mappingCreated = $false
$mappingRemoved = $false
$cleanup = $false
$completed = $false
try {
    Invoke-VwAdb -Device $device -Arguments @('shell','mkdir',$remote) -Phase 'transport-owned-directory' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('push',$phoneExe,($remote + '/bench')) -Phase 'transport-client-transfer' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('shell','chmod','700',($remote + '/bench')) -Phase 'transport-client-permissions' | Out-Null
    $folder = Join-Path $root 'server'
    $server = Start-Process -FilePath $hostExe -ArgumentList @('serve','tcp','127.0.0.1:0',('"'+$folder+'"'),'1200') -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $root 'server-stdout.log') -RedirectStandardError (Join-Path $root 'server-stderr.log')
    # Retain the process handle so ExitCode remains available after a quick exit.
    $null = $server.Handle
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath (Join-Path $folder 'ready.json'))) {
        if ($server.HasExited -or $watch.Elapsed.TotalSeconds -gt 10) { throw 'TCP server readiness failed' }
        Write-Host ('TCP server readiness: ' + [int]$watch.Elapsed.TotalSeconds + 's/10s')
        Start-Sleep -Milliseconds 250
    }
    $ready = Get-Content -Raw -LiteralPath (Join-Path $folder 'ready.json') | ConvertFrom-Json
    if ($ready.transport -ne 'tcp' -or $ready.port -lt 1 -or $ready.port -gt 65535) { throw 'Invalid server receipt' }
    $mapping = 'tcp:' + $ready.port
    Invoke-VwAdb -Device $device -Arguments @('reverse','--no-rebind',$mapping,$mapping) -Phase 'transport-owned-reverse' | Out-Null
    $mappingCreated = $true
    # A task-owned script records its shell PID, then exec replaces that process.
    # This allows exact path/PID cleanup if the host adb command times out.
    $script = Join-Path $root 'run-client.sh'
    $scriptText = '#!/system/bin/sh' + "`n" + 'echo $$ > ' + $remote + '/client.pid' + "`n" + 'exec ' + $remote + '/bench run tcp 127.0.0.1:' + $ready.port + ' - ' + $remote + '/client.json ' + $Profile + "`n"
    [IO.File]::WriteAllText($script, $scriptText, [Text.UTF8Encoding]::new($false))
    Invoke-VwAdb -Device $device -Arguments @('push',$script,($remote + '/run-client.sh')) -Phase 'transport-owned-client-script' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('shell','sh',($remote + '/run-client.sh')) -Phase 'transport-adb-benchmark' -TimeoutSeconds $TimeoutSeconds | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('pull',($remote + '/client.json'),(Join-Path $root 'client.json')) -Phase 'transport-result-collection' | Out-Null
    if (-not $server.WaitForExit(20000) -or $server.ExitCode -ne 0) { throw 'Server did not complete cleanly' }
    $report = Get-Content -Raw -LiteralPath (Join-Path $root 'client.json') | ConvertFrom-Json
    $expected = if ($Profile -eq 'full') { 1000 } else { 10 }
    $expectedBulk = if ($Profile -eq 'full') { 268435456 } else { 1048576 }
    if (-not $report.completed -or $report.transport -ne 'tcp' -or $report.profile -ne $Profile -or -not $report.stream.payloads_verified) { throw 'Incomplete or mismatched client report' }
    if (@($report.stream.echo).Count -ne 3 -or @($report.stream.echo | Where-Object { $_.rtt.count -ne $expected -or $_.raw_rtt_ms.Count -ne $expected }).Count -ne 0) { throw 'Echo sample census differs' }
    if (($report.stream.echo.bytes -join ',') -ne '64,4096,1048576' -or $report.stream.bulk_bytes -ne $expectedBulk -or $report.stream.bulk_elapsed_ms -le 0) { throw 'Payload sizes differ from the selected profile' }
    $serverReport = Get-Content -Raw -LiteralPath (Join-Path $folder 'complete.json') | ConvertFrom-Json
    if (-not $serverReport.completed -or $serverReport.transport -ne 'tcp') { throw 'Missing server completion acknowledgment' }
    $completed = $true
    Write-Host ('S23 adb TCP: verified; bulk ' + [math]::Round($report.stream.bulk_mib_per_second,2) + ' MiB/s')
} finally {
    if ($mappingCreated) {
        # --no-rebind above prevents taking another task's existing mapping.
        try { Invoke-VwAdb -Device $device -Arguments @('reverse','--remove',$mapping) -Phase 'transport-owned-reverse-cleanup' | Out-Null; $mappingRemoved = $true } catch { Write-Warning 'Owned reverse cleanup could not be confirmed' }
    }
    if ($server) {
        if (-not $server.HasExited) { $server.Kill(); [void]$server.WaitForExit(5000) }
        $server.Dispose()
    }
    if ($remote -match '^/data/local/tmp/vw-transport-[0-9a-f]{32}$') {
        # Never kill by process name or touch another task's Android directory.
        $cleanupCommand = Get-TransportCleanupCommand $remote 'client.pid'
        try { Invoke-VwAdb -Device $device -Arguments @('shell',$cleanupCommand) -Phase 'transport-owned-device-cleanup' -Capture | Out-Null; $cleanup = $true }
        catch { Write-Warning 'Owned Android benchmark cleanup could not be confirmed' }
    }
    [ordered]@{schema=1;device_model=$device.Model;carrier='adb_reverse_plain_tcp';profile=$Profile;completed=$completed;host_sha256=(Get-TransportHash $hostExe);android_sha256=(Get-TransportHash $phoneExe);owned_device_cleanup_confirmed=$cleanup;owned_reverse_removed=$mappingRemoved;screenshots_created=0} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'binding.json') -Encoding UTF8
    Write-Host ('Text evidence retained in .local/' + (Split-Path -Leaf $root))
}
if (-not $cleanup -or ($mappingCreated -and -not $mappingRemoved)) { throw 'Benchmark cleanup incomplete; acceptance remains open' }
