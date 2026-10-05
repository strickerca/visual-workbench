Set-StrictMode -Version Latest
# These functions are shared by the actual inner and outer owners. The mock
# regressions call these same control paths, without starting a process or Job.
function Initialize-M4InputReaders {
    if(-not ('VwInputOutputR57' -as [type])){Add-Type -Path (Join-Path $PSScriptRoot 'input_process_output.cs')}
}
function New-M4InputChild($Name,$Process,$Job,$Output){
    return [pscustomobject]@{Name=$Name;Process=$Process;Job=$Job;Output=$Output;Started='No';Assigned=$false;Assignment='No';OutputStarted=$false;ErrorStarted=$false;GateState='Absent'}
}
function Start-M4InputChild($Entry){
    if($Entry.Process -is [Diagnostics.Process]){
        # Refusal before invoking Start is positively input-free. A failure
        # after entering Start is different: .NET can create the child before
        # its redirected readers and associated Process identity are published.
        $info=$Entry.Process.StartInfo
        if($info.UseShellExecute -or [string]::IsNullOrWhiteSpace($info.FileName) -or -not [IO.Path]::IsPathFullyQualified($info.FileName) -or -not [IO.File]::Exists($info.FileName)){
            throw 'Owned process executable validation failed before Start.'
        }
        if(-not [string]::IsNullOrEmpty($info.WorkingDirectory) -and -not [IO.Directory]::Exists($info.WorkingDirectory)){
            throw 'Owned process working directory validation failed before Start.'
        }
    }
    $Entry.Started='Unknown'
    try{
        if(-not $Entry.Process.Start()){$Entry.Started='No';throw 'Owned process did not start.'}
        $Entry.Started='Yes'
    }catch{
        if($Entry.Started -ceq 'Unknown'){
            try{if($Entry.Process.Id -gt 0){$Entry.Started='Yes'}}
            catch{} # No associated Id does not prove native creation failed.
        }
        throw
    }
    $Entry.Assignment='Unknown'
    $Entry.Job.Assign($Entry.Process);$Entry.Assigned=$true;$Entry.Assignment='Yes'
    $Entry.Output.StartOutput($Entry.Process);$Entry.OutputStarted=$true
    $Entry.Output.StartError($Entry.Process);$Entry.ErrorStarted=$true
    if(@($Entry.Job.ActiveProcessIds) -notcontains $Entry.Process.Id){throw 'Actual owned Job membership missing.'}
}
function Test-M4InputChildExited($Entry){
    if($Entry.Started -ceq 'No'){return $true}
    if($Entry.Started -cne 'Yes'){return $false}
    try{return $Entry.Process.HasExited}catch{return $false}
}
function Test-M4InputChildSettled($Entry){
    if(-not(Test-M4InputChildExited $Entry)){return $false}
    try{
        # Even failed Assign may have assigned the process. The actual Job must
        # be empty; every reader Task that exists must have actually completed.
        return $Entry.Job.ActiveProcessCount -eq 0 -and $Entry.Output.IsComplete -and $Entry.Output.Lines.IsEmpty
    }catch{return $false}
}
function Stop-M4InputPregrantChild($Entry){
    if($Entry.GateState -cne 'Absent'){throw 'Possible grant owner cannot be terminated.'}
    if($Entry.Started -ceq 'No'){return}
    if($Entry.Started -cne 'Yes'){return} # Retain unknown Start outcome.
    if(-not(Test-M4InputChildExited $Entry)){$Entry.Process.Kill($true)}
    # Pre-grant descendants are input-free, including ambiguous Assign results.
    if($Entry.Job.ActiveProcessCount -gt 0){$Entry.Job.Terminate(1)}
}
function Wait-M4InputChildSettlement($Entry,[scriptblock]$Drain={param($Child)},$ProgressClock=[Diagnostics.Stopwatch]::StartNew()){
    $next=0
    while($true){
        & $Drain $Entry
        if(Test-M4InputChildSettled $Entry){return}
        if($ProgressClock.Elapsed.TotalSeconds -ge $next){try{Write-Host 'Retaining actual inputful Job/IO until exit and joins'}catch{};$next+=2}
        Start-Sleep -Milliseconds 20
    }
}
function Complete-M4InputOuterOwner($Entry,[scriptblock]$Wait){
    while($true){
        try{
            if($Entry.GateState -ceq 'Absent'){Stop-M4InputPregrantChild $Entry}
            & $Wait
            return
        }catch{
            # Cleanup errors cannot unwind and close a Job with unknown owners.
            try{Write-Host 'Retained input owner alive; awaiting actual child Job and output settlement'}catch{}
            Start-Sleep -Milliseconds 2000
        }
    }
}
function Publish-M4InputGate($Entry,[string]$Path,[byte[]]$Bytes,[scriptblock]$WriteStaged={param($Pending,$Content)
    $file=[IO.File]::Open($Pending,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$file.Write($Content);$file.Flush($true)}finally{$file.Dispose()}
},[scriptblock]$MoveStaged={param($Pending,$Final) [IO.File]::Move($Pending,$Final)}){
    if($Entry.Started -cne 'Yes' -or -not $Entry.Assigned -or -not $Entry.OutputStarted -or -not $Entry.ErrorStarted){throw 'Complete owner startup is required before gate publication.'}
    # Only the final name is consumed by the child. Partial writes stay private.
    $pending=$Path+'.pending'
    & $WriteStaged $pending $Bytes
    $Entry.GateState='Unknown'
    try{& $MoveStaged $pending $Path;$Entry.GateState='Published'}
    catch{
        try{[void][IO.File]::GetAttributes($Path)}
        catch{
            $errorType=$_.Exception.GetBaseException()
            if($errorType -is [IO.FileNotFoundException] -or $errorType -is [IO.DirectoryNotFoundException]){$Entry.GateState='Absent'}
        } # An ambiguous final name might already have enabled the child.
        throw
    }
}
