#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
$count=0
function Assert-Case([bool]$Pass,[string]$Name){if(-not $Pass){throw ('Input owner contract failed: '+$Name)};$script:count++}
$nonce='0123456789abcdef0123456789abcdef'
function Gate {
    # Deserialize exactly as the runner does, including integer CLR types.
    return ('{"schema":"M4_INPUT_PARENT_V1","run_id":"'+$nonce+'","nonce":"'+$nonce+'","assigned":true,"child_pid":101,"child_created":"134000000000000001","parent_pid":100,"parent_created":"134000000000000000"}'|ConvertFrom-Json)
}
function Valid($g){Test-M4InputParentGate $g $nonce 101 '134000000000000001' 100 '134000000000000000'}
Assert-Case (Valid (Gate)) 'actual assignment identities accepted'
Assert-Case (-not(Valid $null)) 'missing startup gate refused'
$g=Gate;$g.assigned=$false;Assert-Case (-not(Valid $g)) 'unassigned refused'
$g=Gate;$g.assigned='true';Assert-Case (-not(Valid $g)) 'string assignment refused'
$g=Gate;$g.child_pid=102L;Assert-Case (-not(Valid $g)) 'foreign runner refused'
$g=Gate;$g.parent_pid=99L;Assert-Case (-not(Valid $g)) 'foreign parent refused'
$g=Gate;$g.child_created='134000000000000002';Assert-Case (-not(Valid $g)) 'reused child pid refused'
$g=Gate;$g.parent_created='134000000000000002';Assert-Case (-not(Valid $g)) 'reused parent pid refused'
$g=Gate;$g.nonce='ffffffffffffffffffffffffffffffff';Assert-Case (-not(Valid $g)) 'foreign nonce refused'
$g=Gate;$g.run_id='ffffffffffffffffffffffffffffffff';Assert-Case (-not(Valid $g)) 'foreign run refused'
$g=Gate;$g|Add-Member -NotePropertyName unknown -NotePropertyValue 1;Assert-Case (-not(Valid $g)) 'extra gate field refused'
$receipt="schema=1`nrun_id=$nonce`nhost_pid=101`nactual_owners_retired=true`ninput_grant_attempted=true`ninput_granted=null`ncontroller_input_complete=false`neditor_effect=false`nlatency_acceptance=false`n"
Assert-Case (Test-M4InputHostRetired $receipt $nonce 101) 'actual settlement with uncertain grant accepted'
Assert-Case (-not(Test-M4InputHostRetired ($receipt.Replace('actual_owners_retired=true','actual_owners_retired=false')) $nonce 101)) 'pending host refused'
Assert-Case (-not(Test-M4InputHostRetired $receipt $nonce 102)) 'foreign host terminal refused'
Assert-Case (-not(Test-M4InputHostRetired ($receipt+"actual_owners_retired=true`n") $nonce 101)) 'duplicate terminal refused'
Assert-Case (-not(Test-M4InputSettlement $null $true)) 'unknown native remains held'
Assert-Case (-not(Test-M4InputSettlement $true $false)) 'pending phone remains held'
Assert-Case (Test-M4InputSettlement $true $true) 'both actual retirements permit release'
Assert-Case (-not(Test-M4InputSettlement 'true' $true)) 'truthy text does not prove settlement'
$runner=[pscustomobject]@{run_id=$nonce;retained_input_nonce=$nonce;controller_input_requested=$true;input_retirement_confirmed=$true;host_actual_retirement=$true;phone_actual_retirement=$true;process_cleanup=$true}
Assert-Case (Test-M4InputRunnerRetired $runner $nonce) 'correlated actual runner settlement accepted'
Assert-Case (-not(Test-M4InputRunnerRetired $null $nonce)) 'process exit without receipt remains pending'
$runner.phone_actual_retirement=$false
Assert-Case (-not(Test-M4InputRunnerRetired $runner $nonce)) 'empty outer Job cannot replace phone retirement'
$runner.phone_actual_retirement=$true;$runner.retained_input_nonce='ffffffffffffffffffffffffffffffff'
Assert-Case (-not(Test-M4InputRunnerRetired $runner $nonce)) 'foreign runner receipt refused'
Write-Output ('Input owner contract cases passed: '+$count)
