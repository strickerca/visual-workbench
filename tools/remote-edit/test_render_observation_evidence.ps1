#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'render_observation_evidence.ps1')
function Assert($Ok,[string]$Message){if(-not $Ok){throw $Message}}
function Receipt {return [ordered]@{phone_render_observed_count=0;phone_render_records=@();phone_render_records_validated=$false;phone_actual_retirement=$false;phone_render_partial_record=$null;phone_render_observations_omitted=0;phone_phase_transitions=@();phone_phase_transitions_omitted=0;phone_last_phase=$null;phone_stream_snapshots=[ordered]@{};host_source_selections=@()}}
function FeedRecord($State,$Receipt,[int]$Number=1){
    $record=@{
        remote_render_scope="1,01a10700-0000-7000-8000-000000000001,$Number,01a10700-0000-7000-8000-000000000002,1"
        remote_render_owner='00000000-0000-4000-8000-000000000001'
        remote_render_ticket=[string]$Number;remote_render_frame='1';remote_render_pts_us='100'
        remote_render_codec='c2.qti.hevc.decoder';remote_render_timing='200,300'
        remote_render_capabilities='hardware=true,low_latency_advertised=false,requested=true,configure_accepted=true'
    }
    foreach($key in $record.Keys){Add-M4RenderObservationEvidence $State $Receipt ('INSTRUMENTATION_STATUS: '+$key+'='+$record[$key])}
    Add-M4RenderObservationEvidence $State $Receipt 'INSTRUMENTATION_STATUS_CODE: 0'
}
$cases=0
$r=Receipt;$s=New-M4RenderObservationEvidence;FeedRecord $s $r
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS_CODE: -2'
Assert ($r.phone_render_observed_count -eq 1 -and @($r.phone_render_records).Count -eq 1) 'Later failure lost completed observation'
Assert (-not $r.phone_render_records_validated -and -not $r.phone_actual_retirement) 'Observation invented validation/retirement';$cases++
FeedRecord $s $r 2
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_render_ticket=3'
Assert ($r.phone_render_observed_count -eq 2 -and $r.phone_render_partial_record.remote_render_ticket -ceq '3') 'Partial record replaced completed census'
Assert ($r.phone_render_partial_record.remote_render_scope -ceq '[withheld]') 'Missing fields not withheld';$cases++
$r=Receipt;$s=New-M4RenderObservationEvidence
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_render_codec=C:\private\fixture secret'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_render_untrusted=private value'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS_CODE: 0'
Assert ($r.phone_render_records[0].remote_render_codec -ceq '[withheld]' -and @($r.phone_render_records[0].PSObject.Properties).Count -eq 8) 'Unbounded values or keys escaped';$cases++
$r=Receipt;$s=New-M4RenderObservationEvidence
1..5|ForEach-Object{FeedRecord $s $r $_}
Assert ($r.phone_render_observed_count -eq 5 -and @($r.phone_render_records).Count -eq 3 -and $r.phone_render_observations_omitted -eq 2) 'Evidence list/census bound';$cases++
$r=Receipt;$s=New-M4RenderObservationEvidence
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_phase=initial_rendered'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_phase=initial_rendered'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_phase=waiting_native_render_callback'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS_CODE: -2'
Assert (@($r.phone_phase_transitions).Count -eq 2 -and $r.phone_phase_transitions[0].phase -ceq 'initial_rendered' -and $r.phone_last_phase -ceq 'waiting_native_render_callback') 'Ring-independent phase evidence lost';$cases++
$r=Receipt;$s=New-M4RenderObservationEvidence
1..70|ForEach-Object {Add-M4RenderObservationEvidence $s $r ('INSTRUMENTATION_STATUS: remote_phase='+$(if($_%2){'waiting'}else{'rendered'}))}
Assert (@($r.phone_phase_transitions).Count -eq 64 -and $r.phone_phase_transitions_omitted -eq 6) 'Phase transition bound';$cases++
# Exercise production diagnostic entry and cleanup drain function definitions.
# The real pipeline must route phone lines through the accumulator during finally.
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'integration.ps1'),[ref]$tokens,[ref]$errors)
Assert ($errors.Count -eq 0) 'Integration syntax errors'
foreach($name in @('Add-IntegrationDiagnostic','Drain-IntegrationDiagnostics')){
    $definition=$ast.Find({param($n)$n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $name},$true)
    Assert ($null -ne $definition) 'Production diagnostic/drain missing'
    . ([scriptblock]::Create($definition.Extent.Text))
}
function Get-IntegrationDiagnosticLine([string]$Line){return $null}
$ControllerInput=$false # Exact input-free branch used by this extracted production drain test.
$receipt=Receipt;$renderObservationEvidence=New-M4RenderObservationEvidence
$queue=[Collections.Concurrent.ConcurrentQueue[string]]::new()
$queue.Enqueue('INSTRUMENTATION_STATUS: remote_render_ticket=7');$queue.Enqueue('INSTRUMENTATION_STATUS_CODE: 0')
$owner=[pscustomobject]@{Name='real-phone-controller';Output=[pscustomobject]@{Lines=$queue}}
Drain-IntegrationDiagnostics $owner
Assert ($receipt.phone_render_observed_count -eq 1 -and $receipt.phone_render_records[0].remote_render_ticket -ceq '7') 'Actual finally drain discarded observation';$cases++
# Per-view snapshots must retain zeros and terminal state despite higher earlier maxima.
$r=Receipt;$s=New-M4RenderObservationEvidence
function Snapshot($State,$Receipt,[int]$View,[string]$Stage,[string]$Count){
    foreach($line in @("remote_diag_stream_media_received=$Count","remote_diag_view_index=$View","remote_diag_stage=$Stage",'remote_diag_decoder=absent','remote_diag_remote_status=Disconnected')){Add-M4RenderObservationEvidence $State $Receipt ('INSTRUMENTATION_STATUS: '+$line)}
    Add-M4RenderObservationEvidence $State $Receipt 'INSTRUMENTATION_STATUS_CODE: 0'
}
Snapshot $s $r 0 'native_render_observed' '2';Snapshot $s $r 2 'render_timeout' '0'
Assert ($r.phone_stream_snapshots['0:native_render_observed'].media_received -eq 2 -and $r.phone_stream_snapshots['2:render_timeout'].media_received -eq 0) 'Maxima hid fresh owner zero ingress';$cases++
Snapshot $s $r 2 'render_timeout' '1'
Assert ($r.phone_stream_snapshots.Count -eq 2 -and $r.phone_stream_snapshots['2:render_timeout'].media_received -eq 1) 'Latest stage snapshot did not replace in bound';$cases++
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_diag_private_secret=C:\private\fixture'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_diag_stage=private_unknown'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS: remote_diag_view_index=2'
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS_CODE: 0'
Assert ($r.phone_stream_snapshots.Count -eq 2) 'Unknown diagnostic stage created a snapshot'
Assert (-not $r.phone_render_records_validated -and -not $r.phone_actual_retirement) 'Diagnostic snapshot invented validation';$cases++
Write-Output "Incremental render evidence cases passed: $cases"

$r=Receipt;$s=New-M4RenderObservationEvidence
foreach($line in @('remote_diag_view_index=2','remote_diag_stage=waiting_fresh_carrier','remote_diag_carrier_snapshot_available=true','remote_diag_carrier_status=Offline','remote_diag_carrier_failure=untrusted_or_revoked','remote_diag_carrier_epoch=0','remote_diag_carrier_available=false')){Add-M4RenderObservationEvidence $s $r ('INSTRUMENTATION_STATUS: '+$line)}
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS_CODE: 0'
Assert ($r.phone_stream_snapshots['2:waiting_fresh_carrier'].carrier_failure -ceq 'untrusted_or_revoked' -and $r.phone_stream_snapshots['2:waiting_fresh_carrier'].carrier_epoch -ceq '0') 'Actual failed carrier state lost';$cases++
foreach($line in @('remote_diag_view_index=2','remote_diag_stage=render_timeout','remote_diag_carrier_snapshot_available=false','remote_diag_carrier_failure=private','remote_diag_carrier_epoch=18446744073709551616')){Add-M4RenderObservationEvidence $s $r ('INSTRUMENTATION_STATUS: '+$line)}
Add-M4RenderObservationEvidence $s $r 'INSTRUMENTATION_STATUS_CODE: 0'
Assert ($r.phone_stream_snapshots['2:render_timeout'].PSObject.Properties.Name -cnotcontains 'carrier_epoch') 'Unavailable query fabricated epoch';$cases++
$r=[ordered]@{host_carrier_snapshot_available=$false;host_carrier_diagnostic=$null;host_first_carrier_failure=$null;host_actual_retirement=$false}
Assert (Add-M4HostCarrierObservation $r 'REMOTE_HOST_CARRIER:status=RECONNECTING;failure=untrusted_or_revoked;epoch=4;available=false') 'Actual host failure rejected';$cases++
Assert (Add-M4HostCarrierObservation $r 'REMOTE_HOST_CARRIER:status=SYNCED;failure=none;epoch=5;available=true') 'Actual new carrier rejected';$cases++
Assert ($r.host_first_carrier_failure -ceq 'untrusted_or_revoked' -and -not $r.host_carrier_diagnostic.acceptance -and -not $r.host_actual_retirement) 'Snapshot erased first failure or acquired authority';$cases++
Assert (Add-M4HostCarrierObservation $r 'REMOTE_HOST_CARRIER_UNAVAILABLE') 'Unavailable diagnostic rejected'
Assert (-not $r.host_carrier_snapshot_available -and $r.host_carrier_diagnostic.epoch -ceq '5') 'Unavailable query overwrote actual epoch';$cases++
foreach($line in @('REMOTE_HOST_CARRIER:status=SYNCED;failure=private;epoch=5;available=true','REMOTE_HOST_CARRIER:status=SYNCED;failure=none;epoch=18446744073709551616;available=true')){Assert (-not (Add-M4HostCarrierObservation $r $line)) 'Malformed carrier observation admitted';$cases++}
Write-Output "Carrier snapshot assertions plus prior render evidence: $cases"
