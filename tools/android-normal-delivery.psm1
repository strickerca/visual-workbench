Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Reviewed owner-user update of the NORMAL installed Android app. The HIL
# helpers deliberately refuse this package; this module is its only route.
# Windows PowerShell 5.1 compatible. Device, tool and file effects are injected
# so tools/tests/android-normal-delivery-fixtures.ps1 can exercise every
# decision without adb, apksigner or a phone.
Import-Module (Join-Path $PSScriptRoot 'process.psm1')
Import-Module (Join-Path $PSScriptRoot 'app-test/device.psm1')

$script:NormalPackage = 'com.visualworkbench.android'
# apps/android/src/main/AndroidManifest.xml: .MainActivity owns MAIN/LAUNCHER.
$script:NormalActivity = 'com.visualworkbench.android.MainActivity'
$script:NormalComponent = 'com.visualworkbench.android/.MainActivity'
$script:AuthorizedModel = 'SM-S918U'

function Get-VwNormalDeliveryConstants {
    return [pscustomobject]@{ Package = $script:NormalPackage; Activity = $script:NormalActivity; Component = $script:NormalComponent; Model = $script:AuthorizedModel }
}

function Stop-VwDelivery {
    param([Parameter(Mandatory = $true)][string]$Code)
    if ($Code -cnotmatch '^[a-z][a-z0-9_]{0,79}$') { $Code = 'invalid_failure_code' }
    throw ('VW_DELIVERY:' + $Code)
}

function Get-VwStreamSha256 {
    param([Parameter(Mandatory = $true)][IO.Stream]$Stream)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $Stream.Position = 0; $digest = $sha.ComputeHash($Stream); $Stream.Position = 0 } finally { $sha.Dispose() }
    return ([BitConverter]::ToString($digest) -replace '-', '').ToLowerInvariant()
}

function Get-VwFileSha256 {
    param([Parameter(Mandatory = $true)][string]$Path)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try { return Get-VwStreamSha256 $stream } finally { $stream.Dispose() }
}

function Assert-VwPlainPath {
    param([Parameter(Mandatory = $true)][string]$Path, [switch]$Directory)
    $item = Get-Item -LiteralPath ([IO.Path]::GetFullPath($Path)) -Force
    if ($item.PSIsContainer -ne [bool]$Directory) { Stop-VwDelivery 'path_type_refused' }
    $cursor = $item
    while ($null -ne $cursor) {
        if (($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { Stop-VwDelivery 'path_redirected' }
        $cursor = if ($cursor -is [IO.DirectoryInfo]) { $cursor.Parent } else { $cursor.Directory }
    }
}

function Get-VwApkSignerDigest {
    param([AllowEmptyCollection()][string[]]$Lines)
    $text = @($Lines) -join "`n"
    if ($text.Length -gt 262144) { Stop-VwDelivery 'signer_output_limit' }
    if ($text -cnotmatch '(?m)^Verifies\s*$') { Stop-VwDelivery 'signature_not_verified' }
    $signers = [regex]::Matches($text, '(?m)^Signer #([0-9]+) certificate SHA-256 digest: ([0-9a-fA-F]{64})\s*$')
    # One current signer only; multi-signer or unparseable output is refused.
    if ($signers.Count -ne 1 -or $signers[0].Groups[1].Value -cne '1') { Stop-VwDelivery 'signer_inventory' }
    return $signers[0].Groups[2].Value.ToLowerInvariant()
}

function Get-VwApkBadging {
    param([AllowEmptyCollection()][string[]]$Lines)
    $pattern = "^package: name='([A-Za-z][A-Za-z0-9_.]{0,200})' versionCode='([0-9]{1,10})' versionName='([^'\r\n]{0,64})'"
    $packageLines = @(@($Lines) | Where-Object { $_ -cmatch $pattern })
    if ($packageLines.Count -ne 1) { Stop-VwDelivery 'apk_badging' }
    $null = $packageLines[0] -cmatch $pattern
    $name = $Matches[1]; $versionCode = [long]$Matches[2]; $versionName = $Matches[3]
    $activities = @(@($Lines) | ForEach-Object { if ($_ -cmatch "^launchable-activity: name='([A-Za-z0-9_.$]{1,200})'") { $Matches[1] } })
    return [pscustomobject]@{ Package = $name; VersionCode = $versionCode; VersionName = $versionName; LaunchableActivities = $activities }
}

function Get-VwAndroidProfileIds {
    param([AllowEmptyCollection()][string[]]$Lines)
    # Reuse the reviewed owner-profile validation: exactly one user 0, unique
    # ids, no unparsed rows. Ids are then read with the same row grammar.
    try { $null = Get-VwAppHilProfileCount -Lines $Lines } catch { Stop-VwDelivery 'profile_inventory' }
    $ids = @()
    foreach ($line in @(@($Lines) | Where-Object { $_.Trim() } | Select-Object -Skip 1)) {
        if ($line -cnotmatch '^\s*UserInfo\{([0-9]{1,10}):') { Stop-VwDelivery 'profile_inventory' }
        $ids += $Matches[1]
    }
    return $ids
}

function Get-VwPackageUserStates {
    param([Parameter(Mandatory = $true)][string]$Package, [AllowEmptyCollection()][string[]]$Lines)
    # One `dumpsys package PKG` read covers every profile. Per-user
    # `pm list packages --user N` is refused for shell on Secure Folder/work
    # profiles (DISALLOW_DEBUGGING_FEATURES), so it cannot prove absence there.
    # Exactly one package section; its `User N: ... installed=bool` rows only.
    $rows = @($Lines)
    $header = '^  Package \[' + [regex]::Escape($Package) + '\] \([0-9A-Fa-f]{1,16}\):\s*$'
    $starts = @(for ($index = 0; $index -lt $rows.Count; $index++) { if ($rows[$index] -cmatch $header) { $index } })
    if ($starts.Count -ne 1) { Stop-VwDelivery 'package_user_states' }
    $states = @{}
    for ($index = $starts[0] + 1; $index -lt $rows.Count; $index++) {
        $line = [string]$rows[$index]
        if ($line -cmatch '^\S' -or $line -cmatch '^ {0,2}\S') { break }
        if ($line -cmatch '^\s+User ([0-9]{1,10}):') {
            if ($line -cnotmatch '^\s+User ([0-9]{1,10}): .*\binstalled=(true|false)\b') { Stop-VwDelivery 'package_user_states' }
            if ($states.ContainsKey($Matches[1])) { Stop-VwDelivery 'package_user_states' }
            $states[$Matches[1]] = $Matches[2] -ceq 'true'
        }
    }
    if ($states.Count -eq 0 -or -not $states.ContainsKey('0')) { Stop-VwDelivery 'package_user_states' }
    return $states
}

function Get-VwProcessState {
    param([int]$ExitCode, [AllowEmptyCollection()][string[]]$Lines)
    $rows = @(@($Lines) | Where-Object { $_ -and $_.Trim() })
    if ($ExitCode -eq 0 -and $rows.Count -eq 1 -and $rows[0].Trim() -cmatch '^[0-9]{1,10}(?: [0-9]{1,10})*$') { return 'alive' }
    if ($ExitCode -eq 1 -and $rows.Count -eq 0) { return 'absent' }
    return 'unknown'
}

function Get-VwInstalledApkSet {
    param([AllowEmptyCollection()][string[]]$Lines)
    $rows = @(@($Lines) | Where-Object { $_ -and $_.Trim() } | ForEach-Object { $_.Trim() })
    if ($rows.Count -lt 1 -or $rows.Count -gt 32) { Stop-VwDelivery 'installed_path_inventory' }
    $directory = $null; $names = @{}; $files = @()
    foreach ($row in $rows) {
        if ($row -cnotmatch '^package:(/data/app/[A-Za-z0-9/_=+~.-]+)/(base|split_[A-Za-z0-9_.-]+)\.apk$') { Stop-VwDelivery 'installed_path_inventory' }
        $parent = $Matches[1]; $name = $Matches[2] + '.apk'
        if ($parent.Contains('//') -or $parent.Contains('/./') -or $parent.Contains('/../') -or $parent.EndsWith('/.') -or $parent.EndsWith('/..')) { Stop-VwDelivery 'installed_path_inventory' }
        if ($null -eq $directory) { $directory = $parent } elseif ($parent -cne $directory) { Stop-VwDelivery 'installed_path_inventory' }
        if ($names.ContainsKey($name)) { Stop-VwDelivery 'installed_path_inventory' }
        $names[$name] = $true
        $files += [pscustomobject]@{ Name = $name; Remote = $parent + '/' + $name; Sha256 = $null }
    }
    if (-not $names.ContainsKey('base.apk')) { Stop-VwDelivery 'installed_path_inventory' }
    $base = @($files | Where-Object { $_.Name -ceq 'base.apk' })
    $splits = @($files | Where-Object { $_.Name -cne 'base.apk' } | Sort-Object Name)
    return @($base + $splits)
}

function Get-VwDeviceDigest {
    param([Parameter(Mandatory = $true)][string]$Remote, [AllowEmptyCollection()][string[]]$Lines)
    $rows = @(@($Lines) | Where-Object { $_ -and $_.Trim() })
    if ($rows.Count -ne 1 -or $rows[0].Trim() -cnotmatch '^([0-9a-f]{64})\s+(\S+)$' -or $Matches[2] -cne $Remote) { Stop-VwDelivery 'device_digest' }
    return $Matches[1]
}

function Test-VwInstallSuccess {
    param([AllowEmptyCollection()][string[]]$Lines)
    $text = @($Lines) -join "`n"
    return ($text -cmatch '(?m)^Success\s*$') -and ($text -cnotmatch '(?m)^\s*(?:Failure|Error|Exception)\b')
}

function Get-VwInstallFailureCode {
    param([AllowEmptyCollection()][string[]]$Lines)
    $match = [regex]::Match((@($Lines) -join "`n"), 'INSTALL_(?:FAILED|PARSE_FAILED)_[A-Z0-9_]{1,60}')
    if ($match.Success) { return $match.Value.ToLowerInvariant() }
    return 'install_not_confirmed'
}

function Test-VwLaunchStatus {
    param([AllowEmptyCollection()][string[]]$Lines)
    $text = @($Lines) -join "`n"
    return ($text -cmatch '(?m)^Status:\s*ok\s*$') -and ($text -cnotmatch '(?m)^\s*(?:Error|Exception)\b')
}

function Test-VwNormalForeground {
    param([AllowEmptyCollection()][string[]]$Lines)
    # Owner user 0 and the exact normal package only; the isolated .hil package
    # and other profiles never satisfy this predicate.
    $resumed = @(@($Lines) | Where-Object { $_ -cmatch '^\s*(?:topResumedActivity=|mResumedActivity:|ResumedActivity:)' })
    foreach ($line in $resumed) {
        if ($line -cmatch '\bu0 com\.visualworkbench\.android/(?:\.|com\.visualworkbench\.android\.)MainActivity\b') { return $true }
    }
    return $false
}

function Read-VwAdbServerBytes {
    param([IO.Stream]$Stream, [int]$Count)
    $bytes = New-Object byte[] $Count; $offset = 0
    while ($offset -lt $Count) {
        $read = $Stream.Read($bytes, $offset, $Count - $offset)
        if ($read -eq 0) { throw 'Existing adb server closed the connection.' }
        $offset += $read
    }
    return [Text.Encoding]::ASCII.GetString($bytes)
}

function Assert-VwExistingAdbServer {
    # Never start, kill or restart the shared adb server. A server started
    # inside a bounded job would be terminated with that job.
    param([Parameter(Mandatory = $true)][string]$AdbPath, [Parameter(Mandatory = $true)][string]$Root)
    if ($env:ADB_SERVER_SOCKET -or ($env:ANDROID_ADB_SERVER_PORT -and $env:ANDROID_ADB_SERVER_PORT -ne '5037')) { throw 'Nondefault adb server refused.' }
    $version = Invoke-VwProcess -FilePath $AdbPath -ArgumentList @('version') -WorkingDirectory $Root -Phase 'normal-delivery-adb-client' -TimeoutSeconds 15 -RedactValues @($Root, $env:USERPROFILE) -Capture
    $client = [regex]::Matches(($version.Lines -join "`n"), '(?m)^Android Debug Bridge version 1\.0\.(\d+)\s*$')
    if ($version.ExitCode -ne 0 -or $client.Count -ne 1) { throw 'adb client version unavailable.' }
    $tcp = New-Object Net.Sockets.TcpClient
    try {
        $task = $tcp.ConnectAsync('127.0.0.1', 5037)
        if (-not $task.Wait(1500)) { throw 'Existing adb server unavailable; start it yourself before delivery.' }
        # GetResult() returns a VoidTaskResult object in PowerShell 7; keep it off the pipeline.
        $null = $task.GetAwaiter().GetResult()
        $stream = $tcp.GetStream(); $stream.ReadTimeout = 1500; $stream.WriteTimeout = 1500
        $request = [Text.Encoding]::ASCII.GetBytes('000chost:version')
        $stream.Write($request, 0, $request.Length)
        if ((Read-VwAdbServerBytes $stream 4) -cne 'OKAY') { throw 'Existing adb server refused.' }
        $length = Read-VwAdbServerBytes $stream 4
        if ($length -notmatch '^[0-9a-fA-F]{4}$') { throw 'adb response refused.' }
        $count = [Convert]::ToInt32($length, 16)
        if ($count -lt 1 -or $count -gt 8) { throw 'adb response refused.' }
        $server = Read-VwAdbServerBytes $stream $count
        if ($server -notmatch '^[0-9a-fA-F]{1,8}$' -or [Convert]::ToInt32($server, 16) -ne [int]$client[0].Groups[1].Value) { throw 'adb client/server mismatch; automatic restart refused.' }
    } finally { $tcp.Dispose() }
}

function Write-VwPrivateText {
    param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Text)
    $bytes = (New-Object Text.UTF8Encoding -ArgumentList $false).GetBytes($Text)
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}

function Test-VwPlainItem($Item) {
    return ($Item.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0
}

# Final run statuses whose backup a later verified delivery supersedes. A
# rollback-failed backup is the owner's recovery copy and is never pruned; a
# backup whose run receipt is missing or unreadable is kept as well.
$script:PrunableOutcomes = @('delivered', 'refused', 'rolled-back', 'install-failed-unchanged', 'install-timeout-unchanged',
    'delivered-foreground-inconclusive', 'delivered-launch-inconclusive', 'delivered-process-exited')

function Get-VwOwnBackupPlan {
    # The exact files and directories to delete when the directory holds only
    # what this module writes: its backup.json, the APK files that manifest
    # names and, after a failed readback, a readback directory with only
    # base.apk. $null otherwise.
    param([Parameter(Mandatory = $true)][IO.DirectoryInfo]$Directory)
    if (-not (Test-VwPlainItem $Directory)) { return $null }
    $manifestItem = Get-Item -LiteralPath (Join-Path $Directory.FullName 'backup.json') -Force -ErrorAction SilentlyContinue
    if ($null -eq $manifestItem -or $manifestItem.PSIsContainer -or -not (Test-VwPlainItem $manifestItem) -or $manifestItem.Length -gt 65536) { return $null }
    $manifest = [IO.File]::ReadAllText($manifestItem.FullName) | ConvertFrom-Json
    if ($manifest.kind -cne 'android-normal-delivery-backup' -or $manifest.run_id -cne $Directory.Name) { return $null }
    $expected = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    [void]$expected.Add('backup.json')
    foreach ($file in @($manifest.files)) {
        if ([string]$file.name -cnotmatch '^(?:base|split_[A-Za-z0-9_.-]+)\.apk$') { return $null }
        [void]$expected.Add([string]$file.name)
    }
    $files = @(); $directories = @()
    foreach ($child in @(Get-ChildItem -LiteralPath $Directory.FullName -Force)) {
        if (-not (Test-VwPlainItem $child)) { return $null }
        if (-not $child.PSIsContainer) {
            if (-not $expected.Contains($child.Name)) { return $null }
            $files += $child.FullName; continue
        }
        if ($child.Name -cne 'readback') { return $null }
        foreach ($inner in @(Get-ChildItem -LiteralPath $child.FullName -Force)) {
            if ($inner.PSIsContainer -or $inner.Name -cne 'base.apk' -or -not (Test-VwPlainItem $inner)) { return $null }
            $files += $inner.FullName
        }
        $directories += $child.FullName
    }
    return [pscustomobject]@{ Files = $files; Directories = $directories; Written = $manifestItem.LastWriteTimeUtc }
}

function Get-VwDeliveryRunOutcome {
    # The final status recorded in a run's own receipt, or $null when absent.
    param([Parameter(Mandatory = $true)][string]$Local, [Parameter(Mandatory = $true)][string]$RunId)
    $item = Get-Item -LiteralPath (Join-Path $Local ('android-normal-delivery-' + $RunId + '.json')) -Force -ErrorAction SilentlyContinue
    if ($null -eq $item -or $item.PSIsContainer -or -not (Test-VwPlainItem $item) -or $item.Length -gt 1048576) { return $null }
    try { return [string](([IO.File]::ReadAllText($item.FullName) | ConvertFrom-Json).status) } catch { return $null }
}

function Remove-VwOldDeliveryBackups {
    # Called only after a verified delivery. Among this module's own backups,
    # keeps the newest $Keep (the current run's included) and removes older ones
    # whose run outcome a later delivery supersedes. Deletion is file by file and
    # never recursive, so a link that appears later is never followed.
    param([Parameter(Mandatory = $true)][string]$Parent, [Parameter(Mandatory = $true)][string]$Local,
        [Parameter(Mandatory = $true)][string]$Current, [Parameter(Mandatory = $true)][int]$Keep)
    Assert-VwPlainPath $Parent -Directory
    $removed = 0; $skipped = 0; $owned = @()
    foreach ($directory in @(Get-ChildItem -LiteralPath $Parent -Directory -Force)) {
        if ($directory.Name -cnotmatch '^[0-9a-f]{32}$' -or $directory.Name -ceq $Current) { continue }
        $plan = $null
        try { $plan = Get-VwOwnBackupPlan $directory } catch { $plan = $null }
        # Ownership is decided before retention, so a foreign directory can
        # never take a kept slot from a real previous backup.
        if ($null -eq $plan) { $skipped++; continue }
        $owned += [pscustomobject]@{ Directory = $directory; Plan = $plan; Written = $plan.Written; Name = $directory.Name }
    }
    $owned = @($owned | Sort-Object -Property @{ Expression = 'Written'; Descending = $true }, @{ Expression = 'Name'; Descending = $true })
    foreach ($entry in @($owned | Select-Object -Skip ([Math]::Max(0, $Keep - 1)))) {
        if ((Get-VwDeliveryRunOutcome $Local $entry.Name) -cnotin $script:PrunableOutcomes) { $skipped++; continue }
        try {
            foreach ($path in @($entry.Plan.Files)) {
                $item = Get-Item -LiteralPath $path -Force
                if ($item.PSIsContainer -or -not (Test-VwPlainItem $item)) { throw 'Backup file changed.' }
                [IO.File]::Delete($path)
            }
            foreach ($path in @($entry.Plan.Directories)) { [IO.Directory]::Delete($path, $false) }
            [IO.Directory]::Delete($entry.Directory.FullName, $false)
            $removed++
        } catch { $skipped++ }
    }
    return [pscustomobject]@{ Removed = $removed; Skipped = $skipped }
}

function Invoke-VwNormalDelivery {
    <#
    Delivers one hash-pinned APK as a same-signer update of the normal package
    for owner user 0 on the selected authorized model, then reads back the
    installed bytes and launches it normally. Never uninstalls, downgrades or
    clears data. Returns the text-only receipt it writes under .local/.

    Injected runners (all return an object with ExitCode and Lines):
      SelectDevice: { } -> object with Model (and Serial, never recorded)
      Adb:          { param($Device, [string]$Phase, [string[]]$Arguments, [int]$Seconds) }
      ApkSigner:    { param([string]$Phase, [string]$Path) }   # apksigner verify --verbose --print-certs
      Aapt:         { param([string]$Phase, [string]$Path) }   # aapt2 dump badging
      AssertAdbServer: { } throws unless the existing adb server is alive
        (checked again right before install and before rollback, so an
        `adb -s` call never spawns a server inside a bounded job).

    Rollback policy: automatic rollback happens only for failures after the
    install attempt and BEFORE launch (install outcome unknown, readback or
    profile-scope mismatch, device errors). Once the new build has been
    launched it may already have migrated app data (for example the vw-store
    schema); reinstalling older code could then leave an app that cannot open
    its own data while the receipt claims success. Launch failure, launch
    timeout, missing foreground or an exited process are therefore reported
    as distinct non-rollback statuses for owner diagnosis.
    #>
    [CmdletBinding()]
    param(
        [Parameter(Mandatory = $true)][string]$ProjectRoot,
        [Parameter(Mandatory = $true)][string]$ApkPath,
        [Parameter(Mandatory = $true)][string]$ApkSha256,
        [Parameter(Mandatory = $true)][string]$ExpectedModel,
        [Parameter(Mandatory = $true)][scriptblock]$SelectDevice,
        [Parameter(Mandatory = $true)][scriptblock]$Adb,
        [Parameter(Mandatory = $true)][scriptblock]$ApkSigner,
        [Parameter(Mandatory = $true)][scriptblock]$Aapt,
        [Parameter(Mandatory = $true)][scriptblock]$AssertAdbServer,
        [ValidatePattern('^[0-9a-f]{32}$')][string]$RunId = ([guid]::NewGuid().ToString('N')),
        [ValidateRange(1, 5)][int]$ForegroundAttempts = 3,
        [ValidateRange(0, 10)][int]$ForegroundDelaySeconds = 2,
        [ValidateRange(0, 600)][int]$InstallSettleSeconds = 30,
        [ValidateRange(0, 30)][int]$ProcessCheckDelaySeconds = 3,
        # Backups kept after a verified delivery, this run's included.
        [ValidateRange(1, 10)][int]$KeepBackups = 2
    )
    $root = [IO.Path]::GetFullPath($ProjectRoot)
    $package = $script:NormalPackage
    $receipt = [ordered]@{
        schema = 1; kind = 'android-normal-delivery'; run_id = $RunId
        created_utc = [DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
        package = $package; expected_model = $ExpectedModel; device_model = $null; owner_user = 0
        status = 'refused'; failure_phase = $null; failure_code = $null
        phases = New-Object Collections.Generic.List[object]
        input = [ordered]@{ apk_name = [IO.Path]::GetFileName($ApkPath); sha256 = $null; bytes = $null; signer_sha256 = $null; version_code = $null; version_name = $null; launcher_activity = $null }
        device_scope = [ordered]@{ current_user = $null; profile_count = $null; installed_for_owner = $false; other_profiles_with_package = $null; other_profiles_modified = $false }
        installed_before = [ordered]@{ signer_sha256 = $null; version_code = $null; version_name = $null; files = @() }
        backup = [ordered]@{ directory = $null; complete = $false; device_digest_bound = $false; retained = $false; skipped_identical = $false; keep_latest = $KeepBackups; pruned_previous = 0; prune_skipped = 0 }
        install = [ordered]@{ attempted = $false; skipped_identical = $false; success_reported = $false; timed_out = $false; settled_after_timeout = $null; arguments = @('install', '-r', '--user', '0', '<apk>'); downgrade_flag = $false; uninstall_performed = $false; data_cleared = $false }
        readback = [ordered]@{ file_count = $null; sha256 = $null; matches = $false }
        launch = [ordered]@{ component = $script:NormalComponent; attempted = $false; status_ok = $false; foreground = $false; attempts = 0; process_state_after_launch = $null }
        rollback = [ordered]@{ needed = $false; attempted = $false; succeeded = $false; reason = $null; failure_code = $null; may_require_downgrade = $false; policy = 'pre-launch-failures-only' }
        privacy = [ordered]@{ device_serial_recorded = $false; local_paths_recorded = $false; screenshots_created = 0 }
    }
    $ctx = @{ phase = 'local-state'; device = $null; installAttempted = $false; launchAttempted = $false; identical = $false; lock = $null; apkStream = $null; local = $null; backupDir = $null; backup = @(); profileIds = @() }

    function Enter-Phase([string]$Name) {
        $ctx.phase = $Name
        $receipt.phases.Add([ordered]@{ name = $Name; status = 'started' })
        Write-Host "[normal-delivery] $Name"
    }
    function Set-PhaseStatus([string]$Status) {
        if ($receipt.phases.Count -gt 0) { $receipt.phases[$receipt.phases.Count - 1].status = $Status }
    }
    function Invoke-Device([string]$Phase, [string[]]$Arguments, [int]$Seconds = 30, [switch]$AllowFailure) {
        $result = & $Adb $ctx.device $Phase $Arguments $Seconds
        if (-not $AllowFailure -and $result.ExitCode -ne 0) { Stop-VwDelivery ('adb_' + ($Phase -replace '^normal-delivery-', '' -replace '-', '_')) }
        return $result
    }
    function Read-InstalledState {
        $paths = Invoke-Device 'normal-delivery-installed-paths' @('shell', 'pm', 'path', '--user', '0', $package)
        $set = @(Get-VwInstalledApkSet $paths.Lines)
        foreach ($file in $set) {
            $digest = Invoke-Device 'normal-delivery-device-digest' @('shell', 'sha256sum', $file.Remote) 60
            $file.Sha256 = Get-VwDeviceDigest $file.Remote $digest.Lines
        }
        return $set
    }
    function Test-StateMatches($State, $Expected) {
        if (@($State).Count -ne @($Expected).Count -or @($Expected).Count -eq 0) { return $false }
        foreach ($entry in @($Expected)) {
            $found = @(@($State) | Where-Object { $_.Name -ceq $entry.name })
            if ($found.Count -ne 1 -or $found[0].Sha256 -cne $entry.sha256) { return $false }
        }
        return $true
    }
    function ConvertTo-Expected($State) {
        return @(@($State) | ForEach-Object { [ordered]@{ name = $_.Name; sha256 = $_.Sha256 } })
    }
    function Read-SettledState {
        # A killed adb client does not stop the device-side install session.
        # Require two identical delayed reads before deciding anything.
        $previous = $null
        for ($read = 1; $read -le 3; $read++) {
            if ($InstallSettleSeconds -gt 0) { Start-Sleep -Seconds $InstallSettleSeconds }
            $state = @(Read-InstalledState)
            if ($null -ne $previous -and (Test-StateMatches $state (ConvertTo-Expected $previous))) { return $state }
            $previous = $state
        }
        Stop-VwDelivery 'install_timeout_unsettled'
    }
    function Assert-PackageScope([string]$Stage) {
        $dump = Invoke-Device ('normal-delivery-package-scope-' + $Stage) @('shell', 'dumpsys', 'package', $package) 60
        $states = Get-VwPackageUserStates -Package $package -Lines $dump.Lines
        $ids = @($ctx.profileIds)
        if (@($states.Keys).Count -ne $ids.Count -or @($ids | Where-Object { -not $states.ContainsKey($_) }).Count -ne 0) { Stop-VwDelivery 'package_user_states_inconsistent' }
        $owner = [bool]$states['0']
        $others = @($ids | Where-Object { $_ -cne '0' -and $states[$_] }).Count
        $receipt.device_scope.installed_for_owner = $owner
        $receipt.device_scope.other_profiles_with_package = $others
        if (-not $owner) { Stop-VwDelivery 'package_not_installed_for_owner' }
        # The APK code path is shared by every profile holding the package; an
        # update would also change those profiles, so they are never touched.
        if ($others -ne 0) { Stop-VwDelivery 'package_installed_in_other_profile' }
    }
    function Get-LaunchStatus([string]$Outcome) {
        # Identical bytes were never installed: say so instead of 'delivered'.
        if ($ctx.identical) { return 'already-installed-' + $Outcome }
        return 'delivered-' + $Outcome
    }
    function Invoke-Rollback([string]$Reason) {
        $rollback = $receipt.rollback
        $rollback.reason = $Reason
        try {
            # Never let an `adb -s` call spawn a replacement server in a job.
            try { $null = & $AssertAdbServer } catch { $rollback.failure_code = 'adb_server_unavailable'; Stop-VwDelivery 'adb_server_unavailable' }
            $current = $null
            try { $current = @(Read-InstalledState) } catch { $current = $null }
            if ($null -ne $current -and (Test-StateMatches $current $ctx.backup)) {
                # Android installs are atomic: a refused install keeps the old
                # bytes. A timed-out install reaches here only after settled reads.
                $rollback.needed = $false
                $receipt.status = if ($Reason -like 'install_timeout*') { 'install-timeout-unchanged' } else { 'install-failed-unchanged' }
                return
            }
            $rollback.needed = $true
            if (-not $receipt.backup.complete) { $rollback.failure_code = 'backup_incomplete'; Stop-VwDelivery 'backup_incomplete' }
            Enter-Phase 'rollback'
            $rollback.attempted = $true
            $files = @()
            foreach ($entry in @($ctx.backup)) {
                $local = Join-Path $ctx.backupDir $entry.name
                Assert-VwPlainPath $local
                if ((Get-VwFileSha256 $local) -cne $entry.sha256) { $rollback.failure_code = 'backup_changed'; Stop-VwDelivery 'backup_changed' }
                $files += $local
            }
            # Same flags as delivery: replace for owner user 0; never -d, uninstall or clear.
            $arguments = if ($files.Count -eq 1) { @('install', '-r', '--user', '0', $files[0]) } else { @('install-multiple', '-r', '--user', '0') + $files }
            $result = Invoke-Device 'normal-delivery-rollback-install' $arguments 300 -AllowFailure
            if ($result.ExitCode -ne 0 -or -not (Test-VwInstallSuccess $result.Lines)) {
                $rollback.failure_code = Get-VwInstallFailureCode $result.Lines
                Stop-VwDelivery 'rollback_install_failed'
            }
            $after = @(Read-InstalledState)
            if (-not (Test-StateMatches $after $ctx.backup)) { $rollback.failure_code = 'rollback_readback_mismatch'; Stop-VwDelivery 'rollback_readback_mismatch' }
            $rollback.succeeded = $true
            $receipt.status = 'rolled-back'
            Set-PhaseStatus 'passed'
            Write-Host '[normal-delivery] rollback restored the backed-up installed APK bytes.'
        } catch {
            if (-not $rollback.failure_code) {
                $rollback.failure_code = if ($_.Exception.Message -cmatch '^VW_DELIVERY:([a-z][a-z0-9_]{0,79})$') { $Matches[1] } else { 'rollback_error' }
            }
            $receipt.status = 'rollback-failed'
            if ($ctx.phase -ceq 'rollback') { Set-PhaseStatus 'failed' }
            Write-Host ('[normal-delivery] ROLLBACK FAILED; the backup is retained under ' + $receipt.backup.directory)
        }
    }

    try {
        Enter-Phase 'local-state'
        Assert-VwPlainPath $root -Directory
        $ctx.local = Join-Path $root '.local'
        if (-not (Test-Path -LiteralPath $ctx.local)) { [void][IO.Directory]::CreateDirectory($ctx.local) }
        Assert-VwPlainPath $ctx.local -Directory
        $lockPath = Join-Path $ctx.local 'android-normal-delivery.lock'
        if (Test-Path -LiteralPath $lockPath) { Assert-VwPlainPath $lockPath }
        try { $ctx.lock = [IO.File]::Open($lockPath, [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None) }
        catch { Stop-VwDelivery 'delivery_lock_held' }
        Set-PhaseStatus 'passed'

        Enter-Phase 'inputs'
        if ($ExpectedModel -cne $script:AuthorizedModel) { Stop-VwDelivery 'model_not_authorized' }
        if ($ApkSha256 -cnotmatch '^[0-9a-f]{64}$') { Stop-VwDelivery 'apk_sha256_format' }
        $apk = [IO.Path]::GetFullPath($ApkPath)
        if ([IO.Path]::GetExtension($apk) -cne '.apk') { Stop-VwDelivery 'apk_extension' }
        if (-not (Test-Path -LiteralPath $apk -PathType Leaf)) { Stop-VwDelivery 'apk_missing' }
        Assert-VwPlainPath $apk
        # Hold a read-share lease: the hashed bytes cannot be replaced before install.
        $ctx.apkStream = [IO.File]::Open($apk, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $receipt.input.bytes = $ctx.apkStream.Length
        if ($ctx.apkStream.Length -le 0 -or $ctx.apkStream.Length -gt 2GB) { Stop-VwDelivery 'apk_size' }
        if ((Get-VwStreamSha256 $ctx.apkStream) -cne $ApkSha256) { Stop-VwDelivery 'apk_sha256_mismatch' }
        $receipt.input.sha256 = $ApkSha256
        Set-PhaseStatus 'passed'

        Enter-Phase 'apk-signer'
        $signed = & $ApkSigner 'normal-delivery-apk-signer' $apk
        if ($signed.ExitCode -ne 0) { Stop-VwDelivery 'apk_signature_unverified' }
        $receipt.input.signer_sha256 = Get-VwApkSignerDigest $signed.Lines
        Set-PhaseStatus 'passed'

        Enter-Phase 'apk-identity'
        $badging = & $Aapt 'normal-delivery-apk-badging' $apk
        if ($badging.ExitCode -ne 0) { Stop-VwDelivery 'apk_badging' }
        $identity = Get-VwApkBadging $badging.Lines
        if ($identity.Package -cne $package) { Stop-VwDelivery 'apk_package_not_normal' }
        if (@($identity.LaunchableActivities) -cnotcontains $script:NormalActivity) { Stop-VwDelivery 'apk_launcher_activity' }
        $receipt.input.version_code = $identity.VersionCode
        $receipt.input.version_name = $identity.VersionName
        $receipt.input.launcher_activity = $script:NormalActivity
        Set-PhaseStatus 'passed'

        Enter-Phase 'device-selection'
        $ctx.device = & $SelectDevice
        if ($null -eq $ctx.device -or $null -eq $ctx.device.PSObject.Properties['Model'] -or [string]$ctx.device.Model -cne $ExpectedModel) { Stop-VwDelivery 'device_model_mismatch' }
        $receipt.device_model = [string]$ctx.device.Model
        Set-PhaseStatus 'passed'

        Enter-Phase 'owner-scope'
        $current = Invoke-Device 'normal-delivery-current-user' @('shell', 'am', 'get-current-user')
        $currentUser = (@($current.Lines) -join "`n").Trim()
        $receipt.device_scope.current_user = if ($currentUser -cmatch '^[0-9]{1,10}$') { [long]$currentUser } else { $null }
        if ($currentUser -cne '0') { Stop-VwDelivery 'owner_user_not_current' }
        $users = Invoke-Device 'normal-delivery-profiles' @('shell', 'pm', 'list', 'users')
        $ctx.profileIds = @(Get-VwAndroidProfileIds $users.Lines)
        $receipt.device_scope.profile_count = $ctx.profileIds.Count
        Assert-PackageScope 'before'
        Set-PhaseStatus 'passed'

        Enter-Phase 'backup'
        $state = @(Read-InstalledState)
        # The device digest already proves the installed bytes are the pinned
        # input: nothing is installed or rolled back, so no backup is pulled.
        $ctx.identical = $state.Count -eq 1 -and $state[0].Name -ceq 'base.apk' -and $state[0].Sha256 -ceq $ApkSha256
        if ($ctx.identical) {
            $files = @([ordered]@{ name = 'base.apk'; sha256 = $ApkSha256; bytes = $receipt.input.bytes })
            $receipt.installed_before.files = $files
            $receipt.backup.skipped_identical = $true
            Set-PhaseStatus 'skipped'
            Enter-Phase 'installed-signer'
            # Same bytes as the verified input, so the same signer and version.
            $receipt.installed_before.signer_sha256 = $receipt.input.signer_sha256
            $receipt.installed_before.version_code = $identity.VersionCode
            $receipt.installed_before.version_name = $identity.VersionName
            Set-PhaseStatus 'skipped'
        } else {
        $parent = Join-Path $ctx.local 'android-normal-backups'
        if (-not (Test-Path -LiteralPath $parent)) { [void][IO.Directory]::CreateDirectory($parent) }
        Assert-VwPlainPath $parent -Directory
        $ctx.backupDir = Join-Path $parent $RunId
        if (Test-Path -LiteralPath $ctx.backupDir) { Stop-VwDelivery 'backup_collision' }
        [void][IO.Directory]::CreateDirectory($ctx.backupDir)
        Assert-VwPlainPath $ctx.backupDir -Directory
        $receipt.backup.directory = '.local/android-normal-backups/' + $RunId
        $receipt.backup.retained = $true
        $files = @()
        foreach ($file in $state) {
            $local = Join-Path $ctx.backupDir $file.Name
            Invoke-Device 'normal-delivery-backup-pull' @('pull', $file.Remote, $local) 300 | Out-Null
            if (-not (Test-Path -LiteralPath $local -PathType Leaf)) { Stop-VwDelivery 'backup_pull_missing' }
            Assert-VwPlainPath $local
            $digest = Get-VwFileSha256 $local
            if ($digest -cne $file.Sha256) { Stop-VwDelivery 'backup_digest_mismatch' }
            $files += [ordered]@{ name = $file.Name; sha256 = $digest; bytes = (Get-Item -LiteralPath $local).Length }
        }
        $ctx.backup = $files
        $receipt.installed_before.files = $files
        $manifest = [ordered]@{ schema = 1; kind = 'android-normal-delivery-backup'; run_id = $RunId; package = $package; owner_user = 0; files = $files }
        Write-VwPrivateText (Join-Path $ctx.backupDir 'backup.json') ($manifest | ConvertTo-Json -Depth 6)
        $receipt.backup.complete = $true
        $receipt.backup.device_digest_bound = $true
        Set-PhaseStatus 'passed'

        Enter-Phase 'installed-signer'
        $signers = @()
        foreach ($entry in $files) {
            $installedSigned = & $ApkSigner 'normal-delivery-installed-signer' (Join-Path $ctx.backupDir $entry.name)
            if ($installedSigned.ExitCode -ne 0) { Stop-VwDelivery 'installed_signature_unverified' }
            $signers += Get-VwApkSignerDigest $installedSigned.Lines
        }
        if (@($signers | Select-Object -Unique).Count -ne 1) { Stop-VwDelivery 'installed_signer_inconsistent' }
        $receipt.installed_before.signer_sha256 = $signers[0]
        if ($signers[0] -cne $receipt.input.signer_sha256) { Stop-VwDelivery 'signer_mismatch' }
        $installedBadging = & $Aapt 'normal-delivery-installed-badging' (Join-Path $ctx.backupDir 'base.apk')
        if ($installedBadging.ExitCode -ne 0) { Stop-VwDelivery 'installed_badging' }
        $installedIdentity = Get-VwApkBadging $installedBadging.Lines
        if ($installedIdentity.Package -cne $package) { Stop-VwDelivery 'installed_package_identity' }
        $receipt.installed_before.version_code = $installedIdentity.VersionCode
        $receipt.installed_before.version_name = $installedIdentity.VersionName
        if ($identity.VersionCode -lt $installedIdentity.VersionCode) { Stop-VwDelivery 'version_downgrade_refused' }
        # A newer versionCode makes the automatic -r rollback a downgrade that
        # Android refuses; record it so a failed rollback is not a surprise.
        $receipt.rollback.may_require_downgrade = $identity.VersionCode -gt $installedIdentity.VersionCode
        Set-PhaseStatus 'passed'
        }

        Enter-Phase 'install'
        if ($ctx.identical) {
            # Already the exact delivered bytes: nothing to install or roll back.
            $receipt.install.skipped_identical = $true
            Set-PhaseStatus 'skipped'
        } else {
            try { $null = & $AssertAdbServer } catch { Stop-VwDelivery 'adb_server_unavailable' }
            $ctx.installAttempted = $true
            $receipt.install.attempted = $true
            $installed = Invoke-Device 'normal-delivery-install' @('install', '-r', '--user', '0', $apk) 300 -AllowFailure
            $receipt.install.success_reported = $installed.ExitCode -eq 0 -and (Test-VwInstallSuccess $installed.Lines)
            if ($installed.ExitCode -eq 124) {
                # Timeout: the outcome is unknown until settled delayed reads.
                $receipt.install.timed_out = $true
                $settled = @(Read-SettledState)
                if (Test-StateMatches $settled @([ordered]@{ name = 'base.apk'; sha256 = $ApkSha256 })) {
                    $receipt.install.settled_after_timeout = 'delivered-bytes'
                } elseif (Test-StateMatches $settled $ctx.backup) {
                    $receipt.install.settled_after_timeout = 'previous-bytes'
                    Stop-VwDelivery 'install_timeout_unchanged'
                } else {
                    $receipt.install.settled_after_timeout = 'unknown-bytes'
                    Stop-VwDelivery 'install_timeout_unknown'
                }
            } elseif (-not $receipt.install.success_reported) { Stop-VwDelivery (Get-VwInstallFailureCode $installed.Lines) }
            Set-PhaseStatus 'passed'
        }

        Enter-Phase 'readback'
        if ($ctx.identical) {
            # The device digest already bound these exact installed bytes.
            $receipt.readback.file_count = 1
            $receipt.readback.sha256 = $ApkSha256
            $receipt.readback.matches = $true
            Set-PhaseStatus 'skipped'
        } else {
            $paths = Invoke-Device 'normal-delivery-installed-paths' @('shell', 'pm', 'path', '--user', '0', $package)
            $after = @(Get-VwInstalledApkSet $paths.Lines)
            $receipt.readback.file_count = $after.Count
            if ($after.Count -ne 1) { Stop-VwDelivery 'readback_unexpected_splits' }
            $readDirectory = Join-Path $ctx.backupDir 'readback'
            [void][IO.Directory]::CreateDirectory($readDirectory)
            $readFile = Join-Path $readDirectory 'base.apk'
            Invoke-Device 'normal-delivery-readback-pull' @('pull', $after[0].Remote, $readFile) 300 | Out-Null
            if (-not (Test-Path -LiteralPath $readFile -PathType Leaf)) { Stop-VwDelivery 'readback_pull_missing' }
            Assert-VwPlainPath $readFile
            $readDigest = Get-VwFileSha256 $readFile
            $receipt.readback.sha256 = $readDigest
            $deviceDigest = Invoke-Device 'normal-delivery-device-digest' @('shell', 'sha256sum', $after[0].Remote) 60
            if ($readDigest -cne $ApkSha256 -or (Get-VwDeviceDigest $after[0].Remote $deviceDigest.Lines) -cne $ApkSha256) { Stop-VwDelivery 'readback_sha256_mismatch' }
            Remove-Item -LiteralPath $readFile -Force
            Remove-Item -LiteralPath $readDirectory -Force
            Assert-PackageScope 'after'
            $receipt.readback.matches = $true
            Set-PhaseStatus 'passed'
        }

        # From here on the new build may run and migrate app data: no automatic
        # rollback (see the policy above). Failures become non-rollback statuses.
        Enter-Phase 'launch'
        $ctx.launchAttempted = $true
        $receipt.launch.attempted = $true
        $launch = Invoke-Device 'normal-delivery-launch' @('shell', 'am', 'start', '-W', '--user', '0', '-a', 'android.intent.action.MAIN', '-c', 'android.intent.category.LAUNCHER', '-n', $script:NormalComponent) 30 -AllowFailure
        $receipt.launch.status_ok = $launch.ExitCode -eq 0 -and (Test-VwLaunchStatus $launch.Lines)
        if (-not $receipt.launch.status_ok) { Stop-VwDelivery $(if ($launch.ExitCode -eq 124 -or (@($launch.Lines) -join "`n") -cmatch '(?m)^Status:\s*timeout\s*$') { 'launch_timeout' } else { 'launch_failed' }) }
        Set-PhaseStatus 'passed'

        Enter-Phase 'foreground'
        for ($attempt = 1; $attempt -le $ForegroundAttempts; $attempt++) {
            $receipt.launch.attempts = $attempt
            $activities = Invoke-Device 'normal-delivery-foreground' @('shell', 'dumpsys', 'activity', 'activities') 30
            if (Test-VwNormalForeground $activities.Lines) { $receipt.launch.foreground = $true; break }
            if ($attempt -lt $ForegroundAttempts -and $ForegroundDelaySeconds -gt 0) { Start-Sleep -Seconds $ForegroundDelaySeconds }
        }
        if (-not $receipt.launch.foreground) {
            # Verified bytes stay installed. A shared phone can hold focus
            # elsewhere; never stop another app to win foreground.
            Set-PhaseStatus 'inconclusive'
            $receipt.status = Get-LaunchStatus 'foreground-inconclusive'
            $receipt.failure_phase = 'foreground'
            $receipt.failure_code = 'foreground_not_observed'
        } else {
            Set-PhaseStatus 'passed'
            Enter-Phase 'process-check'
            # Detect an immediate crash after the first frame; report only.
            if ($ProcessCheckDelaySeconds -gt 0) { Start-Sleep -Seconds $ProcessCheckDelaySeconds }
            $processes = Invoke-Device 'normal-delivery-process-check' @('shell', 'pidof', $package) 30 -AllowFailure
            $processState = Get-VwProcessState $processes.ExitCode $processes.Lines
            $receipt.launch.process_state_after_launch = $processState
            if ($processState -ceq 'alive') {
                Set-PhaseStatus 'passed'
                $receipt.status = if ($ctx.identical) { 'already-installed' } else { 'delivered' }
                if (-not $ctx.identical) {
                    # Verified delivery: this run's backup holds the previous
                    # install; older backups beyond the retention limit go.
                    try {
                        $pruned = Remove-VwOldDeliveryBackups -Parent (Join-Path $ctx.local 'android-normal-backups') -Local $ctx.local -Current $RunId -Keep $KeepBackups
                        $receipt.backup.pruned_previous = $pruned.Removed; $receipt.backup.prune_skipped = $pruned.Skipped
                    } catch { Write-Host '[normal-delivery] older backups were not pruned; the delivery result is unchanged.' }
                }
            } elseif ($processState -ceq 'absent') {
                Set-PhaseStatus 'failed'
                $receipt.status = Get-LaunchStatus 'process-exited'
                $receipt.failure_phase = 'process-check'
                $receipt.failure_code = 'process_exited_after_launch'
            } else {
                Set-PhaseStatus 'inconclusive'
                $receipt.status = Get-LaunchStatus 'launch-inconclusive'
                $receipt.failure_phase = 'process-check'
                $receipt.failure_code = 'process_state_unknown'
            }
        }
    } catch {
        $code = if ($_.Exception.Message -cmatch '^VW_DELIVERY:([a-z][a-z0-9_]{0,79})$') { $Matches[1] } else { 'unexpected_error' }
        if ($code -ceq 'unexpected_error') { Write-Host "[normal-delivery] unexpected error during $($ctx.phase): $($_.Exception.Message)" }
        $receipt.failure_phase = $ctx.phase
        $receipt.failure_code = $code
        Set-PhaseStatus 'failed'
        $receipt.status = 'refused'
        if ($ctx.launchAttempted) {
            # Verified bytes were launched (or a launch was attempted): keep them.
            $receipt.status = Get-LaunchStatus 'launch-inconclusive'
        } elseif ($ctx.installAttempted) {
            $receipt.status = 'failed-after-install'
            Invoke-Rollback $code
        }
    } finally {
        if ($null -ne $ctx.apkStream) { $ctx.apkStream.Dispose() }
        try {
            if ($null -ne $ctx.lock) {
                $name = 'android-normal-delivery-' + $RunId + '.json'
                try {
                    $json = $receipt | ConvertTo-Json -Depth 10
                    $serial = if ($null -ne $ctx.device -and $null -ne $ctx.device.PSObject.Properties['Serial']) { [string]$ctx.device.Serial } else { '' }
                    $forbidden = @($root, $root.Replace('\', '\\'), $root.Replace('\', '/'))
                    if ($serial) { $forbidden += $serial }
                    if (@($forbidden | Where-Object { $json.IndexOf($_, [StringComparison]::OrdinalIgnoreCase) -ge 0 }).Count -ne 0) {
                        # Keep the outcome: write only fixed codes, no free text.
                        Write-Host '[normal-delivery] receipt privacy check matched a serial or local path; writing a minimal redacted receipt.'
                        $json = [ordered]@{ schema = 1; kind = 'android-normal-delivery'; run_id = $RunId; package = $package; status = $receipt.status
                            failure_phase = $receipt.failure_phase; failure_code = $receipt.failure_code; rollback_succeeded = $receipt.rollback.succeeded
                            receipt_redacted = $true; redaction_reason = 'privacy_check' } | ConvertTo-Json -Depth 4
                    }
                    Write-VwPrivateText (Join-Path $ctx.local $name) $json
                    Write-Host ("[normal-delivery] receipt: .local/$name; status=" + $receipt.status)
                } catch {
                    Write-Host ("[normal-delivery] WARNING: receipt could not be written; status=" + $receipt.status)
                }
            }
        } finally {
            if ($null -ne $ctx.lock) { $ctx.lock.Dispose() }
        }
    }
    return $receipt
}

Export-ModuleMember -Function Get-VwNormalDeliveryConstants, Get-VwApkSignerDigest, Get-VwApkBadging, Get-VwAndroidProfileIds, Get-VwPackageUserStates, Get-VwProcessState, Get-VwInstalledApkSet, Get-VwDeviceDigest, Test-VwInstallSuccess, Get-VwInstallFailureCode, Test-VwLaunchStatus, Test-VwNormalForeground, Assert-VwExistingAdbServer, Assert-VwPlainPath, Get-VwFileSha256, Invoke-VwNormalDelivery
