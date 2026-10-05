#requires -Version 7.2
[CmdletBinding()]
param([string]$Runner=(Join-Path $PSScriptRoot 'integration.ps1'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($Runner,[ref]$tokens,[ref]$errors)
if($errors.Count){throw 'Runner syntax errors'}
# Parse and execute only the two pure text classifiers; never the runner body.
foreach($name in @('Get-IntegrationDiagnosticLine','Add-IntegrationDiagnostic')){
    $function=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name},$true)
    if($null -eq $function){throw 'Missing classifier'}
    . ([scriptblock]::Create($function.Extent.Text))
}
$source=[IO.File]::ReadAllText($Runner)
foreach($name in @('streamCounterKeys','hostStreamCounterKeys')){
    $definition=$ast.Find({param($node) $node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -ceq ('$'+$name)},$true)
    . ([scriptblock]::Create($definition.Extent.Text))
}
$streamCounters=@{phone=@{};host=@{}}
$diagnostics=[Collections.Generic.List[object]]::new();$diagnosticTotal=0;$diagnosticDropped=0;$phoneRetired=$false
$ControllerInput=$false
$count=0
function Assert-Case([bool]$Pass,[string]$Name){if(-not $Pass){throw ('Counter case failed: '+$Name)};$script:count++}
function Host-Json([string]$Number='1'){
    return '{'+(($hostStreamCounterKeys|ForEach-Object {'"'+$_+'":'+$Number}) -join ',')+'}'
}
Add-IntegrationDiagnostic 'real-native-session-host' ('REMOTE_HOST_STREAM:'+(Host-Json))
Assert-Case ($streamCounters.host.Count -eq 11) 'closed host shape accepted'
Add-IntegrationDiagnostic 'real-phone-controller' 'INSTRUMENTATION_STATUS: remote_diag_stream_rendered=18446744073709551615'
Assert-Case ($streamCounters.phone.rendered -eq [UInt64]::MaxValue) 'uint64 maximum accepted'
Add-IntegrationDiagnostic 'real-phone-controller' 'INSTRUMENTATION_STATUS: remote_diag_stream_rendered=0'
Assert-Case ($streamCounters.phone.rendered -eq [UInt64]::MaxValue) 'phone maxima monotonic'
Add-IntegrationDiagnostic 'real-native-session-host' ('REMOTE_HOST_STREAM:'+(Host-Json '3'))
Add-IntegrationDiagnostic 'real-native-session-host' ('REMOTE_HOST_STREAM:'+(Host-Json '2'))
Assert-Case ($streamCounters.host.capture_polls -eq 3) 'host maxima monotonic'
foreach($line in @(
    'REMOTE_HOST_STREAM:{"capture_polls":9}',
    ('REMOTE_HOST_STREAM:'+((Host-Json '9').Replace('"capture_idle":9','"capture_polls":9'))),
    ('REMOTE_HOST_STREAM:'+((Host-Json '9').Replace('"capture_idle":9','"unknown":9'))),
    ('REMOTE_HOST_STREAM:'+((Host-Json '9').Replace('"capture_idle":9','"capture_idle":18446744073709551616'))),
    ('REMOTE_HOST_STREAM:'+((Host-Json '9').TrimEnd('}'))),
    ('REMOTE_HOST_STREAM:'+(' '*4097)),
    (' REMOTE_HOST_STREAM:'+(Host-Json '9')),
    ('REMOTE_HOST_STREAM:'+(Host-Json '9')+' trailing')
)){
    Add-IntegrationDiagnostic 'real-native-session-host' $line
    Assert-Case ($streamCounters.host.capture_polls -eq 3 -and $streamCounters.host.Count -eq 11) 'invalid host input cannot partly update'
}
foreach($line in @(
    'INSTRUMENTATION_STATUS: remote_diag_stream_capture_idle=18446744073709551616',
    'INSTRUMENTATION_STATUS: remote_diag_stream_unknown=1',
    ' INSTRUMENTATION_STATUS: remote_diag_stream_capture_idle=1',
    'INSTRUMENTATION_STATUS: remote_diag_stream_capture_idle=1 trailing',
    'INSTRUMENTATION_STATUS: remote_diag_stream_capture_idle=-1'
)){
    Add-IntegrationDiagnostic 'real-phone-controller' $line
    Assert-Case ($streamCounters.phone.Count -eq 1) 'invalid phone input ignored'
}
for($i=0;$i -lt 100;$i++){Add-IntegrationDiagnostic 'real-phone-controller' 'INSTRUMENTATION_STATUS: remote_phase=waiting_native_render_callback'}
Assert-Case ($diagnostics.Count -eq 64 -and $diagnosticDropped -eq 36) 'bounded diagnostic ring rotates'
Assert-Case ($streamCounters.host.capture_polls -eq 3 -and $streamCounters.phone.rendered -eq [UInt64]::MaxValue) 'counters survive ring overflow'
Add-IntegrationDiagnostic 'unrelated' 'INSTRUMENTATION_STATUS: remote_diag_stream_capture_idle=99'
Assert-Case ($streamCounters.phone.Count -eq 1) 'foreign owner ignored'
Write-Output ('Stream aggregate counter cases passed: '+$count)
