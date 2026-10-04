#requires -Version 7.2
[CmdletBinding()]
param(
    [string]$ProjectRoot=(Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [ValidateRange(15,300)][int]$TimeoutSeconds=90,
    [ValidateSet('startup','mcp')][string]$Mode='startup',
    [switch]$Execute
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $Execute){throw 'Source-only by default. The central validation owner must pass -Execute.'}
if(-not $IsWindows){throw 'Desktop startup verification requires Windows.'}
$ProjectRoot=[IO.Path]::GetFullPath($ProjectRoot).TrimEnd('\')
Import-Module (Join-Path $ProjectRoot 'tools/process.psm1')
Import-Module (Join-Path $PSScriptRoot 'lease.psm1')
$parser=Join-Path $ProjectRoot 'tools/desktop-test/reports.py'
$runId=[Guid]::NewGuid().ToString('N')
$parent=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$private=Join-Path $parent ('VisualWorkbench-desktop-startup-'+$runId)
$reportDirectory=Join-Path $parent 'VisualWorkbench-desktop-text-receipts'
$receiptPath=Join-Path $reportDirectory ($runId+'.json')
$created=$false;$lease=$null;$phase='setup';$passed=$false;$treesClean=$true;$privateClean=$false
$receipt=[ordered]@{schema=1;kind='desktop-startup-run';run_id=$runId;status='failed';failure_phase='setup';
    failure_code=$null;assertions=$null;process_runs=@();cleanup=$null;
    ordinary_preferences_and_native_cache_may_initialize=$true;project_actions_requested=$false;
    firewall_actions_requested=$false;screenshots_created=0;visual_acceptance=$false;release_acceptance=$false;mode=$Mode}
function Assert-Plain([string]$Path,[bool]$Directory){
    $item=Get-Item -LiteralPath ([IO.Path]::GetFullPath($Path)) -Force
    if($item.PSIsContainer -ne $Directory){throw 'Path type refused.'}
    $cursor=$item
    while($null -ne $cursor){
        if(($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Redirected path refused.'}
        $cursor=if($cursor -is [IO.DirectoryInfo]){$cursor.Parent}else{$cursor.Directory}
    }
}
function Save-Private([string]$Name,[string]$Text){
    if($Name -notin @('.owner-v1','startup.txt','process.json')){throw 'Private name refused.'}
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes($Text)
    if($bytes.Length -gt 1MB){throw 'Private text limit refused.'}
    $stream=[IO.File]::Open((Join-Path $private $Name),[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$stream.Write($bytes);$stream.Flush($true)}finally{$stream.Dispose()}
}
function Run-Owned([string]$Name,[string]$Executable,[string[]]$Arguments,[int]$Seconds){
    $script:phase=$Name
    try{
        if($Name -eq 'desktop-startup'){
            # Only the fixed wrapper's short status words reach the outer helper.
            # Application stdout/stderr never enters its unbounded line reader.
            $expectedLauncher=Join-Path $ProjectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev/VisualWorkbenchDev.exe'
            $expectedArgument=if($Mode -eq 'mcp'){'--mcp-runtime-smoke'}else{'--startup-smoke'}
            if($Executable -cne $expectedLauncher -or $Arguments.Count -ne 1 -or $Arguments[0] -cne $expectedArgument){throw 'Packaged invocation refused.'}
            $Executable=Join-Path $PSHOME 'pwsh.exe'
            $Arguments=@('-NoProfile','-NonInteractive','-File',(Join-Path $PSScriptRoot 'capture.ps1'),'-ProjectRoot',$ProjectRoot,'-PrivateDirectory',$private,'-RunId',$runId,'-TimeoutSeconds',[string]$Seconds,'-Mode',$Mode)
            $Seconds+=30
        }
        $result=Invoke-VwProcess -FilePath $Executable -ArgumentList $Arguments -WorkingDirectory $ProjectRoot -Phase $Name -TimeoutSeconds $Seconds -ParentExitGraceSeconds 10 -Capture
    }
    catch{$script:treesClean=$false;throw}
    try{$proof=Get-Content -Raw -LiteralPath ($result.LogPath+'.json') | ConvertFrom-Json -AsHashtable}
    catch{$script:treesClean=$false;throw}
    $clean=$proof.contained_in_windows_job -eq $true -and $proof.process_tree_cleanup_confirmed -eq $true -and $proof.output_streams_completed -eq $true
    $script:treesClean=$script:treesClean -and $clean
    $script:receipt.process_runs+=@{phase=$Name;exit_code=$result.ExitCode;elapsed_seconds=$proof.elapsed_seconds;process_tree_cleanup_confirmed=$clean}
    if($Name -eq 'desktop-startup'){
        # The bounded byte reader exclusively wrote these private files. Bind
        # both the inner packaged child and outer wrapper cleanup before parsing.
        $innerPath=Join-Path $private 'process.json'
        if((Get-Item -LiteralPath $innerPath).Length -gt 8192){throw 'Capture receipt limit refused.'}
        $inner=Get-Content -LiteralPath $innerPath -Raw | ConvertFrom-Json -AsHashtable
        $innerClean=$inner.contained_in_windows_job -eq $true -and $inner.process_tree_cleanup_confirmed -eq $true -and $inner.output_streams_completed -eq $true
        $script:treesClean=$script:treesClean -and $innerClean
        $script:receipt.process_runs+=@{phase='packaged-child';exit_code=$inner.exit_code;elapsed_seconds=$inner.elapsed_seconds;process_tree_cleanup_confirmed=$innerClean;
            capture_byte_limit=$inner.capture_byte_limit;captured_bytes=$inner.captured_bytes;capture_overflow=$inner.capture_overflow}
        if($null -ne $inner.failure_reason){$script:receipt.failure_code=$inner.failure_reason}
        if(-not $innerClean -or $inner.exit_code -ne 0 -or $inner.capture_overflow){throw 'Packaged child refused.'}
    }
    if($result.ExitCode -ne 0 -or -not $clean){
        foreach($line in $result.Lines){if($line -cmatch '^DESKTOP_REPORT_REJECTED:([a-z_]{1,80})$'){$script:receipt.failure_code=$Matches[1]}}
        throw 'Bounded phase or process cleanup failed.'
    }
    return $result
}
function Parse([string]$Name,[string[]]$Arguments){
    Run-Owned $Name 'python.exe' (@($parser)+$Arguments+@('--root',$ProjectRoot)) 180 | Out-Null
}
try{
    Assert-Plain $ProjectRoot $true;Assert-Plain $parent $true
    # New-Item must succeed exclusively. Never claim/delete an existing path.
    New-Item -ItemType Directory -Path $private -ErrorAction Stop | Out-Null
    $created=$true
    Save-Private '.owner-v1' ("VW desktop startup v1`n"+$runId+"`n")
    Parse 'desktop-source-check' @('check','--output',(Join-Path $private 'before.json'))
    $before=Get-Content -LiteralPath (Join-Path $private 'before.json') -Raw | ConvertFrom-Json -AsHashtable
    if($before.distribution.relative_directory -cne 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev' -or $before.distribution.launcher -cne 'VisualWorkbenchDev.exe'){throw 'Launcher ownership refused.'}
    $distribution=Join-Path $ProjectRoot $before.distribution.relative_directory
    $phase='desktop-distribution-lease'
    $lease=Lock-VwDesktopDistribution -Directory $distribution -Inventory $before.distribution.files
    # Check again only AFTER every existing runtime file is pinned. A concurrent
    # build either fails the lease or changes this hash check, before launch.
    Parse 'desktop-pinned-check' @('check','--output',(Join-Path $private 'pinned.json'))
    $pinned=Get-Content -LiteralPath (Join-Path $private 'pinned.json') -Raw | ConvertFrom-Json -AsHashtable
    if($before.nonce -cne $pinned.nonce -or
        (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $private 'before.json')).Hash -cne
        (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $private 'pinned.json')).Hash){throw 'Build changed before launch.'}
    $launchArgument=if($Mode -eq 'mcp'){'--mcp-runtime-smoke'}else{'--startup-smoke'}
    Run-Owned 'desktop-startup' (Join-Path $distribution 'VisualWorkbenchDev.exe') @($launchArgument) $TimeoutSeconds | Out-Null
    Parse 'desktop-collect' @('collect','--mode',$Mode,'--log',(Join-Path $private 'startup.txt'),'--process',(Join-Path $private 'process.json'),'--output',(Join-Path $private 'report.json'))
    $receipt.assertions=Get-Content -LiteralPath (Join-Path $private 'report.json') -Raw | ConvertFrom-Json -AsHashtable
    $passed=$true
}catch{
    $receipt.failure_phase=$phase
    # Never persist raw OS/Java exceptions or arbitrary application stdout.
    Write-Host "[desktop-startup] FAILED phase=$phase"
}finally{
    if($null -ne $lease){$lease.Dispose();$lease=$null}
    if($created){
        try{
            Assert-Plain $private $true
            if([IO.Path]::GetFullPath($private) -cne (Join-Path $parent ('VisualWorkbench-desktop-startup-'+$runId))){throw 'Cleanup containment refused.'}
            $owner=Join-Path $private '.owner-v1';Assert-Plain $owner $false
            if((Get-Item -LiteralPath $owner).Length -gt 128 -or [IO.File]::ReadAllText($owner) -cne ("VW desktop startup v1`n"+$runId+"`n")){throw 'Cleanup owner refused.'}
            $allowed=@('.owner-v1','before.json','pinned.json','startup.txt','process.json','report.json')
            $children=@(Get-ChildItem -LiteralPath $private -Force)
            foreach($child in $children){if($child.Name -cnotin $allowed){throw 'Unexpected private child preserved.'};Assert-Plain $child.FullName $false}
            foreach($child in $children){Remove-Item -LiteralPath $child.FullName -Force}
            Remove-Item -LiteralPath $private # Nonrecursive; never removes a new child.
            $privateClean=$true
        }catch{Write-Host '[desktop-startup] Private cleanup not confirmed; preserved uncertain paths.'}
    }else{$privateClean=$true}
    $receipt.cleanup=@{owned_process_trees=$treesClean;private_raw_text_disposed=$privateClean;runtime_leases_released=($null -eq $lease)}
    if($passed -and $treesClean -and $privateClean){$receipt.status='passed';$receipt.failure_phase=$null}
    if(-not (Test-Path -LiteralPath $reportDirectory)){New-Item -ItemType Directory -Path $reportDirectory -ErrorAction Stop | Out-Null}
    Assert-Plain $reportDirectory $true
    $output=[IO.File]::Open($receiptPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$bytes=[Text.UTF8Encoding]::new($false).GetBytes(($receipt | ConvertTo-Json -Depth 20));$output.Write($bytes);$output.Flush($true)}finally{$output.Dispose()}
    Write-Host "[desktop-startup] receipt=$runId.json status=$($receipt.status)"
}
if($receipt.status -ne 'passed'){exit 1}
