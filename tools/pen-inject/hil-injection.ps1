param(
    [Parameter(Mandatory = $true)][switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$Scenario = 'normal',
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
if (-not $OwnerReady) { throw 'Owner must reserve the desktop before input injection' }
$receipt = Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/pen-inject/build_receipt.py', 'check') -WorkingDirectory $projectRoot -Phase 'win-pen-build-receipt' -TimeoutSeconds 30
if ($receipt.ExitCode -ne 0) {
    $build = Invoke-VwProcess -FilePath powershell.exe -ArgumentList @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'build.ps1', 'build-pen-inject', '-TimeoutSeconds', "$TimeoutSeconds") -WorkingDirectory $projectRoot -Phase 'win-pen-build' -TimeoutSeconds ($TimeoutSeconds * 4 + 120) -ParentExitGraceSeconds 20
    if ($build.ExitCode -ne 0) { throw 'Windows pen probe build failed; no windows or input started' }
}
$runRoot = Join-Path $projectRoot ('.local/win-pen-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($runRoot) | Out-Null
Copy-Item -LiteralPath (Join-Path $projectRoot '.local/pen-inject-build.json') -Destination (Join-Path $runRoot 'build-receipt.json')
$harness = Join-Path $projectRoot 'target/debug/pen-harness.exe'
$injector = Join-Path $projectRoot 'target/debug/pen-inject.exe'
$children = [Collections.Generic.List[Diagnostics.Process]]::new()
function Start-Harness([string]$Name) {
    $folder = Join-Path $runRoot $Name
    # Start-Process joins ArgumentList; quote the already resolved path explicitly.
    # This is the visible test surface explicitly requested by -OwnerReady, not a
    # background helper. SW_HIDE would override the harness's first ShowWindow call.
    $child = Start-Process -FilePath $harness -ArgumentList @('--out', ('"' + $folder + '"'), '120') -WorkingDirectory $projectRoot -WindowStyle Normal -PassThru -RedirectStandardOutput (Join-Path $runRoot "$Name-stdout.log") -RedirectStandardError (Join-Path $runRoot "$Name-stderr.log")
    $children.Add($child)
    $ready = Join-Path $folder 'target.json'
    $watch = [Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $ready -PathType Leaf)) {
        if ($child.HasExited) { throw 'Harness exited before readiness; inspect retained local stderr' }
        if ($watch.Elapsed.TotalSeconds -gt 15) { throw 'Harness readiness timed out after 15s' }
        Write-Host "Waiting for $Name harness readiness ($([int]$watch.Elapsed.TotalSeconds)s/15s)"
        Start-Sleep -Milliseconds 500
    }
    $target = Get-Content -Raw -LiteralPath $ready | ConvertFrom-Json
    if ($target.pid -ne $child.Id) { throw 'Harness receipt does not identify the child we started' }
    return $target
}
function Close-Harnesses {
    foreach ($child in $children) {
        if (-not $child.HasExited) {
            $child.Refresh()
            [void]$child.CloseMainWindow()
            if (-not $child.WaitForExit(5000)) {
                # This executable creates no subprocesses; the outer bounded Job owns the full run.
                $child.Kill()
                [void]$child.WaitForExit(5000)
                throw 'Harness did not close cleanly; no recorder completion claim is allowed'
            }
        }
    }
}
try {
    $target = Start-Harness 'target'
    $commands = Join-Path $runRoot 'commands.csv'
    if ($Scenario -eq 'guards') {
        $sink = Start-Harness 'sink'
        $arguments = @('--guard-trials', "$($target.hwnd)", "$($target.pid)", "$($sink.hwnd)", "$($sink.pid)", '--owner-ready', $commands)
    } else {
        $arguments = @('--run', "$($target.hwnd)", "$($target.pid)", '--owner-ready', $commands, $Scenario)
    }
    $run = Invoke-VwProcess -FilePath $injector -ArgumentList $arguments -WorkingDirectory $projectRoot -Phase "win-pen-$Scenario" -TimeoutSeconds 90
    # Drain posted pointer messages before graceful recorder completion.
    Start-Sleep -Milliseconds 500
    Close-Harnesses
    if ($run.ExitCode -ne 0) { throw 'Injection run failed or was inconclusive; retained journals must not be marked passed' }
    if ($Scenario -eq 'guards') {
        $analysis = Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/pen-inject/guard_report.py', $runRoot) -WorkingDirectory $projectRoot -Phase 'win-pen-guard-analysis' -TimeoutSeconds 30
    } else {
        $analysis = Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/pen-inject/analyze.py', $commands, (Join-Path $runRoot 'target'), '--out', (Join-Path $runRoot 'comparison.json')) -WorkingDirectory $projectRoot -Phase 'win-pen-compare' -TimeoutSeconds 30
    }
    if ($analysis.ExitCode -ne 0) { throw 'Measurement did not meet acceptance (expected for no-refresh); inspect retained text report' }
    Write-Host "Windows pen $Scenario HIL passed its scoped checks; editor and elevated-window acceptance are separate."
} finally {
    # Iterate every child even when one cleanup fails. Never enumerate/kill unrelated apps.
    foreach ($child in $children) {
        try {
            if (-not $child.HasExited) {
                [void]$child.CloseMainWindow()
                if (-not $child.WaitForExit(5000)) { $child.Kill(); [void]$child.WaitForExit(5000) }
            }
        } finally { $child.Dispose() }
    }
    Write-Host ('Text evidence retained in .local/' + (Split-Path -Leaf $runRoot) + '; no screenshots created.')
}
