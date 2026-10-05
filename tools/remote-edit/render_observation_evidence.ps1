Set-StrictMode -Version Latest
# Observation retention only. This cannot set validated/retired/success flags.
function New-M4RenderObservationEvidence {
    return [pscustomobject]@{Pending=@{};PendingSeen=$false;Clock=[Diagnostics.Stopwatch]::StartNew();LastPhase=$null;Diagnostic=@{}}
}
function ConvertTo-M4BoundedRenderObservation($Record) {
    $patterns=[ordered]@{
        remote_render_scope='[0-9]{1,20},[a-f0-9-]{36},[0-9]{1,20},[a-f0-9-]{36},[0-9]{1,10}'
        remote_render_owner='[a-f0-9-]{36}'
        remote_render_ticket='[0-9]{1,20}'
        remote_render_frame='[0-9]{1,20}'
        remote_render_pts_us='-?[0-9]{1,20}'
        remote_render_codec='[A-Za-z0-9_.-]{1,128}'
        remote_render_timing='-?[0-9]{1,20},-?[0-9]{1,20}'
        remote_render_capabilities='hardware=(?:true|false),low_latency_advertised=(?:true|false),requested=(?:true|false),configure_accepted=(?:true|false)'
    }
    $safe=[ordered]@{}
    foreach($key in $patterns.Keys){
        $value=$Record[$key]
        $safe[$key]=if($value -is [string] -and $value -cmatch ('\A'+$patterns[$key]+'\z')){$value}else{'[withheld]'}
    }
    return [pscustomobject]$safe
}
function Add-M4RenderObservationEvidence($State,$Receipt,[string]$Line) {
    if($null -eq $Line -or $Line.Length -gt 4096){return}
    Add-M4StreamSnapshotEvidence $State $Receipt $Line
    if($Line -cmatch '^INSTRUMENTATION_STATUS: (remote_render_[a-z_]{1,64})=(.{0,2048})$'){
        $State.PendingSeen=$true
        $key=$Matches[1];$value=$Matches[2]
        if($key -cin @('remote_render_scope','remote_render_owner','remote_render_ticket','remote_render_frame','remote_render_pts_us','remote_render_codec','remote_render_timing','remote_render_capabilities')){$State.Pending[$key]=$value}
        $Receipt.phone_render_partial_record=ConvertTo-M4BoundedRenderObservation $State.Pending
        return
    }
    if($Line -ceq 'INSTRUMENTATION_STATUS_CODE: 0' -and $State.PendingSeen){
        $Receipt.phone_render_observed_count=[long]$Receipt.phone_render_observed_count+1
        if(@($Receipt.phone_render_records).Count -lt 3){$Receipt.phone_render_records=@($Receipt.phone_render_records)+@(ConvertTo-M4BoundedRenderObservation $State.Pending)}
        else{$Receipt.phone_render_observations_omitted=[long]$Receipt.phone_render_observations_omitted+1}
        $State.Pending=@{};$State.PendingSeen=$false;$Receipt.phone_render_partial_record=$null
        return
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_phase=([a-z_]{1,64})$'){
        $phase=$Matches[1];$Receipt.phone_last_phase=$phase
        if($phase -cne $State.LastPhase){
            if(@($Receipt.phone_phase_transitions).Count -lt 64){
                $Receipt.phone_phase_transitions=@($Receipt.phone_phase_transitions)+@([pscustomobject]@{phase=$phase;runner_elapsed_ms=$State.Clock.ElapsedMilliseconds})
            }else{$Receipt.phone_phase_transitions_omitted=[long]$Receipt.phone_phase_transitions_omitted+1}
            $State.LastPhase=$phase
        }
    }
}

# Fixed fields and stages only; a bounded last snapshot per view/stage survives
# the diagnostic ring without becoming render/retirement validation.
function Add-M4StreamSnapshotEvidence($State,$Receipt,[string]$Line) {
    $numeric=@('capture_polls','capture_idle','encoded_frames','config_enqueued','media_enqueued','config_received','media_received','media_no_config','media_retired','media_admitted','frame_taken','state_published','state_stale','frames_emitted','frame_scope_discarded','frames_received','scope_discarded','inactive_discarded','surface_waited','surface_deadline','surface_invalidated','decoder_opened','decoder_refused','startup_failed','frames_queued','queue_refused','rendered')
    $stages=@('waiting_fresh_carrier','waiting_native_render_callback','native_render_observed','actual_render_accepted','render_timeout','before_phone_owner_retirement','after_decoder_retirement','waiting_actual_pc_grant_mapping','waiting_native_covering_frame')
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_stream_([a-z_]{1,40})=([0-9]{1,20})$'){
        $key=$Matches[1];$number=[UInt64]0
        if($key -cin $numeric -and [UInt64]::TryParse($Matches[2],[ref]$number)){$State.Diagnostic[$key]=$number};return
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_view_index=([0-2])$'){$State.Diagnostic['view_index']=[int]$Matches[1];return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_stage=([a-z_]{1,64})$'){
        if($Matches[1] -cin $stages){$State.Diagnostic['stage']=$Matches[1]};return
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_(native_snapshot_available|carrier_snapshot_available|carrier_available|config_present|scope_present)=(true|false)$'){$State.Diagnostic[$Matches[1]]=$Matches[2] -ceq 'true';return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_carrier_status=(Reconnecting|Syncing|Synced|Offline)$'){$State.Diagnostic['carrier_status']=$Matches[1];return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_carrier_failure=(none|untrusted_or_revoked|storage|capacity|timeout|invalid_peer_message|closed|connection|upgrade_required|withheld)$'){$State.Diagnostic['carrier_failure']=$Matches[1];return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_carrier_epoch=([0-9]{1,20})$'){$epoch=[UInt64]0;if([UInt64]::TryParse($Matches[1],[ref]$epoch)){$State.Diagnostic['carrier_epoch']=[string]$epoch};return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_decoder=(absent|Starting|Ready|RecoveryRequired|Retiring|Retired)$'){$State.Diagnostic['decoder']=$Matches[1];return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: remote_diag_remote_status=(Unavailable|Disconnected|Selecting|Viewing|PendingFocus|PendingGrant|Controlling|Paused|Sealed|Closing|Closed)$'){$State.Diagnostic['remote_status']=$Matches[1];return}
    if($Line -ceq 'INSTRUMENTATION_STATUS_CODE: 0'){
        if($State.Diagnostic.ContainsKey('view_index') -and $State.Diagnostic.ContainsKey('stage')){
            $key=[string]$State.Diagnostic.view_index+':'+$State.Diagnostic.stage
            if($Receipt.phone_stream_snapshots.Contains($key) -or $Receipt.phone_stream_snapshots.Count -lt 27){
                $copy=[ordered]@{runner_elapsed_ms=$State.Clock.ElapsedMilliseconds}
                foreach($field in $State.Diagnostic.Keys){$copy[$field]=$State.Diagnostic[$field]}
                $Receipt.phone_stream_snapshots[$key]=[pscustomobject]$copy
            }
        }
        $State.Diagnostic=@{}
    }elseif($Line -cmatch '^INSTRUMENTATION_STATUS_CODE: -[0-9]+$'){$State.Diagnostic=@{}}
}

# Actual carrier status only; no readiness, render or ownership authority.
function Add-M4HostCarrierObservation($Receipt,[string]$Line){
    if($Line -ceq 'REMOTE_HOST_CARRIER_UNAVAILABLE'){$Receipt.host_carrier_snapshot_available=$false;return $true}
    if($Line -cnotmatch '^REMOTE_HOST_CARRIER:status=(RECONNECTING|SYNCING|SYNCED|OFFLINE);failure=(none|untrusted_or_revoked|storage|capacity|timeout|invalid_peer_message|closed|connection|upgrade_required|withheld);epoch=([0-9]{1,20});available=(true|false)$'){return $false}
    $status=$Matches[1];$failure=$Matches[2];$available=$Matches[4] -ceq 'true';$epoch=[UInt64]0
    if(-not [UInt64]::TryParse($Matches[3],[ref]$epoch)){return $false}
    $Receipt.host_carrier_snapshot_available=$true
    $Receipt.host_carrier_diagnostic=[ordered]@{status=$status;failure=$failure;epoch=[string]$epoch;available=$available;acceptance=$false}
    if($failure -cne 'none' -and $null -eq $Receipt.host_first_carrier_failure){$Receipt.host_first_carrier_failure=$failure}
    return $true
}
