param([string]$Runner=(Join-Path $PSScriptRoot 'core-unit.ps1'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. $Runner
$cases=0
function Refuse([string[]]$Lines,[string[]]$Required=@()) {
    $refused=$false
    try { [void](Assert-VwCoreUnitResult -Lines $Lines -RequiredTests $Required) } catch { $refused=$true }
    if(-not $refused){throw 'Invalid test census accepted'}
    $script:cases++
}
$valid=@('running 2 tests','test session::one ... ok','test session::two ... ok','test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 144 filtered out; finished in 0.02s')
if((Assert-VwCoreUnitResult $valid @('session::two')) -ne 2){throw 'Valid test census refused'};$cases++
$single=@('running 1 test','test session::one ... ok','test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 145 filtered out; finished in 0.00s')
if((Assert-VwCoreUnitResult $single) -ne 1){throw 'Valid single test refused'};$cases++
Refuse @('running 0 tests','test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 146 filtered out; finished in 0.00s')
Refuse $valid @('session::missing')
Refuse @()
Refuse @('running 2 tests','test session::one ... ok',$valid[3])
Refuse @('running 2 tests','test session::one ... ok','test session::one ... ok',$valid[3])
Refuse @('running 3 tests',$valid[1],$valid[2],$valid[3])
Refuse @($valid + @($valid[3]))
Refuse @($valid | ForEach-Object {$_ -replace '0 ignored','1 ignored'})
Refuse @($valid | ForEach-Object {$_ -replace '0 failed','1 failed'})
Refuse @($valid | ForEach-Object {$_ -replace 'test result: ok','test result: FAILED'})
Write-Output "Core unit census regressions passed: $cases"
