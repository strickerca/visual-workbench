#requires -Version 7.2
[CmdletBinding()]
param(
    [string]$ProjectRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [ValidateSet('IN2019')][string]$ExpectedModel = 'IN2019',
    [ValidateRange(60, 3600)][int]$TimeoutSeconds = 1200,
    [ValidateRange(30, 300)][int]$InventoryTimeoutSeconds = 300
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Shared FFI instrumentation runner requires Windows.' }
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
Import-Module (Join-Path $ProjectRoot 'tools/process.psm1')
Import-Module (Join-Path $ProjectRoot 'tools/android-device.psm1')
$parser = Join-Path $ProjectRoot 'tools/ffi-test/shared_reports.py'
$package = 'com.visualworkbench.shared.test'
$runId = [Guid]::NewGuid().ToString('N')
$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$owned = Join-Path $temporaryParent ('VisualWorkbench-shared-ffi-' + $runId)
$state = Join-Path $owned 'state.json'
$report = Join-Path $owned 'assertions.json'
$receipt = [ordered]@{
    schema = 1; kind = 'shared-ffi-jvm-android'; run_id = $runId; model = $ExpectedModel
    status = 'failed'; failure_phase = 'setup'; process_runs = @(); assertions = $null; apk = $null
    cleanup = [ordered]@{ process_trees = $false; owned_test_package = $false; environment_restored = $false; private_work_disposed = $false }
}
$phase = 'setup'
$device = $null
$apk = $null
$created = $false
$packageAbsentBefore = $false
$connectedStarted = $false
$assertionsPassed = $false
$processesClean = $true
$lock = $null
$oldEnvironment = @{}

function Assert-PlainPath([string]$Path, [switch]$Directory) {
    $item = Get-Item -LiteralPath ([IO.Path]::GetFullPath($Path)) -Force
    if ($item.PSIsContainer -ne [bool]$Directory) { throw 'Path type refused.' }
    $cursor = $item
    while ($null -ne $cursor) {
        if (($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected path refused.' }
        $cursor = if ($cursor -is [IO.DirectoryInfo]) { $cursor.Parent } else { $cursor.Directory }
    }
}
function Get-FileDigest([string]$Path) {
    Assert-PlainPath $Path
    $stream = [IO.File]::OpenRead($Path)
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { return [Convert]::ToHexString($hasher.ComputeHash($stream)).ToLowerInvariant() }
    finally { $hasher.Dispose(); $stream.Dispose() }
}
function Run-Owned([string]$Name, [string]$Executable, [string[]]$Arguments, [int]$Seconds, [string]$WorkingDirectory = $ProjectRoot) {
    $script:phase = $Name
    $redact = @($ProjectRoot, $owned, $env:USERPROFILE)
    if ($null -ne $device) { $redact += $device.Serial }
    try {
        # Assembly only builds local sources; retain its redacted compiler
        # diagnostics so a failed build is diagnosable. Other commands keep
        # in-memory capture for inventory parsing and private test/device data.
        $privateOutput = $Name -ne 'shared-ffi-assemble-test'
        $result = Invoke-VwProcess -FilePath $Executable -ArgumentList $Arguments -WorkingDirectory $WorkingDirectory -Phase $Name -TimeoutSeconds $Seconds -ParentExitGraceSeconds 20 -RedactValues $redact -SensitiveCapture:$privateOutput
    } catch { $script:processesClean = $false; throw }
    try { $proof = Get-Content -Raw -LiteralPath ($result.LogPath + '.json') | ConvertFrom-Json }
    catch { $script:processesClean = $false; throw }
    $clean = $proof.contained_in_windows_job -eq $true -and $proof.process_tree_cleanup_confirmed -eq $true -and $proof.output_streams_completed -eq $true
    $script:processesClean = $script:processesClean -and $clean
    $script:receipt.process_runs += [ordered]@{
        phase = $Name; exit_code = $result.ExitCode; elapsed_seconds = $proof.elapsed_seconds
        timeout_seconds = $Seconds; process_tree_cleanup_confirmed = $clean
    }
    if ($result.ExitCode -ne 0 -or -not $clean) {
        # Only our static parser refusal codes may leave sensitive capture.
        foreach ($line in $result.Lines) {
            if ($line -cmatch '^SHARED_FFI_REPORT_REJECTED:([a-z_]{1,80})$') { Write-Host ('Shared FFI report refused: ' + $Matches[1]) }
        }
        throw 'Bounded phase or process cleanup failed.'
    }
    return $result
}
function Run-Parser([string]$Name, [string[]]$Arguments) {
    Run-Owned -Name $Name -Executable 'python.exe' -Arguments (@($parser) + $Arguments + @('--root', $ProjectRoot)) -Seconds 120 | Out-Null
}
function Run-SharedGradle([string]$Name, [string[]]$Tasks, [int]$Seconds = $TimeoutSeconds) {
    $arguments = $Tasks + @('--console=plain', '--no-daemon', '--no-parallel', '--max-workers=2', '--no-configuration-cache', '--no-build-cache', '--rerun-tasks')
    return Run-Owned -Name $Name -Executable (Join-Path $ProjectRoot 'apps/gradlew.bat') -Arguments $arguments -Seconds $Seconds -WorkingDirectory (Join-Path $ProjectRoot 'apps')
}
function Read-ServerBytes([IO.Stream]$Stream, [int]$Count) {
    $bytes = [byte[]]::new($Count)
    $offset = 0
    while ($offset -lt $Count) {
        $got = $Stream.Read($bytes, $offset, $Count - $offset)
        if ($got -eq 0) { throw 'Existing adb server closed its version response.' }
        $offset += $got
    }
    return [Text.Encoding]::ASCII.GetString($bytes)
}
function Assert-ExistingAdbServer {
    # Avoid adb's automatic server start/replacement inside a contained job.
    if ($env:ADB_SERVER_SOCKET -or ($env:ANDROID_ADB_SERVER_PORT -and $env:ANDROID_ADB_SERVER_PORT -ne '5037')) { throw 'Nondefault adb server requires a separately reviewed runner.' }
    $sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } elseif ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { Join-Path $env:LOCALAPPDATA 'Android/Sdk' }
    $adb = Join-Path $sdk 'platform-tools/adb.exe'
    if (-not (Test-Path -LiteralPath $adb -PathType Leaf)) {
        $command = Get-Command adb.exe -ErrorAction SilentlyContinue
        if (-not $command) { throw 'adb is unavailable.' }
        $adb = $command.Source
    }
    $version = Run-Owned -Name 'shared-ffi-adb-client-version' -Executable $adb -Arguments @('version') -Seconds 15
    $versionMatches = [regex]::Matches(($version.Lines -join "`n"), '(?m)^Android Debug Bridge version 1\.0\.(\d+)\s*$')
    if ($versionMatches.Count -ne 1) { throw 'adb client version could not be verified.' }
    $client = [Net.Sockets.TcpClient]::new()
    try {
        $pending = $client.ConnectAsync('127.0.0.1', 5037)
        if (-not $pending.Wait(1500)) { throw 'No ready existing adb server.' }
        $pending.GetAwaiter().GetResult()
        $stream = $client.GetStream(); $stream.ReadTimeout = 1500; $stream.WriteTimeout = 1500
        $request = [Text.Encoding]::ASCII.GetBytes('000chost:version')
        $stream.Write($request, 0, $request.Length)
        if ((Read-ServerBytes $stream 4) -cne 'OKAY') { throw 'Existing adb server refused version.' }
        $lengthText = Read-ServerBytes $stream 4
        if ($lengthText -notmatch '^[0-9a-fA-F]{4}$') { throw 'Invalid adb version size.' }
        $length = [Convert]::ToInt32($lengthText, 16)
        if ($length -lt 1 -or $length -gt 8) { throw 'Invalid adb version size.' }
        $server = Read-ServerBytes $stream $length
        if ($server -notmatch '^[0-9a-fA-F]{1,8}$' -or [Convert]::ToInt32($server, 16) -ne [int]$versionMatches[0].Groups[1].Value) { throw 'adb server/client mismatch; no automatic restart permitted.' }
    } finally { $client.Dispose() }
}
function Get-TestPackagePath {
    # `pm path` exits 1 for the required absent state. Inventory first with the
    # successful empty-result command, then inspect only an exact present match.
    $inventory = Invoke-VwAdb -Device $device -Arguments @('shell', 'pm', 'list', 'packages', '--user', '0', $package) -Phase 'shared-ffi-package-inventory' -TimeoutSeconds 20 -Capture
    $found = @($inventory.Lines | Where-Object { $_.Trim() })
    if ($found.Count -eq 0) { return $null }
    if ($found.Count -ne 1 -or $found[0] -cne ('package:' + $package)) { throw 'Ambiguous test package inventory.' }
    $result = Invoke-VwAdb -Device $device -Arguments @('shell', 'pm', 'path', '--user', '0', $package) -Phase 'shared-ffi-package-path' -TimeoutSeconds 20 -Capture
    $lines = @($result.Lines | Where-Object { $_.Trim() })
    if ($lines.Count -ne 1 -or $lines[0] -notmatch '^package:(/data/app/[A-Za-z0-9/_=+~.-]+/base\.apk)$') { throw 'Unexpected test package inventory.' }
    return $Matches[1]
}
function Assert-SingleOwnerUser {
    # AGP's install/uninstall may affect every profile. This runner deliberately
    # refuses a multi-user device rather than replacing another user's package.
    $users = Invoke-VwAdb -Device $device -Arguments @('shell', 'pm', 'list', 'users') -Phase 'shared-ffi-user-inventory' -TimeoutSeconds 20 -Capture
    $entries = @($users.Lines | Where-Object { $_ -match 'UserInfo\{' })
    if ($entries.Count -ne 1 -or $entries[0] -notmatch 'UserInfo\{0:') { throw 'Shared FFI requires the sole owner user; other profiles are preserved.' }
    $current = Invoke-VwAdb -Device $device -Arguments @('shell', 'am', 'get-current-user') -Phase 'shared-ffi-current-user' -TimeoutSeconds 20 -Capture
    if (($current.Lines -join "`n").Trim() -cne '0') { throw 'Shared FFI requires the owner user to be current.' }
}
function Remove-OwnedTestPackage {
    if (-not $connectedStarted) { return $true }
    if (-not $packageAbsentBefore -or $null -eq $device -or $null -eq $apk) { return $false }
    $installed = Get-TestPackagePath
    if ($null -eq $installed) { return $true }
    $hash = Invoke-VwAdb -Device $device -Arguments @('shell', 'sha256sum', $installed) -Phase 'shared-ffi-owned-apk-hash' -TimeoutSeconds 30 -Capture
    $lines = @($hash.Lines | Where-Object { $_.Trim() })
    if ($lines.Count -ne 1 -or $lines[0] -notmatch '^([0-9a-f]{64})\s+' -or $Matches[1] -cne $apk.sha256) { return $false }
    Invoke-VwAdb -Device $device -Arguments @('shell', 'am', 'force-stop', $package) -Phase 'shared-ffi-stop-owned-test' -TimeoutSeconds 20 -Capture | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('uninstall', $package) -Phase 'shared-ffi-uninstall-owned-test' -TimeoutSeconds 30 -Capture | Out-Null
    return $null -eq (Get-TestPackagePath)
}

try {
    Assert-PlainPath $ProjectRoot -Directory
    $local = Join-Path $ProjectRoot '.local'
    if (-not (Test-Path -LiteralPath $local)) { [IO.Directory]::CreateDirectory($local) | Out-Null }
    Assert-PlainPath $local -Directory
    $lockPath = Join-Path $local 'shared-ffi-hil.lock'
    if (Test-Path -LiteralPath $lockPath) { Assert-PlainPath $lockPath }
    $lock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    Assert-PlainPath $temporaryParent -Directory
    if (Test-Path -LiteralPath $owned) { throw 'Private work collision.' }
    [IO.Directory]::CreateDirectory($owned) | Out-Null
    $created = $true
    Assert-PlainPath $owned -Directory
    $markerStream = [IO.File]::Open((Join-Path $owned '.owner-v1'), [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $markerBytes = [Text.Encoding]::ASCII.GetBytes($runId)
        $markerStream.Write($markerBytes, 0, $markerBytes.Length); $markerStream.Flush($true)
    } finally { $markerStream.Dispose() }
    Run-Parser 'shared-ffi-bind-inputs' @('begin', '--state', $state)
    $inventory = Run-SharedGradle 'shared-ffi-task-inventory' @(':shared:tasks', '--all') $InventoryTimeoutSeconds
    foreach ($task in @('assembleAndroidTest', 'connectedAndroidTest', 'desktopTest')) {
        if (($inventory.Lines -join "`n") -notmatch ('(?m)^' + [regex]::Escape($task) + '(?:\s+-|\s*$)')) { throw 'Required KMP task was not registered; no task-name fallback.' }
    }
    $buildStartedNs = [long]([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()) * [long]1000000
    Run-SharedGradle 'shared-ffi-assemble-test' @(':shared:assembleAndroidTest') | Out-Null
    $apkInfo = Join-Path $owned 'apk-info.json'
    Run-Parser 'shared-ffi-apk-inventory' @('apk-info', '--since-ns', [string]$buildStartedNs, '--output', $apkInfo)
    $apk = Get-Content -Raw -LiteralPath $apkInfo | ConvertFrom-Json
    Run-Owned -Name 'shared-ffi-check-packaging' -Executable 'python.exe' -Arguments @((Join-Path $ProjectRoot 'tools/ffi-test/check_apk.py'), $apk.path) -Seconds 120 | Out-Null
    $apkBinding = Join-Path $owned 'apk-binding.json'
    Run-Parser 'shared-ffi-bind-apk' @('apk-bind', '--apk', $apk.path, '--output', $apkBinding)
    $receipt.apk = Get-Content -Raw -LiteralPath $apkBinding | ConvertFrom-Json
    Run-Parser 'shared-ffi-mark-desktop' @('mark', '--state', $state, '--phase', 'desktop')
    Run-SharedGradle 'shared-ffi-jvm-golden' @(':shared:desktopTest', '--tests', 'com.visualworkbench.shared.DesktopCoreSmokeTest') | Out-Null
    $phase = 'device-selection'
    Assert-ExistingAdbServer
    $device = Get-VwAndroidDevice -Root $ProjectRoot -ExpectedModel 'IN2019'
    Assert-SingleOwnerUser
    if ($null -ne (Get-TestPackagePath)) { throw 'Existing shared test package is preserved; refusing replacement.' }
    $packageAbsentBefore = $true
    foreach ($name in @('ANDROID_SERIAL', 'VW_ANDROID_EXPECTED_SERIAL', 'VW_ANDROID_EXPECTED_MODEL')) { $oldEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process') }
    [Environment]::SetEnvironmentVariable('ANDROID_SERIAL', $device.Serial, 'Process')
    [Environment]::SetEnvironmentVariable('VW_ANDROID_EXPECTED_SERIAL', $device.Serial, 'Process')
    [Environment]::SetEnvironmentVariable('VW_ANDROID_EXPECTED_MODEL', 'IN2019', 'Process')
    Run-Parser 'shared-ffi-mark-android' @('mark', '--state', $state, '--phase', 'android')
    $connectedStarted = $true
    Run-SharedGradle 'shared-ffi-android-golden' @(':shared:connectedAndroidTest') | Out-Null
    # The APK was packaged a second time by --rerun-tasks. A changed hash is
    # inconclusive until rebound; APK timestamps/signatures can differ while
    # embedded native/golden bytes still match. Cleanup uses the actual final APK.
    $apk.sha256 = Get-FileDigest $apk.path
    $finalBinding = Join-Path $owned 'apk-final-binding.json'
    Run-Owned -Name 'shared-ffi-check-final-packaging' -Executable 'python.exe' -Arguments @((Join-Path $ProjectRoot 'tools/ffi-test/check_apk.py'), $apk.path) -Seconds 120 | Out-Null
    Run-Parser 'shared-ffi-bind-final-apk' @('apk-bind', '--apk', $apk.path, '--output', $finalBinding)
    $receipt.apk = Get-Content -Raw -LiteralPath $finalBinding | ConvertFrom-Json
    Run-Parser 'shared-ffi-collect-reports' @('collect', '--state', $state, '--output', $report)
    $receipt.assertions = Get-Content -Raw -LiteralPath $report | ConvertFrom-Json
    $assertionsPassed = $receipt.assertions.assertions_passed -eq $true
} catch {
    $receipt.failure_phase = $phase
    Write-Host "Shared FFI phase failed: $phase (private command/XML details are not published)."
} finally {
    # A cancelled Gradle test can have rebuilt/installed the test APK before
    # returning failure. Bind cleanup to that final on-disk artifact as well.
    if ($connectedStarted -and $null -ne $apk) {
        try { $apk.sha256 = Get-FileDigest $apk.path } catch { $apk = $null }
    }
    try { $receipt.cleanup.owned_test_package = Remove-OwnedTestPackage } catch { $receipt.cleanup.owned_test_package = $false }
    try {
        foreach ($name in $oldEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $oldEnvironment[$name], 'Process') }
        $receipt.cleanup.environment_restored = $true
    } catch { $receipt.cleanup.environment_restored = $false }
    if ($created) {
        try {
            $resolved = [IO.Path]::GetFullPath($owned)
            if ([IO.Path]::GetDirectoryName($resolved) -cne $temporaryParent -or [IO.Path]::GetFileName($resolved) -cne ('VisualWorkbench-shared-ffi-' + $runId)) { throw 'Cleanup containment failed.' }
            Assert-PlainPath $resolved -Directory
            $marker = Join-Path $resolved '.owner-v1'; Assert-PlainPath $marker
            if ((Get-Content -Raw -LiteralPath $marker) -cne $runId) { throw 'Cleanup ownership failed.' }
            foreach ($entry in Get-ChildItem -LiteralPath $resolved -Recurse -Force) {
                if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or -not [IO.Path]::GetFullPath($entry.FullName).StartsWith($resolved + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Cleanup redirected child.' }
            }
            Remove-Item -LiteralPath $resolved -Recurse -Force
            $receipt.cleanup.private_work_disposed = -not (Test-Path -LiteralPath $resolved)
        } catch { $receipt.cleanup.private_work_disposed = $false }
    } else { $receipt.cleanup.private_work_disposed = $true }
    $receipt.cleanup.process_trees = $processesClean
    if ($assertionsPassed -and $processesClean -and $receipt.cleanup.owned_test_package -and $receipt.cleanup.environment_restored -and $receipt.cleanup.private_work_disposed) {
        $receipt.status = 'passed'; $receipt.failure_phase = $null
    }
    if ($null -ne $lock) {
        try {
            $destination = Join-Path $ProjectRoot ('.local/shared-ffi-android-' + $runId + '.json')
            $stream = [IO.File]::Open($destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
            try {
                $bytes = [Text.Encoding]::UTF8.GetBytes(($receipt | ConvertTo-Json -Depth 12))
                $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true)
            } finally { $stream.Dispose() }
            Write-Host ('Shared FFI receipt: shared-ffi-android-' + $runId + '.json; status=' + $receipt.status)
        } finally { $lock.Dispose() }
    }
}
if ($receipt.status -ne 'passed') { exit 1 }
Write-Host 'Shared JVM and IN2019 native FFI golden/boundary assertions: PASS. S23/S Pen and performance remain pending.'
exit 0
