#requires -Version 7.2
[CmdletBinding()]
param([string]$Runner=(Join-Path $PSScriptRoot 'integration.ps1'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Load only the saved pure diagnostic functions, never runner initialization,
# process modules, network, APK, native, C#, filesystem or device actions.
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($Runner,[ref]$tokens,[ref]$errors)
if($errors.Count -ne 0){throw 'Diagnostic runner AST parse failed.'}
foreach($name in @('Save-M4ControllerValidationReport','Save-M4HeldValidationReport','Add-M4HostRetirementDiagnostic','Get-IntegrationDiagnosticLine','Add-IntegrationDiagnostic','Get-IntegrationFailureCode')){
    $found=@($ast.FindAll({param($node)$node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name},$true))
    if($found.Count -ne 1){throw 'Exact pure diagnostic function missing.'}
    . ([scriptblock]::Create($found[0].Extent.Text))
}
function Add-M4HostCarrierObservation($Receipt,[string]$Line){return $false}
$ControllerInput=$false
function Add-M4RenderObservationEvidence($State,$Receipt,[string]$Line){}
$renderObservationEvidence=$null
$receipt=[ordered]@{host_first_run_failure=$null;host_retirement_diagnostic=$null;host_grant_diagnostic=$null;host_grant_failure=$null;phone_actual_retirement=$false;host_actual_retirement=$false}
$count=0
function Expect-Diagnostic($line,$expected){
    $actual=Get-IntegrationDiagnosticLine $line
    if($actual -cne $expected){throw 'Exact sanitized diagnostic mismatch.'}
    $script:count++
}
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_display_message=Hardware HEVC refused: PreviousOwnerPending' 'remote_diag_display_message=Hardware HEVC refused: PreviousOwnerPending'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_decoder_reason=DecoderFailure:CodecException' 'remote_diag_decoder_reason=DecoderFailure:CodecException'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_viewport=1440x900' 'remote_diag_viewport=1440x900'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_remote_status=Sealed' 'remote_diag_remote_status=Sealed'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_phase=waiting_native_render_callback' 'phone_phase=waiting_native_render_callback'
Expect-Diagnostic 'REMOTE_HOST_PHASE:capture_selected' 'host_phase=capture_selected'
Expect-Diagnostic 'REMOTE_HOST_DIAGNOSTIC:status=sealed;reason=encoder_failure;configured=false;scope=true;link=SYNCED' 'REMOTE_HOST_DIAGNOSTIC:status=sealed;reason=encoder_failure;configured=false;scope=true;link=SYNCED'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: stack=java.lang.IllegalStateException: endpoint private-body-must-not-survive' 'exception_type=java.lang.IllegalStateException'
Expect-Diagnostic 'Exception in thread "main" com.visualworkbench.bindings.core.SessionException$Worker: private arbitrary body' 'exception_type=com.visualworkbench.bindings.core.SessionException$Worker'
Expect-Diagnostic '    at com.visualworkbench.android.remote.RemoteNormalPathInstrumentedTest.rendered(RemoteNormalPathInstrumentedTest.kt:81)' 'stack_location=com.visualworkbench.android.remote.RemoteNormalPathInstrumentedTest.rendered(RemoteNormalPathInstrumentedTest.kt:81)'
Expect-Diagnostic 'INSTRUMENTATION_STATUS_CODE: -2' 'phone_status_code=-2'
Expect-Diagnostic 'FAILURES!!!' 'phone_terminal_failure'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_remote_reason=192.0.2.7' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_display_message=C:\private\artifact.png' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_display_message=https://private.invalid' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_display_message=ABCDEFGHIJKLMNOPabcdefghijklmnop' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remoteBootstrap=privateQrBody' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_render_scope=private-unrelated-body' $null
Expect-Diagnostic ('INSTRUMENTATION_STATUS: remote_diag_display_message='+('a'*257)) $null
Expect-Diagnostic 'REMOTE_HOST_DIAGNOSTIC:status=sealed;reason=192.0.2.7;configured=false;scope=true;link=SYNCED' $null
Expect-Diagnostic '    at com.private.Class.method(C:\private\artifact.kt:1)' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS_CODE: -1 trailing-data' $null
$script:diagnostics=[Collections.Generic.List[object]]::new();$script:diagnosticTotal=0;$script:diagnosticDropped=0
for($index=0;$index -lt 70;$index++){Add-IntegrationDiagnostic 'real-phone-controller' ('INSTRUMENTATION_STATUS: remote_diag_stage=stage'+$index)}
if($diagnostics.Count -ne 64 -or $diagnosticTotal -ne 70 -or $diagnosticDropped -ne 6 -or $diagnostics[0].text -cne 'remote_diag_stage=stage6' -or $diagnostics[63].text -cne 'remote_diag_stage=stage69'){throw 'Bounded latest failure record/census changed.'};$count++
Add-IntegrationDiagnostic 'unowned-child' 'REMOTE_HOST_PHASE:private_body'
if($diagnostics.Count -ne 64 -or $diagnosticTotal -ne 70){throw 'Unowned child acquired diagnostic authority.'};$count++
if((Get-IntegrationFailureCode 'Real phone instrumentation reported failure.') -cne 'phone-instrumentation-failed' -or (Get-IntegrationFailureCode 'private endpoint 192.0.2.7') -cne 'unclassified-failure'){throw 'Failure-body privacy/correlation changed.'};$count++
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=16;pts_us=0;render_request_ns=-123;callback_render_ns=10;callback_enqueue_ns=500' 'remote_diag_render_identity=mask=16;pts_us=0;render_request_ns=-123;callback_render_ns=10;callback_enqueue_ns=500'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=7;pts_us=none;render_request_ns=none;callback_render_ns=1000000000000;callback_enqueue_ns=none' 'remote_diag_render_identity=mask=7;pts_us=none;render_request_ns=none;callback_render_ns=1000000000000;callback_enqueue_ns=none'
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=256;pts_us=0;render_request_ns=0;callback_render_ns=0;callback_enqueue_ns=0' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=0;pts_us=0;render_request_ns=0;callback_render_ns=1000000000001;callback_enqueue_ns=0' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=0;pts_us=0;render_request_ns=0;callback_render_ns=0;callback_enqueue_ns=0;ticket=1' $null
Expect-Diagnostic 'INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=0;pts_us=192.0.2.1;render_request_ns=0;callback_render_ns=0;callback_enqueue_ns=0' $null
# Decisive fields are independent of the ring and never establish retirement.
Add-IntegrationDiagnostic 'real-native-session-host' 'REMOTE_HOST_FAILURE:code=timeout'
Add-IntegrationDiagnostic 'real-native-session-host' 'REMOTE_HOST_FAILURE:code=closed'
Add-IntegrationDiagnostic 'real-native-session-host' 'REMOTE_HOST_RETIREMENT:first=closed;current=pending;attempts=101;failures=101'
for($index=0;$index -lt 70;$index++){Add-IntegrationDiagnostic 'real-phone-controller' ('INSTRUMENTATION_STATUS: remote_diag_stage=stage'+$index)}
if($receipt.host_first_run_failure -cne 'timeout' -or $receipt.host_retirement_diagnostic.first -cne 'closed' -or $receipt.host_retirement_diagnostic.failures -ne 101){throw 'Ring erased decisive cause/counters.'};$count++
foreach($line in @('REMOTE_HOST_FAILURE:code=private','REMOTE_HOST_FAILURE:code=none','REMOTE_HOST_RETIREMENT:first=closed;current=pending;attempts=100;failures=100','REMOTE_HOST_RETIREMENT:first=pending;current=pending;attempts=102;failures=102','REMOTE_HOST_RETIREMENT:first=closed;current=pending;attempts=9223372036854775808;failures=102','REMOTE_HOST_RETIREMENT:first=closed;current=pending;attempts=102;failures=103','REMOTE_HOST_RETIREMENT:first=none;current=none;attempts=102;failures=102')){
    if(Add-M4HostRetirementDiagnostic $receipt $line){throw 'Invalid or regressed evidence accepted.'};$count++
}
Add-IntegrationDiagnostic 'real-native-session-host' 'REMOTE_HOST_RETIREMENT:first=closed;current=none;attempts=102;failures=101'
if($receipt.host_retirement_diagnostic.current -cne 'none' -or $receipt.host_actual_retirement -or $receipt.phone_actual_retirement -or $receipt.host_retirement_diagnostic.acceptance){throw 'Counters manufactured retirement.'};$count++
# Structured failed-input observations survive the caller's later throw.
$evidence=[ordered]@{schema=1;events=@([ordered]@{ordinal=0;index=3});acceptance=$false;records_validated=$false;snapshot_complete=$true}
$payload=[ordered]@{schema=2;status='rejected';rejection_reason='balanced_native_pointer_receiver';records_validated=$false;receiver_evidence=$evidence}
$reply=[pscustomobject]@{Lines=@($payload|ConvertTo-Json -Depth 8 -Compress);ExitCode=1}
$r=Save-M4ControllerValidationReport $reply
if($receipt.controller_input_validation_reason -cne 'balanced_native_pointer_receiver' -or $receipt.receiver_journal_evidence.events[0].index -ne 3 -or $r.records_validated){throw 'Rejected input evidence lost'};$count++
$payload.status='observed';$payload.rejection_reason='not_validated'
[void](Save-M4ControllerValidationReport ([pscustomobject]@{Lines=@($payload|ConvertTo-Json -Depth 8 -Compress)}))
if($receipt.controller_input_validation_reason -cne 'balanced_native_pointer_receiver'){throw 'Cleanup observation erased originating rejection'};$count++
foreach($bad in @('private-body','C:/private/path','none trailing')){
 $payload.rejection_reason=$bad;$refused=$false
 try{[void](Save-M4ControllerValidationReport ([pscustomobject]@{Lines=@($payload|ConvertTo-Json -Depth 8 -Compress)}))}catch{$refused=$true}
 if(-not $refused){throw 'Unknown rejection label published'};$count++
}
$refused=$false;try{[void](Save-M4ControllerValidationReport ([pscustomobject]@{Lines=@('x'*131073)}))}catch{$refused=$true}
if(-not $refused){throw 'Oversized response accepted'};$count++
# Grant evidence is closed, monotonic and independent from acceptance/retirement.
$receipt.host_grant_diagnostic=$null;$receipt.host_grant_failure=$null
$grantLine='REMOTE_HOST_GRANT:stage=before_grant;code=authentication;grant=unknown;witness=unarmed;display_available=true;status=pending_focus;reason=target_focus_required'
Add-IntegrationDiagnostic 'real-native-session-host' $grantLine
if($receipt.host_grant_failure.stage -cne 'before_grant' -or $receipt.host_grant_failure.code -cne 'authentication' -or $receipt.host_grant_failure.grant -cne 'unknown'){throw 'Unknown grant failure lost'};$count++
foreach($line in @(
 'REMOTE_HOST_GRANT:stage=native_grant_returned;code=none;grant=unknown;witness=unarmed;display_available=false;status=unavailable;reason=unavailable',
 'REMOTE_HOST_GRANT:stage=controlling_observed;code=none;grant=confirmed;witness=unarmed;display_available=true;status=controlling;reason=none',
 'REMOTE_HOST_GRANT:stage=witness_arm_started;code=invalid;grant=confirmed;witness=arming;display_available=true;status=controlling;reason=none')){
 if(-not (Add-M4HostRetirementDiagnostic $receipt $line)){throw 'Valid grant observation refused'};$count++
}
for($index=0;$index -lt 70;$index++){Add-IntegrationDiagnostic 'real-phone-controller' ('INSTRUMENTATION_STATUS: remote_diag_stage=stage'+$index)}
if($receipt.host_grant_failure.code -cne 'authentication' -or $receipt.host_grant_diagnostic.code -cne 'invalid' -or $receipt.host_grant_diagnostic.grant -cne 'confirmed' -or $receipt.host_grant_diagnostic.acceptance -or $receipt.host_actual_retirement -or $receipt.phone_actual_retirement){throw 'Ring erased grant evidence or invented acceptance'};$count++
$valid='REMOTE_HOST_GRANT:stage=witness_arm_started;code=invalid;grant=confirmed;witness=arming;display_available=true;status=controlling;reason=none'
foreach($bad in @(
 $valid.Replace('stage=witness_arm_started','stage=private'),
 $valid.Replace('code=invalid','code=C:/private'),
 $valid.Replace('grant=confirmed','grant=unknown'),
 $valid.Replace('witness=arming','witness=armed'),
 $valid.Replace('display_available=true','display_available=False'),
 $valid.Replace('display_available=true','display_available=false'),
 $valid.Replace('status=controlling','status=https://private.invalid'),
 $valid.Replace('reason=none','reason=C:/private'),
 ($valid+';private=secret'),
 $grantLine)){
 if(Add-M4HostRetirementDiagnostic $receipt $bad){throw 'Unclosed or regressed grant observation accepted'};$count++
}
$receipt.host_grant_diagnostic=$null;$receipt.host_grant_failure=$null
if(-not (Add-M4HostRetirementDiagnostic $receipt 'REMOTE_HOST_GRANT:stage=before_grant;code=authentication;grant=unknown;witness=unarmed;display_available=false;status=unavailable;reason=unavailable')){throw 'Unavailable snapshot lost original typed error'};$count++
$heldEvidence=[ordered]@{schema=1;records=@([ordered]@{remote_lifecycle_scenario='pause-held'});prefix=[ordered]@{sha256=('a'*64)};trigger=[ordered]@{counter=123};witness=[ordered]@{cause='owner_pause'};acceptance=$false;records_validated=$false}
$heldPayload=[ordered]@{schema=3;status='rejected';rejection_reason='lifecycle_shape';records_validated=$false;held_evidence=$heldEvidence}
$heldReply=[pscustomobject]@{Lines=@($heldPayload|ConvertTo-Json -Depth 8 -Compress);ExitCode=1}
$v=Save-M4HeldValidationReport $heldReply
if($receipt.held_lifecycle_validation_reason -cne 'lifecycle_shape' -or $receipt.held_lifecycle_records[0].remote_lifecycle_scenario -cne 'pause-held' -or $receipt.held_lifecycle_evidence.trigger.counter -ne 123 -or $v.records_validated){throw 'Held failure evidence lost before throw'};$count++
$heldPayload.status='observed';$heldPayload.rejection_reason='not_validated'
[void](Save-M4HeldValidationReport ([pscustomobject]@{Lines=@($heldPayload|ConvertTo-Json -Depth 8 -Compress)}))
if($receipt.held_lifecycle_validation_reason -cne 'lifecycle_shape'){throw 'Cleanup erased held originating reason'};$count++
$heldPayload.rejection_reason='C:/private';$refused=$false;try{[void](Save-M4HeldValidationReport ([pscustomobject]@{Lines=@($heldPayload|ConvertTo-Json -Depth 8 -Compress)}))}catch{$refused=$true}
if(-not $refused){throw 'Private held reason accepted'};$count++
if($count -ne 65){throw 'Exact diagnostic regression census changed.'}
Write-Output 'Integration diagnostic pure cases: 65 passed; no process/device execution.'
