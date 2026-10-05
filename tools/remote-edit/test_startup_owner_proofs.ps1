#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
function Assert($Value,[string]$Reason){if(-not $Value){throw $Reason}}
function Receipt {return [ordered]@{host_no_owner_startup_proven=$false;phone_no_owner_startup_proven=$false;phone_actual_retirement=$false;host_actual_retirement=$false;status='failed'}}
$run='0123456789abcdef0123456789abcdef';$cases=0
function Host($State,$Receipt,[string]$Line){Add-M4StartupOwnerProof $State $Receipt 'real-native-session-host' $Line $run 123}
function Phone($State,$Receipt,[string[]]$Lines){foreach($line in $Lines){Add-M4StartupOwnerProof $State $Receipt 'real-phone-controller' $line $run 123}}
$hostProof="REMOTE_HOST_NO_OWNERS:run=$run;pid=123"
$phoneProof=@("INSTRUMENTATION_STATUS: remote_no_owner_run_id=$run",'INSTRUMENTATION_STATUS: remote_phase=actual_phone_no_owners_created','INSTRUMENTATION_STATUS_CODE: 0')
$s=New-M4StartupOwnerProof;$r=Receipt;Host $s $r $hostProof
Assert ($s.HostNoOwners -and $r.host_no_owner_startup_proven -and -not $r.host_actual_retirement -and $r.status -ceq 'failed') 'Bound proof became acceptance or failed to retain evidence';$cases++
foreach($line in @($hostProof.Replace('pid=123','pid=124'),$hostProof.Replace($run,('f'*32)),$hostProof.Replace('pid=123','pid=9999999999'))){
 $s=New-M4StartupOwnerProof;$r=Receipt;Host $s $r $line;Assert (-not $s.HostNoOwners) 'Unbound host proof accepted';$cases++
}
$s=New-M4StartupOwnerProof;$r=Receipt;Host $s $r 'REMOTE_HOST_PHASE:host_owners_starting';Host $s $r $hostProof
Assert (-not $s.HostNoOwners) 'Post-owner host refusal substituted no-owner proof';$cases++
$s=New-M4StartupOwnerProof;$r=Receipt;Host $s $r $hostProof;Host $s $r $hostProof
Assert (-not $s.HostNoOwners) 'Duplicate host proof accepted';$cases++
foreach($order in @(@(0,1,2),@(1,0,2))){
 $s=New-M4StartupOwnerProof;$r=Receipt;Phone $s $r @($phoneProof[$order[0]],$phoneProof[$order[1]])
 Assert (-not $s.PhoneNoOwners) 'Partial status bundle accepted'
 Phone $s $r @($phoneProof[$order[2]])
 Assert ($s.PhoneNoOwners -and $r.phone_no_owner_startup_proven -and -not $r.phone_actual_retirement -and $r.status -ceq 'failed') 'Complete exact phone proof not retained';$cases++
}
$invalid=@(
 @($phoneProof[0].Replace($run,('f'*32)),$phoneProof[1],$phoneProof[2]),
 @($phoneProof[0],$phoneProof[1],'INSTRUMENTATION_STATUS: extra=unknown',$phoneProof[2]),
 @($phoneProof[0],$phoneProof[0],$phoneProof[1],$phoneProof[2]),
 @($phoneProof[0],$phoneProof[1],'INSTRUMENTATION_STATUS_CODE: -2'),
 @($phoneProof[1],$phoneProof[2]),
 @($phoneProof[0],$phoneProof[2])
)
foreach($lines in $invalid){$s=New-M4StartupOwnerProof;$r=Receipt;Phone $s $r $lines;Assert (-not $s.PhoneNoOwners) 'Malformed phone proof accepted';$cases++}
foreach($prior in @('INSTRUMENTATION_STATUS: remote_phase=phone_owners_starting','INSTRUMENTATION_STATUS: remote_phase=controller_input_ready','INSTRUMENTATION_STATUS: remote_render_ticket=1')){
 $s=New-M4StartupOwnerProof;$r=Receipt;Phone $s $r @($prior,'INSTRUMENTATION_STATUS_CODE: 0');Phone $s $r $phoneProof
 Assert (-not $s.PhoneNoOwners) 'Post-owner phone proof accepted';$cases++
}
$s=New-M4StartupOwnerProof;$r=Receipt;Phone $s $r $phoneProof;Phone $s $r @('INSTRUMENTATION_STATUS: remote_phase=phone_owners_starting','INSTRUMENTATION_STATUS_CODE: 0')
Assert (-not $s.PhoneNoOwners -and -not $r.phone_no_owner_startup_proven) 'Later owner-start phase left no-owner authority';$cases++
$s=New-M4StartupOwnerProof;$r=Receipt
Add-M4StartupOwnerProof $s $r 'owned-wgc-target' $hostProof $run 123
Assert (-not $s.HostNoOwners) 'Wrong producer accepted';$cases++
Write-Output "Startup owner proof cases passed: $cases"
