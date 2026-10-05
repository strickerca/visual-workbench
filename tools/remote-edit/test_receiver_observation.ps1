# Pure source-bound parser/filter checks. No integration scripts are executed.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'receiver_observation.ps1')
$cases=0
function Assert-Receiver($Condition,[string]$Reason){if(-not $Condition){throw $Reason};$script:cases++}
$line='REMOTE_RECEIVER_COUNTS:down=1;update=2;up=1;mouse_down=1;mouse_up=1;wheel=0;journal_bytes=1170'
$value=Get-M4ReceiverObservation $line
Assert-Receiver ($value.counts -join ',' -ceq '1,2,1,1,1,0') 'Exact six categories lost'
Assert-Receiver ($value.journal_handle_bytes -eq 1170 -and $value.acceptance -eq $false) 'Observation became acceptance'
$unknown=Get-M4ReceiverObservation ($line.Replace('1170','unknown'))
Assert-Receiver ($null -eq $unknown.journal_handle_bytes) 'Unknown file query became zero'
foreach($bad in @(
    ('prefix'+$line),($line+'suffix'),($line+"`n"+$line),
    ($line.Replace('down=1','down=-1')),($line.Replace('down=1','down=20001')),
    ($line.Replace('1170','16777217')),($line.Replace('1170','999999999')),
    ($line.Replace('1170','1.5')),($line.Replace('wheel=0','wheel=true')),
    ($line.Replace('up=1;mouse_down','up=1;up=1;mouse_down'))
)) {Assert-Receiver ($null -eq (Get-M4ReceiverObservation $bad)) 'Malformed receiver diagnostic admitted'}
$zero=Get-M4ReceiverObservation ($line.Replace('down=1;update=2;up=1;mouse_down=1;mouse_up=1','down=0;update=0;up=0;mouse_down=0;mouse_up=0').Replace('1170','0'))
Assert-Receiver ($zero.journal_handle_bytes -eq 0) 'Actual zero not preserved'
$source=[IO.File]::ReadAllText((Join-Path $PSScriptRoot 'integration_input_owner.ps1'))
$filter=[regex]::Match($source,"if\(\`$line -cmatch '([^']+)'\)")
Assert-Receiver $filter.Success 'Actual retained filter not found'
$pattern=$filter.Groups[1].Value
foreach($safe in @('Real remote integration: phase=phone-background_retired elapsed=52s','Real remote integration stopped in phase=phone-native-render-callback. Private details and addresses are withheld.')){Assert-Receiver ([regex]::IsMatch($safe,$pattern)) 'Existing safe phase dropped'}
foreach($bad in @('Real remote integration: phase=http://private elapsed=52s','Real remote integration stopped in phase=C:\private. Private details and addresses are withheld.')){Assert-Receiver (-not [regex]::IsMatch($bad,$pattern)) 'Arbitrary progress text leaked'}
foreach($safe in @('Remote host failure: code=closed','Remote host retirement: first=closed current=pending attempts=99 failures=99')){Assert-Receiver ([regex]::IsMatch($safe,$pattern)) 'Closed host diagnostic dropped'}
foreach($bad in @('Remote host failure: code=private','Remote host retirement: first=closed current=http attempts=99 failures=99')){Assert-Receiver (-not [regex]::IsMatch($bad,$pattern)) 'Unknown host code leaked'}
Write-Output ('PASS {0} receiver observation/filter cases; no runtime actions' -f $cases)
