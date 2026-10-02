param([ValidateSet('smoke','full')][string]$Profile = 'full')
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
$projectRoot = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/process.psm1')
$buildCheck = Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/transport/build_receipt.py','check') -WorkingDirectory $projectRoot -Phase 'transport-build-binding' -TimeoutSeconds 30
if ($buildCheck.ExitCode -ne 0) { throw 'Transport gated-build binding failed' }
$device = Get-VwAndroidDevice -Root $projectRoot -ExpectedModel 'SM-S918U'
$id = [guid]::NewGuid().ToString('N')
$root = Join-Path $projectRoot ('.local/transport-wifi-' + $id)
[IO.Directory]::CreateDirectory($root) | Out-Null
$remote = '/data/local/tmp/vw-transport-' + $id
$hostExe = Join-Path $projectRoot 'target/release/transport-bench.exe'
$phoneExe = Join-Path $projectRoot 'target/aarch64-linux-android/release/transport-bench'
$cleanup = $false
$completed = $false
$frequency = $null
try {
    $address = Invoke-VwAdb -Device $device -Arguments @('shell','ip','-4','-o','addr','show','dev','wlan0') -Phase 'transport-phone-wifi-interface' -TimeoutSeconds 15 -Capture
    $matches = [regex]::Matches(($address.Lines -join "`n"), '\binet\s+(\d{1,3}(?:\.\d{1,3}){3})/')
    if ($matches.Count -ne 1) { throw 'Exactly one IPv4 address on the authorized phone Wi-Fi interface is required' }
    $peer = [Net.IPAddress]::Parse($matches[0].Groups[1].Value)
    $octets = $peer.GetAddressBytes()
    if (-not ($octets[0] -eq 10 -or ($octets[0] -eq 172 -and $octets[1] -ge 16 -and $octets[1] -le 31) -or ($octets[0] -eq 192 -and $octets[1] -eq 168))) { throw 'Wi-Fi benchmark is restricted to a private local address' }
    $wifi = Invoke-VwAdb -Device $device -Arguments @('shell','cmd','wifi','status') -Phase 'transport-phone-wifi-band' -TimeoutSeconds 15 -Capture
    $frequency = if (($wifi.Lines -join "`n") -match '(?i)frequency:\s*(\d+)') { [int]$Matches[1] } else { $null }
    Invoke-VwAdb -Device $device -Arguments @('shell','mkdir',$remote) -Phase 'wifi-owned-directory' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('push',$phoneExe,($remote+'/bench')) -Phase 'wifi-server-transfer' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('shell','chmod','700',($remote+'/bench')) -Phase 'wifi-server-permissions' | Out-Null
    $script = Join-Path $root 'server.sh'
    $contents = '#!/system/bin/sh' + "`n" + 'echo $$ > ' + $remote + '/server.pid' + "`n" + 'exec ' + $remote + '/bench serve quic ' + $peer.ToString() + ':0 ' + $remote + '/server 1200 --allow-lan' + "`n"
    [IO.File]::WriteAllText($script,$contents,[Text.UTF8Encoding]::new($false))
    Invoke-VwAdb -Device $device -Arguments @('push',$script,($remote+'/server.sh')) -Phase 'wifi-owned-server-script' | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('shell',('sh '+$remote+'/server.sh > '+$remote+'/server.log 2>&1 < /dev/null &')) -Phase 'wifi-server-start' -TimeoutSeconds 15 | Out-Null
    $ready = $null
    for ($attempt=1; $attempt -le 10; $attempt++) {
        try {
            $r = Invoke-VwAdb -Device $device -Arguments @('shell','cat',($remote+'/server/ready.json')) -Phase ('wifi-server-ready-'+$attempt) -TimeoutSeconds 5 -Capture
            $ready = ($r.Lines -join "`n") | ConvertFrom-Json
            break
        } catch { Write-Host "Wi-Fi server readiness $attempt/10"; Start-Sleep -Milliseconds 300 }
    }
    if (-not $ready -or $ready.transport -ne 'quic' -or $ready.port -lt 1 -or $ready.port -gt 65535) { throw 'Phone QUIC server failed readiness' }
    Invoke-VwAdb -Device $device -Arguments @('pull',($remote+'/server/server.der'),(Join-Path $root 'server.der')) -Phase 'wifi-public-certificate' | Out-Null
    $result = Invoke-VwProcess -FilePath $hostExe -ArgumentList @('run','quic',($peer.ToString()+':'+$ready.port),(Join-Path $root 'server.der'),(Join-Path $root 'client.json'),$Profile,'--allow-lan') -WorkingDirectory $projectRoot -Phase 'wifi-quic-measurement' -TimeoutSeconds 1210 -RedactValues @($peer.ToString(),$projectRoot,$env:USERPROFILE)
    if ($result.ExitCode -ne 0) { throw 'Wi-Fi QUIC incomplete; firewall or link interference is not ruled out' }
    Invoke-VwAdb -Device $device -Arguments @('pull',($remote+'/server/complete.json'),(Join-Path $root 'server-complete.json')) -Phase 'wifi-server-completion' | Out-Null
    $report = Get-Content -Raw -LiteralPath (Join-Path $root 'client.json') | ConvertFrom-Json
    $expected = if ($Profile -eq 'full') { 1000 } else { 10 }
    $expectedBulk = if ($Profile -eq 'full') { 268435456 } else { 1048576 }
    $expectedDatagrams = if ($Profile -eq 'full') { 1200 } else { 120 }
    if (-not $report.completed -or $report.transport -ne 'quic' -or $report.profile -ne $Profile -or -not $report.stream.payloads_verified) { throw 'Incomplete QUIC receipt' }
    if (($report.stream.echo.bytes -join ',') -ne '64,4096,1048576' -or @($report.stream.echo | Where-Object { $_.rtt.count -ne $expected -or $_.raw_rtt_ms.Count -ne $expected }).Count -ne 0) { throw 'Echo sample census differs' }
    if ($report.stream.bulk_bytes -ne $expectedBulk -or $report.stream.bulk_elapsed_ms -le 0 -or $report.datagrams.sent -ne $expectedDatagrams -or ($report.datagrams.received + $report.datagrams.missing_roundtrip_echoes) -ne $expectedDatagrams) { throw 'Incomplete bulk/datagram receipt' }
    $serverReport = Get-Content -Raw -LiteralPath (Join-Path $root 'server-complete.json') | ConvertFrom-Json
    if (-not $serverReport.completed -or $serverReport.transport -ne 'quic') { throw 'Missing server completion acknowledgment' }
    $completed = $true
    Write-Host ('S23 Wi-Fi QUIC (phone server): verified; bulk '+[math]::Round($report.stream.bulk_mib_per_second,2)+' MiB/s')
} finally {
    if ($remote -match '^/data/local/tmp/vw-transport-[0-9a-f]{32}$') {
        try {
            $command = Get-TransportCleanupCommand $remote 'server.pid'
            Invoke-VwAdb -Device $device -Arguments @('shell',$command) -Phase 'wifi-owned-device-cleanup' -TimeoutSeconds 15 -Capture | Out-Null
            $cleanup = $true
        } catch { Write-Warning 'Owned phone server cleanup not confirmed' }
    }
    # Contains a private link address; discard the task-owned generated shell file.
    if (Test-Path -LiteralPath (Join-Path $root 'server.sh')) { Remove-Item -LiteralPath (Join-Path $root 'server.sh') }
    [ordered]@{schema=1;device_model=$device.Model;carrier='wifi_quic';server='phone';client='windows';profile=$Profile;completed=$completed;phone_wifi_frequency_mhz=$frequency;host_inbound_firewall_tested=$false;firewall_rules_changed=$false;host_sha256=(Get-TransportHash $hostExe);android_sha256=(Get-TransportHash $phoneExe);owned_device_cleanup_confirmed=$cleanup} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'binding.json') -Encoding UTF8
    Write-Host ('Text evidence retained in .local/'+(Split-Path -Leaf $root))
}
if (-not $cleanup) { throw 'Owned server cleanup incomplete' }
