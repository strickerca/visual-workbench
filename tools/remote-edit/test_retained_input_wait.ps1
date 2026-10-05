#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'integration_input_owner.ps1'),[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Retained owner syntax errors'}
$definition=$ast.Find({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Wait-RetainedInputOwner'},$true)
# Exercise the real wait loop with deterministic owners and only a progress
# clock. No Job, process, input, native DLL or Android device is created.
. ([scriptblock]::Create($definition.Extent.Text))
$entry=[pscustomobject]@{}
function Test-M4InputChildSettled {param($Child) return $true}
$job=[pscustomobject]@{ActiveProcessCount=0}
$output=[pscustomobject]@{Lines=[Collections.Concurrent.ConcurrentQueue[string]]::new();IsComplete=$true}
$script:observations=0;$script:sleeps=0
function Test-RunnerSettlement {$script:observations++;return $script:observations -ge 4}
function Start-Sleep {param([int]$Milliseconds) $script:sleeps++;if($script:sleeps -gt 10){throw 'Test exceeded observation bound'}}
$late=[pscustomobject]@{Elapsed=[pscustomobject]@{TotalSeconds=901}}
Wait-RetainedInputOwner -ProgressClock $late
if($observations -ne 4 -or $sleeps -ne 3){throw 'Exited at old deadline or before actual terminal confirmation'}
Write-Output 'Retained input wait cases passed: 1 (unknown terminal proof through 901s progress clock)'
