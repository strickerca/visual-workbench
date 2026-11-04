#requires -Version 7.2
[CmdletBinding()]
param([string]$Runner=(Join-Path $PSScriptRoot 'integration.ps1'),[string]$Producer=(Join-Path $PSScriptRoot '../../apps/android/src/remoteIntegrationTest/kotlin/com/visualworkbench/android/remote/RemoteNormalPathInstrumentedTest.kt'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'input_observation_evidence.ps1')
$count=0
function Check($Condition,[string]$Message){if(-not $Condition){throw $Message};$script:count++}
$state=New-M4InputObservationEvidence;$observed=New-M4InputObservationReceipt
$uuid='12345678-1234-1234-1234-123456789abc'
$balanced=[ordered]@{remote_input_binding="1,$uuid,2,$uuid,3,$uuid";remote_input_sequences='1,3';remote_input_owner=$uuid;remote_input_frame='42';remote_input_ticket='8';remote_input_pts_us='123';remote_input_timing='1,2,3,2,4';remote_input_echo='3,0,0';remote_input_route='surface_touch,software_generated=true,ghost_ack_fade=true,physical_pen_fidelity=false,editor_effect=false,latency_acceptance=false'}
foreach($key in $balanced.Keys){Add-M4InputObservationEvidence $state $observed "INSTRUMENTATION_STATUS: $key=$($balanced[$key])"}
Check ($observed.balanced.partial_record.remote_input_frame -ceq '42') 'Partial lost'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: 0'
Check ($observed.balanced.observed_count -eq 1 -and @($observed.balanced.records).Count -eq 1 -and $null -eq $observed.balanced.partial_record) 'Complete bundle lost'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_phase=waiting_fresh_carrier'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: -2'
Check ($observed.balanced.records[0].remote_input_timing -ceq '1,2,3,2,4' -and -not $observed.balanced.records_validated -and -not $observed.balanced.acceptance) 'Later failure manufactured validation or erased record'
foreach($kind in @('pause-held','background-held','disconnect-held')){
    Add-M4InputObservationEvidence $state $observed "INSTRUMENTATION_STATUS: remote_lifecycle_scenario=$kind"
    Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_lifecycle_phone_settled=true'
    Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: 0'
}
Check ($observed.held.observed_count -eq 3 -and $observed.held.records[2].remote_lifecycle_scenario -ceq 'disconnect-held' -and -not $observed.held.records_validated) 'Held observations lost'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_frame=51'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_frame=52'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_secret=private-path-or-QR'
Check ($observed.balanced.duplicate_fields -eq 1 -and $observed.balanced.unknown_fields -eq 1 -and $observed.balanced.partial_record.remote_input_frame -ceq '51') 'Duplicate or unknown field escaped'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_owner=C:\private\secret'
Check ($observed.balanced.withheld_fields -eq 1 -and $observed.balanced.partial_record.remote_input_owner -ceq '[withheld]') 'Malformed value leaked'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: -2'
Check ($observed.balanced.aborted_bundles -eq 1 -and $observed.balanced.partial_status -eq -2 -and $observed.balanced.observed_count -eq 1) 'Failed status accepted bundle'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_ticket=63'
Check (@($observed.balanced.partial_record.PSObject.Properties).Count -eq 1) 'Failed bundle contaminated successor'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: 0'
for($i=0;$i -lt 10;$i++){Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_frame=99';Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: 0'}
Check (@($observed.balanced.records).Count -eq 4 -and $observed.balanced.observed_count -eq 12 -and $observed.balanced.omitted_count -eq 8) 'Observation bound changed'
$observed.balanced.unknown_fields=[long]::MaxValue
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_input_unknown=x'
Check ($observed.balanced.unknown_fields -eq [long]::MaxValue) 'Counter overflowed'
$serialized=ConvertTo-Json -InputObject $observed -Depth 12 -Compress
Check (-not $serialized.Contains('private') -and -not $serialized.Contains('secret')) 'Raw malformed value escaped'
Add-M4InputObservationEvidence $state $observed ('INSTRUMENTATION_STATUS: remote_input_owner='+('x'*5000))
Check ($observed.balanced.withheld_fields -eq 1) 'Oversized line processed'
# Exercise the actual runner entry and actual finally drain, with unrelated
# observation helpers stubbed; no runner initialization or process commands.
$tokens=$null;$errors=$null;$ast=[Management.Automation.Language.Parser]::ParseFile($Runner,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Runner parse failed'}
foreach($name in @('Add-IntegrationDiagnostic','Drain-IntegrationDiagnostics')){
    $node=@($ast.FindAll({param($n)$n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $name},$true));if($node.Count -ne 1){throw 'Exact runner function missing'}
    . ([scriptblock]::Create($node[0].Extent.Text))
}
function Add-M4RenderObservationEvidence($State,$Receipt,[string]$Line){}
function Get-IntegrationDiagnosticLine([string]$Line){return $null}
$ControllerInput=$true;$startupProof=$null;$renderObservationEvidence=$null
$inputObservationEvidence=New-M4InputObservationEvidence
$receipt=[ordered]@{input_observations=(New-M4InputObservationReceipt);input_granted=$null;input_retirement_confirmed=$false}
$queue=[Collections.Concurrent.ConcurrentQueue[string]]::new()
$queue.Enqueue('INSTRUMENTATION_STATUS: remote_input_frame=177')
$queue.Enqueue('INSTRUMENTATION_STATUS_CODE: 0')
$queue.Enqueue('INSTRUMENTATION_STATUS: remote_lifecycle_phone_settled=true')
$owner=[pscustomobject]@{Name='real-phone-controller';Output=[pscustomobject]@{Lines=$queue}}
Drain-IntegrationDiagnostics $owner
Check ($receipt.input_observations.balanced.records[0].remote_input_frame -ceq '177' -and $receipt.input_observations.held.partial_record.remote_lifecycle_phone_settled -ceq 'true') 'Finally drain lost full or partial evidence'
Add-IntegrationDiagnostic 'unowned-child' 'INSTRUMENTATION_STATUS: remote_input_frame=999'
Check ($receipt.input_observations.balanced.observed_count -eq 1 -and $null -eq $receipt.input_granted -and -not $receipt.input_retirement_confirmed -and -not $receipt.input_observations.balanced.records_validated) 'Observation changed authority'
$source=Get-Content -Raw -LiteralPath $Runner
Check ($source -cnotmatch 'input_observations[^\r\n]*records_validated\s*=\s*\$true') 'Observed projection acquired unrelated strict validation'
# Derive names from the actual Android producer, including delayed putString calls.
$producerText=Get-Content -LiteralPath $Producer -Raw
$producerKeys=@([regex]::Matches($producerText,'putString\("(remote_lifecycle_[a-z0-9_]+)"')|ForEach-Object{$_.Groups[1].Value}|Sort-Object -Unique)
$patterns=Get-M4InputObservationPatterns 'held'
Check ($producerKeys.Count -eq 10) 'Actual producer lifecycle key census changed'
Check (($producerKeys -join ',') -ceq (@($patterns.Keys|Sort-Object) -join ',')) 'Producer and closed schema differ'
$values=@{remote_lifecycle_scenario='pause-held';remote_lifecycle_binding="3,$uuid,5,$uuid,1,$uuid";remote_lifecycle_sequences='1,2';remote_lifecycle_phone_up_admitted='false';remote_lifecycle_native_prefix_sha256=('a'*64);remote_lifecycle_native_prefix_bytes='2547';remote_lifecycle_stale_tail_refused='true';remote_lifecycle_route='surface_touch,software_generated=true,host_safety_release=true,physical_pen_fidelity=false';remote_lifecycle_phone_settled='true';remote_lifecycle_return_granted='false'}
$state=New-M4InputObservationEvidence;$observed=New-M4InputObservationReceipt;$actual=@{}
foreach($key in $producerKeys){
    $line="INSTRUMENTATION_STATUS: $key=$($values[$key])"
    $parsed=Get-M4HeldStatusLine $line
    Check ($null -ne $parsed -and $parsed.Key -ceq $key -and $parsed.Value -ceq $values[$key]) 'Production strict reader dropped actual Android key'
    $actual[$parsed.Key]=$parsed.Value;Add-M4InputObservationEvidence $state $observed $line
}
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: 0'
Check ($actual.Count -eq 10 -and @($observed.held.records[0].PSObject.Properties).Count -eq 10 -and $observed.held.records[0].remote_lifecycle_native_prefix_sha256 -ceq ('a'*64)) 'Production observation reader dropped actual digest key'
$refused=$false;try{[void](Get-M4HeldStatusLine 'INSTRUMENTATION_STATUS: remote_lifecycle_unknown256=private')}catch{$refused=$true}
Check $refused 'Unknown strict lifecycle key admitted'
$actualCalls=@($ast.FindAll({param($n)$n -is [Management.Automation.Language.CommandAst] -and $n.GetCommandName() -ceq 'Get-M4HeldStatusLine'},$true))
Check ($actualCalls.Count -eq 1) 'Tested strict reader is not wired into production runner'
# Owner-physical: closed keys come from the actual Android producer; only the
# exact physical route value is retained, and the strict reader is wired once.
$physicalKeys=@([regex]::Matches($producerText,'putString\("(remote_physical_[a-z0-9_]+)"')|ForEach-Object{$_.Groups[1].Value}|Sort-Object -Unique)
$physicalPatterns=Get-M4InputObservationPatterns 'physical'
Check ($physicalKeys.Count -eq 11 -and ($physicalKeys -join ',') -ceq (@($physicalPatterns.Keys|Sort-Object) -join ',')) 'Producer and closed physical schema differ'
$physicalValues=@{remote_physical_binding="3,$uuid,5,$uuid,1,$uuid";remote_physical_block='paint-pressure';remote_physical_strokes='6,6';remote_physical_samples='1234';remote_physical_sequences='1,1234';remote_physical_owner=$uuid;remote_physical_ack='40,12,1234';remote_physical_echo='1234,0,0';remote_physical_witness='2000,0';remote_physical_journal=('b'*64)+',345678';remote_physical_route='surface_touch,software_generated=false,physical_owner_gesture=true,window_delivered=true,synthetic_dispatches=0,ack_covered=true,latency_acceptance=false'}
$state=New-M4InputObservationEvidence;$observed=New-M4InputObservationReceipt
foreach($key in $physicalKeys){
    $line="INSTRUMENTATION_STATUS: $key=$($physicalValues[$key])"
    $parsed=Get-M4PhysicalStatusLine $line
    if($null -eq $parsed -or $parsed.Value -cne $physicalValues[$key]){throw 'Strict physical reader dropped actual key'}
    Add-M4InputObservationEvidence $state $observed $line
}
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS_CODE: 0'
Check ($observed.physical.observed_count -eq 1 -and @($observed.physical.records[0].PSObject.Properties).Count -eq 11 -and $observed.physical.withheld_fields -eq 0 -and -not $observed.physical.records_validated -and -not $observed.physical.acceptance) 'Physical observation lost or gained authority'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_physical_route=surface_touch,software_generated=true,ghost_ack_fade=true,physical_pen_fidelity=false,editor_effect=false,latency_acceptance=false'
Check ($observed.physical.withheld_fields -eq 1 -and $observed.physical.partial_record.remote_physical_route -ceq '[withheld]') 'Synthetic route admitted as physical'
Add-M4InputObservationEvidence $state $observed 'INSTRUMENTATION_STATUS: remote_physical_route=surface_touch,software_generated=false,physical_owner_gesture=false,window_delivered=true,synthetic_dispatches=0,ack_covered=true,latency_acceptance=false'
Check ($observed.physical.duplicate_fields -eq 1 -and $observed.balanced.observed_count -eq 0) 'Physical route value escaped or crossed kinds'
$refused=$false;try{[void](Get-M4PhysicalStatusLine 'INSTRUMENTATION_STATUS: remote_physical_secret=private')}catch{$refused=$true}
Check $refused 'Unknown strict physical key admitted'
$physicalCalls=@($ast.FindAll({param($n)$n -is [Management.Automation.Language.CommandAst] -and $n.GetCommandName() -ceq 'Get-M4PhysicalStatusLine'},$true))
Check ($physicalCalls.Count -eq 1) 'Strict physical reader is not wired into production runner'
Check ($source -cmatch "if\(\`$ownerPhysical\)\{\s*# Physical provenance is claimed only after its closed validator passed\.\s*if\(\`$receipt\.physical_records_validated -ne \`$true" -and $source -cmatch '\}else\{\$receipt\.software_generated_input=\$true\}') 'Physical route branch at software_generated_input missing'
if($count -ne 37){throw 'Unexpected test census'}
Write-Output 'Input observation pure cases: 37 passed; no process/device execution.'
