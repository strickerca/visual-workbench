#requires -Version 7.2
<#
Source-only companion to the real pair-cli. Run only after the gated Windows and
Android CLI builds. This does not build, install an app, change routes/firewall,
read existing trust contents, or invoke the interactive pairing fallback.

Example: pwsh -File tools/pair-cli/run_adb.ps1 -WindowsExecutable <built exe>
  -AndroidExecutable <built arm64 ELF>

The two loopback reverse mappings are created with --no-rebind. The QR credential
is copied directly from the protected app trust directory to one private phone
directory; its contents and hash never enter a log or receipt. Successful CLI
revocation records remain in app trust. A failed run does not claim revocation.
#>
[CmdletBinding()]
param(
    [string]$ProjectRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [string]$WindowsExecutable,
    [string]$AndroidExecutable,
    [ValidateSet('IN2019')][string]$ExpectedModel = 'IN2019',
    [ValidateRange(60, 300)][int]$TimeoutSeconds = 180
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'The pairing host harness requires Windows.' }
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
Import-Module (Join-Path $ProjectRoot 'tools/process.psm1')
Import-Module (Join-Path $ProjectRoot 'tools/android-device.psm1')
if (-not $WindowsExecutable) { $WindowsExecutable = Join-Path $ProjectRoot 'target/debug/pair-cli.exe' }
if (-not $AndroidExecutable) { $AndroidExecutable = Join-Path $ProjectRoot 'target/aarch64-linux-android/debug/pair-cli' }

function Assert-PlainPath([string]$Path, [switch]$Directory) {
    $full = [IO.Path]::GetFullPath($Path)
    $item = Get-Item -LiteralPath $full -Force
    if ($item.PSIsContainer -ne [bool]$Directory) { throw 'Unexpected owned path type.' }
    $cursor = $item
    while ($null -ne $cursor) {
        if (($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Reparse-point path refused.' }
        $cursor = if ($cursor -is [IO.DirectoryInfo]) { $cursor.Parent } else { $cursor.Directory }
    }
    return $full
}
function Start-OwnedProcess([string]$Role, [string]$Executable, [string[]]$Arguments) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    foreach ($argument in $Arguments) { $info.ArgumentList.Add($argument) }
    $info.WorkingDirectory = $ProjectRoot
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    foreach ($name in @($info.Environment.Keys)) {
        if ($name -match '(?i)(TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY)') { $info.Environment.Remove($name) | Out-Null }
    }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $info
    $output = [VwProcessOutputV4]::new(@($ProjectRoot, $env:USERPROFILE, $device.Serial, $remote, $transferPath))
    $output.Attach($process)
    $job = [VwProcessJobV4]::new()
    $started = $false
    $assigned = $false
    try {
        if (-not $process.Start()) { throw 'Owned process did not start.' }
        $started = $true
        $job.Assign($process)
        $assigned = $true
        $process.BeginOutputReadLine()
        $process.BeginErrorReadLine()
        return [pscustomobject]@{
            Role = $Role; Process = $process; Job = $job; Output = $output
            Markers = [Collections.Generic.List[string]]::new(); ExitedAt = $null
        }
    } catch {
        if ($started -and -not $assigned) { $script:startupCleanup = $false }
        try {
            if ($started -and -not $process.HasExited) { $process.Kill($true); if (-not $process.WaitForExit(5000)) { $script:startupCleanup = $false } }
            if ($assigned -and $job.ActiveProcessCount -ne 0) { $job.Terminate(124); $script:startupCleanup = $false }
        } catch { $script:startupCleanup = $false }
        $job.Dispose(); $process.Dispose()
        throw 'Could not establish owned process containment.'
    }
}
function Read-OwnedMarkers($Owned) {
    $line = $null
    $count = 0
    while ($count -lt 128 -and $Owned.Output.Lines.TryDequeue([ref]$line)) {
        $count++
        # Fail closed without retaining or echoing unrecognized output. Only
        # the nonsecret static CLI markers below may enter this receipt.
        $allowed = if ($Owned.Role -eq 'host') {
            @('PAIRING_READY', 'PAIRING_WAIT', 'PAIR_OK', 'OPS_READY', 'OPS_OK count=1', 'HOST_REVOKED', 'COMPLETE role=host ops=1 refused=1')
        } else { @('PAIR_OK', 'OPS_OK count=1', 'COMPLETE role=phone ops=1 refused=1 local_revoked=1') }
        if ($line -notin $allowed -or $Owned.Markers.Count -ge 128) { throw 'CLI output failed the bounded static-marker contract.' }
        $Owned.Markers.Add($line)
    }
    if (-not $Owned.Output.Lines.IsEmpty) { throw 'CLI output exceeded its bounded marker rate.' }
    if ($Owned.Process.HasExited -and $null -eq $Owned.ExitedAt) { $Owned.ExitedAt = $watch.Elapsed.TotalSeconds }
    if ($null -ne $Owned.ExitedAt -and $watch.Elapsed.TotalSeconds - $Owned.ExitedAt -gt 5 -and
        ($Owned.Job.ActiveProcessCount -ne 0 -or -not $Owned.Output.IsComplete)) { throw 'CLI descendants or output failed to settle.' }
}
function Stop-OwnedProcess($Owned) {
    if ($null -eq $Owned) { return $true }
    $settled = $false
    try {
        if ($Owned.Job.ActiveProcessCount -gt 0) { $Owned.Job.Terminate(124) }
        $end = [Diagnostics.Stopwatch]::StartNew()
        while ($Owned.Job.ActiveProcessCount -gt 0 -and $end.Elapsed.TotalSeconds -lt 10) { Start-Sleep -Milliseconds 100 }
        $settled = $Owned.Job.ActiveProcessCount -eq 0
    } finally {
        if (-not $Owned.Output.IsComplete) {
            try { $Owned.Process.CancelOutputRead() } catch { }
            try { $Owned.Process.CancelErrorRead() } catch { }
        }
        $Owned.Job.Dispose(); $Owned.Process.Dispose()
    }
    return $settled
}
function Invoke-PairAdb([string[]]$Arguments, [string]$Phase, [switch]$Cleanup) {
    $seconds = if ($Cleanup) { 25 } else { [int][Math]::Min(30, [Math]::Ceiling($TimeoutSeconds - $watch.Elapsed.TotalSeconds)) }
    if ($seconds -le 0) { throw 'Pairing harness deadline exceeded.' }
    # Invoke-VwAdb runs adb.exe directly and retains captured output in memory
    # only. Even adb push errors cannot expose private local transfer paths.
    return Invoke-VwAdb -Device $device -Arguments $Arguments -Phase $Phase -TimeoutSeconds $seconds -Capture
}
function Get-OwnedMappings {
    $listing = Invoke-PairAdb -Arguments @('reverse', '--list') -Phase 'pair-reverse-inventory' -Cleanup
    $rows = @()
    foreach ($line in $listing.Lines) {
        $parts = $line.Trim() -split '\s+'
        if ($parts.Count -eq 3) { $rows += [pscustomobject]@{ Remote = $parts[1]; Local = $parts[2] } }
        elseif ($line.Trim()) { throw 'Unrecognized reverse mapping inventory.' }
    }
    return $rows
}
function Get-FreeLoopbackPort {
    $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
    try { $listener.Start(); return ([Net.IPEndPoint]$listener.LocalEndpoint).Port }
    finally { $listener.Stop() }
}
function Remove-OwnedPhoneArtifacts {
    if ($remote -notmatch '^/data/local/tmp/vw-pair-[0-9a-f]{32}$') { throw 'Unowned phone cleanup target.' }
    # Match only this task's exact executable/script argv. PID reuse cannot
    # authorize killing another executable, and no name-wide kill is used.
    # pair-cli has no subprocess API; all matching owned processes are checked
    # again after TERM and KILL. Unrecognized survivors fail cleanup.
    $pattern = '^(' + $remote + '/pair-cli|(/system/bin/sh|sh) ' + $remote + '/run.sh)( |$)'
    $command = @'
set -eu
d='__DIRECTORY__'
pat='__PATTERN__'
owner='__OWNER__'
test ! -L "$d"
if [ ! -e "$d" ]; then exit 0; fi
test -d "$d"
prove_owner() {
  test ! -L "$d/.owner-v1" && test -f "$d/.owner-v1" &&
    test "$(wc -c < "$d/.owner-v1")" -eq 64 &&
    test "$(cat "$d/.owner-v1")" = "$owner"
}
# A failed/uncertain mkdir never grants cleanup authority. The exclusive marker
# was written only after this run's mkdir succeeded, and its token is separate
# from the directory name. Inspect no process or child before that proof.
prove_owner
if [ -e "$d/client.pid" ]; then
  test ! -L "$d/client.pid"
  test "$(wc -c < "$d/client.pid")" -le 20
  p=$(cat "$d/client.pid")
  case "$p" in ''|*[!0-9]*) exit 1;; esac
fi
owned() {
  pgrep -f "$pat" && return 0
  code=$?
  test "$code" -eq 1 || return 2
}
ids=$(owned) || exit 1
for p in $ids; do
  test -d "/proc/$p" || continue
  current=$(tr '\000' ' ' < "/proc/$p/cmdline") || exit 1
  printf '%s' "$current" | grep -Eq "$pat" || exit 1
  # The reviewed CLI is process-leaf-only. An unexpected descendant is a failed
  # ownership proof, never permission for a broad process-tree or name kill.
  if pgrep -P "$p" >/dev/null; then exit 1; else test "$?" -eq 1; fi
  kill -TERM "$p" 2>/dev/null || test ! -d "/proc/$p"
done
for pass in 1 2 3 4 5; do
  ids=$(owned) || exit 1
  test -n "$ids" || break
  sleep 1
done
ids=$(owned) || exit 1
for p in $ids; do
  current=$(tr '\000' ' ' < "/proc/$p/cmdline") || exit 1
  printf '%s' "$current" | grep -Eq "$pat" || exit 1
  kill -KILL "$p" 2>/dev/null || test ! -d "/proc/$p"
done
for pass in 1 2 3 4 5; do
  ids=$(owned) || exit 1
  test -n "$ids" || break
  sleep 1
done
test -z "$ids"
prove_owner
for leaf in pair-cli pair.qr run.sh client.pid; do test ! -L "$d/$leaf"; done
rm -f "$d/pair-cli" "$d/pair.qr" "$d/run.sh" "$d/client.pid"
rm "$d/.owner-v1"
rmdir "$d"
test ! -e "$d"
'@
    $command = $command.Replace('__DIRECTORY__', $remote).Replace('__PATTERN__', $pattern).Replace('__OWNER__', $phoneOwner)
    Invoke-PairAdb -Arguments @('shell', $command) -Phase 'pair-owned-phone-cleanup' -Cleanup | Out-Null
}

$runId = [guid]::NewGuid().ToString('N')
$transferName = 'pair-hil-' + $runId
$trustRoot = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)) 'Visual Workbench/trust'
$transferPath = [IO.Path]::GetFullPath((Join-Path $trustRoot ($transferName + '.qr')))
$remote = '/data/local/tmp/vw-pair-' + $runId
$phoneOwner = [guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N')
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$local = Join-Path $tempRoot ('VisualWorkbench-pair-adb-' + $runId)
$scriptPath = Join-Path $local 'run.sh'
$device = $null; $hostOwned = $null; $phoneOwned = $null
$phoneCreationAttempted = $false; $claimedPhone = $false; $claimedLocal = $false; $claimedTransfer = $false
$mappings = [Collections.Generic.List[string]]::new()
$removedMappings = 0
$mappingUncertain = $false; $script:startupCleanup = $true
$hostCleanup = $true; $phoneJobCleanup = $true; $deviceCleanup = $true; $localCleanup = $true; $transferCleanup = $true
$success = $false; $failedPhase = $null; $phase = 'prerequisites'
$hostMarkers = @(); $phoneMarkers = @(); $hostExit = $null; $phoneExit = $null
$hostHash = $null; $phoneHash = $null; $sources = [ordered]@{}
$watch = [Diagnostics.Stopwatch]::StartNew()
try {
    $WindowsExecutable = Assert-PlainPath $WindowsExecutable
    $AndroidExecutable = Assert-PlainPath $AndroidExecutable
    if ($WindowsExecutable -cne [IO.Path]::GetFullPath((Join-Path $ProjectRoot 'target/debug/pair-cli.exe')) -or $AndroidExecutable -cne [IO.Path]::GetFullPath((Join-Path $ProjectRoot 'target/aarch64-linux-android/debug/pair-cli'))) { throw 'Only the centrally gated pairing outputs are admitted.' }
    $binding = Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/pair-cli/build_receipt.py', 'check') -WorkingDirectory $ProjectRoot -Phase 'pair-build-binding' -TimeoutSeconds 30 -Capture
    if ($binding.ExitCode -ne 0) { throw 'Pairing source/binary build binding differs.' }
    if ([IO.Path]::GetFileName($WindowsExecutable) -cne 'pair-cli.exe' -or [IO.Path]::GetFileName($AndroidExecutable) -cne 'pair-cli') { throw 'Expected the two gated pairing executables.' }
    $header = [byte[]]::new(20)
    $file = [IO.File]::OpenRead($AndroidExecutable)
    try {
        if ($file.Read($header, 0, 20) -ne 20 -or $header[0] -ne 127 -or $header[1] -ne 69 -or $header[2] -ne 76 -or $header[3] -ne 70 -or $header[4] -ne 2 -or $header[5] -ne 1 -or $header[18] -ne 183 -or $header[19] -ne 0) { throw 'Android pairing executable must be an arm64 little-endian ELF.' }
    } finally { $file.Dispose() }
    $hostHash = (Get-FileHash -LiteralPath $WindowsExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    $phoneHash = (Get-FileHash -LiteralPath $AndroidExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    foreach ($relative in @('Cargo.lock', 'contracts/vw_protocol.proto', 'tools/pair-cli/src/main.rs', 'tools/pair-cli/Cargo.toml', 'tools/process.psm1', 'tools/android-device.psm1')) {
        $sources[$relative] = (Get-FileHash -LiteralPath (Join-Path $ProjectRoot $relative) -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $sources['tools/pair-cli/run_adb.ps1'] = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if (Test-Path -LiteralPath $transferPath) { throw 'Unique transfer path already exists.' }
    if (Test-Path -LiteralPath $local) { throw 'Unique local harness directory already exists.' }
    [IO.Directory]::CreateDirectory($local) | Out-Null; $claimedLocal = $true
    $null = Assert-PlainPath $local -Directory
    $phase = 'explicit-device-selection'
    $device = Get-VwAndroidDevice -Root $ProjectRoot -ExpectedModel 'IN2019'
    if ($device.Model -cne $ExpectedModel) { throw 'Device authorization mismatch.' }
    $phase = 'owned-phone-directory'
    $phoneCreationAttempted = $true; $deviceCleanup = $false
    $createPhone = @'
set -eu
umask 077
d='__DIRECTORY__'
test ! -e "$d"
test ! -L "$d"
mkdir -m 700 "$d"
(set -C; printf '%s' '__OWNER__' > "$d/.owner-v1")
test ! -L "$d/.owner-v1"
test -f "$d/.owner-v1"
test "$(wc -c < "$d/.owner-v1")" -eq 64
test "$(cat "$d/.owner-v1")" = '__OWNER__'
'@
    $createPhone = $createPhone.Replace('__DIRECTORY__', $remote).Replace('__OWNER__', $phoneOwner)
    Invoke-PairAdb -Arguments @('shell', $createPhone) -Phase 'pair-owned-phone-directory' | Out-Null
    $claimedPhone = $true
    Invoke-PairAdb -Arguments @('push', $AndroidExecutable, ($remote + '/pair-cli')) -Phase 'pair-owned-binary-transfer' | Out-Null
    Invoke-PairAdb -Arguments @('shell', 'chmod', '700', ($remote + '/pair-cli')) -Phase 'pair-owned-binary-permissions' | Out-Null
    $hashOutput = Invoke-PairAdb -Arguments @('shell', 'sha256sum', ($remote + '/pair-cli')) -Phase 'pair-binary-binding'
    if ($hashOutput.Lines.Count -ne 1 -or $hashOutput.Lines[0] -notmatch '^([0-9a-f]{64})\s+' -or $Matches[1] -cne $phoneHash) { throw 'Transferred Android binary differs.' }
    $pairPort = Get-FreeLoopbackPort
    $opsPort = Get-FreeLoopbackPort
    if ($pairPort -eq $opsPort) { throw 'Distinct loopback ports were unavailable.' }
    $phase = 'host-ready'
    $claimedTransfer = $true; $transferCleanup = $false
    $hostOwned = Start-OwnedProcess 'host' $WindowsExecutable @('host-qr-tcp', ('127.0.0.1:' + $pairPort), ('127.0.0.1:' + $opsPort), $transferName)
    $hostCleanup = $false
    $readyStart = $watch.Elapsed.TotalSeconds
    $readyProgress = $readyStart
    while (-not $hostOwned.Markers.Contains('PAIRING_READY')) {
        Read-OwnedMarkers $hostOwned
        if ($hostOwned.Process.HasExited -or $watch.Elapsed.TotalSeconds - $readyStart -ge 20 -or $watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw 'Pairing readiness failed.' }
        if ($watch.Elapsed.TotalSeconds - $readyProgress -ge 10) { Write-Host '[pair-host-ready] RUNNING'; $readyProgress = $watch.Elapsed.TotalSeconds }
        Start-Sleep -Milliseconds 100
    }
    $null = Assert-PlainPath $trustRoot -Directory
    $null = Assert-PlainPath $transferPath
    $transferMetadata = Get-Item -LiteralPath $transferPath
    if ($transferMetadata.Length -le 0 -or $transferMetadata.Length -gt 4096 -or $transferMetadata.DirectoryName -cne [IO.Path]::GetFullPath($trustRoot)) { throw 'Private transfer metadata is invalid.' }
    $phase = 'owned-reverse'
    foreach ($port in @($pairPort, $opsPort)) {
        $mapping = 'tcp:' + $port
        $mappingUncertain = $true
        Invoke-PairAdb -Arguments @('reverse', '--no-rebind', $mapping, $mapping) -Phase 'pair-owned-reverse-create' | Out-Null
        $mappings.Add($mapping)
        $mappingUncertain = $false
    }
    $currentMappings = @(Get-OwnedMappings)
    foreach ($mapping in $mappings) {
        if (@($currentMappings | Where-Object { $_.Remote -ceq $mapping -and $_.Local -ceq $mapping }).Count -ne 1) { throw 'Owned reverse mapping was not confirmed.' }
    }
    $phase = 'private-transfer'
    # Never read, hash, stage, print or copy this credential through local scratch.
    Invoke-PairAdb -Arguments @('push', $transferPath, ($remote + '/pair.qr')) -Phase 'pair-private-transfer' | Out-Null
    Invoke-PairAdb -Arguments @('shell', 'chmod', '600', ($remote + '/pair.qr')) -Phase 'pair-private-transfer-permissions' | Out-Null
    $script = '#!/system/bin/sh' + "`n" + 'set -eu' + "`n" + 'umask 077' + "`n" +
        'cd ' + $remote + "`n" + 'echo $$ > client.pid' + "`n" +
        'exec ' + $remote + '/pair-cli phone-qr-tcp ' + $remote + '/pair.qr 127.0.0.1:' + $opsPort + "`n"
    [IO.File]::WriteAllText($scriptPath, $script, [Text.UTF8Encoding]::new($false))
    Invoke-PairAdb -Arguments @('push', $scriptPath, ($remote + '/run.sh')) -Phase 'pair-owned-phone-script' | Out-Null
    $phase = 'pair-ops-revocation'
    $phoneOwned = Start-OwnedProcess 'phone' $device.AdbPath @('-s', $device.Serial, 'shell', 'sh', ($remote + '/run.sh'))
    $phoneJobCleanup = $false
    $progress = $watch.Elapsed.TotalSeconds
    while ($true) {
        Read-OwnedMarkers $hostOwned; Read-OwnedMarkers $phoneOwned
        if ($hostOwned.Process.HasExited -and $phoneOwned.Process.HasExited -and $hostOwned.Output.IsComplete -and $phoneOwned.Output.IsComplete) { break }
        if ($watch.Elapsed.TotalSeconds -ge $TimeoutSeconds) { throw 'Pairing/OPS/revocation deadline exceeded.' }
        if ($watch.Elapsed.TotalSeconds - $progress -ge 10) { Write-Host ('[pair-ops-revocation] RUNNING ' + [int]$watch.Elapsed.TotalSeconds + 's'); $progress = $watch.Elapsed.TotalSeconds }
        Start-Sleep -Milliseconds 100
    }
    $hostExit = $hostOwned.Process.ExitCode; $phoneExit = $phoneOwned.Process.ExitCode
    $hostMarkers = $hostOwned.Markers.ToArray(); $phoneMarkers = $phoneOwned.Markers.ToArray()
    foreach ($marker in @('PAIRING_READY', 'PAIR_OK', 'OPS_READY', 'OPS_OK count=1', 'HOST_REVOKED', 'COMPLETE role=host ops=1 refused=1')) {
        if (@($hostMarkers | Where-Object { $_ -ceq $marker }).Count -ne 1) { throw 'Host did not prove every required stage exactly once.' }
    }
    foreach ($marker in @('PAIR_OK', 'OPS_OK count=1', 'COMPLETE role=phone ops=1 refused=1 local_revoked=1')) {
        if (@($phoneMarkers | Where-Object { $_ -ceq $marker }).Count -ne 1) { throw 'Phone did not prove every required stage exactly once.' }
    }
    if ($hostExit -ne 0 -or $phoneExit -ne 0 -or $hostOwned.Job.ActiveProcessCount -ne 0 -or $phoneOwned.Job.ActiveProcessCount -ne 0) { throw 'CLI completion or process ownership verification failed.' }
    $success = $true
} catch {
    # Exceptions can contain private paths or identifiers. Keep the failure phase
    # and static result, never the original exception/command/credential content.
    $failedPhase = $phase
    Write-Host ('[pair-adb] FAILED phase=' + $failedPhase)
} finally {
    if ($hostOwned) { $hostMarkers = $hostOwned.Markers.ToArray(); if ($hostOwned.Process.HasExited) { $hostExit = $hostOwned.Process.ExitCode } }
    if ($phoneOwned) { $phoneMarkers = $phoneOwned.Markers.ToArray(); if ($phoneOwned.Process.HasExited) { $phoneExit = $phoneOwned.Process.ExitCode } }
    try { $phoneJobCleanup = Stop-OwnedProcess $phoneOwned } catch { $phoneJobCleanup = $false }
    try { $hostCleanup = Stop-OwnedProcess $hostOwned } catch { $hostCleanup = $false }
    if ($phoneCreationAttempted -and $device) {
        try { Remove-OwnedPhoneArtifacts; $deviceCleanup = $true } catch { $deviceCleanup = $false }
    }
    if ($device) {
        foreach ($mapping in $mappings) {
            try {
                $existing = @(Get-OwnedMappings | Where-Object { $_.Remote -ceq $mapping })
                if ($existing.Count -eq 0) { $removedMappings++; continue }
                if ($existing.Count -ne 1 -or $existing[0].Local -cne $mapping) { throw 'Reverse ownership changed; preserving replacement.' }
                Invoke-PairAdb -Arguments @('reverse', '--remove', $mapping) -Phase 'pair-owned-reverse-cleanup' -Cleanup | Out-Null
                if (@(Get-OwnedMappings | Where-Object { $_.Remote -ceq $mapping }).Count -ne 0) { throw 'Reverse removal unconfirmed.' }
                $removedMappings++
            } catch { Write-Warning 'One owned reverse mapping could not be safely confirmed removed.' }
        }
    }
    if ($claimedTransfer) {
        try {
            if ([IO.Path]::GetDirectoryName($transferPath) -cne [IO.Path]::GetFullPath($trustRoot) -or [IO.Path]::GetFileName($transferPath) -cne ($transferName + '.qr')) { throw 'Unowned transfer cleanup path.' }
            if (Test-Path -LiteralPath $transferPath) { $null = Assert-PlainPath $transferPath; Remove-Item -LiteralPath $transferPath -Force }
            $transferCleanup = -not (Test-Path -LiteralPath $transferPath)
        } catch { $transferCleanup = $false }
    }
    if ($claimedLocal) {
        try {
            if ([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($local)) -cne $tempRoot.TrimEnd('\') -or [IO.Path]::GetFileName($local) -cne ('VisualWorkbench-pair-adb-' + $runId)) { throw 'Unowned local cleanup path.' }
            $null = Assert-PlainPath $local -Directory
            if (Test-Path -LiteralPath $scriptPath) { $null = Assert-PlainPath $scriptPath; Remove-Item -LiteralPath $scriptPath -Force }
            [IO.Directory]::Delete($local, $false); $localCleanup = -not (Test-Path -LiteralPath $local)
        } catch { $localCleanup = $false }
    }
    $watch.Stop()
    $cleanup = $hostCleanup -and $phoneJobCleanup -and $deviceCleanup -and $localCleanup -and $transferCleanup -and $removedMappings -eq $mappings.Count -and -not $mappingUncertain -and $script:startupCleanup
    $receipt = [ordered]@{
        schema = 1; harness = 'pair-cli-adb-reverse-tls'; device_model = 'IN2019'
        validation_succeeded = ($success -and $cleanup); failed_phase = $failedPhase
        elapsed_seconds = [Math]::Round($watch.Elapsed.TotalSeconds, 3); timeout_seconds = $TimeoutSeconds
        host_binary_sha256 = $hostHash; android_binary_sha256 = $phoneHash; source_sha256 = $sources
        source_binding_scope = 'current source and executable bytes; gated build provenance is a separate required receipt'
        host_exit = $hostExit; phone_exit = $phoneExit; host_markers = $hostMarkers; phone_markers = $phoneMarkers
        host_revocation_asserted = ($hostMarkers -contains 'COMPLETE role=host ops=1 refused=1')
        phone_revocation_asserted = ($phoneMarkers -contains 'COMPLETE role=phone ops=1 refused=1 local_revoked=1')
        owned_host_tree_cleanup_confirmed = $hostCleanup; owned_adb_tree_cleanup_confirmed = $phoneJobCleanup
        owned_phone_cleanup_confirmed = $deviceCleanup; owned_local_cleanup_confirmed = $localCleanup
        owned_phone_creation_confirmed = $claimedPhone
        owned_private_transfer_cleanup_confirmed = $transferCleanup
        owned_reverse_created = $mappings.Count; owned_reverse_removed = $removedMappings
        reverse_mutation_uncertain = $mappingUncertain; startup_process_cleanup_confirmed = $script:startupCleanup
        trust_contents_read_by_harness = $false; persistent_trust_deleted_by_harness = $false
        phone_identity = 'ephemeral process fixture; not Android Keystore acceptance'
        screenshots_created = 0; secret_contents_logged = $false
    }
    $receiptRoot = Join-Path $tempRoot 'VisualWorkbench-setup-text-logs'
    [IO.Directory]::CreateDirectory($receiptRoot) | Out-Null
    $receiptName = 'pair-adb-' + $runId + '.json'
    $receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $receiptRoot $receiptName) -Encoding utf8
    Write-Host ('[pair-adb] text receipt=' + $receiptName)
}
if (-not $success -or -not $cleanup) { throw 'Pairing harness failed or owned cleanup is unconfirmed; inspect the text receipt.' }
Write-Host '[pair-adb] PASS: two roles proved pairing, one exact OPS acknowledgment, revocation refusal, and owned cleanup.'
