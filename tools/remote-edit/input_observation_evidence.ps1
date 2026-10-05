Set-StrictMode -Version Latest
# Observation only: no grants, settlement, success, or validation authority.
function New-M4InputObservationEvidence {
    return @{balanced=@{Pending=[ordered]@{};Seen=$false};held=@{Pending=[ordered]@{};Seen=$false}}
}
function New-M4InputObservationReceipt {
    $result=[ordered]@{}
    foreach($kind in @('balanced','held')){
        $result[$kind]=[ordered]@{records=@();observed_count=0L;omitted_count=0L;partial_record=$null;partial_status=$null;duplicate_fields=0L;unknown_fields=0L;withheld_fields=0L;aborted_bundles=0L;records_validated=$false;acceptance=$false}
    }
    return $result
}
function Add-M4InputObservationCount($Record,[string]$Key) {
    if($Record[$Key] -lt [long]::MaxValue){$Record[$Key]=[long]$Record[$Key]+1L}
}
function Copy-M4InputObservation($Record) {
    $copy=[ordered]@{}
    foreach($key in $Record.Keys){$copy[$key]=$Record[$key]}
    return [pscustomobject]$copy
}
function Get-M4InputObservationPatterns([string]$Kind) {
    $uuid='[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}'
    $binding='[0-9]{1,20},'+$uuid+',[0-9]{1,20},'+$uuid+',[0-9]{1,10},'+$uuid
    if($Kind -ceq 'balanced'){
        return @{
            remote_input_binding=$binding;remote_input_sequences='[0-9]{1,20},[0-9]{1,20}'
            remote_input_owner=$uuid;remote_input_frame='[0-9]{1,20}';remote_input_ticket='[0-9]{1,20}';remote_input_pts_us='-?[0-9]{1,20}'
            remote_input_timing='[0-9]{1,20}(?:,[0-9]{1,20}){4}';remote_input_echo='[0-9]{1,20}(?:,[0-9]{1,20}){2}'
            remote_input_route='surface_touch,software_generated=true,ghost_ack_fade=true,physical_pen_fidelity=false,editor_effect=false,latency_acceptance=false'
        }
    }
    return @{
        remote_lifecycle_scenario='pause-held|background-held|disconnect-held';remote_lifecycle_binding=$binding
        remote_lifecycle_sequences='[0-9]{1,20},[0-9]{1,20}';remote_lifecycle_phone_up_admitted='true|false'
        remote_lifecycle_native_prefix_sha256='[a-f0-9]{64}';remote_lifecycle_native_prefix_bytes='[0-9]{1,6}'
        remote_lifecycle_stale_tail_refused='true|false';remote_lifecycle_phone_settled='true|false';remote_lifecycle_return_granted='true|false'
        remote_lifecycle_route='surface_touch,software_generated=true,host_safety_release=true,physical_pen_fidelity=false'
    }
}
function Get-M4HeldStatusLine([string]$Line) {
    if($null -eq $Line -or $Line.Length -gt 4096){return $null}
    if($Line -cnotmatch '\AINSTRUMENTATION_STATUS: (remote_lifecycle_[a-z0-9_]{1,64})=(.{1,2048})\z'){return $null}
    $key=$Matches[1];$value=$Matches[2];$patterns=Get-M4InputObservationPatterns 'held'
    if(-not $patterns.ContainsKey($key)){throw 'Unknown held lifecycle field.'}
    return [pscustomobject]@{Key=$key;Value=$value}
}
function Add-M4InputObservationEvidence($State,$Receipt,[string]$Line) {
    if($null -eq $Line -or $Line.Length -gt 4096){return}
    if($Line -cmatch '\AINSTRUMENTATION_STATUS: (remote_(input|lifecycle)_[a-z0-9_]{1,64})=(.{0,2048})\z'){
        $key=$Matches[1];$kind=if($Matches[2] -ceq 'input'){'balanced'}else{'held'};$value=$Matches[3]
        $pending=$State[$kind];$observed=$Receipt[$kind];$patterns=Get-M4InputObservationPatterns $kind
        $pending.Seen=$true
        if(-not $patterns.ContainsKey($key)){Add-M4InputObservationCount $observed 'unknown_fields'}
        elseif($pending.Pending.Contains($key)){Add-M4InputObservationCount $observed 'duplicate_fields'}
        else{
            if($value -cnotmatch ('\A(?:'+$patterns[$key]+')\z')){$value='[withheld]';Add-M4InputObservationCount $observed 'withheld_fields'}
            $pending.Pending[$key]=$value
        }
        $observed.partial_record=Copy-M4InputObservation $pending.Pending;$observed.partial_status=$null
        return
    }
    if($Line -cmatch '\AINSTRUMENTATION_STATUS_CODE: (-?[0-9]{1,10})\z'){
        $status=[long]$Matches[1]
        foreach($kind in @('balanced','held')){
            $pending=$State[$kind];$observed=$Receipt[$kind]
            if(-not $pending.Seen){continue}
            if($status -eq 0){
                Add-M4InputObservationCount $observed 'observed_count'
                if(@($observed.records).Count -lt 4){$observed.records=@($observed.records)+@(Copy-M4InputObservation $pending.Pending)}
                else{Add-M4InputObservationCount $observed 'omitted_count'}
                $observed.partial_record=$null;$observed.partial_status=$null
            }else{
                Add-M4InputObservationCount $observed 'aborted_bundles'
                $observed.partial_record=Copy-M4InputObservation $pending.Pending;$observed.partial_status=$status
            }
            $pending.Pending=[ordered]@{};$pending.Seen=$false
        }
    }
}
