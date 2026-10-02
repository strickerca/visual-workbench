Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $root 'tools/process.psm1')
$childScript = Join-Path $PSScriptRoot 'runner-child.ps1'
$sentinelArguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $childScript + '" -Mode sleep'
$sentinel = Start-Process powershell.exe -ArgumentList $sentinelArguments -WindowStyle Hidden -PassThru
$checks = [ordered]@{}
try {
    $baseArguments = @('-NoProfile','-ExecutionPolicy','Bypass','-File',$childScript,'-Mode')
    $ok = Invoke-VwProcess -FilePath powershell.exe -ArgumentList ($baseArguments + 'ok') -WorkingDirectory $root -Phase t003-runner-success -TimeoutSeconds 10 -Capture
    $checks.success = $ok.ExitCode -eq 0 -and $ok.Lines -contains 'synthetic-success'
    $timeout = Invoke-VwProcess -FilePath powershell.exe -ArgumentList ($baseArguments + 'sleep') -WorkingDirectory $root -Phase t003-runner-timeout -TimeoutSeconds 2 -Capture
    $checks.timeout = $timeout.ExitCode -eq 124
    # Enabling the compiler exception must not tolerate an ordinary survivor.
    $orphan = Invoke-VwProcess -FilePath powershell.exe -ArgumentList ($baseArguments + 'orphan') -WorkingDirectory $root -Phase t003-runner-survivor -TimeoutSeconds 10 -Capture -CleanCompilerTelemetry
    $checks.survivor_rejected = $orphan.ExitCode -eq 1
    $childPid = @($orphan.Lines | Where-Object { $_ -match '^owned-child-pid=(\d+)$' } | ForEach-Object { [int]$_.Split('=')[1] })
    $checks.child_cleanup = $childPid.Count -eq 1 -and -not (Get-Process -Id $childPid[0] -ErrorAction SilentlyContinue)
    $checks.unrelated_preserved = -not $sentinel.HasExited
    foreach ($run in @($ok,$timeout,$orphan)) {
        $receipt = Get-Content -LiteralPath ($run.LogPath + '.json') -Raw | ConvertFrom-Json
        if (-not $receipt.process_tree_cleanup_confirmed) { throw 'Runner cleanup unconfirmed' }
    }
    foreach ($value in $checks.Values) { if (-not $value) { throw 'Runner regression failed' } }
    $checks | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $root 'docs/evidence/T003_RUNNER_REGRESSIONS.json')
    Write-Output 'Five runner checks passed; all owned cleanup confirmed.'
} finally {
    if (-not $sentinel.HasExited) { $sentinel.Kill(); [void]$sentinel.WaitForExit(5000) }
    $sentinel.Dispose()
}
