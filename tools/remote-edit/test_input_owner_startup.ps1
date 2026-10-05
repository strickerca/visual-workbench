#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'input_owner_startup.ps1')
. (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
# CLR properties throw through the same .NET adapter as Diagnostics.Process.
# PowerShell ScriptProperty getters can swallow their inner exception and yield
# null; that is intentionally Unknown, not proof of a never-started process.
Add-Type -TypeDefinition @'
using System;
using System.Diagnostics;
public sealed class M4StartupProcessMockR57b {
    public string Failure; public bool DidStart, Exited; public int Kills, ExitQueries;
    public bool Start() { if(Failure=="start") return false; if(Failure=="post-create")throw new System.IO.IOException("redirected stream construction failed after native creation");DidStart=true; return true; }
    public int Id { get { if(!DidStart)throw new InvalidOperationException("no associated process");return 42; } }
    public bool HasExited { get { ExitQueries++; if(!DidStart)throw new InvalidOperationException("unstarted query");return Exited; } }
    public void Kill(bool tree) { Kills++; }
}
'@
$script:cases=0;$script:sleeps=0
function Assert($Value,[string]$Message){if(-not $Value){throw $Message}}
function Start-Sleep {param([int]$Milliseconds) $script:sleeps++;if($script:sleeps -gt 50){throw 'Mock observation bound exceeded'}}
function New-MockChild([string]$Failure){
    $p=[M4StartupProcessMockR57b]::new();$p.Failure=$Failure
    $j=[pscustomobject]@{Failure=$Failure;ActiveProcessCount=0;ActiveProcessIds=@();Terminates=0}
    $j|Add-Member ScriptMethod Assign {param($Process) if($this.Failure -ceq 'assign'){throw 'mock assign failed'};$this.ActiveProcessCount=1;$this.ActiveProcessIds=@(42)}
    $j|Add-Member ScriptMethod Terminate {param($Code) $this.Terminates++}
    $o=[pscustomobject]@{Failure=$Failure;Out=$false;Err=$false;Joined=$false;Lines=[Collections.Concurrent.ConcurrentQueue[string]]::new()}
    $o|Add-Member ScriptMethod StartOutput {param($Process) if($this.Failure -ceq 'stdout'){throw 'mock stdout failed'};$this.Out=$true}
    $o|Add-Member ScriptMethod StartError {param($Process) if($this.Failure -ceq 'stderr'){throw 'mock stderr failed'};$this.Err=$true}
    $o|Add-Member ScriptProperty IsComplete {return (-not $this.Out -and -not $this.Err) -or $this.Joined}
    return New-M4InputChild 'mock' $p $j $o
}
# Execute production Start -> catch -> pregrant stop -> actual settlement for
# each failed stage. Kill requests alone do not flip exit or reader completion.
foreach($failure in @('start','assign','stdout','stderr')){
    $child=New-MockChild $failure;$failed=$false
    try{Start-M4InputChild $child}catch{$failed=$true}
    Assert $failed ('Missing injected failure: '+$failure)
    Assert ($child.GateState -ceq 'Absent') 'Failed startup published a grant gate'
    Stop-M4InputPregrantChild $child
    if($failure -ceq 'start'){
        Assert (Test-M4InputChildSettled $child) 'Proven unstarted child cannot settle'
        Assert ($child.Process.ExitQueries -eq 0 -and $child.Process.Kills -eq 0) 'Unstarted child queried or killed'
    }else{
        Assert (-not(Test-M4InputChildSettled $child)) 'Kill request fabricated actual exit'
        $script:joins=0
        Wait-M4InputChildSettlement $child {param($c)
            $script:joins++
            if($script:joins -ge 3){$c.Process.Exited=$true}
            if($script:joins -ge 4){$c.Job.ActiveProcessCount=0}
            if($script:joins -ge 5){$c.Output.Joined=$true}
        } ([pscustomobject]@{Elapsed=[pscustomobject]@{TotalSeconds=901}})
        $expected=if($child.Output.Out -or $child.Output.Err){5}else{if($failure -ceq 'assign'){3}else{4}}
        Assert ($script:joins -eq $expected) 'Settlement ignored an actual owner or waited for nonexistent reader EOF'
    }
    $script:cases++
}
# Partial staging failure never exposes the final gate or enables a grant.
$child=New-MockChild '';Start-M4InputChild $child
$script:moved=$false;$failed=$false
try{Publish-M4InputGate $child 'unused-gate' ([byte[]](1,2)) {param($p,$b) throw 'partial staged write'} {param($p,$f) $script:moved=$true}}catch{$failed=$true}
Assert ($failed -and -not $moved -and $child.GateState -ceq 'Absent') 'Partial gate became visible'
Stop-M4InputPregrantChild $child
Assert ($child.Process.Kills -eq 1) 'Partial staging lost safe pregrant abort'
$script:cases++
# Complete publication requires all startup stages, then switches to retaining
# input-capable ownership; no terminating cleanup is reachable afterward.
$child=New-MockChild '';Start-M4InputChild $child
Publish-M4InputGate $child 'unused-gate' ([byte[]](1,2)) {param($p,$b)} {param($p,$f)}
Assert ($child.GateState -ceq 'Published') 'Complete gate not recorded'
$script:waits=0
Complete-M4InputOuterOwner $child {$script:waits++;if($script:waits -lt 3){throw 'mock pending observation error'}}
Assert ($script:waits -eq 3 -and $child.Process.Kills -eq 0 -and $child.Job.Terminates -eq 0) 'Published owner killed or abandoned on observation failure'
$script:cases++
# Exercise the actual outer wait loop when the inner rejects a published gate:
# process exit and empty Job are insufficient until its correlated cleanup
# receipt confirms no native/phone owners and completed cleanup.
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'integration_input_owner.ps1'),[ref]$tokens,[ref]$errors)
Assert ($errors.Count -eq 0) 'Outer syntax errors'
$definition=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Wait-RetainedInputOwner'},$true)
. ([scriptblock]::Create($definition.Extent.Text))
$entry=New-MockChild '';Start-M4InputChild $entry;$entry.GateState='Published'
$entry.Process.Exited=$true;$entry.Job.ActiveProcessCount=0;$entry.Output.Joined=$true
$output=$entry.Output;$script:receiptChecks=0;$nonce='0123456789abcdef0123456789abcdef'
function Test-RunnerSettlement {
    $script:receiptChecks++
    $receipt=[pscustomobject]@{run_id=$nonce;retained_input_nonce=$nonce;controller_input_requested=$true;input_retirement_confirmed=($script:receiptChecks -ge 4);host_actual_retirement=$true;phone_actual_retirement=$true;process_cleanup=$true}
    return Test-M4InputRunnerRetired $receipt $nonce
}
Complete-M4InputOuterOwner $entry {Wait-RetainedInputOwner ([pscustomobject]@{Elapsed=[pscustomobject]@{TotalSeconds=901}})}
Assert ($script:receiptChecks -eq 4 -and $entry.Process.Kills -eq 0) 'Rejected inner gate bypassed actual retirement receipt'
$script:cases++
# Actual Diagnostics.Process with an empty FileName is refused before calling
# Start. This is the positive no-creation proof; absent Id alone is insufficient.
$child=New-MockChild '';$child.Process=[Diagnostics.Process]::new();$child.Process.StartInfo.UseShellExecute=$false;$failed=$false
try{Start-M4InputChild $child}catch{$failed=$true}
Assert ($failed -and $child.Started -ceq 'No') 'Known pre-Start validation failure not classified'
Assert (Test-M4InputChildSettled $child) 'Actual never-associated process failed terminal cleanup'
$child.Process.Dispose();$script:cases++
# A missing identity without the precise .NET exception proves nothing. The
# original ScriptProperty failure shape must remain retained, never aborted.
$child=New-MockChild ''
$ambiguous=[pscustomobject]@{Id=$null;Kills=0}
$ambiguous|Add-Member ScriptMethod Start {throw 'unknown start outcome'}
$ambiguous|Add-Member ScriptMethod Kill {param($Tree) $this.Kills++}
$child.Process=$ambiguous;$failed=$false
try{Start-M4InputChild $child}catch{$failed=$true}
Stop-M4InputPregrantChild $child
Assert ($failed -and $child.Started -ceq 'Unknown' -and $ambiguous.Kills -eq 0 -and -not(Test-M4InputChildSettled $child)) 'Ambiguous Start was treated as proven unstarted'
$script:cases++
# A CLR Id getter throwing no-associated-process after an arbitrary Start
# exception cannot prove no child: .NET may have created it before stream setup.
$child=New-MockChild 'post-create';$failed=$false
try{Start-M4InputChild $child}catch{$failed=$true}
Stop-M4InputPregrantChild $child
Assert ($failed -and $child.Started -ceq 'Unknown' -and $child.Process.Kills -eq 0 -and -not(Test-M4InputChildSettled $child)) 'Post-create failure fabricated absent owner'
$script:cases++
Write-Output "Input owner startup control-flow cases passed: $cases"
