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
$probe='giant_images::producer::picked::tests::quarantine_probe'
$wrapper='giant_images::producer::picked::tests::cleanup_failure_retains_the_real_admission_in_an_isolated_process'
$reason='Executed by the bounded quarantine wrapper; do not leak this process-global permit into unrelated tests'
$ignored='test '+$probe+' ... ignored, '+$reason
$wrapperOk='test '+$wrapper+' ... ok'
$header='---- '+$wrapper+' stdout ----'
$marker='quarantine probe executed: 1 passed; same4MiB retained and third input refused'
$end='test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 143 filtered out; finished in 0.02s'
$quarantine=@('running 3 tests',$valid[1],$wrapperOk,$ignored,'','successes:','',$header,$marker,'','successes:',('    '+$wrapper),'',$end)
if((Assert-VwCoreUnitResult $quarantine @('session::one',$wrapper)) -ne 2){throw 'Exact wrapper-accounted quarantine census refused'};$cases++
# A filtered run without the probe stays zero-ignored. No child proof is inferred
# or needed when there is no ignored case to account for.
$wrapperOnly=@('running 1 test',$wrapperOk,'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 145 filtered out; finished in 0.00s')
if((Assert-VwCoreUnitResult $wrapperOnly @($wrapper)) -ne 1){throw 'Zero-ignored wrapper filter refused'};$cases++
Refuse @($quarantine | Where-Object {$_ -cne $marker})
Refuse @($quarantine | Where-Object {$_ -cne $header})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $header){'---- session::foreign stdout ----'}else{$_}})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $header){'---- session::foreign stdout ----'}})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $header){'foreign text'}})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $marker){$marker}})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $header){$header}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $wrapperOk){'test session::two ... ok'}else{$_}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $wrapperOk){'test '+$wrapper+' ... FAILED'}else{$_}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $ignored){'test '+$probe+' ... ignored, other reason'}else{$_}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $ignored){'test session::other ... ignored, '+$reason}else{$_}})
Refuse @($quarantine | Where-Object {$_ -cne $ignored})
Refuse @($quarantine | ForEach-Object {$_ -replace '1 ignored','0 ignored'})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $ignored){$ignored}})
Refuse @($quarantine | ForEach-Object {$_ -replace '1 ignored','2 ignored'})
Refuse @($quarantine | ForEach-Object {$_ -replace '^running 3 tests$','running 2 tests'})
Refuse $valid @($wrapper)
Refuse $quarantine @($probe)
Refuse @($quarantine | ForEach-Object {if($_ -ceq $header){$marker;$header}elseif($_ -cne $marker){$_}})
Refuse @(@($quarantine | Where-Object {$_ -cne $marker})+@($marker))
Refuse @($quarantine | ForEach-Object {if($_ -ceq $wrapperOk){$header;$wrapperOk}elseif($_ -cne $header){$_}})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $wrapperOk){$wrapperOk}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $ignored){'test '+$probe+' ... skipped'}else{$_}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq 'successes:'){'failures:'}else{$_}})
Refuse @($quarantine | Where-Object {$_ -cne 'successes:'})
Refuse @($quarantine | ForEach-Object {$_;if($_ -ceq $header){'successes:'}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $ignored){'test '+$probe+' ... ignored'}else{$_}})
Refuse @($quarantine | ForEach-Object {if($_ -ceq $ignored){'test foreign-name ... ignored, '+$reason}else{$_}})
Refuse @($quarantine | ForEach-Object {$_ -replace '0 failed','1 failed'})
Refuse @('running 1 test',$ignored,'test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 145 filtered out; finished in 0.00s')
if($cases -ne 44){throw "Unexpected census fixture count: $cases"}
Write-Output "Core unit census regressions passed: $cases"
