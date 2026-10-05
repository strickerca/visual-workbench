Set-StrictMode -Version Latest
function Test-M4InputParentGate($Gate,[string]$Nonce,[int]$ChildId,[string]$ChildCreated,[int]$ParentId,[string]$ParentCreated){
    if($null -eq $Gate -or $Nonce -cnotmatch '^[a-f0-9]{32}$'){return $false}
    $keys=@($Gate.PSObject.Properties.Name|Sort-Object)
    if(($keys -join ',') -cne 'assigned,child_created,child_pid,nonce,parent_created,parent_pid,run_id,schema'){return $false}
    return $Gate.schema -is [string] -and $Gate.assigned -is [bool] -and
        $Gate.child_pid -is [long] -and $Gate.parent_pid -is [long] -and
        $Gate.child_created -is [string] -and $Gate.parent_created -is [string] -and
        $Gate.nonce -is [string] -and $Gate.run_id -is [string] -and
        $Gate.schema -ceq 'M4_INPUT_PARENT_V1' -and $Gate.assigned -ceq $true -and
        $Gate.nonce -ceq $Nonce -and $Gate.run_id -ceq $Nonce -and
        $Gate.child_pid -eq $ChildId -and $Gate.child_created -ceq $ChildCreated -and
        $Gate.parent_pid -eq $ParentId -and $Gate.parent_created -ceq $ParentCreated
}
function Test-M4InputHostRetired([string]$Text,[string]$RunId,[int]$HostId){
    if($Text.Length -gt 2048 -or $RunId -cnotmatch '^[a-f0-9]{32}$'){return $false}
    $fields=@{}
    foreach($line in ($Text -split "`n")){
        if(-not $line){continue}
        if($line -cnotmatch '^([a-z_]+)=([a-z0-9]+)$' -or $fields.ContainsKey($Matches[1])){return $false}
        $fields[$Matches[1]]=$Matches[2]
    }
    if((@($fields.Keys|Sort-Object) -join ',') -cne 'actual_owners_retired,controller_input_complete,editor_effect,host_pid,input_grant_attempted,input_granted,latency_acceptance,run_id,schema'){return $false}
    return $fields.schema -ceq '1' -and $fields.run_id -ceq $RunId -and
        $fields.host_pid -ceq ([string]$HostId) -and $fields.actual_owners_retired -ceq 'true'
}
function Test-M4InputSettlement($NativeRetired,$PhoneRetired){
    return $NativeRetired -is [bool] -and $NativeRetired -eq $true -and
        $PhoneRetired -is [bool] -and $PhoneRetired -eq $true
}
function Test-M4InputRunnerRetired($Receipt,[string]$Nonce){
    if($null -eq $Receipt -or $Nonce -cnotmatch '^[a-f0-9]{32}$'){return $false}
    foreach($field in @('run_id','retained_input_nonce','controller_input_requested','input_retirement_confirmed','host_actual_retirement','phone_actual_retirement','process_cleanup')){
        if(@($Receipt.PSObject.Properties.Name) -cnotcontains $field){return $false}
    }
    if($Receipt.run_id -cne $Nonce -or $Receipt.retained_input_nonce -cne $Nonce){return $false}
    foreach($field in @('controller_input_requested','input_retirement_confirmed','host_actual_retirement','phone_actual_retirement','process_cleanup')){
        if($Receipt.$field -isnot [bool] -or $Receipt.$field -ne $true){return $false}
    }
    return $true
}

# Bound startup proof is distinct from a normal native retirement receipt. A
# missing phase, child exit, deadline, or unstarted reader is never such proof.
function New-M4StartupOwnerProof {
    return [pscustomobject]@{HostStarted=$false;HostNoOwners=$false;HostProofSeen=$false;PhoneStarted=$false;PhoneNoOwners=$false;PhoneProofSeen=$false;PhoneRecord=@{};PhoneRecordInvalid=$false}
}
function Add-M4StartupOwnerProof($State,$Receipt,[string]$Owner,[string]$Line,[string]$RunId,[int]$HostId){
    if($null -eq $Line -or $Line.Length -gt 4096 -or $RunId -cnotmatch '^[a-f0-9]{32}$'){return}
    if($Owner -ceq 'real-native-session-host'){
        if($Line -cmatch '^REMOTE_HOST_PHASE:(host_owners_starting|owned_input_grant_attempted|actual_owned_input_granted)$'){
            $State.HostStarted=$true;$State.HostNoOwners=$false;$Receipt.host_no_owner_startup_proven=$false
        }
        elseif($Line -cmatch '^REMOTE_HOST_NO_OWNERS:run=([a-f0-9]{32});pid=([1-9][0-9]{0,9})$'){
            [int]$actualId=0
            $valid=-not $State.HostStarted -and -not $State.HostProofSeen -and $Matches[1] -ceq $RunId -and [int]::TryParse($Matches[2],[ref]$actualId) -and $actualId -eq $HostId -and $HostId -gt 0
            $State.HostProofSeen=$true;$State.HostNoOwners=$valid;$Receipt.host_no_owner_startup_proven=$valid
        }
        return
    }
    if($Owner -cne 'real-phone-controller'){return}
    if($Line -cmatch '^INSTRUMENTATION_STATUS: ([a-zA-Z0-9_]{1,128})=(.*)$'){
        $key=$Matches[1];$value=$Matches[2]
        if($key -ceq 'remote_phase' -and $value -cin @('phone_owners_starting','initial_controller_ready','controller_input_ready','controller_input_dispatch_attempted','controller_input_dispatch_completed')){
            $State.PhoneStarted=$true;$State.PhoneNoOwners=$false;$Receipt.phone_no_owner_startup_proven=$false
        }
        if($key -clike 'remote_render_*' -or $key -clike 'remote_input_*'){$State.PhoneStarted=$true;$State.PhoneNoOwners=$false;$Receipt.phone_no_owner_startup_proven=$false}
        if($State.PhoneRecord.ContainsKey($key) -or $State.PhoneRecord.Count -ge 128 -or $value.Length -gt 2048){$State.PhoneRecordInvalid=$true}
        else{$State.PhoneRecord[$key]=$value}
        return
    }
    if($Line -cmatch '^INSTRUMENTATION_STATUS_CODE: (-?[0-9]+)$'){
        $record=$State.PhoneRecord
        if($record.ContainsKey('remote_no_owner_run_id') -or ($record.ContainsKey('remote_phase') -and $record.remote_phase -ceq 'actual_phone_no_owners_created')){
            $valid=$Matches[1] -ceq '0' -and -not $State.PhoneRecordInvalid -and -not $State.PhoneStarted -and -not $State.PhoneProofSeen -and $record.Count -eq 2 -and $record.ContainsKey('remote_no_owner_run_id') -and $record.ContainsKey('remote_phase') -and $record.remote_no_owner_run_id -ceq $RunId -and $record.remote_phase -ceq 'actual_phone_no_owners_created'
            $State.PhoneProofSeen=$true;$State.PhoneNoOwners=$valid;$Receipt.phone_no_owner_startup_proven=$valid
        }
        $State.PhoneRecord=@{};$State.PhoneRecordInvalid=$false
    }
}
