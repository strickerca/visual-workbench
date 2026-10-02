$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../android-device.psm1') -Force
$module = Get-Module android-device
$good = 'crash_recovery complete iterations=100 stages=[24, 22, 15, 21, 18] interrupted=61 committed=39 in_transaction=37 snapshot_runs=50 elapsed_ms=15000 children_reaped=100'
$result = & $module { param($line) Get-VwStoreCrashSummary -Lines @($line) } $good
if ($result.iterations -ne 100 -or $result.children_reaped -ne 100 -or $result.stages.Count -ne 5) { throw 'Valid numeric crash receipt was rejected' }
foreach ($bad in @('', $good.Replace('iterations=100','iterations=99'), $good.Replace('children_reaped=100','children_reaped=99'), $good.Replace('committed=39','committed=40'), $good.Replace('interrupted=61 committed=39','interrupted=100 committed=0'), $good.Replace('stages=[24','stages=[0'), $good.Replace('in_transaction=37','in_transaction=99'), $good.Replace('snapshot_runs=50','snapshot_runs=0'), ($good + "`n" + $good))) {
    $failed = $false
    try { & $module { param($line) Get-VwStoreCrashSummary -Lines @($line) } $bad | Out-Null } catch { $failed = $true }
    if (-not $failed) { throw 'Invalid or duplicate crash coverage receipt was accepted' }
}
Write-Host 'Storage crash receipt fixtures: PASS (1 valid, 9 invalid numeric summaries; no device access)'
