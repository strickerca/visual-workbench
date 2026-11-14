#requires -Version 7.2
[CmdletBinding()]
param([string]$Runner=(Join-Path $PSScriptRoot 'integration.ps1'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Pure fixture cases for the owner-physical journal pull and validator report.
# Adb and file leases are stubbed; no device, process or runner body executes.
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($Runner,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Runner parse failed'}
foreach($name in @('Hash-Bytes','Receive-M4PhysicalJournal','Save-M4PhysicalValidationReport')){
    $node=@($ast.FindAll({param($n)$n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $name},$true))
    if($node.Count -ne 1){throw 'Exact runner function missing'}
    . ([scriptblock]::Create($node[0].Extent.Text))
}
$count=0
function Check($Condition,[string]$Message){if(-not $Condition){throw $Message};$script:count++}
$runId='0123456789abcdef0123456789abcdef'
$packages=@('com.visualworkbench.android.hil','com.visualworkbench.android.hil.test')
$script:adbCalls=[Collections.Generic.List[object]]::new();$script:pinned=$null
$script:statLines=@();$script:pullLines=@()
function Adb([string]$name,[string[]]$arguments,[int]$seconds=30){
    $script:adbCalls.Add([pscustomobject]@{Name=$name;Arguments=$arguments;Seconds=$seconds})
    switch -CaseSensitive ($name){
        'stat-owned-physical-journal'{return [pscustomobject]@{ExitCode=0;Lines=$script:statLines}}
        'pull-owned-physical-journal'{return [pscustomobject]@{ExitCode=0;Lines=$script:pullLines}}
        'publish-physical-journal-pulled'{return [pscustomobject]@{ExitCode=0;Lines=@()}}
        default{throw 'Unexpected ADB phase'}
    }
}
function Pin-File([string]$path,[string]$hash){$script:pinned=[pscustomobject]@{Path=$path;Hash=$hash}}
function Reset-Case([byte[]]$Bytes,[string]$Claim){
    $script:owned=Join-Path ([IO.Path]::GetTempPath()) ('vw-physical-pull-test-'+[guid]::NewGuid().ToString('N'))
    [void][IO.Directory]::CreateDirectory($script:owned)
    $script:receipt=[ordered]@{physical_journal_sha256=$null;physical_journal_bytes=$null;physical_evidence=$null;physical_validation_reason=$null;physical_validation=$null}
    $script:physicalJournal=$null;$script:adbCalls.Clear();$script:pinned=$null
    $script:statLines=@([string]$Bytes.Length)
    $text=[Convert]::ToBase64String($Bytes)
    $script:pullLines=@(for($i=0;$i -lt $text.Length;$i+=76){$text.Substring($i,[Math]::Min(76,$text.Length-$i))})
    return [pscustomobject]@{remote_physical_journal=$Claim}
}
function Remove-Case{if(Test-Path -LiteralPath $script:owned){Remove-Item -LiteralPath $script:owned -Recurse -Force}}
function Refused([scriptblock]$Body){try{& $Body;return $false}catch{return $true}}
$bytes=[Text.Encoding]::UTF8.GetBytes('{"schema":1,"kind":"remote-physical-gesture-journal"}'+('x'*200))
$sha=[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
try{
    $record=Reset-Case $bytes ($sha+','+$bytes.Length)
    Receive-M4PhysicalJournal $record
    $local=Join-Path $owned 'physical-journal.json'
    Check ([IO.File]::ReadAllBytes($local).Length -eq $bytes.Length -and $physicalJournal.Sha256 -ceq $sha -and $physicalJournal.Bytes -eq $bytes.Length) 'Exact journal bytes not retained'
    Check ($receipt.physical_journal_sha256 -ceq $sha -and $receipt.physical_journal_bytes -eq $bytes.Length -and $pinned.Path -ceq $local -and $pinned.Hash -ceq $sha) 'Journal hash/lease not recorded'
    $names=@($adbCalls|ForEach-Object{$_.Name})
    Check (($names -join ',') -ceq 'stat-owned-physical-journal,pull-owned-physical-journal,publish-physical-journal-pulled') 'Pull must stat, pull, then confirm'
    $expected=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes((@('M4_PHYSICAL_JOURNAL_PULLED_V1',$runId,$sha) -join "`n")))
    $gate=$adbCalls[2].Arguments[1]
    Check ($gate.Contains($expected) -and $gate.Contains("run-as $($packages[0]) sh -c") -and $gate.Contains("no_backup/remote-integration-$runId/physical-journal-pulled") -and $adbCalls[1].Arguments[1] -ceq "run-as $($packages[0]) base64 no_backup/remote-integration-$runId/physical-journal.json") 'Exact owned run-as paths and hash gate required'
    Remove-Case
    $record=Reset-Case $bytes ($sha+','+$bytes.Length);$script:statLines=@([string]($bytes.Length+1))
    Check (Refused {Receive-M4PhysicalJournal $record}) 'Size mismatch admitted'
    Check ($adbCalls.Count -eq 1 -and $null -eq $physicalJournal) 'Mismatched size reached pull or gate'
    Remove-Case
    $record=Reset-Case $bytes (('0'*64)+','+$bytes.Length)
    Check (Refused {Receive-M4PhysicalJournal $record}) 'Hash mismatch admitted'
    Check (@($adbCalls|Where-Object{$_.Name -ceq 'publish-physical-journal-pulled'}).Count -eq 0 -and -not(Test-Path -LiteralPath (Join-Path $owned 'physical-journal.json'))) 'Mismatched hash was confirmed or retained'
    Remove-Case
    foreach($claim in @("$sha,6291457","$sha,0","$sha","$($sha.ToUpperInvariant()),10","$sha,10,1")){
        $record=Reset-Case $bytes $claim
        Check (Refused {Receive-M4PhysicalJournal $record}) ('Malformed or oversized claim admitted: '+$claim.Length)
        Remove-Case
    }
    $record=Reset-Case $bytes ($sha+','+$bytes.Length);$script:pullLines=@('not base64!')
    Check (Refused {Receive-M4PhysicalJournal $record}) 'Non-base64 transfer admitted'
    Remove-Case
    # Validator report projection.
    $record=Reset-Case $bytes ($sha+','+$bytes.Length);Receive-M4PhysicalJournal $record
    $passed=@{schema=1;operation='remote-physical-gesture';status='passed';rejection_reason='none';records_validated=$true;physical_owner_gesture=$true;evidence=@{schema=1;acceptance=$false};validation=@{physical_owner_gesture=$true;software_generated_input=$false;latency_acceptance=$false;journal_sha256=$sha;run_id=$runId}}
    $report=Save-M4PhysicalValidationReport ([pscustomobject]@{Lines=@(($passed|ConvertTo-Json -Depth 6 -Compress))})
    Check ($report.status -ceq 'passed' -and $receipt.physical_validation.journal_sha256 -ceq $sha -and $receipt.physical_validation_reason -ceq 'none') 'Passed report not retained'
    $forged=$passed.Clone();$forged.validation=@{physical_owner_gesture=$true;software_generated_input=$false;latency_acceptance=$false;journal_sha256=('c'*64);run_id=$runId}
    Check (Refused {Save-M4PhysicalValidationReport ([pscustomobject]@{Lines=@(($forged|ConvertTo-Json -Depth 6 -Compress))})}) 'Report for different journal admitted'
    $foreign=$passed.Clone();$foreign.validation=@{physical_owner_gesture=$true;software_generated_input=$false;latency_acceptance=$false;journal_sha256=$sha;run_id=('f'*32)}
    Check (Refused {Save-M4PhysicalValidationReport ([pscustomobject]@{Lines=@(($foreign|ConvertTo-Json -Depth 6 -Compress))})}) 'Report for a different run admitted'
    $rejected=@{schema=1;operation='remote-physical-gesture';status='rejected';rejection_reason='non_physical_origin';records_validated=$false;physical_owner_gesture=$false;evidence=@{schema=1;acceptance=$false}}
    $receipt.physical_validation=$null
    $report=Save-M4PhysicalValidationReport ([pscustomobject]@{Lines=@(($rejected|ConvertTo-Json -Depth 6 -Compress))})
    Check ($report.status -ceq 'rejected' -and $receipt.physical_validation_reason -ceq 'non_physical_origin' -and $null -eq $receipt.physical_validation) 'Rejected report gained validation'
    $bad=$rejected.Clone();$bad.evidence=@{schema=1;acceptance=$true}
    Check (Refused {Save-M4PhysicalValidationReport ([pscustomobject]@{Lines=@(($bad|ConvertTo-Json -Depth 6 -Compress))})}) 'Evidence claiming acceptance admitted'
    $bad=$rejected.Clone();$bad.rejection_reason='C:\private\path'
    Check (Refused {Save-M4PhysicalValidationReport ([pscustomobject]@{Lines=@(($bad|ConvertTo-Json -Depth 6 -Compress))})}) 'Open rejection text admitted'
}finally{if($null -ne (Get-Variable -Name owned -Scope Script -ErrorAction SilentlyContinue)){Remove-Case}}
if($count -ne 20){throw "Unexpected test census $count"}
Write-Output 'Physical journal pull/report pure cases: 20 passed; no process/device execution.'
