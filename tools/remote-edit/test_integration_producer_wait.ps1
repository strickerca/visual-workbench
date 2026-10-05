#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
function Assert($Value,[string]$Reason){if(-not $Value){throw $Reason}}
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'integration.ps1'),[ref]$tokens,[ref]$errors)
Assert ($errors.Count -eq 0) 'Integration syntax errors'
$definition=$ast.Find({param($n)$n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Await-File'},$true)
Assert ($null -ne $definition) 'Actual producer wait missing'
. ([scriptblock]::Create($definition.Extent.Text))
# Exercise the production wait's control flow with no process, device, file,
# sleeping or input. Publication occurs through the actual drain call path.
function Test-Path {param($LiteralPath,$PathType) return $script:ready}
function Assert-Plain($Path){}
function Start-Sleep {param($Milliseconds) $script:polls++;Assert ($script:polls -lt 10) 'Wait made no bounded progress'}
function Drain-IntegrationDiagnostics($Child){
 $script:drains++
 if($script:publishAt -gt 0 -and $script:drains -ge $script:publishAt){$script:ready=$true}
}
function Setup {
 $script:ready=$false;$script:drains=0;$script:polls=0;$script:publishAt=0
 $script:owned='C:\owned-fixture';$script:phase='before'
 $script:phone=[pscustomobject]@{Name='phone';Process=[pscustomobject]@{HasExited=$true}}
 $script:producer=[pscustomobject]@{Name='host';Process=[pscustomobject]@{HasExited=$false}}
 $script:children=@($script:phone,$script:producer)
}
$cases=0
Setup;$publishAt=4;$path=Await-File 'host-receipt.txt' 15 $producer
Assert ($path -ceq 'C:\owned-fixture\host-receipt.txt' -and $drains -eq 4) 'Expected phone exit refused pending host receipt';$cases++
Setup;$producer.Process.HasExited=$true;$publishAt=2
[void](Await-File 'host-receipt.txt' 15 $producer)
Assert $ready 'Publication concurrent with producer exit was refused';$cases++
Setup;$producer.Process.HasExited=$true;$refused=$false
try{[void](Await-File 'offer.txt' 30 $producer)}catch{$refused=$_.Exception.Message -ceq 'Owned child exited before expected readiness.'}
Assert $refused 'Actual producer exit without file not refused';$cases++
Setup;$refused=$false
try{[void](Await-File 'offer.txt' 0 $producer)}catch{$refused=$_.Exception.Message -ceq 'Owned native/session readiness timed out.'}
Assert ($refused -and $drains -eq 0) 'Absolute readiness deadline weakened';$cases++
Setup;$ready=$true;$producer.Process.HasExited=$true
[void](Await-File 'host-receipt.txt' 15 $producer)
Assert ($drains -eq 0) 'Already-published terminal file refused';$cases++
Write-Output "Integration producer wait cases passed: $cases"
