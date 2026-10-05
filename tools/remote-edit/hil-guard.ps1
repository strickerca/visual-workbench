param(
    [Parameter(Mandatory=$true)][switch]$OwnerReady,
    [switch]$Unattended,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedHelperSha256,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedHarnessSha256
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if (-not $OwnerReady) { throw 'Explicit owner-ready desktop reservation is required for the owned HIL windows and input' }
$projectRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
$helper=Join-Path $projectRoot 'target/debug/vw-input-helper.exe'
$harness=Join-Path $projectRoot 'target/debug/vw-remote-guard-hil.exe'
foreach ($entry in @(@($helper,$ExpectedHelperSha256),@($harness,$ExpectedHarnessSha256))) {
    if (-not (Test-Path -LiteralPath $entry[0] -PathType Leaf) -or
        (Get-FileHash -LiteralPath $entry[0] -Algorithm SHA256).Hash.ToLowerInvariant() -cne $entry[1]) {
        throw 'Exact root-admitted build artifact hash does not match; no window or helper launched'
    }
}
$runRoot=Join-Path $projectRoot ('.local/remote-guard-' + [guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($runRoot)
$children=[Collections.Generic.List[Diagnostics.Process]]::new()
$windowJob=[VwProcessJobV4]::new()
$driver=$null
$cleanup=$false
$success=$false
function Start-OwnedSurface([string]$name,[int]$position) {
    $folder=Join-Path $runRoot $name
    # -OwnerReady explicitly requests these visible interactive test windows.
    # Each has a native300s deadline; no unrelated editor or input process is used.
    $child=Start-Process -FilePath $harness -ArgumentList @('--surface',('"'+$folder+'"'),"$position") -WorkingDirectory $projectRoot -WindowStyle Normal -PassThru -RedirectStandardOutput (Join-Path $runRoot "$name-stdout.log") -RedirectStandardError (Join-Path $runRoot "$name-stderr.log")
    $children.Add($child)
    $windowJob.Assign($child)
    $ready=Join-Path $folder 'target.json'
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while (-not (Test-Path -LiteralPath $ready -PathType Leaf)) {
        if ($child.HasExited) { throw 'Owned surface exited before native readiness' }
        if ($watch.Elapsed.TotalSeconds -ge 15) { throw 'Owned surface native readiness exceeded15s' }
        Write-Host "Owned surface $name readiness $([int]$watch.Elapsed.TotalSeconds)/15s"
        Start-Sleep -Milliseconds 250
    }
    $target=Get-Content -Raw -LiteralPath $ready | ConvertFrom-Json
    if ($target.process_id -ne $child.Id) { throw 'Native window receipt does not match the child actually started' }
    return $target
}
try {
    # Show TARGET last; activation alone never satisfies the balanced click gate.
    $sink=Start-OwnedSurface 'sink' 800
    $target=Start-OwnedSurface 'target' 80
    if($Unattended){Write-Host 'Unattended software-only readiness: one exact-target production-helper guarded click/release; actual NEW receiver pair/release/foreground still required. No physical-human provenance.'}else{Write-Host 'Click and release inside the empty TARGET window within 45s. A NEW balanced receiver mouse pair, released buttons and exact native foreground are required; automatic activation does not start input.'}
    Write-Host 'There are no buttons: click inside the whole TARGET window once. The owned test deliberately moves, resizes, minimizes and rapidly switches these two windows for 100 trials.'
    Write-Host 'After that initial click, leave both windows untouched until the runner stops. This is an instrumented guard test, not a drawing/feel test. Escape closes an owned window and stops acceptance.'
    $journal=Join-Path $runRoot 'cases.jsonl'
    $mode=if($Unattended){'--owner-unattended'}else{'--owner-ready'}
    $arguments=@($mode,$helper,$ExpectedHarnessSha256,$ExpectedHelperSha256,"$($target.window)","$($target.process_id)","$($sink.window)","$($sink.process_id)",$journal)
    $driver=Invoke-VwProcess -FilePath $harness -ArgumentList $arguments -WorkingDirectory $projectRoot -Phase 'remote-guard-production-helper' -TimeoutSeconds 285 -ParentExitGraceSeconds 5
    if ($driver.ExitCode -ne 0) { throw 'Production-helper guard HIL failed or is inconclusive; retain exact journals' }
    $cases=@(Get-Content -LiteralPath $journal | ForEach-Object { $_ | ConvertFrom-Json })
    $mutations=@($cases | Where-Object {$_.case -eq 'target_mutation_latched_fresh_grant'})
    if ($cases.Count -ne 103 -or $mutations.Count -ne 100) { throw 'Bounded native case census is incomplete' }
    foreach ($mode in 0..3) { if (@($mutations | Where-Object {$_.evidence.mode -eq $mode}).Count -ne 25) { throw 'Mutation-mode census is incomplete' } }
    $oldIds=@($mutations | ForEach-Object {$_.evidence.old_binding.input_session_id})
    $freshIds=@($mutations | ForEach-Object {$_.evidence.fresh_binding.input_session_id})
    if (@(($oldIds+$freshIds) | Sort-Object -Unique).Count -ne 200 -or
        @($mutations | ForEach-Object {$_.evidence.trial} | Sort-Object -Unique).Count -ne 100) { throw 'Trial/grant identity census is incomplete' }
    $success=$true
} finally {
    $cleanFailures=[Collections.Generic.List[string]]::new()
    foreach ($child in $children) {
        try {
            if (-not $child.HasExited) {
                [void]$child.CloseMainWindow()
                if (-not $child.WaitForExit(5000)) { $windowJob.Terminate(1); if (-not $child.WaitForExit(5000)) { throw 'Owned window did not retire after Job termination' } }
            }
        } catch { $cleanFailures.Add($_.Exception.Message) }
    }
    try {
        if ($windowJob.ActiveProcessCount -ne 0) {
            $windowJob.Terminate(1)
            $watch=[Diagnostics.Stopwatch]::StartNew()
            while ($windowJob.ActiveProcessCount -ne 0 -and $watch.Elapsed.TotalSeconds -lt 5) { Write-Host 'Observing actual owned surface Job retirement'; Start-Sleep -Milliseconds 100 }
        }
        $cleanup=$windowJob.ActiveProcessCount -eq 0 -and $cleanFailures.Count -eq 0
    } catch { $cleanFailures.Add($_.Exception.Message);$cleanup=$false }
    foreach ($child in $children) { $child.Dispose() }
    $windowJob.Dispose()
    [pscustomobject]@{schema=1;scoped_harness_passed=$success;cleanup_confirmed=$cleanup;helper_sha256=$ExpectedHelperSha256;harness_sha256=$ExpectedHarnessSha256;driver=$driver;cleanup_errors=@($cleanFailures);editor_effect=$false;physical_pen_fidelity=$false;latency_acceptance=$false;integrated_link_acceptance=$false;unattended=[bool]$Unattended;automation_input_requested=[bool]$Unattended;software_only_readiness=[bool]$Unattended;physical_mouse_provenance=$false;screenshots_created=0} | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'receipt.json') -Encoding utf8
    Write-Host ('Text evidence retained at .local/'+(Split-Path -Leaf $runRoot)+'; no screenshots created')
    if (-not $cleanup) { throw 'Actual task-owned Job retirement is unconfirmed; no HIL success may be claimed' }
}
