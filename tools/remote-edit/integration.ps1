#requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)][switch]$Execute,
    [Parameter(Mandatory)][switch]$OwnerReady,
    [Parameter(Mandatory)][string]$ArtifactManifest,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{64}$')][string]$ArtifactManifestSha256,
    [Parameter(Mandatory)][ValidateRange(1,2147483647)][int]$UsbInterfaceIndex,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{64}$')][string]$UsbDeviceInstanceSha256,
    [Parameter(Mandatory)][ValidatePattern('^(?:[0-9]{1,3}\.){3}[0-9]{1,3}$')][string]$PcUsbAddress,
    [Parameter(Mandatory)][ValidatePattern('^(?:[0-9]{1,3}\.){3}[0-9]{1,3}$')][string]$PhoneUsbAddress,
    [Parameter(Mandatory)][ValidateRange(1024,65535)][int]$Port,
    [switch]$ControllerInput,
    [ValidateSet("balanced","pause-held","background-held","disconnect-held")][string]$InputScenario="balanced",
    [string]$RetainedInputGate,
    [string]$RetainedInputNonce
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $Execute -or -not $OwnerReady -or -not $IsWindows){throw 'Central owner execution and explicit owned visible-window readiness required.'}
$projectRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1')
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1')
Import-Module (Join-Path $projectRoot 'tools/app-test/device.psm1')
$names=@('vw_core.dll','vw_host.dll','vw-connection-helper.exe','vw-capture-helper.exe','vw-hevc-helper.exe','vw-input-helper.exe')
$packages=@('com.visualworkbench.android.hil','com.visualworkbench.android.hil.test')
$instrumentationSelection='com.visualworkbench.android.remote.RemoteNormalPathInstrumentedTest'
if($ControllerInput){if([string]::IsNullOrEmpty($RetainedInputGate) -or $RetainedInputNonce -cnotmatch '^[a-f0-9]{32}$'){throw 'Use the retained input owner entry point.'}}elseif($RetainedInputGate -or $RetainedInputNonce){throw 'Unexpected retained input parent gate.'}
if(-not $ControllerInput -and $InputScenario -cne 'balanced'){throw 'Held scenario requires retained controller-input owner.'}
$runId=if($ControllerInput){$RetainedInputNonce}else{[guid]::NewGuid().ToString('N')}
$temporaryParent=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$owned=Join-Path $temporaryParent ('VisualWorkbench-remote-integration-'+$runId)
$phase='artifact_binding';$device=$null;$artifact=$null;$bootstrap=$null
$children=[Collections.Generic.List[object]]::new();$pins=[Collections.Generic.List[IDisposable]]::new()
$directoryPins=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
# Retain non-reparse ancestor directory leases as well as final file handles.
# Deny-delete LIST|ATTR is intentional: ATTR alone did not block rename in the
# centrally measured directory experiment. No zero-access fallback is allowed.
if(-not ('VwRemoteIntegrationDirectoryLeaseV1' -as [type])){
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;
public static class VwRemoteIntegrationDirectoryLeaseV1 {
 [StructLayout(LayoutKind.Sequential)] private struct Info {
  public uint Attributes; public System.Runtime.InteropServices.ComTypes.FILETIME Creation,Access,Write;
  public uint Volume,SizeHigh,SizeLow,Links,IndexHigh,IndexLow;
 }
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern SafeFileHandle CreateFileW(string name,uint access,uint share,IntPtr security,uint mode,uint flags,IntPtr template);
 [DllImport("kernel32.dll",SetLastError=true)] private static extern bool GetFileInformationByHandle(SafeFileHandle file,out Info info);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode,SetLastError=true)] private static extern uint GetFinalPathNameByHandleW(SafeFileHandle file,StringBuilder text,uint size,uint flags);
 public static SafeFileHandle Open(string path) {
  string full=Path.GetFullPath(path);
  if(full.Length<3 || full[1]!=':' || full[2]!='\\' || full.IndexOf(':',2)>=0)throw new IOException("Directory namespace refused");
  var handle=CreateFileW(full,0x81,3,IntPtr.Zero,3,0x02200000,IntPtr.Zero);
  try {
   Info info;var value=new StringBuilder(32768);
   if(handle.IsInvalid || !GetFileInformationByHandle(handle,out info) || (info.Attributes&0x10)==0 || (info.Attributes&0x400)!=0)throw new IOException("Directory lease refused");
   uint count=GetFinalPathNameByHandleW(handle,value,(uint)value.Capacity,0);
   if(count==0 || count>=value.Capacity || !String.Equals(value.ToString(),"\\\\?\\"+full,StringComparison.OrdinalIgnoreCase))throw new IOException("Opened directory namespace refused");
   return handle;
  }catch {handle.Dispose();throw;}
 }
}
'@
}
$installed=@{};$attempted=@{};$preAbsent=@{};$allClean=$true;$passed=$false;$created=$false;$records=[Collections.Generic.List[object]]::new()
$inputRecords=[Collections.Generic.List[object]]::new();$inputRecord=@{}
$lifecycleRecords=[Collections.Generic.List[object]]::new();$lifecycleRecord=@{};$heldPrefix=$null;$lifecycleTriggered=$false
$streamCounters=@{phone=@{};host=@{}}
$hostStreamCounterKeys=@('capture_polls','capture_idle','encoded_frames','config_enqueued','media_enqueued','config_received','media_received','media_no_config','media_retired','media_admitted','frame_taken')
$streamCounterKeys=@('capture_polls','capture_idle','encoded_frames','config_enqueued','media_enqueued','config_received','media_received','media_no_config','media_retired','media_admitted','frame_taken','state_published','state_stale','frames_emitted','frame_scope_discarded','frames_received','scope_discarded','inactive_discarded','surface_waited','surface_deadline','surface_invalidated','decoder_opened','decoder_refused','startup_failed','frames_queued','queue_refused','rendered')
$diagnostics=[Collections.Generic.List[object]]::new();$diagnosticTotal=0;$diagnosticDropped=0;$phoneRetired=$false;$startupProof=$null
$receipt=[ordered]@{schema=1;kind='normal-native-remote-usb-integration';status='failed';failure_phase=$phase;run_id=$runId;model='SM-S918U';source_count=0;artifact_manifest_sha256=$ArtifactManifestSha256;carrier='QuicTether';physical_route_verified=$false;owner_user=0;profiles_preserved=$false;phone_render_records=@();phone_render_observed_count=0;phone_render_records_validated=$false;render_validation_reason_code=$null;phone_render_partial_record=$null;phone_render_observations_omitted=0;phone_phase_transitions=@();phone_phase_transitions_omitted=0;phone_last_phase=$null;phone_stream_snapshots=[ordered]@{};host_source_selections=@();host_carrier_diagnostic=$null;host_first_carrier_failure=$null;host_carrier_snapshot_available=$false;host_first_run_failure=$null;host_retirement_diagnostic=$null;host_grant_diagnostic=$null;host_grant_failure=$null;receiver_observation=$null;receiver_journal_evidence=$null;receiver_evidence_capture_failed=$false;controller_input_validation_reason=$null;controller_input_records_validated=$false;phone_actual_retirement=$false;host_actual_retirement=$false;process_cleanup=$false;owned_packages_cleanup=$false;private_work_disposed=$false;input_grant_attempted=$false;input_granted=$false;input_attempted=$false;input_dispatch_completed=$false;input_retirement_confirmed=$false;retained_input_nonce=$RetainedInputNonce;controller_input_requested=[bool]$ControllerInput;controller_input_scenario=$InputScenario;held_lifecycle_records=@();held_lifecycle_records_validated=$false;held_lifecycle_validation_reason=$null;held_lifecycle_evidence=$null;held_evidence_capture_failed=$false;controller_input_records=@();owned_receiver_counts=$null;software_generated_input=$false;physical_pen_fidelity=$false;editor_effect=$false;latency_acceptance=$false;screenshots_created=0;failure_error_type=$null;failure_error_code=$null;diagnostic_records=@();diagnostic_total=0;diagnostic_dropped=0;phone_retirement_phase_seen=$false;phone_no_owner_startup_proven=$false;host_no_owner_startup_proven=$false}
. (Join-Path $PSScriptRoot 'render_observation_evidence.ps1')
. (Join-Path $PSScriptRoot 'receiver_observation.ps1')
. (Join-Path $PSScriptRoot 'input_observation_evidence.ps1')
$inputObservationEvidence=New-M4InputObservationEvidence
$receipt.input_observations=New-M4InputObservationReceipt
$renderObservationEvidence=New-M4RenderObservationEvidence
# Only these source-owned text shapes enter the bounded public failure record.
# Raw redirected output, exception messages, URLs, QR/bootstrap and media stay out.
function Publish-InputSettlement {
    $settled=Join-Path $owned 'window/input-settled'
    if(-not(Test-Path -LiteralPath (Split-Path -Parent $settled) -PathType Container)){return}
    if(Test-Path -LiteralPath $settled){
        Assert-Plain $settled
        if((Get-Content -Raw -LiteralPath $settled) -cne $runId){throw 'Destination settlement binding mismatch.'}
        return
    }
    $f=[IO.File]::Open($settled,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try{$bytes=[Text.Encoding]::UTF8.GetBytes($runId);$f.Write($bytes);$f.Flush($true)}finally{$f.Dispose()}
}
function Write-HeldTrigger([string]$Kind){
    if($null -eq $heldPrefix -or $lifecycleTriggered){throw 'Held lifecycle trigger ordering refused.'}
    $clock=[VwHeldClockR24]::Read()
    if($clock[1] -ne [long]$heldPrefix.frequency -or $clock[0] -le [long]$heldPrefix.last_received_qpc){throw 'Actual host receive/trigger clock mismatch.'}
    $trigger=[ordered]@{schema=1;run_id=$runId;scenario=$InputScenario;counter=$clock[0];frequency=$clock[1];kind=$Kind}
    $path=Join-Path $owned 'held-trigger.json'
    if(Test-Path -LiteralPath $path){throw 'Held lifecycle trigger already exists.'}
    [IO.File]::WriteAllText($path,($trigger|ConvertTo-Json -Compress),[Text.UTF8Encoding]::new($false))
    $script:lifecycleTriggered=$true
}
function Publish-HeldPhoneGate {
    $values=@('M4_HELD_NATIVE_READY_V1',$runId,$InputScenario,$heldPrefix.sha256,[string]$heldPrefix.bytes,[string]$heldPrefix.counts[0],[string]$heldPrefix.counts[1],[string]$heldPrefix.counts[2])
    $encoded=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($values -join "`n")))
    # Only closed scenario names, hex IDs/hash and base64 enter this exact owned
    # package's private no_backup path. No global ADB/network settings change.
    $gatePath="no_backup/remote-integration-$runId/held-native-ready"
    $command="run-as $($packages[0]) sh -c 'test ! -e $gatePath && test ! -e $gatePath.tmp && echo $encoded | base64 -d > $gatePath.tmp && mv $gatePath.tmp $gatePath'"
    [void](Adb 'publish-owned-held-trigger-gate' @('shell',$command))
}
function Get-IntegrationDiagnosticLine([string]$Line){
    if($null -eq $Line -or $Line.Length -gt 2048){return $null}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_render_identity=mask=([0-9]{1,3});pts_us=(none|-?[0-9]{1,13});render_request_ns=(none|-?[0-9]{1,13});callback_render_ns=(-?[0-9]{1,13});callback_enqueue_ns=(none|-?[0-9]{1,13})$'){
        if([int]$Matches[1] -gt 255){return $null}
        foreach($index in 2..5){if($Matches[$index] -cne 'none' -and ([long]$Matches[$index] -lt -1000000000000 -or [long]$Matches[$index] -gt 1000000000000)){return $null}}
        return $Line.Substring('INSTRUMENTATION_STATUS: '.Length)
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS: (remote_diag_(?:stage|display_message|decoder|decoder_reason|remote_status|remote_reason|config_present|scope_present|viewport|native_snapshot_available))=([A-Za-z0-9 _.,:;()=-]{1,256})$'){
        $key=$Matches[1];$value=$Matches[2]
        if($value -cmatch '(?:[0-9]{1,3}\.){3}[0-9]{1,3}|(?:^|[ (])[A-Za-z]:|[A-Za-z0-9]{32,}'){return $null}
        return ($key+'='+$value)
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_phase=([a-z_]{1,64})$'){return ('phone_phase='+$Matches[1])}
    if($Line -cmatch '^REMOTE_HOST_PHASE:([a-z_]{1,64})$'){return ('host_phase='+$Matches[1])}
    if($Line -cmatch '^REMOTE_HOST_DIAGNOSTIC:status=([A-Za-z][A-Za-z0-9_]{0,63});reason=([A-Za-z][A-Za-z0-9_.:-]{0,95});configured=(true|false);scope=(true|false);link=([A-Z][A-Z0-9_]{0,63})$'){
        if($Line -cmatch '(?:[0-9]{1,3}\.){3}[0-9]{1,3}|[A-Za-z0-9]{32,}'){return $null}
        return $Line
    }
    # Keep actual failure class/location, never its arbitrary body/message.
    if($Line -cmatch '^(?:INSTRUMENTATION_STATUS: stack=|Exception in thread "[A-Za-z0-9 _.-]{1,64}" )?((?:[A-Za-z_$][A-Za-z0-9_$]*\.)+[A-Za-z_$][A-Za-z0-9_$]*)(?::|$)'){
        if($Matches[1].Length -gt 192){return $null}
        return ('exception_type='+$Matches[1])
    }
    if($Line -cmatch '^\s*at ((?:[A-Za-z_$][A-Za-z0-9_$]*\.)+[A-Za-z_$][A-Za-z0-9_$]*)\(([A-Za-z0-9_.]{1,96}):([1-9][0-9]{0,7})\)\s*$'){
        if($Matches[1].Length -gt 256){return $null}
        return ('stack_location='+$Matches[1]+'('+$Matches[2]+':'+$Matches[3]+')')
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS_CODE: (-[12]|0)$'){return ('phone_status_code='+$Matches[1])}
    if($Line -cmatch '^(?:FAILURES!!!|INSTRUMENTATION_FAILED|Process crashed)(?:$|[ :])'){return 'phone_terminal_failure'}
    return $null
}
function Add-M4HostRetirementDiagnostic($Receipt,[string]$Line){
    $codes='none|closed|pending|partial_input|backpressure|timeout|transport|worker|unavailable|invalid|authentication|session_other|cancelled|argument|state|assertion|io|other'
    if($Line.StartsWith('REMOTE_HOST_GRANT:',[StringComparison]::Ordinal)){
        $stages=@('before_grant','native_grant_returned','controlling_observed','witness_arm_started','witness_armed','ready')
        $statuses='unavailable|withheld|selecting|viewing|controlling|pending_focus|pending_grant|paused|sealed|disconnected|closing|closed'
        $reasons='unavailable|withheld|none|target_focus_required|connection_retired|host_revoked|peer_background|owner_pause|background|input_expired|input_worker_unavailable|video_worker_unavailable|partial_input'
        if($Line -cnotmatch ('^REMOTE_HOST_GRANT:stage=('+($stages -join '|')+');code=('+ $codes +');grant=(unknown|confirmed);witness=(not_requested|unarmed|arming|armed);display_available=(true|false);status=('+ $statuses +');reason=('+ $reasons +')$')){return $false}
        $stage=$Matches[1];$code=$Matches[2];$grant=$Matches[3];$witness=$Matches[4];$available=$Matches[5] -ceq 'true';$status=$Matches[6];$reason=$Matches[7]
        $rank=[array]::IndexOf($stages,$stage)
        if(($rank -lt 2 -and $grant -cne 'unknown') -or ($rank -ge 2 -and $grant -cne 'confirmed') -or (-not $available -and ($status -cne 'unavailable' -or $reason -cne 'unavailable')) -or ($available -and ($status -ceq 'unavailable' -or $reason -ceq 'unavailable'))){return $false}
        if(($rank -lt 3 -and $witness -cnotin @('not_requested','unarmed')) -or ($stage -ceq 'witness_arm_started' -and $witness -cne 'arming') -or ($stage -ceq 'witness_armed' -and $witness -cne 'armed') -or ($stage -ceq 'ready' -and $witness -cnotin @('not_requested','armed'))){return $false}
        $prior=$Receipt.host_grant_diagnostic
        if($null -ne $prior -and ([array]::IndexOf($stages,$prior.stage) -gt $rank -or ($prior.grant -ceq 'confirmed' -and $grant -cne 'confirmed'))){return $false}
        $observation=[ordered]@{stage=$stage;code=$code;grant=$grant;witness=$witness;display_available=$available;status=$status;reason=$reason;acceptance=$false}
        $Receipt.host_grant_diagnostic=$observation
        if($code -cne 'none' -and $null -eq $Receipt.host_grant_failure){$Receipt.host_grant_failure=$observation}
        return $true
    }
    if($Line -cmatch ('^REMOTE_HOST_FAILURE:code=('+ $codes +')$')){
        if($Matches[1] -ceq 'none'){return $false}
        if($null -eq $Receipt.host_first_run_failure){$Receipt.host_first_run_failure=$Matches[1];try{Write-Host ('Remote host failure: code='+$Matches[1])}catch{}}
        return $true
    }
    if($Line -cmatch ('^REMOTE_HOST_RETIREMENT:first=('+ $codes +');current=('+ $codes +');attempts=([0-9]{1,19});failures=([0-9]{1,19})$')){
        $first=$Matches[1];$current=$Matches[2];$attempts=[long]0;$failures=[long]0
        if(-not [long]::TryParse($Matches[3],[ref]$attempts) -or -not [long]::TryParse($Matches[4],[ref]$failures) -or $attempts -lt 1 -or $failures -gt $attempts -or (($failures -eq 0) -ne ($first -ceq 'none')) -or ($failures -eq 0 -and $current -cne 'none')){return $false}
        $prior=$Receipt.host_retirement_diagnostic
        if($null -ne $prior -and ($attempts -lt $prior.attempts -or $failures -lt $prior.failures -or ($prior.first -cne 'none' -and $first -cne $prior.first))){return $false}
        $Receipt.host_retirement_diagnostic=[ordered]@{first=$first;current=$current;attempts=$attempts;failures=$failures;acceptance=$false}
        try{Write-Host "Remote host retirement: first=$first current=$current attempts=$attempts failures=$failures"}catch{}
        return $true
    }
    return $false
}
function Add-IntegrationDiagnostic([string]$Owner,[string]$Line){
    if($Owner -cnotin @('real-native-session-host','real-phone-controller','owned-wgc-target')){return}
    if($Owner -ceq 'real-native-session-host' -and (Add-M4HostRetirementDiagnostic $receipt $Line)){return}
    if($Owner -ceq 'owned-wgc-target'){$observed=Get-M4ReceiverObservation $Line;if($null -ne $observed){$receipt.receiver_observation=$observed;return}}
    if($ControllerInput -and $null -ne $startupProof){
        $proofHost=@($children|Where-Object{$_.Name -ceq 'real-native-session-host'})
        $proofHostId=if($proofHost.Count -eq 1 -and $proofHost[0].Started -ceq 'Yes'){$proofHost[0].Process.Id}else{0}
        Add-M4StartupOwnerProof $startupProof $receipt $Owner $Line $runId $proofHostId
    }
    if($Owner -ceq 'real-phone-controller'){
        Add-M4RenderObservationEvidence $renderObservationEvidence $receipt $Line
        if($ControllerInput){Add-M4InputObservationEvidence $inputObservationEvidence $receipt.input_observations $Line}
    }
    if($Owner -ceq 'real-native-session-host' -and $Line -cmatch '^REMOTE_HOST_SELECTION:index=([0-2]);epoch=([0-9]{1,20})$'){
        $index=[int]$Matches[1];$epoch=[UInt64]0
        if([UInt64]::TryParse($Matches[2],[ref]$epoch) -and $epoch -gt 0 -and $epoch%2 -eq 1 -and @($receipt.host_source_selections).Count -lt 3){$receipt.host_source_selections=@($receipt.host_source_selections)+@([pscustomobject]@{index=$index;epoch=[string]$epoch})}
        return
    }
    if($ControllerInput -and $Owner -ceq 'real-native-session-host' -and $Line -ceq 'REMOTE_HOST_PHASE:owned_input_grant_attempted'){$receipt.input_grant_attempted=$true;$receipt.input_granted=$null}
    if($ControllerInput -and $Owner -ceq 'real-native-session-host' -and $Line -ceq 'REMOTE_HOST_PHASE:actual_owned_input_granted'){$receipt.input_grant_attempted=$true;$receipt.input_granted=$true}
    if($ControllerInput -and $Owner -ceq 'real-phone-controller' -and $Line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_input_dispatch_attempted'){$receipt.input_attempted=$true;$receipt.input_dispatch_completed=$null}
    if($ControllerInput -and $Owner -ceq 'real-phone-controller' -and $Line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_input_dispatch_completed'){$receipt.input_attempted=$true;$receipt.input_dispatch_completed=$true}
    if($Owner -ceq 'real-phone-controller' -and $Line -ceq 'INSTRUMENTATION_STATUS: remote_phase=actual_phone_owners_retired'){$script:phoneRetired=$true}
    if($Owner -ceq 'real-native-session-host' -and (Add-M4HostCarrierObservation $receipt $Line)){return}
    # Aggregate counters remain available even when the diagnostic ring rotates.
    # Closed names and integer range exclude arbitrary native output or identifiers.
    if($Owner -ceq 'real-phone-controller' -and $Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_stream_([a-z_]{1,40})=([0-9]{1,20})$'){
        $key=$Matches[1];$number=[UInt64]0
        if($streamCounterKeys -ccontains $key -and [UInt64]::TryParse($Matches[2],[ref]$number)){
            if(-not $streamCounters.phone.ContainsKey($key) -or $number -gt $streamCounters.phone[$key]){$streamCounters.phone[$key]=$number}
        }
        return
    }
    if($Owner -ceq 'real-native-session-host' -and $Line.StartsWith('REMOTE_HOST_STREAM:')){
        $json=$Line.Substring('REMOTE_HOST_STREAM:'.Length)
        if($json.Length -le 4096 -and $json -cmatch '^\{(?:"[a-z_]{1,40}":[0-9]{1,20})(?:,"[a-z_]{1,40}":[0-9]{1,20})*\}$'){
            $pairs=[regex]::Matches($json,'"([a-z_]{1,40})":([0-9]{1,20})')
            $seen=@{};$values=@{};$valid=$pairs.Count -eq $hostStreamCounterKeys.Count
            foreach($pair in $pairs){
                $key=$pair.Groups[1].Value;$number=[UInt64]0
                if($seen.ContainsKey($key) -or $hostStreamCounterKeys -cnotcontains $key -or -not [UInt64]::TryParse($pair.Groups[2].Value,[ref]$number)){$valid=$false;break}
                $seen[$key]=$true;$values[$key]=$number
            }
            if($valid){foreach($key in $values.Keys){if(-not $streamCounters.host.ContainsKey($key) -or $values[$key] -gt $streamCounters.host[$key]){$streamCounters.host[$key]=$values[$key]}}}
        }
        return
    }
    $safe=Get-IntegrationDiagnosticLine $Line
    if($null -eq $safe){return}
    $script:diagnosticTotal++
    if($diagnostics.Count -eq 64){$diagnostics.RemoveAt(0);$script:diagnosticDropped++}
    $diagnostics.Add([pscustomobject]@{owner=$Owner;text=$safe})
}
function Drain-IntegrationDiagnostics($Owner){
    $line=$null
    while($Owner.Output.Lines.TryDequeue([ref]$line)){Add-IntegrationDiagnostic $Owner.Name $line}
}
function Get-IntegrationFailureCode([string]$Message){
    switch -CaseSensitive ($Message){
        'Real phone integration deadline exceeded.' {return 'phone-runner-deadline'}
        'Real host exited during phone integration.' {return 'host-exited'}
        'Real phone instrumentation reported failure.' {return 'phone-instrumentation-failed'}
        'Exact real-path case/render/retirement census failed.' {return 'phone-case-census'}
        'Fresh decoder/capture ownership census failed.' {return 'phone-ownership-census'}
        'Owned native/session readiness timed out.' {return 'native-readiness-deadline'}
        'Owned child exited before expected readiness.' {return 'native-readiness-child-exited'}
        'Host actual retirement failed.' {return 'host-retirement'}
        'Host native helper ownership not retired.' {return 'host-native-retirement'}
        'Exact actual controller input/host census failed.' {return 'controller-input-census'}
        'Owned input receiver retirement failed.' {return 'controller-receiver-retirement'}
        'Owned input receiver census exceeds bound.' {return 'controller-receiver-bound'}
        'Owned input receiver category census failed.' {return 'controller-receiver-categories'}
        'Owned receiver journal exceeds bound.' {return 'controller-receiver-journal-bound'}
        'Unexpected/duplicate actual controller input record.' {return 'controller-input-record'}
        'Unexpected actual controller input phase.' {return 'controller-input-phase'}
        'Unexpected actual controller input completion.' {return 'controller-input-completion'}
        'Unexpected controller input receipt in viewing-only route.' {return 'controller-input-unrequested'}
        'Finite phase or actual process/stream retirement failed.' {return 'finite-phase-or-retirement'}
        'Actual controller input validation rejected observed evidence.' {return 'controller-input-validation-rejected'}
        'Exact render validation rejected observed records.' {return 'render-records-rejected'}
        'Exact render validator response unavailable.' {return 'render-validator-response'}
        'Exact render observation census differs from case records.' {return 'render-observation-census'}
        default {return 'unclassified-failure'}
    }
}
function Hash-Bytes([byte[]]$bytes){[Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()}
function Assert-Plain([string]$path){
    $item=Get-Item -LiteralPath ([IO.Path]::GetFullPath($path)) -Force
    while($null -ne $item){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint)-ne 0){throw 'Redirected integration path refused.'};$item=if($item -is [IO.DirectoryInfo]){$item.Parent}else{$item.Directory}}
}
function Pin-File([string]$path,[string]$hash){
    if($hash -cnotmatch '^[a-f0-9]{64}$'){throw 'Artifact digest refused.'};Assert-Plain $path
    $ancestor=[IO.Directory]::GetParent([IO.Path]::GetFullPath($path))
    while($null -ne $ancestor){if($directoryPins.Add($ancestor.FullName)){$pins.Add([VwRemoteIntegrationDirectoryLeaseV1]::Open($ancestor.FullName))};$ancestor=$ancestor.Parent}
    $f=[IO.File]::Open([IO.Path]::GetFullPath($path),[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try{if($f.Length -le 0 -or $f.Length -gt 2GB){throw 'Artifact length refused.'};$digest=[Security.Cryptography.SHA256]::Create();try{$actual=[Convert]::ToHexString($digest.ComputeHash($f)).ToLowerInvariant()}finally{$digest.Dispose()};if($actual -cne $hash){throw 'Fresh artifact/source digest mismatch.'};$f.Position=0;$pins.Add($f)}catch{$f.Dispose();throw}
}
function Run([string]$name,[string]$file,[string[]]$arguments,[int]$seconds=30,[switch]$AllowNonzero){
    $script:phase=$name
    # SensitiveCapture stores no command output in its log/UI. Keep route JSON
    # raw only in memory so exact private address predicates remain meaningful.
    $redact=@();if($null -ne $bootstrap){$redact+=$bootstrap}
    $r=Invoke-VwProcess -FilePath $file -ArgumentList $arguments -WorkingDirectory $projectRoot -Phase $name -TimeoutSeconds $seconds -SensitiveCapture -RedactValues $redact
    $proof=Get-Content -Raw -LiteralPath ($r.LogPath+'.json')|ConvertFrom-Json
    $clean=$proof.process_tree_cleanup_confirmed -eq $true -and $proof.output_streams_completed -eq $true
    $script:allClean=$allClean -and $clean
    if((-not $AllowNonzero -and $r.ExitCode -ne 0) -or -not $clean){throw 'Finite phase or actual process/stream retirement failed.'};return $r
}
function Save-M4ControllerValidationReport($Result){
    $text=$Result.Lines -join "`n"
    if($text.Length -gt 131072){throw 'Bounded controller validator response unavailable.'}
    try{$report=$text|ConvertFrom-Json -ErrorAction Stop}catch{throw 'Bounded controller validator response unavailable.'}
    if($report.schema -ne 2 -or $report.status -cnotin @('passed','rejected','observed') -or $report.records_validated -isnot [bool] -or $report.rejection_reason -cnotin @('arguments','receipt_file','receipt_file_bytes','duplicate_json_key','native_integer','no_mouse_contamination','exact_pointer_identity','pointer_transition_flags','cancelled_pointer','one_contact_down','one_contact_up','contact_update_before_up','only_noncontact_update_after_up','complete_pointer_contact','input_census','input_shape','input_binding','input_scope','input_session','input_decoder_owner','actual_three_sequence_census','covering_frame','covering_ticket','covering_pts','input_timing','actual_ack_fade_before_creation_expiry','matching_echo_coverage','software_input_limits','receiver_categories','balanced_native_pointer_receiver','receiver_event_census','receiver_event_shape','receiver_clock_shape','receiver_clock_actual','receiver_count_order','actual_pointer_message','actual_pointer_info','unaltered_pen_flags','native_pointer_qpc_order','all_six_receiver_journal_categories','receiver_provenance_shape','scenario','render_census','record_shape','scope_shape','scope_uuid','owner_uuid','integer_shape','integer_bound','codec_name','timing_shape','local_callback_order','capability_shape','fresh_owner_capture_scope','background_and_reconnect_epochs','carrier_loss_and_reconnect_epochs','none','unclassified','not_validated') -or $report.receiver_evidence.schema -ne 1 -or @($report.receiver_evidence.events).Count -gt 64 -or $report.receiver_evidence.acceptance -ne $false -or $report.receiver_evidence.records_validated -ne $false){throw 'Bounded controller validator response unavailable.'}
    # Source-bound Python emits only closed numeric/provenance fields and labels.
    # Retain rejection/evidence before the caller throws and cleanup disposes raw.
    $receipt.receiver_journal_evidence=$report.receiver_evidence
    if($report.status -cne 'observed'){$receipt.controller_input_validation_reason=$report.rejection_reason}
    return $report
}
function Save-M4HeldValidationReport($Result){
    $text=$Result.Lines -join "`n"
    if($text.Length -gt 65536){throw 'Bounded held validator response unavailable.'}
    try{$report=$text|ConvertFrom-Json -ErrorAction Stop}catch{throw 'Bounded held validator response unavailable.'}
    if($report.schema -ne 3 -or $report.status -cnotin @('passed','rejected','observed') -or $report.records_validated -isnot [bool] -or $report.rejection_reason -cnotin @('actual_ack_fade_before_creation_expiry','actual_cause_after_gate_same_clock','actual_device_release_after_requested_cause','actual_host_cause_binding','actual_host_cause_run','actual_host_cause_shape','actual_native_prefix_deadline','actual_native_safety_release','actual_pointer_info','actual_pointer_message','actual_receive_clock','actual_requested_host_cause','actual_three_sequence_census','all_six_receiver_journal_categories','arguments','background_and_reconnect_epochs','balanced_native_pointer_receiver','cancelled_pointer','capability_shape','carrier_loss_and_reconnect_epochs','cause_binding_shape','cause_scope_shape','clock_shape','codec_name','complete_journal','complete_pointer_contact','contact_update_before_up','covering_frame','covering_pts','covering_ticket','down_move_no_up_before_trigger','duplicate_json_key','event_shape','exact_native_release_held_binding','exact_pointer_identity','fresh_owner_capture_scope','fresh_ready_receipt','held_prefix_already_invalid_or_released','initial_grant_binding','input_binding','input_census','input_decoder_owner','input_scope','input_session','input_shape','input_timing','integer_bound','integer_shape','journal_bound','journal_file','journal_file_bytes','lifecycle_census','lifecycle_shape','local_callback_order','matching_echo_coverage','native_integer','native_pointer_qpc_order','native_release_shape','no_mouse_contamination','no_phone_up','none','not_validated','one_contact_down','one_contact_up','only_noncontact_update_after_up','owner_uuid','pointer_info','pointer_order','pointer_transition_flags','receipt_file','receipt_file_bytes','receiver_categories','receiver_clock_actual','receiver_clock_shape','receiver_count_order','receiver_counts','receiver_event_census','receiver_event_shape','receiver_provenance_shape','record_shape','release_strictly_after_trigger','render_census','retirement_and_no_regrant','same_host_qpc_clock','scenario','scenario_binding','scope_shape','scope_uuid','software_input_limits','timing_shape','trigger_binding','trigger_shape','unaltered_pen_flags','unchanged_native_held_prefix','unclassified') -or $report.held_evidence.schema -ne 1 -or @($report.held_evidence.records).Count -gt 4 -or $report.held_evidence.acceptance -ne $false -or $report.held_evidence.records_validated -ne $false){throw 'Bounded held validator response unavailable.'}
    $receipt.held_lifecycle_evidence=$report.held_evidence
    # These are sanitized observations until the caller proves strict success.
    $receipt.held_lifecycle_records=@($report.held_evidence.records)
    if($report.status -cne 'observed'){$receipt.held_lifecycle_validation_reason=$report.rejection_reason}
    return $report
}
function Adb([string]$name,[string[]]$arguments,[int]$seconds=30){Run $name $device.AdbPath (@('-s',$device.Serial)+$arguments) $seconds}
function Start-Owned([string]$name,[string]$file,[string[]]$arguments,[bool]$visible=$false){
    $info=[Diagnostics.ProcessStartInfo]::new($file);$info.WorkingDirectory=$projectRoot;$info.UseShellExecute=$false;$info.CreateNoWindow=-not $visible
    foreach($arg in $arguments){[void]$info.ArgumentList.Add($arg)}
    $info.RedirectStandardOutput=$true;$info.RedirectStandardError=$true
    foreach($key in @($info.Environment.Keys)){if($key -match '(?i)(TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY)'){[void]$info.Environment.Remove($key)}}
    $redact=@($owned,$projectRoot,$PcUsbAddress,$PhoneUsbAddress,$env:USERPROFILE)
    if($null -ne $device){$redact+=$device.Serial};if($null -ne $bootstrap){$redact+=$bootstrap}
    if($ControllerInput){
        $p=[Diagnostics.Process]::new();$p.StartInfo=$info
        $output=[VwInputOutputR57]::new($redact);$job=[VwProcessJobV4]::new()
        $entry=New-M4InputChild $name $p $job $output
        $children.Add($entry)
        if($name -ceq 'real-phone-controller'){$entry.GateState='Unknown'}
        Start-M4InputChild $entry
        return $entry
    }
    $output=[VwProcessOutputV4]::new($redact);$p=[Diagnostics.Process]::new();$p.StartInfo=$info;$output.Attach($p)
    $job=[VwProcessJobV4]::new();$entry=[pscustomobject]@{Name=$name;Process=$p;Job=$job;Output=$output;Assigned=$false}
    $children.Add($entry)
    if(-not $p.Start()){throw 'Owned process did not start.'}
    try{$job.Assign($p);$entry.Assigned=$true}catch{try{$p.Kill($true);[void]$p.WaitForExit(5000)}catch{};throw}
    $p.BeginOutputReadLine();$p.BeginErrorReadLine();return $entry
}
function Await-File([string]$name,[int]$seconds,$Producer){
    $script:phase=$name;$clock=[Diagnostics.Stopwatch]::StartNew();$next=0
    $path=Join-Path $owned $name
    while(-not(Test-Path -LiteralPath $path -PathType Leaf)){
        if($clock.Elapsed.TotalSeconds -ge $seconds){throw 'Owned native/session readiness timed out.'}
        foreach($child in $children){Drain-IntegrationDiagnostics $child}
        # The phone may already have completed normally before host retirement.
        # Only this file's actual producer owns readiness liveness. Recheck after
        # exit because publication and exit can occur between these observations.
        if($Producer.Process.HasExited -and -not(Test-Path -LiteralPath $path -PathType Leaf)){throw 'Owned child exited before expected readiness.'}
        if($clock.Elapsed.TotalSeconds -ge $next){Write-Host "Remote integration phase=$name waiting=$([int]$clock.Elapsed.TotalSeconds)s";$next+=2};Start-Sleep -Milliseconds 25
    };Assert-Plain $path;return $path
}
function Test-SelectedPhoneRoute([string[]]$Lines,[string]$Destination,[string]$Source){
    # Both forms are observed native output. Neither an ignored -j flag nor
    # text from another command/device can relax the exact selected USB route.
    if($Lines.Count -eq 0 -or $Lines.Count -gt 128){return $false}
    $body=$Lines -join "`n"
    if($body.Length -eq 0 -or $body.Length -gt 8192){return $false}
    if($body.TrimStart().StartsWith('[')){
        try{$routes=@($body|ConvertFrom-Json -Depth 16 -ErrorAction Stop)}catch{return $false}
        if($routes.Count -ne 1 -or $null -eq $routes[0]){return $false}
        $fields=$routes[0].PSObject.Properties.Name
        if(-not($fields -ccontains 'dst') -or -not($fields -ccontains 'dev') -or -not($fields -ccontains 'prefsrc')){return $false}
        return ($routes[0].dst -is [string]) -and ($routes[0].dev -is [string]) -and ($routes[0].prefsrc -is [string]) -and $routes[0].dst -ceq $Destination -and $routes[0].dev -ceq 'rndis0' -and $routes[0].prefsrc -ceq $Source
    }
    # Selected S23 toybox ignores -j only for route-get and reports precisely
    # this two-line grammar. Reject extra tokens/lines and overflowing numbers.
    if($Lines.Count -ne 2){return $false}
    $first=$Lines[0].Trim([char[]]@(' ',"`t"));$second=$Lines[1].Trim([char[]]@(' ',"`t"))
    if($second -cne 'cache' -or $first -cnotmatch '\A(?<destination>(?:[0-9]{1,3}\.){3}[0-9]{1,3})[ \t]+dev[ \t]+rndis0[ \t]+table[ \t]+(?<table>0|[1-9][0-9]{0,9})[ \t]+src[ \t]+(?<source>(?:[0-9]{1,3}\.){3}[0-9]{1,3})[ \t]+uid[ \t]+(?<uid>0|[1-9][0-9]{0,9})\z'){return $false}
    [uint32]$table=0;[uint32]$uid=0
    return $Matches.destination -ceq $Destination -and $Matches.source -ceq $Source -and [uint32]::TryParse($Matches.table,[ref]$table) -and [uint32]::TryParse($Matches.uid,[ref]$uid)
}
function Assert-UsbRoute {
    $callerPhase=$script:phase
    $script:phase='physical_usb_route'
    $adapter=@(Get-NetAdapter -IncludeHidden|Where-Object{$_.ifIndex -eq $UsbInterfaceIndex})
    if($adapter.Count -ne 1 -or $adapter[0].Status -ne 'Up'){throw 'USB adapter unavailable; no carrier substitution.'}
    $nic=@(Get-CimInstance Win32_NetworkAdapter|Where-Object{$_.InterfaceIndex -eq $UsbInterfaceIndex})
    if($nic.Count -ne 1 -or -not $nic[0].PhysicalAdapter -or -not $nic[0].NetEnabled){throw 'USB hardware adapter not enabled/present.'}
    $node=$nic[0].PNPDeviceID;$matched=$false
    for($n=0;$n -lt 8 -and $node;$n++){
        if($node -match '^USB\\VID_04E8&PID_' -and (Hash-Bytes ([Text.Encoding]::UTF8.GetBytes($node))) -ceq $UsbDeviceInstanceSha256){$matched=$true;break}
        $parent=Get-PnpDeviceProperty -InstanceId $node -KeyName 'DEVPKEY_Device_Parent' -ErrorAction Stop;$node=[string]$parent.Data
    }
    if(-not $matched){throw 'Selected Samsung USB device ancestor does not match root-pinned instance.'}
    $ip=@(Get-NetIPAddress -InterfaceIndex $UsbInterfaceIndex -AddressFamily IPv4|Where-Object{$_.IPAddress -ceq $PcUsbAddress -and $_.AddressState -eq 'Preferred'})
    if($ip.Count -ne 1){throw 'Selected USB IPv4 unavailable.'}
    $route=@(Find-NetRoute -RemoteIPAddress $PhoneUsbAddress|Where-Object{$_.PSObject.Properties.Name -contains 'InterfaceIndex'})
    if(-not $route -or @($route|Where-Object{$_.InterfaceIndex -ne $UsbInterfaceIndex}).Count -ne 0){throw 'Windows route does not select the explicit USB adapter.'}
    $state=Adb 'phone-usb-config' @('shell','getprop','sys.usb.state')
    if((($state.Lines -join '').Trim().Split(',')) -cnotcontains 'rndis'){throw 'Selected phone RNDIS state unavailable.'}
    $phone=Adb 'phone-usb-interface' @('shell','ip','-j','address','show','dev','rndis0')
    $interfaceJson=$phone.Lines -join ''
    if($interfaceJson.Length -eq 0 -or $interfaceJson.Length -gt 65536 -or -not $interfaceJson.TrimStart().StartsWith('[')){throw 'Bounded phone interface JSON unavailable.'}
    $interfaces=@($interfaceJson|ConvertFrom-Json -Depth 16)
    if($interfaces.Count -eq 0 -or $interfaces.Count -gt 128){throw 'Phone interface JSON census refused.'}
    # Actual selected-device toybox can return other interface/addr_info rows
    # despite dev rndis0. Only one exact native ifname row has route authority.
    $selected=@($interfaces|Where-Object{($_.PSObject.Properties.Name -ccontains 'ifname') -and $_.ifname -ceq 'rndis0'})
    if($selected.Count -ne 1 -or $selected[0].operstate -cne 'UP' -or @($selected[0].addr_info|Where-Object{$_.family -ceq 'inet' -and $_.local -ceq $PhoneUsbAddress}).Count -ne 1){throw 'Phone rndis0 address/link is unavailable.'}
    $actual=Adb 'phone-usb-route' @('shell','ip','-j','route','get',$PcUsbAddress)
    if(-not(Test-SelectedPhoneRoute -Lines @($actual.Lines) -Destination $PcUsbAddress -Source $PhoneUsbAddress)){throw 'Phone route is not the exact selected USB destination/source/interface.'}
    $receipt.physical_route_verified=$true
    # Only successful rechecks restore the actual caller wait stage. Failed
    # route predicates retain their exact inner phase for the failure receipt.
    $script:phase=$callerPhase
}
function Check-Installed([string]$package,[string]$digest){
    $v=Adb 'owned-package-path' @('shell','pm','path','--user','0',$package)
    $lines=@($v.Lines|Where-Object{$_.Trim()});if($lines.Count -ne 1 -or $lines[0] -cnotmatch '^package:(/data/app/[A-Za-z0-9/_=+~.-]+/base\.apk)$'){return $false}
    $v=Adb 'owned-installed-digest' @('shell','sha256sum',$Matches[1]);$s=($v.Lines -join '').Trim()
    return $s -cmatch '^([a-f0-9]{64})\s+' -and $Matches[1] -ceq $digest
}
try {
    if($ControllerInput){
        . (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
        . (Join-Path $PSScriptRoot 'input_owner_startup.ps1')
        Initialize-M4InputReaders
        $startupProof=New-M4StartupOwnerProof
        $script:phase='retained-input-parent-gate'
        $gateClock=[Diagnostics.Stopwatch]::StartNew()
        while(-not(Test-Path -LiteralPath $RetainedInputGate -PathType Leaf)){
            if($gateClock.Elapsed.TotalSeconds -ge 10){throw 'Retained input parent gate unavailable.'}
            if([int]$gateClock.Elapsed.TotalMilliseconds % 2000 -lt 100){Write-Host 'Waiting for actual retained parent Job assignment'};Start-Sleep -Milliseconds 100
        }
        Assert-Plain $RetainedInputGate
        if((Get-Item -LiteralPath $RetainedInputGate).Length -gt 1024){throw 'Retained input parent gate refused.'}
        $gate=Get-Content -Raw -LiteralPath $RetainedInputGate|ConvertFrom-Json
        $self=[Diagnostics.Process]::GetCurrentProcess()
        $nativeParent=Get-CimInstance Win32_Process -Filter ('ProcessId='+$PID)
        $parent=[Diagnostics.Process]::GetProcessById([int]$nativeParent.ParentProcessId)
        try{
            if(-not(Test-M4InputParentGate $gate $RetainedInputNonce $PID ([string]$self.StartTime.ToUniversalTime().ToFileTimeUtc()) $parent.Id ([string]$parent.StartTime.ToUniversalTime().ToFileTimeUtc()))){throw 'Retained input parent gate refused.'}
        }finally{$self.Dispose();$parent.Dispose()}
    }
    Pin-File $ArtifactManifest $ArtifactManifestSha256
    $artifact=Get-Content -Raw -LiteralPath $ArtifactManifest|ConvertFrom-Json
    if($artifact.schema -ne 1 -or ((@($artifact.native.PSObject.Properties.Name|Sort-Object) -join ',') -cne (@($names|Sort-Object) -join ','))){throw 'Exact six-native inventory refused.'}
    $fixtureFeatures=@($artifact.android_fixture_features)
    if($null -eq $artifact.android_fixture_features -or ($fixtureFeatures.Count -ne 0 -and ($fixtureFeatures.Count -ne 1 -or $fixtureFeatures[0] -cne 'integration-carrier-fault'))){throw 'Closed fixture feature inventory required.'}
    $hostFixtureFeatures=@($artifact.host_fixture_features)
    if($null -eq $artifact.host_fixture_features -or ($hostFixtureFeatures.Count -ne 0 -and ($hostFixtureFeatures.Count -ne 1 -or $hostFixtureFeatures[0] -cne 'integration-carrier-fault'))){throw 'Closed host fixture feature inventory required.'}
    if($InputScenario -cne 'balanced' -and ($fixtureFeatures -cnotcontains 'integration-carrier-fault' -or $hostFixtureFeatures -cnotcontains 'integration-carrier-fault')){throw 'Held scenarios require explicit host and Android fixture features before input.'}
    if(@($artifact.sources).Count -lt 25 -or @($artifact.classpath_bindings).Count -lt 10){throw 'Concrete source/classpath bindings missing.'}
    if($artifact.inventory_only -ne $true -or $artifact.root_build_review_required -ne $true -or @($artifact.build_receipts).Count -lt 3){throw 'Root-reviewed fresh build receipt bindings required.'}
    Pin-File $artifact.source_map.path $artifact.source_map.sha256
    foreach($item in $artifact.build_receipts){Pin-File $item.path $item.sha256}
    $requiredSources=@('tools/remote-edit/input_observation_evidence.ps1','tools/remote-edit/test_input_observation_evidence.ps1','tools/remote-edit/receiver_observation.ps1','tools/remote-edit/test_receiver_observation.ps1','tools/remote-edit/test_integration_diagnostics.ps1','apps/desktop/src/test/kotlin/com/visualworkbench/desktop/RemoteIntegrationPublicationTest.kt','tools/remote-edit/test_integration_producer_wait.ps1','tools/remote-edit/test_startup_owner_proofs.ps1','tools/ffi-test/core-unit.ps1','tools/ffi-test/test_core_unit_census.ps1','tools/remote-edit/android_entrypoint_preflight.py','tools/remote-edit/test_android_entrypoint_preflight.py','tools/remote-edit/remote_instrumentation_contract.json','tools/remote-edit/render_observation_evidence.ps1','tools/remote-edit/test_render_observation_evidence.ps1','apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteEditController.kt','apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteHardwareDecoder.kt','apps/android/src/main/kotlin/com/visualworkbench/android/remote/RemoteSurfaceView.kt','apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/NativeRemoteEdit.kt','core/crates/vw-ffi/src/session/remote_edit.rs','core/crates/vw-ffi/src/session/remote_host.rs','host-win/crates/vw-remote-host/src/process.rs','host-win/crates/vw-remote-host/src/process/windows.rs','host-win/crates/vw-remote-host/src/process/input_retirement.rs','host-win/crates/vw-remote-host/src/platform/native_guard/release_witness.rs','host-win/crates/vw-remote-host/src/platform/input.rs','host-win/crates/vw-remote-host/src/platform/video.rs','tools/remote-edit/controller_lifecycle_reports.py','tools/remote-edit/test_controller_lifecycle_reports.py','tools/remote-edit/lifecycle_clock.cs','core/crates/vw-ffi/Cargo.toml','core/crates/vw-ffi/src/session/remote_integration_fault.rs','apps/shared/src/jvmMain/kotlin/com/visualworkbench/shared/RemoteIntegrationCarrierFault.kt','tools/remote-edit/input_owner_contracts.ps1','tools/remote-edit/input_owner_startup.ps1','tools/remote-edit/input_process_output.cs','tools/remote-edit/test_input_owner_startup.ps1','tools/remote-edit/integration_input_owner.ps1','tools/remote-edit/test_input_owner_contracts.ps1','apps/desktop/src/main/kotlin/com/visualworkbench/desktop/RemoteIntegrationHost.kt','apps/android/src/remoteIntegrationTest/kotlin/com/visualworkbench/android/remote/RemoteNormalPathInstrumentedTest.kt')
    foreach($name in $requiredSources){if(@($artifact.sources|Where-Object{$_.path -ceq $name}).Count -ne 1){throw 'Critical normal remote pipeline source missing.'}}
    foreach($item in $artifact.sources){
        if($item.path -cnotmatch '^[A-Za-z0-9_./-]+$' -or $item.path -match '(?:^|/)\.\.(?:/|$)'){throw 'Source inventory path refused.'}
        Pin-File (Join-Path $projectRoot $item.path) $item.sha256
    };$receipt.source_count=@($artifact.sources).Count
    foreach($item in $artifact.classpath_bindings){Pin-File $item.path $item.sha256}
    Pin-File $artifact.classpath.path $artifact.classpath.sha256;Pin-File $artifact.java.path $artifact.java.sha256
    Pin-File $artifact.harness.path $artifact.harness.sha256
    foreach($key in @('main','test')){Pin-File $artifact.apk.$key.path $artifact.apk.$key.sha256}
    foreach($name in $names){Pin-File (Join-Path $projectRoot ('target/debug/'+$name)) $artifact.native.$name}
    $classpath=(Get-Content -Raw -LiteralPath $artifact.classpath.path).Trim()
    if(-not $classpath -or $classpath.Length -gt 65536){throw 'Classpath refused.'}
    # Every directory entry/file used by Java must be covered by the locked
    # root-admitted classpath inventory; arbitrary extra bytecode is refused.
    $bound=@{};foreach($item in $artifact.classpath_bindings){$bound[[IO.Path]::GetFullPath($item.path)]=$true}
    $seen=0
    foreach($path in $classpath.Split([IO.Path]::PathSeparator)){
        Assert-Plain $path
        if(Test-Path -LiteralPath $path -PathType Container){foreach($file in Get-ChildItem -LiteralPath $path -File -Recurse -Force){Assert-Plain $file.FullName;if(-not $bound.ContainsKey($file.FullName)){throw 'Unlisted Java classpath content.'};$seen++;if($seen -gt 32768){throw 'Classpath census limit.'}}}
        elseif(-not $bound.ContainsKey([IO.Path]::GetFullPath($path))){throw 'Unlisted Java dependency.'}
    }
    # Inspect the final pinned test APK before device discovery/installation.
    # A green ordinary JVM suite does not validate androidTest JUnit signatures.
    $preflight=Run 'instrumentation-entrypoint-preflight' 'python.exe' @('tools/remote-edit/android_entrypoint_preflight.py','--apk-test',$artifact.apk.test.path,'--contract',(Join-Path $projectRoot 'tools/remote-edit/remote_instrumentation_contract.json'),'--selection',$instrumentationSelection) 60
    $entrypoint=($preflight.Lines -join "`n")|ConvertFrom-Json
    $boundEntrypoint=$artifact.entrypoint_preflight
    foreach($value in @($entrypoint,$boundEntrypoint)){
        if($null -eq $value -or $value.schema -ne 1 -or $value.status -cne 'passed' -or $value.apk_sha256 -cne $artifact.apk.test.sha256 -or $value.selection -cne $instrumentationSelection -or $value.test_count -ne 1 -or $value.tests_executed -ne $false){throw 'Final APK instrumentation entrypoint preflight refused.'}
    }
    if($entrypoint.contract_sha256 -cne $boundEntrypoint.contract_sha256 -or (@($entrypoint.methods) -join ',') -cne (@($boundEntrypoint.methods) -join ',')){throw 'Instrumentation contract binding changed.'}
    $device=Get-VwAndroidDevice -Root $projectRoot -ExpectedModel 'SM-S918U'
    Assert-UsbRoute
    $v=Adb 'owner-user' @('shell','am','get-current-user');if(($v.Lines -join '').Trim() -cne '0'){throw 'Owner user0 required.'}
    $v=Adb 'profile-inventory' @('shell','pm','list','users');$profileCount=Get-VwAppHilProfileCount -Lines $v.Lines
    foreach($package in $packages){$v=Adb 'global-test-package-absence' @('shell','dumpsys','package',$package);if(-not(Test-VwAppHilPackageAbsent -Package $package -Lines $v.Lines)){throw 'Existing HIL package preserved; refusing integration.'};$preAbsent[$package]=$true}
    [void][IO.Directory]::CreateDirectory($owned);$created=$true;Assert-Plain $owned
    [IO.File]::WriteAllText((Join-Path $owned '.owner-v1'),$runId)
    # Prove the APK's actual target before any install; a digest alone cannot
    # turn a normal app APK into an isolated package or preserve owner data.
    $sdk=if($env:ANDROID_HOME){$env:ANDROID_HOME}elseif($env:ANDROID_SDK_ROOT){$env:ANDROID_SDK_ROOT}else{Join-Path $env:LOCALAPPDATA 'Android/Sdk'}
    foreach($key in @('main','test')){
        $xml=Run 'verify-isolated-apk-manifest' (Join-Path $sdk 'cmdline-tools/latest/bin/apkanalyzer.bat') @('manifest','print',$artifact.apk.$key.path) 60
        $doc=[xml]($xml.Lines -join "`n");$expected=if($key -ceq 'main'){$packages[0]}else{$packages[1]}
        if($doc.manifest.package -cne $expected){throw 'APK belongs to a different application; no installation.'}
        if($key -ceq 'test'){
            $entries=@($doc.manifest.instrumentation)
            if($entries.Count -ne 1 -or $entries[0].GetAttribute('targetPackage','http://schemas.android.com/apk/res/android') -cne $packages[0] -or $entries[0].GetAttribute('name','http://schemas.android.com/apk/res/android') -cne 'androidx.test.runner.AndroidJUnitRunner'){throw 'Actual instrumentation target refused.'}
        }
        [void](Run 'verify-apk-signature' (Join-Path $sdk 'build-tools/36.1.0/apksigner.bat') @('verify','--verbose',$artifact.apk.$key.path) 60)
    }
    $zip=[IO.Compression.ZipFile]::OpenRead($artifact.apk.main.path)
    try{
        $entries=@($zip.Entries|Where-Object{$_.FullName -ceq 'lib/arm64-v8a/libvw_core.so'})
        if($entries.Count -ne 1){throw 'Exact ARM64 core entry unavailable.'}
        $stream=$entries[0].Open();$digest=[Security.Cryptography.SHA256]::Create()
        try{$actual=[Convert]::ToHexString($digest.ComputeHash($stream)).ToLowerInvariant()}finally{$stream.Dispose();$digest.Dispose()}
        if($actual -cne $artifact.android_core_sha256){throw 'Android native core differs from root-admitted fresh build.'}
    }finally{$zip.Dispose()}
    foreach($key in @('main','test')){
        $package=if($key -ceq 'main'){$packages[0]}else{$packages[1]}
        # ADB timeout can occur after actual installation. Retain its reviewed
        # expected digest before invoking the command, independent of success text.
        $attempted[$package]=$artifact.apk.$key.sha256
        $v=Adb 'install-owned-integration' @('install','--user','0','-t',$artifact.apk.$key.path) 120
        if(($v.Lines -join "`n") -cnotmatch '(?m)^Success\s*$'){throw 'APK install success unavailable.'}
        $installed[$package]=$artifact.apk.$key.sha256
        if(-not(Check-Installed $package $installed[$package])){throw 'Installed exact APK mismatch.'}
    }
    if($InputScenario -cne 'balanced'){[IO.File]::WriteAllText((Join-Path $owned 'held-scenario'),$InputScenario,[Text.UTF8Encoding]::new($false))}
    if($ControllerInput){[IO.File]::WriteAllText((Join-Path $owned 'controller-input-owner'),$runId,[Text.UTF8Encoding]::new($false))}
    $surfaceMode=if($ControllerInput){'--surface-controller'}else{'--surface'}
    $surface=Start-Owned 'owned-wgc-target' $artifact.harness.path @($surfaceMode,(Join-Path $owned 'window'),'80') $true
    $script:phase='owned_target_readiness';$clock=[Diagnostics.Stopwatch]::StartNew()
    $targetFile=Join-Path $owned 'window/target.json'
    while(-not(Test-Path -LiteralPath $targetFile -PathType Leaf)){if($surface.Process.HasExited -or $clock.Elapsed.TotalSeconds -ge 15){throw 'Owned target native readiness failed.'};Write-Host 'Waiting for owned capture target';Start-Sleep -Milliseconds 250}
    Assert-Plain $targetFile;$target=Get-Content -Raw -LiteralPath $targetFile|ConvertFrom-Json
    if($target.process_id -ne $surface.Process.Id){throw 'Target child/native PID mismatch.'}
    $config=@('M4_REMOTE_INTEGRATION_V1',$PcUsbAddress,[string]$target.window,[string]$target.process_id,[string]$target.process_created,[string]$Port)
    foreach($name in $names){$config+=[string]$artifact.native.$name}
    $configPath=Join-Path $owned 'host-config.txt';[IO.File]::WriteAllLines($configPath,$config,[Text.UTF8Encoding]::new($false))
    $hostOwner=Start-Owned 'real-native-session-host' $artifact.java.path @('-cp',$classpath,'com.visualworkbench.desktop.RemoteIntegrationHost',$configPath,$runId)
    if($ControllerInput){
        if(-not $hostOwner.Assigned -or @($hostOwner.Job.ActiveProcessIds) -notcontains $hostOwner.Process.Id){throw 'Actual host parent Job gate failed.'}
        $self=[Diagnostics.Process]::GetCurrentProcess()
        try{
            $hostCreated=[DateTimeOffset]::new($hostOwner.Process.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
            $runnerCreated=[DateTimeOffset]::new($self.StartTime.ToUniversalTime()).ToUnixTimeMilliseconds()
            $gateLines=@('M4_INPUT_JOB_V1',$runId,$RetainedInputNonce,[string]$hostOwner.Process.Id,[string]$hostCreated,[string]$PID,[string]$runnerCreated,'assigned')
            Publish-M4InputGate $hostOwner (Join-Path $owned 'host-input-job-ready') ([Text.Encoding]::UTF8.GetBytes(($gateLines -join [Environment]::NewLine)+[Environment]::NewLine))
        }finally{$self.Dispose()}
    }
    $offer=Get-Content -LiteralPath (Await-File 'offer.txt' 30 $hostOwner)
    if($offer.Count -ne 3){throw 'Bounded QR offer unavailable.'}
    $bootstrap=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes(($offer+@("${PcUsbAddress}:$Port")) -join "`n"))
    # QR is delivered in memory through the explicitly owned instrumentation
    # invocation. It and addresses are redacted before any output/log recording.
    $instrumentArgs=@('-s',$device.Serial,'shell','am','instrument','--user','0','-w','-r','-e','class',$instrumentationSelection,'-e','remoteRunId',$runId,'-e','remoteBootstrap',$bootstrap)
    if($ControllerInput){$instrumentArgs+=@('-e','remoteControllerInput','true','-e','remoteInputScenario',$InputScenario)}
    $instrumentArgs+=($packages[1]+'/androidx.test.runner.AndroidJUnitRunner')
    $instrument=Start-Owned 'real-phone-controller' $device.AdbPath $instrumentArgs
    $clock=[Diagnostics.Stopwatch]::StartNew();$next=0;$restart=1;$phoneRetired=$false;$ok=$false;$failure=$false;$record=@{}
    $script:phase='phone-controller-start'
    while(-not $instrument.Process.HasExited -or -not $instrument.Output.IsComplete -or -not $instrument.Output.Lines.IsEmpty){
        Drain-IntegrationDiagnostics $hostOwner
        if($clock.Elapsed.TotalSeconds -ge 180){throw 'Real phone integration deadline exceeded.'}
        if($hostOwner.Process.HasExited){throw 'Real host exited during phone integration.'}
        $line=$null
        while($instrument.Output.Lines.TryDequeue([ref]$line)){
            Add-IntegrationDiagnostic $instrument.Name $line
            if($line -cmatch '^INSTRUMENTATION_STATUS: (remote_input_[a-z_]+)=(.{1,2048})$'){
                if(-not $ControllerInput -or $inputRecord.ContainsKey($Matches[1])){throw 'Unexpected/duplicate actual controller input record.'}
                $inputRecord[$Matches[1]]=$Matches[2]
            }
            elseif($null -ne ($heldStatus=Get-M4HeldStatusLine $line)){
                if(-not $ControllerInput -or $InputScenario -ceq 'balanced' -or $lifecycleRecord.ContainsKey($heldStatus.Key)){throw 'Unexpected held lifecycle record.'}
                $lifecycleRecord[$heldStatus.Key]=$heldStatus.Value
            }
            elseif($line -ceq 'INSTRUMENTATION_STATUS_CODE: 0' -and $lifecycleRecord.Count -gt 0){$lifecycleRecords.Add([pscustomobject]$lifecycleRecord);$lifecycleRecord=@{}}
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_input_held_ready'){
                if($InputScenario -ceq 'balanced'){throw 'Unexpected held pre-Down phase.'}
                $armed=Await-File 'held-witness-armed' 15 $hostOwner;Assert-Plain $armed
                if([IO.File]::ReadAllText($armed) -cne $runId){throw 'Exact host witness arm missing.'}
                $gatePath="no_backup/remote-integration-$runId/held-host-armed"
                $encoded=[Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($runId))
                $command="run-as $($packages[0]) sh -c 'test ! -e $gatePath && test ! -e $gatePath.tmp && echo $encoded | base64 -d > $gatePath.tmp && mv $gatePath.tmp $gatePath'"
                [void](Adb 'publish-exact-host-witness-arm' @('shell',$command))
            }
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_input_held_admitted'){
                if($InputScenario -ceq 'balanced' -or $null -ne $heldPrefix){throw 'Unexpected held input phase.'}
                $journal=Join-Path $owned 'window/received.jsonl';Assert-Plain $journal
                [void](Run 'prove-actual-native-held-prefix' 'python.exe' @((Join-Path $PSScriptRoot 'controller_lifecycle_reports.py'),'ready',$journal,(Join-Path $owned 'held-native-prefix.json'),$runId,$InputScenario) 15)
                $heldPrefix=Get-Content -Raw -LiteralPath (Join-Path $owned 'held-native-prefix.json')|ConvertFrom-Json
                Write-HeldTrigger $(if($InputScenario -ceq 'disconnect-held'){'phone_carrier_abort_gate'}else{'phone_lifecycle_gate'})
                Publish-HeldPhoneGate
            }
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_lifecycle_triggered'){
                if(-not $lifecycleTriggered){throw 'Phone lifecycle trigger lacks preceding actual native prefix.'}
            }
            elseif($line -ceq 'INSTRUMENTATION_STATUS_CODE: 0' -and $inputRecord.Count -gt 0){$inputRecords.Add([pscustomobject]$inputRecord);$inputRecord=@{}}
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_input_ready'){
                if(-not $ControllerInput){throw 'Unexpected actual controller input phase.'}
                $script:phase='phone-controller-input-grant';Assert-UsbRoute
                if($InputScenario -cne 'balanced' -and -not ('VwHeldClockR24' -as [type])){Add-Type -Path (Join-Path $PSScriptRoot 'lifecycle_clock.cs')}
                # Record possible native grant before publishing its trigger.
                $receipt.input_grant_attempted=$null;$receipt.input_granted=$null;$receipt.input_attempted=$null;$receipt.input_dispatch_completed=$null
                [IO.File]::WriteAllText((Join-Path $owned 'input-ready'),$runId,[Text.UTF8Encoding]::new($false))
            }
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=controller_input_complete'){
                if(-not $ControllerInput){throw 'Unexpected actual controller input completion.'}
                $script:phase='phone-controller-input-complete'
                [IO.File]::WriteAllText((Join-Path $owned 'input-complete'),$runId,[Text.UTF8Encoding]::new($false))
            }
            elseif($line -cmatch '^INSTRUMENTATION_STATUS: (remote_render_[a-z_]+)=(.{1,2048})$'){$record[$Matches[1]]=$Matches[2]}
            elseif($line -cmatch '^INSTRUMENTATION_STATUS_CODE: 0$' -and $record.Count -gt 0){$records.Add([pscustomobject]$record);$record=@{}}
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=initial_controller_ready'){
                $script:phase='phone-initial-render'
                Assert-UsbRoute
                [IO.File]::WriteAllText((Join-Path $owned 'initial'),'owned-real-controller-ready')
            }elseif($line -cmatch '^INSTRUMENTATION_STATUS: remote_phase=(background_retired|link_reconnected)$'){
                $script:phase='phone-'+$Matches[1]
                Assert-UsbRoute
                [IO.File]::WriteAllText((Join-Path $owned "restart-$restart"),'owned-fresh-source');$restart++
            }elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=waiting_native_render_callback'){$script:phase='phone-native-render-callback'}
            elseif($line -ceq 'INSTRUMENTATION_STATUS: remote_phase=actual_phone_owners_retired'){$phoneRetired=$true}
            elseif($line -cmatch '^OK \(1 test\)$'){$ok=$true}
            elseif($line -match 'FAILURES!!!|INSTRUMENTATION_FAILED|Process crashed|INSTRUMENTATION_STATUS_CODE: -[12]'){
                # AndroidJUnit success uses -1; distinguish final case success
                # from actual -2 failure. No arbitrary console line proves pass.
                if($line -notmatch '^INSTRUMENTATION_STATUS_CODE: -1$'){$failure=$true}
            }
        }
        # AndroidJUnit emits -2 after the test's actual finally/retirement attempt.
        # Retain its bounded diagnostics before failing; never wait for a pass.
        if($failure){throw 'Real phone instrumentation reported failure.'}
        if($clock.Elapsed.TotalSeconds -ge $next){Write-Host "Real remote integration: phase=$phase elapsed=$([int]$clock.Elapsed.TotalSeconds)s";$next+=2}
        Start-Sleep -Milliseconds 25
    }
    $instrument.Process.WaitForExit()
    if($instrument.Process.ExitCode -ne 0 -or -not $ok -or $failure -or -not $phoneRetired -or $records.Count -ne 3 -or $restart -ne 3){throw 'Exact real-path case/render/retirement census failed.'}
    if(@($records|ForEach-Object{$_.remote_render_owner}|Sort-Object -Unique).Count -ne 3 -or @($records|ForEach-Object{$_.remote_render_scope}|Sort-Object -Unique).Count -ne 3){throw 'Fresh decoder/capture ownership census failed.'}
    [IO.File]::WriteAllText((Join-Path $owned 'render-records.json'),($records|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    # This census is observation only. Preserve it even if the strict parser or
    # its process fails, so a later failure cannot masquerade as zero renders.
    if($receipt.phone_render_observed_count -ne $records.Count){throw 'Exact render observation census differs from case records.'}
    $validation=Run 'validate-exact-render-records' 'python.exe' @((Join-Path $projectRoot 'tools/remote-edit/integration_reports.py'),(Join-Path $owned 'render-records.json'),$InputScenario) 30 -AllowNonzero
    $validationText=$validation.Lines -join "`n"
    if($validationText.Length -gt 16384){throw 'Exact render validator response unavailable.'}
    try{$verdict=$validationText|ConvertFrom-Json -ErrorAction Stop}catch{throw 'Exact render validator response unavailable.'}
    if($verdict.schema -ne 2 -or $verdict.observed_count -ne $records.Count -or $verdict.reason_code -notin (@(0..15)+@(99)) -or $verdict.status -cnotin @('passed','rejected') -or @($verdict.observed_records).Count -ne 3){throw 'Exact render validator response unavailable.'}
    # Parser emits only closed, bounded evidence fields; rejected originals are
    # withheld there. These records remain explicitly unvalidated on failure.
    $receipt.phone_render_records=@($verdict.observed_records)
    $receipt.render_validation_reason_code=$verdict.reason_code
    if($validation.ExitCode -ne 0 -or $verdict.status -cne 'passed' -or $verdict.records_validated -ne $true -or $verdict.reason_code -ne 0){throw 'Exact render validation rejected observed records.'}
    $receipt.phone_render_records_validated=$true;$receipt.phone_actual_retirement=$true
    [IO.File]::WriteAllText((Join-Path $owned 'stop'),'stop-after-actual-phone-retirement')
    [void](Await-File 'host-receipt.txt' 15 $hostOwner)
    if(-not $hostOwner.Process.WaitForExit(15000) -or $hostOwner.Process.ExitCode -ne 0){throw 'Host actual retirement failed.'}
    $proof=Get-Content -Raw -LiteralPath (Join-Path $owned 'host-receipt.txt')
    if($proof -notmatch '(?m)^actual_owners_retired=true$'){throw 'Host native helper ownership not retired.'}
    if($ControllerInput){
        if(($InputScenario -ceq 'balanced' -and ($inputRecords.Count -ne 1 -or $lifecycleRecords.Count -ne 0)) -or ($InputScenario -cne 'balanced' -and ($inputRecords.Count -ne 0 -or $lifecycleRecords.Count -ne 1 -or -not $lifecycleTriggered)) -or $inputRecord.Count -ne 0 -or $lifecycleRecord.Count -ne 0 -or $proof -cnotmatch '(?m)^input_granted=true$' -or $proof -cnotmatch '(?m)^controller_input_complete=true$'){throw 'Exact actual controller input/host census failed.'}
        # Close the paint-only owned surface after both actual native host and
        # phone owners retired, then read its complete real receiver journal.
        if(-not(Test-M4InputHostRetired $proof $runId $hostOwner.Process.Id)){throw 'Host actual retirement binding failed.'}
        Publish-InputSettlement
        [void]$surface.Process.CloseMainWindow()
        if(-not $surface.Process.WaitForExit(5000) -or $surface.Process.ExitCode -ne 0){throw 'Owned input receiver retirement failed.'}
        $receiverFile=Join-Path $owned 'window/counts.json';Assert-Plain $receiverFile
        if((Get-Item -LiteralPath $receiverFile).Length -gt 128){throw 'Owned input receiver census exceeds bound.'}
        $receiver=Get-Content -Raw -LiteralPath $receiverFile|ConvertFrom-Json
        if($receiver.Count -ne 6){throw 'Owned input receiver category census failed.'}
        $receivedFile=Join-Path $owned 'window/received.jsonl';Assert-Plain $receivedFile
        if((Get-Item -LiteralPath $receivedFile).Length -gt 262144){throw 'Owned receiver journal exceeds bound.'}
        $inputPath=Join-Path $owned 'controller-input-records.json';[IO.File]::WriteAllText($inputPath,(ConvertTo-Json -InputObject @($inputRecords) -Depth 8),[Text.UTF8Encoding]::new($false))
        if($InputScenario -ceq 'balanced'){
        $inputValidation=Run 'validate-actual-controller-input' 'python.exe' @((Join-Path $projectRoot 'tools/remote-edit/controller_input_reports.py'),$inputPath,(Join-Path $owned 'render-records.json'),$receiverFile,$receivedFile) 30 -AllowNonzero
        $inputVerdict=Save-M4ControllerValidationReport $inputValidation
        if($inputValidation.ExitCode -ne 0 -or $inputVerdict.status -cne 'passed' -or $inputVerdict.records_validated -ne $true -or $inputVerdict.rejection_reason -cne 'none'){throw 'Actual controller input validation rejected observed evidence.'}
        $receipt.controller_input_records_validated=$true
        }else{
            $lifecyclePath=Join-Path $owned 'controller-lifecycle-records.json';[IO.File]::WriteAllText($lifecyclePath,(ConvertTo-Json -InputObject @($lifecycleRecords) -Depth 8),[Text.UTF8Encoding]::new($false))
            $heldValidation=Run 'validate-held-lifecycle-release' 'python.exe' @((Join-Path $PSScriptRoot 'controller_lifecycle_reports.py'),$lifecyclePath,(Join-Path $owned 'render-records.json'),$receiverFile,$receivedFile,(Join-Path $owned 'held-native-prefix.json'),(Join-Path $owned 'held-trigger.json'),(Join-Path $owned 'held-release-witness.json')) 30 -AllowNonzero
            $heldVerdict=Save-M4HeldValidationReport $heldValidation
            if($heldValidation.ExitCode -ne 0 -or $heldVerdict.status -cne 'passed' -or $heldVerdict.records_validated -ne $true){throw 'Actual held lifecycle validation rejected.'}
            $receipt.held_lifecycle_records_validated=$true
        }
        $receipt.controller_input_records=@($inputRecords);$receipt.owned_receiver_counts=$receiver;$receipt.input_granted=$true;$receipt.input_attempted=$true;$receipt.input_dispatch_completed=$true;$receipt.software_generated_input=$true
    }elseif($inputRecords.Count -ne 0 -or $inputRecord.Count -ne 0){throw 'Unexpected controller input receipt in viewing-only route.'}
    $receipt.host_actual_retirement=$true
    if($ControllerInput){$receipt.input_retirement_confirmed=$true}
    $v=Adb 'final-profile-inventory' @('shell','pm','list','users');$receipt.profiles_preserved=(Get-VwAppHilProfileCount -Lines $v.Lines)-eq $profileCount
    if(-not $receipt.profiles_preserved){throw 'Profile inventory changed.'};$passed=$true
} catch {
    $receipt.failure_phase=$phase
    $receipt.failure_error_type=$_.Exception.GetType().FullName
    $receipt.failure_error_code=Get-IntegrationFailureCode $_.Exception.Message
    if($phase -ceq 'physical_usb_route'){$receipt.status='unavailable'}
    Write-Host "Real remote integration stopped in phase=$phase. Private details and addresses are withheld."
} finally {
    while($true){
        try{
    if($created -and -not(Test-Path -LiteralPath (Join-Path $owned 'stop'))){[IO.File]::WriteAllText((Join-Path $owned 'stop'),'bounded-owner-stop')}
    if($ControllerInput){
        # A timeout/error never kills inputful owners or removes its release HWND.
        # Host/native and phone owners emit terminal proof only after actual joins.
        # Unknown/exited-without-proof stays Pending; root retains this process.
        $hostEntry=@($children|Where-Object{$_.Name -ceq 'real-native-session-host'})
        $phoneEntry=@($children|Where-Object{$_.Name -ceq 'real-phone-controller'})
        # A host whose exact gate never became visible cannot grant input.
        # Retire its actual process tree and any started readers before releasing
        # the destination. No impossible native terminal receipt is required.
        if($hostEntry.Count -eq 1 -and $hostEntry[0].GateState -ceq 'Absent'){
            Stop-M4InputPregrantChild $hostEntry[0]
            Wait-M4InputChildSettlement $hostEntry[0] {param($child) Drain-IntegrationDiagnostics $child}
        }
        $nativeComplete=$hostEntry.Count -eq 0 -or ($hostEntry.Count -eq 1 -and $hostEntry[0].GateState -ceq 'Absent' -and (Test-M4InputChildSettled $hostEntry[0]))
        $phoneComplete=$phoneEntry.Count -eq 0 -or ($phoneEntry.Count -eq 1 -and $phoneEntry[0].Started -ceq 'No' -and (Test-M4InputChildSettled $phoneEntry[0]))
        $retireReport=[Diagnostics.Stopwatch]::StartNew();$retireNext=0
        while(-not(Test-M4InputSettlement $nativeComplete $phoneComplete)){
            foreach($entry in $children){Drain-IntegrationDiagnostics $entry}
            if(-not $nativeComplete -and $hostEntry.Count -eq 1){
                # Exact producer no-owner proof still requires actual process,
                # whole Job and reader-task settlement. It is failed startup,
                # never render/input acceptance or a timeout-based inference.
                if($null -ne $startupProof -and $startupProof.HostNoOwners -and (Test-M4InputChildSettled $hostEntry[0])){
                    $nativeComplete=$true;$receipt.input_grant_attempted=$false;$receipt.input_granted=$false
                }
                try{
                    $hostTerminal=Join-Path $owned 'host-receipt.txt'
                    if($hostEntry[0].Process.HasExited -and (Test-Path -LiteralPath $hostTerminal -PathType Leaf)){
                        Assert-Plain $hostTerminal
                        if((Get-Item -LiteralPath $hostTerminal).Length -le 2048){
                            $hostText=Get-Content -Raw -LiteralPath $hostTerminal
                            $nativeComplete=Test-M4InputHostRetired $hostText $runId $hostEntry[0].Process.Id
                            if($hostText -cmatch '(?m)^input_grant_attempted=(true|false)$'){$receipt.input_grant_attempted=$Matches[1] -ceq 'true'}
                            if($hostText -cmatch '(?m)^input_granted=false$'){$receipt.input_granted=$false}
                            if($hostText -cmatch '(?m)^input_granted=true$'){$receipt.input_granted=$true}
                        }
                    }
                }catch{} # query uncertainty retains every actual owner
            }
            $phoneComplete=$phoneEntry.Count -eq 0 -or ($phoneEntry.Count -eq 1 -and (($phoneEntry[0].Started -ceq 'No') -or $phoneRetired -or ($null -ne $startupProof -and $startupProof.PhoneNoOwners)) -and (Test-M4InputChildSettled $phoneEntry[0]))
            if($retireReport.Elapsed.TotalSeconds -ge $retireNext){Write-Host 'Pending actual input/native/phone retirement; owned destination and Jobs remain alive';$retireNext+=2}
            Start-Sleep -Milliseconds 20
        }
        $receipt.input_retirement_confirmed=$true
        $receipt.host_actual_retirement=$nativeComplete;$receipt.phone_actual_retirement=$phoneComplete
        # The exact destination checks this run-correlated marker on WM_CLOSE.
        # Publish only after both native and phone owners actually settled.
        Publish-InputSettlement
        foreach($entry in $children){
            if($entry.Name -ceq 'owned-wgc-target' -and -not(Test-M4InputChildExited $entry)){
                if($entry.OutputStarted -and $entry.ErrorStarted){[void]$entry.Process.CloseMainWindow()}
                else{Stop-M4InputPregrantChild $entry}
            }
            Wait-M4InputChildSettlement $entry {param($child) Drain-IntegrationDiagnostics $child}
        }
    }else{
    foreach($entry in $children){
        try {
            if(-not $entry.Process.HasExited){
                if($entry.Name -ceq 'owned-wgc-target'){[void]$entry.Process.CloseMainWindow()}
                if(-not $entry.Process.WaitForExit(5000)){if($entry.Assigned){$entry.Job.Terminate(1)}else{$entry.Process.Kill($true)}}
            }
            $clock=[Diagnostics.Stopwatch]::StartNew()
            while(($entry.Job.ActiveProcessCount -ne 0 -or -not $entry.Output.IsComplete) -and $clock.Elapsed.TotalSeconds -lt 10){Write-Host 'Observing actual task-owned tree and output-stream retirement';Start-Sleep -Milliseconds 100}
            if($entry.Job.ActiveProcessCount -ne 0 -or -not $entry.Output.IsComplete){$allClean=$false}
            Drain-IntegrationDiagnostics $entry
        }catch{$allClean=$false}
    }
    }
    # Capture final receiver observations only after actual destination/owner
    # settlement, including when an earlier rendering/input phase threw.
    $receiverEvidenceRetained=$true
    if($ControllerInput -and $created -and $allClean -and (Test-Path -LiteralPath (Join-Path $owned 'window/received.jsonl') -PathType Leaf)){
        if($null -eq $receipt.receiver_journal_evidence -or $receipt.receiver_journal_evidence.snapshot_complete -ne $true){
            $savedPhase=$phase
            try{
                $journalPath=Join-Path $owned 'window/received.jsonl';$countsPath=Join-Path $owned 'window/counts.json'
                Assert-Plain $journalPath;Assert-Plain $countsPath
                $observation=Run 'retain-final-receiver-evidence' 'python.exe' @((Join-Path $PSScriptRoot 'controller_input_reports.py'),'observe-receiver',$countsPath,$journalPath) 30 -AllowNonzero
                $observed=Save-M4ControllerValidationReport $observation
                if($observation.ExitCode -ne 0 -or $observed.receiver_evidence.snapshot_complete -ne $true){throw 'Receiver observation unavailable.'}
            }catch{$receiverEvidenceRetained=$false;$receipt.receiver_evidence_capture_failed=$true}
            finally{$script:phase=$savedPhase}
        }
    }
    if($ControllerInput -and $InputScenario -cne 'balanced' -and $created -and $allClean -and $null -eq $receipt.held_lifecycle_evidence){
        $savedPhase=$phase
        try{
            $lifecyclePath=Join-Path $owned 'controller-lifecycle-observed.json'
            [IO.File]::WriteAllText($lifecyclePath,(ConvertTo-Json -InputObject @($lifecycleRecords) -Depth 8),[Text.UTF8Encoding]::new($false))
            $observation=Run 'retain-final-held-evidence' 'python.exe' @((Join-Path $PSScriptRoot 'controller_lifecycle_reports.py'),'observe',$lifecyclePath,(Join-Path $owned 'held-native-prefix.json'),(Join-Path $owned 'held-trigger.json'),(Join-Path $owned 'held-release-witness.json')) 30 -AllowNonzero
            $observed=Save-M4HeldValidationReport $observation
            if($observation.ExitCode -ne 0 -or $observed.status -cne 'observed'){throw 'Held observation unavailable.'}
        }catch{$receiverEvidenceRetained=$false;$receipt.held_evidence_capture_failed=$true}
        finally{$script:phase=$savedPhase}
    }
    $packageClean=$true
    if($null -ne $device){foreach($package in @($packages[1],$packages[0])){
        if(-not $attempted.ContainsKey($package)){continue}
        try{
            if($preAbsent[$package] -ne $true){throw 'Pre-install global ownership is unknown; preserve package.'}
            $v=Adb 'attempted-owner-package-inventory' @('shell','pm','list','packages','--user','0',$package)
            $actual=@($v.Lines|Where-Object{$_.Trim()})
            if(@($actual|Where-Object{$_ -cnotmatch '^package:[A-Za-z0-9_.]+$'}).Count -ne 0){throw 'Attempted package inventory is ambiguous.'}
            $present=@($actual|Where-Object{$_ -ceq ('package:'+$package)}).Count
            if($present -gt 1){throw 'Attempted package inventory is ambiguous.'}
            if($present -eq 1){
                if(-not(Check-Installed $package $attempted[$package])){throw 'Changed/ambiguous attempted package preserved; cleanup Pending.'}
                [void](Adb 'stop-owned-integration-app' @('shell','am','force-stop','--user','0',$package))
                [void](Adb 'remove-owner-only-integration-app' @('shell','pm','uninstall','--user','0',$package) 60)
            }
            # Absence is re-observed even if install never returned Success.
            $v=Adb 'confirm-attempted-owner-package-absence' @('shell','pm','list','packages','--user','0',$package)
            $actual=@($v.Lines|Where-Object{$_.Trim()})
            if(@($actual|Where-Object{$_ -cnotmatch '^package:[A-Za-z0-9_.]+$'}).Count -ne 0 -or @($actual|Where-Object{$_ -ceq ('package:'+$package)}).Count -ne 0){throw 'Attempted user0 package absence unconfirmed; cleanup Pending.'}
        }catch{$packageClean=$false}
    }}
    if($allClean){foreach($entry in $children){$entry.Process.Dispose();$entry.Job.Dispose()};foreach($pin in $pins){$pin.Dispose()}}
    else{
        # Query/join uncertainty is Pending. Keep Job, IO and file leases alive
        # until this centrally owned runner itself is actually retired.
        $global:VwRemoteIntegrationPendingOwners=@($children.ToArray(),$pins.ToArray())
    }
    if($created -and $allClean -and $receiverEvidenceRetained){try{
        $resolved=[IO.Path]::GetFullPath($owned)
        if([IO.Path]::GetDirectoryName($resolved) -cne $temporaryParent -or [IO.Path]::GetFileName($resolved) -cne ('VisualWorkbench-remote-integration-'+$runId)){throw 'Private work containment refused.'}
        Assert-Plain $resolved
        if((Get-Content -Raw -LiteralPath (Join-Path $resolved '.owner-v1')) -cne $runId){throw 'Private owner marker mismatch.'}
        foreach($item in Get-ChildItem -LiteralPath $resolved -Recurse -Force){Assert-Plain $item.FullName;if(-not $item.FullName.StartsWith($resolved+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Private child containment refused.'}}
        Remove-Item -LiteralPath $resolved -Recurse -Force
        $receipt.private_work_disposed=-not(Test-Path -LiteralPath $resolved)
    }catch{$receipt.private_work_disposed=$false}}elseif(-not $created){$receipt.private_work_disposed=$true}
    $receipt.stream_counter_maxima=$streamCounters
    $receipt.diagnostic_records=@($diagnostics);$receipt.diagnostic_total=$diagnosticTotal;$receipt.diagnostic_dropped=$diagnosticDropped
    $receipt.phone_retirement_phase_seen=$phoneRetired
    $receipt.process_cleanup=$allClean;$receipt.owned_packages_cleanup=$packageClean
    if($passed -and $allClean -and $packageClean -and $receipt.private_work_disposed){$receipt.status='passed';$receipt.failure_phase=$null}
    $destination=Join-Path $projectRoot ('.local/remote-normal-integration-'+$runId+'.json')
    $json=$receipt|ConvertTo-Json -Depth 16
    $f=[IO.File]::Open($destination,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$bytes=[Text.Encoding]::UTF8.GetBytes($json);$f.Write($bytes);$f.Flush($true)}finally{$f.Dispose()}
    Write-Host ('Text-only integration receipt: remote-normal-integration-'+$runId+'.json; status='+$receipt.status)
            break
        }catch{
            if(-not $ControllerInput -or ($receipt.input_retirement_confirmed -eq $true -and $receipt.process_cleanup -eq $true)){throw}
            # Keep the concrete Jobs/readers/leases in this live runner. A
            # failure writing stop/settlement/receipt never releases their owners.
            try{Write-Host 'Retained cleanup query or IO pending; preserving all input owners'}catch{}
            Start-Sleep -Milliseconds 2000
        }
    }
}
if($receipt.status -cne 'passed'){exit 1}
