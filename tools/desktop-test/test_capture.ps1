#requires -Version 7.2
# Synthetic child processes only. The parent executes this through its bounded
# Windows Job lane; this source author does not launch it.
[CmdletBinding()]
param([string]$ProjectRoot=(Split-Path -Parent (Split-Path -Parent $PSScriptRoot)))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $IsWindows){throw 'Windows capture fixture required.'}
Import-Module (Join-Path $ProjectRoot 'tools/process.psm1')
Import-Module (Join-Path $PSScriptRoot 'capture.psm1')
$pwsh=Join-Path $PSHOME 'pwsh.exe'
function Child([string]$Body,[int]$Seconds=10,[int]$Limit=4096,[switch]$RetainFailureOutput){
    $code=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($Body))
    Invoke-VwBoundedDesktopChild -Executable $pwsh -Arguments @('-NoProfile','-NonInteractive','-EncodedCommand',$code) -WorkingDirectory $ProjectRoot -TimeoutSeconds $Seconds -ByteLimit $Limit -RetainFailureOutput:$RetainFailureOutput
}
function Assert-Clean($Value){
    if(-not $Value.Proof.contained_in_windows_job -or -not $Value.Proof.process_tree_cleanup_confirmed -or -not $Value.Proof.output_streams_completed){throw 'Owned fixture tree or stream cleanup failed.'}
    if($Value.Proof.captured_bytes -ge $Value.Proof.capture_byte_limit){throw 'Reader retained bytes above its bound.'}
}
$normal=Child '[Console]::Out.Write("first");[Console]::Error.Write("second")'
Assert-Clean $normal
if($normal.ExitCode -ne 0 -or [Text.Encoding]::UTF8.GetString($normal.Bytes) -cne "first`nsecond"){throw 'Bounded exact streams changed.'}
Write-Host '[desktop-capture-fixtures] exact-streams PASS'

foreach($body in @(
    '[Console]::Out.Write((''x'' * 2097152));Start-Sleep -Seconds 60',
    'for($i=0;$i -lt 100000;$i++){[Console]::Out.WriteLine(''a bounded fixture line'')};Start-Sleep -Seconds 60',
    '[Console]::Error.Write((''e'' * 2097152));Start-Sleep -Seconds 60'
)){
    $value=Child $body
    Assert-Clean $value
    if($value.ExitCode -eq 0 -or $value.Proof.failure_reason -cne 'output_limit' -or -not $value.Proof.capture_overflow -or $null -ne $value.Bytes){throw 'Noisy output was not an explicit bounded failure.'}
}
Write-Host '[desktop-capture-fixtures] newline-unterminated-stderr-overflow PASS'

$timeout=Child 'Start-Sleep -Seconds 60' 1
Assert-Clean $timeout
if($timeout.ExitCode -ne 124 -or $timeout.Proof.failure_reason -cne 'timeout' -or $null -ne $timeout.Bytes){throw 'Timeout fixture did not fail with cleanup.'}
Write-Host '[desktop-capture-fixtures] timeout PASS'

$nonzero=Child 'exit 17'
Assert-Clean $nonzero
if($nonzero.ExitCode -ne 17 -or $null -ne $nonzero.Bytes){throw 'Nonzero child was accepted.'}
Write-Host '[desktop-capture-fixtures] nonzero PASS'

$diagnostic=Child '[Console]::Error.Write("bounded failure");exit 17' -RetainFailureOutput
Assert-Clean $diagnostic
try {
    if($diagnostic.ExitCode -ne 17 -or [Text.Encoding]::UTF8.GetString($diagnostic.Bytes) -cne "`nbounded failure"){throw 'Failure diagnostic changed the child status or bytes.'}
} finally {if($null -ne $diagnostic.Bytes){[Array]::Clear($diagnostic.Bytes,0,$diagnostic.Bytes.Length)}}
$diagnosticOverflow=Child '[Console]::Error.Write((''x'' * 2097152));exit 17' -RetainFailureOutput
Assert-Clean $diagnosticOverflow
if($diagnosticOverflow.ExitCode -eq 0 -or $diagnosticOverflow.Proof.failure_reason -cne 'output_limit' -or $null -ne $diagnosticOverflow.Bytes){throw 'Diagnostic output bypassed the byte limit.'}
Write-Host '[desktop-capture-fixtures] bounded-failure-diagnostic PASS'

# A descendant is kept alive until the test's exact Job timeout. The fixture
# never searches for or kills processes by name or touches unrelated instances.
$descendant=Child '$arg=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes(''Start-Sleep -Seconds 60''));$p=Start-Process -WindowStyle Hidden -FilePath (Join-Path $PSHOME ''pwsh.exe'') -ArgumentList @(''-NoProfile'',''-NonInteractive'',''-EncodedCommand'',$arg) -PassThru;Start-Sleep -Seconds 60' 3
Assert-Clean $descendant
if($descendant.ExitCode -ne 124 -or $descendant.Proof.failure_reason -cne 'timeout'){throw 'Owned descendant timeout was not refused.'}
Write-Host '[desktop-capture-fixtures] descendant-cleanup PASS'

$badUtf8=Child '$s=[Console]::OpenStandardOutput();$s.Write([byte[]](255,254),0,2);$s.Flush()'
Assert-Clean $badUtf8
if($badUtf8.ExitCode -eq 0 -or $badUtf8.Proof.failure_reason -cne 'output_encoding'){throw 'Malformed UTF-8 output was accepted.'}
Write-Host '[desktop-capture-fixtures] invalid-utf8 PASS'
