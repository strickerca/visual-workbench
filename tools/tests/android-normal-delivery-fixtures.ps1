Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Synthetic decision fixtures for tools/android-normal-delivery.psm1. A fake
# adb/apksigner/aapt2 simulate the phone and tools; no device command runs.
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$module = Import-Module (Join-Path $repoRoot 'tools/android-normal-delivery.psm1') -Force -PassThru
$constants = Get-VwNormalDeliveryConstants
$script:checks = 0
function Check([bool]$Passed, [string]$Label) { if (-not $Passed) { throw "Normal delivery fixture failed: $Label" }; $script:checks++ }
function Test-Refused([scriptblock]$Action) { try { & $Action | Out-Null; return $false } catch { return $true } }

$utf8 = New-Object Text.UTF8Encoding -ArgumentList $false
function Get-TextSha256([string]$Text) {
    $encoding = New-Object Text.UTF8Encoding -ArgumentList $false
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($encoding.GetBytes($Text))) -replace '-', '').ToLowerInvariant() } finally { $sha.Dispose() }
}
$signerA = 'ab' * 32
$signerB = 'cd' * 32
$serial = 'FAKE-SERIAL-0001'
function New-ApkText([string]$Id, [string]$Signer = $signerA, [string]$Package = 'com.visualworkbench.android', [int]$VersionCode = 1, [switch]$NoLauncher) {
    $launcher = if ($NoLauncher) { 'none' } else { 'main' }
    return "FAKEAPK;id=$Id;pkg=$Package;vc=$VersionCode;signer=$Signer;launcher=$launcher"
}

# ---------- Parser checks ----------
$signerLines = @('Verifies', 'Verified using v2 scheme (APK Signature Scheme v2): true', 'Number of signers: 1', 'Signer #1 certificate DN: CN=Android Debug', ('Signer #1 certificate SHA-256 digest: ' + $signerA.ToUpperInvariant()))
Check ((Get-VwApkSignerDigest $signerLines) -ceq $signerA) 'signer digest parsed and lowercased'
Check (Test-Refused { Get-VwApkSignerDigest ($signerLines + @('Signer #2 certificate SHA-256 digest: ' + $signerB)) }) 'two signers refused'
Check (Test-Refused { Get-VwApkSignerDigest @('DOES NOT VERIFY', ('Signer #1 certificate SHA-256 digest: ' + $signerA)) }) 'unverified signature refused'
Check (Test-Refused { Get-VwApkSignerDigest @('Verifies') }) 'missing digest refused'
$badging = Get-VwApkBadging @("package: name='com.visualworkbench.android' versionCode='7' versionName='0.0.1-dev' platformBuildVersionName='16'", "launchable-activity: name='com.visualworkbench.android.MainActivity'  label='' icon=''")
Check ($badging.Package -ceq 'com.visualworkbench.android' -and $badging.VersionCode -eq 7 -and @($badging.LaunchableActivities) -ccontains $constants.Activity) 'badging parsed'
Check (Test-Refused { Get-VwApkBadging @('ERROR: dump failed') }) 'unparseable badging refused'
$ids = @(Get-VwAndroidProfileIds @('Users:', '	UserInfo{0:Owner:c13} running', '	UserInfo{150:Secure Folder:1030}'))
Check ($ids.Count -eq 2 -and $ids[0] -ceq '0' -and $ids[1] -ceq '150') 'profile ids parsed through reviewed owner-profile validation'
foreach ($bad in @(@('Users:'), @('Users:', '	UserInfo{150:Private:1030}'), @('Error: permission denied'), @('Users:', '	UserInfo{0:Owner:13}', 'garbage'))) {
    Check (Test-Refused { Get-VwAndroidProfileIds $bad }) 'ambiguous profile inventory refused'
}
# One dumpsys read covers every profile (pm list --user N is refused for
# shell on Secure Folder/work profiles). Rows outside the package section and
# sibling packages are ignored.
function New-PackageDump([hashtable]$States, [string[]]$Order, [switch]$DuplicateUser, [switch]$NoInstalledField, [switch]$TwoSections) {
    $rows = @('Activity Resolver Table:', '  Non-Data Actions:', '      android.intent.action.MAIN:', '        5d3a9b2 com.visualworkbench.android/.MainActivity filter 1a2b', '', 'Packages:',
        '  Package [com.visualworkbench.android.hil] (77aa01):', '    userId=10301', '    User 0: ceDataInode=9 installed=true hidden=false', '    User 150: ceDataInode=9 installed=true hidden=false',
        '  Package [com.visualworkbench.android] (5d3a9b2):', '    userId=10234', '    versionCode=1 minSdk=29 targetSdk=36', '    pkgFlags=[ HAS_CODE ALLOW_CLEAR_USER_DATA ]')
    foreach ($id in $Order) {
        $installed = if ($States.ContainsKey($id) -and $States[$id]) { 'true' } else { 'false' }
        $field = if ($NoInstalledField) { '' } else { ' installed=' + $installed }
        $rows += ('    User ' + $id + ': ceDataInode=1' + $field + ' hidden=false suspended=false stopped=false notLaunched=false enabled=0')
        $rows += @('      gids=[3003]', '      runtime permissions:')
        if ($DuplicateUser) { $rows += ('    User ' + $id + ': ceDataInode=1 installed=false hidden=false') }
    }
    if ($TwoSections) { $rows += @('Hidden system packages:', '  Package [com.visualworkbench.android] (5d3a9b3):', '    User 0: ceDataInode=1 installed=true') }
    $rows += @('', 'Queries:', '  system apps queryable: false', 'Dexopt state:', '  [com.visualworkbench.android]', '    User 150: installed=true')
    return $rows
}
$states = Get-VwPackageUserStates 'com.visualworkbench.android' (New-PackageDump @{ '0' = $true } @('0', '150'))
Check ($states.Count -eq 2 -and $states['0'] -eq $true -and $states['150'] -eq $false) 'per-user package states parsed from one dumpsys section'
Check ((Get-VwPackageUserStates 'com.visualworkbench.android' (New-PackageDump @{ '0' = $true; '150' = $true } @('0', '150')))['150'] -eq $true) 'other-profile installation detected'
Check (Test-Refused { Get-VwPackageUserStates 'com.visualworkbench.android' @('Unable to find package: com.visualworkbench.android') }) 'missing package section refused'
Check (Test-Refused { Get-VwPackageUserStates 'com.visualworkbench.android' (New-PackageDump @{ '0' = $true } @('0') -DuplicateUser) }) 'duplicate user row refused'
Check (Test-Refused { Get-VwPackageUserStates 'com.visualworkbench.android' (New-PackageDump @{ '0' = $true } @('0') -NoInstalledField) }) 'user row without installed state refused'
Check (Test-Refused { Get-VwPackageUserStates 'com.visualworkbench.android' (New-PackageDump @{ '0' = $true } @('0') -TwoSections) }) 'two package sections refused'
Check (Test-Refused { Get-VwPackageUserStates 'com.visualworkbench.android' (New-PackageDump @{ '150' = $true } @('150')) }) 'section without owner user refused'
Check ((Get-VwProcessState 0 @('12345')) -ceq 'alive' -and (Get-VwProcessState 0 @('12345 12399')) -ceq 'alive') 'pidof alive parsed'
Check ((Get-VwProcessState 1 @()) -ceq 'absent') 'pidof absent parsed'
Check ((Get-VwProcessState 0 @('pidof: permission denied')) -ceq 'unknown' -and (Get-VwProcessState 124 @()) -ceq 'unknown') 'pidof unknown parsed'
$dir = '/data/app/~~Zx1Q==/com.visualworkbench.android-Ab2C=='
$set = @(Get-VwInstalledApkSet @("package:$dir/split_config.xxhdpi.apk", "package:$dir/base.apk"))
Check ($set.Count -eq 2 -and $set[0].Name -ceq 'base.apk' -and $set[1].Name -ceq 'split_config.xxhdpi.apk' -and $set[0].Remote -ceq "$dir/base.apk") 'base/split inventory ordered with base first'
foreach ($bad in @(@(), @("package:$dir/split_a.apk"), @("package:$dir/base.apk", "package:$dir/base.apk"), @("package:$dir/base.apk", 'package:/data/app/~~other==/x/split_a.apk'), @('package:/system/app/x/base.apk'), @('package:/data/app/../x/base.apk'), @("package:$dir/base.apk; rm -rf /"))) {
    Check (Test-Refused { Get-VwInstalledApkSet $bad }) 'unsafe installed path inventory refused'
}
Check ((Get-VwDeviceDigest "$dir/base.apk" @(($signerA + '  ' + "$dir/base.apk"))) -ceq $signerA) 'device digest parsed'
Check (Test-Refused { Get-VwDeviceDigest "$dir/base.apk" @(($signerA + '  /data/app/other/base.apk')) }) 'device digest for another path refused'
Check (Test-VwInstallSuccess @('Performing Streamed Install', 'Success')) 'install success parsed'
Check (-not (Test-VwInstallSuccess @('Performing Streamed Install', 'adb: failed to install x: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: signatures do not match]'))) 'install failure is not success'
Check ((Get-VwInstallFailureCode @('Failure [INSTALL_FAILED_VERSION_DOWNGRADE]')) -ceq 'install_failed_version_downgrade') 'install failure code extracted'
Check ((Get-VwInstallFailureCode @('something else')) -ceq 'install_not_confirmed') 'unknown install failure code'
Check (Test-VwLaunchStatus @('Starting: Intent { act=android.intent.action.MAIN }', 'Status: ok', 'LaunchState: COLD')) 'launch status ok'
Check (-not (Test-VwLaunchStatus @('Error: Activity class {x} does not exist.'))) 'launch error refused'
Check (Test-VwNormalForeground @('  topResumedActivity=ActivityRecord{1a2b u0 com.visualworkbench.android/.MainActivity t42}')) 'normal owner foreground recognized'
Check (Test-VwNormalForeground @('  mResumedActivity: ActivityRecord{1a2b u0 com.visualworkbench.android/com.visualworkbench.android.MainActivity t42}')) 'full component foreground recognized'
Check (-not (Test-VwNormalForeground @('  topResumedActivity=ActivityRecord{1a2b u0 com.visualworkbench.android.hil/com.visualworkbench.android.MainActivity t42}'))) 'HIL package foreground refused'
Check (-not (Test-VwNormalForeground @('  topResumedActivity=ActivityRecord{1a2b u150 com.visualworkbench.android/.MainActivity t42}'))) 'other profile foreground refused'
Check (-not (Test-VwNormalForeground @('    mLastResumedActivity: ActivityRecord{1a2b u0 com.visualworkbench.android/.MainActivity t42}', '      Hist #0: ActivityRecord{1a2b u0 com.visualworkbench.android/.MainActivity t42} topResumedActivity'))) 'only anchored resumed-activity rows count'
Check (Test-VwNormalForeground @('    ResumedActivity: ActivityRecord{1a2b u0 com.visualworkbench.android/.MainActivity t42}')) 'ResumedActivity row recognized'

# ---------- Source drift: launcher activity and package ----------
[xml]$manifest = Get-Content -Raw -LiteralPath (Join-Path $repoRoot 'apps/android/src/main/AndroidManifest.xml')
$androidNs = 'http://schemas.android.com/apk/res/android'
$launchers = @($manifest.SelectNodes('/manifest/application/activity') | Where-Object {
    $activity = $_
    @($activity.SelectNodes('intent-filter') | Where-Object {
        @($_.SelectNodes('action') | Where-Object { $_.GetAttribute('name', $androidNs) -ceq 'android.intent.action.MAIN' }).Count -eq 1 -and
        @($_.SelectNodes('category') | Where-Object { $_.GetAttribute('name', $androidNs) -ceq 'android.intent.category.LAUNCHER' }).Count -eq 1
    }).Count -gt 0
})
Check ($launchers.Count -eq 1 -and $launchers[0].GetAttribute('name', $androidNs) -ceq '.MainActivity' -and $constants.Component -ceq ($constants.Package + '/.MainActivity')) 'manifest launcher matches delivery component'
$gradle = Get-Content -Raw -LiteralPath (Join-Path $repoRoot 'apps/android/build.gradle.kts')
Check ($gradle -cmatch ('(?m)^\s*applicationId = "' + [regex]::Escape($constants.Package) + '"\s*$')) 'gradle applicationId matches delivery package'
Check ($constants.Model -ceq 'SM-S918U') 'authorized model pinned'

# ---------- Fake phone and tools ----------
function New-FakePhone {
    param([string]$InstalledText)
    $phone = @{
        currentUser = '0'
        users = @('Users:', '	UserInfo{0:Owner:c13} running')
        packageUsers = @{ '0' = $true }
        dir = $dir
        files = [ordered]@{ 'base.apk' = $InstalledText }
        installModes = New-Object Collections.Generic.List[string]
        launchMode = 'ok'
        foreground = $true
        processAlive = $true
        packageAbsent = $false
        dumpIds = $null
        serverChecks = New-Object Collections.Generic.List[bool]
        calls = New-Object Collections.Generic.List[object]
    }
    $phone.installModes.Add('success')
    return $phone
}
function New-FakeAdb($Phone) {
    # GetNewClosure captures only these locals.
    $hashText = ${function:Get-TextSha256}
    $packageDump = ${function:New-PackageDump}
    $encoding = New-Object Text.UTF8Encoding -ArgumentList $false
    return {
        param($Device, [string]$Phase, [string[]]$Arguments, [int]$Seconds)
        $Phone.calls.Add($Arguments)
        $line = $Arguments -join ' '
        $result = { param([int]$Exit, [string[]]$Lines) [pscustomobject]@{ ExitCode = $Exit; Lines = @($Lines) } }
        $installFrom = {
            param([string[]]$Paths)
            $mode = if ($Phone.installModes.Count -gt 0) { $Phone.installModes[0] } else { 'success' }
            if ($Phone.installModes.Count -gt 1) { $Phone.installModes.RemoveAt(0) }
            if ($mode -ceq 'failure') { return & $result 1 @('Performing Streamed Install', 'adb: failed to install: Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Existing package signatures do not match]') }
            if ($mode -ceq 'timeout-unchanged') { return & $result 124 @('Performing Streamed Install') }
            $Phone.files = [ordered]@{}
            $Phone.dir = '/data/app/~~N' + $Phone.calls.Count + '==/com.visualworkbench.android-Q' + $Phone.calls.Count + '=='
            foreach ($path in $Paths) {
                $name = [IO.Path]::GetFileName($path)
                if ($Paths.Count -eq 1) { $name = 'base.apk' }
                $text = [IO.File]::ReadAllText($path)
                if ($mode -ceq 'corrupt') { $text += ';corrupted' }
                $Phone.files[$name] = $text
            }
            if ($mode -ceq 'timeout-installed') { return & $result 124 @('Performing Streamed Install') }
            return & $result 0 @('Performing Streamed Install', 'Success')
        }
        if ($line -ceq 'shell am get-current-user') { return & $result 0 @($Phone.currentUser) }
        if ($line -ceq 'shell pm list users') { return & $result 0 $Phone.users }
        if ($line -cmatch '^shell pm list packages --user ([0-9]+) ') {
            # Real S23 behavior for Secure Folder/work profiles from shell.
            if ($Matches[1] -cne '0') { return & $result 255 @('Exception occurred while executing ''list'':', ('java.lang.SecurityException: Shell does not have permission to access user ' + $Matches[1])) }
            return & $result 0 @('package:com.visualworkbench.android.hil', 'package:com.visualworkbench.android')
        }
        if ($line -ceq 'shell dumpsys package com.visualworkbench.android') {
            if ($Phone.packageAbsent) { return & $result 0 @('Unable to find package: com.visualworkbench.android') }
            $order = if ($null -ne $Phone.dumpIds) { @($Phone.dumpIds) } else { @($Phone.users | ForEach-Object { if ($_ -cmatch 'UserInfo\{([0-9]+):') { $Matches[1] } }) }
            return & $result 0 (& $packageDump $Phone.packageUsers $order)
        }
        if ($line -ceq 'shell pidof com.visualworkbench.android') {
            if ($Phone.processAlive) { return & $result 0 @('12345') }
            return & $result 1 @()
        }
        if ($line -ceq 'shell pm path --user 0 com.visualworkbench.android') {
            return & $result 0 @($Phone.files.Keys | ForEach-Object { 'package:' + $Phone.dir + '/' + $_ })
        }
        if ($line -cmatch '^shell sha256sum (/data/app/\S+)/([A-Za-z0-9_.]+)$') {
            $remote = $Matches[1] + '/' + $Matches[2]
            if ($Matches[1] -cne $Phone.dir -or -not $Phone.files.Contains($Matches[2])) { return & $result 1 @('sha256sum: No such file or directory') }
            return & $result 0 @((& $hashText $Phone.files[$Matches[2]]) + '  ' + $remote)
        }
        if ($Arguments.Count -eq 3 -and $Arguments[0] -ceq 'pull') {
            $name = $Arguments[1].Substring($Arguments[1].LastIndexOf('/') + 1)
            [IO.File]::WriteAllText($Arguments[2], $Phone.files[$name], $encoding)
            return & $result 0 @($Arguments[1] + ': 1 file pulled, 0 skipped.')
        }
        if ($Arguments.Count -eq 5 -and $line.StartsWith('install -r --user 0 ')) { return & $installFrom @($Arguments[4]) }
        if ($Arguments.Count -gt 5 -and $line.StartsWith('install-multiple -r --user 0 ')) { return & $installFrom @($Arguments[4..($Arguments.Count - 1)]) }
        if ($line -ceq 'shell am start -W --user 0 -a android.intent.action.MAIN -c android.intent.category.LAUNCHER -n com.visualworkbench.android/.MainActivity') {
            if ($Phone.launchMode -ceq 'ok') { return & $result 0 @('Starting: Intent { act=android.intent.action.MAIN cat=[android.intent.category.LAUNCHER] }', 'Status: ok', 'LaunchState: COLD') }
            if ($Phone.launchMode -ceq 'timeout') { return & $result 0 @('Starting: Intent { act=android.intent.action.MAIN cat=[android.intent.category.LAUNCHER] }', 'Status: timeout', 'LaunchState: UNKNOWN (0)') }
            return & $result 0 @('Starting: Intent { }', 'Error: Activity class {com.visualworkbench.android/.MainActivity} does not exist.')
        }
        if ($line -ceq 'shell dumpsys activity activities') {
            if ($Phone.foreground) { return & $result 0 @('  topResumedActivity=ActivityRecord{1a2b u0 com.visualworkbench.android/.MainActivity t42}') }
            return & $result 0 @('  topResumedActivity=ActivityRecord{1a2b u0 com.other.app/.Main t41}')
        }
        throw "Fake adb received an unexpected command: $line"
    }.GetNewClosure()
}
function New-FakeServer($Phone) {
    return {
        if ($Phone.serverChecks.Count -gt 0) {
            $alive = $Phone.serverChecks[0]; $Phone.serverChecks.RemoveAt(0)
            if (-not $alive) { throw 'Existing adb server unavailable.' }
        }
    }.GetNewClosure()
}
$fakeSigner = {
    param([string]$Phase, [string]$Path)
    $text = [IO.File]::ReadAllText($Path)
    if ($text -cnotmatch 'signer=([0-9a-f]{64})') { return [pscustomobject]@{ ExitCode = 1; Lines = @('DOES NOT VERIFY') } }
    return [pscustomobject]@{ ExitCode = 0; Lines = @('Verifies', 'Verified using v2 scheme (APK Signature Scheme v2): true', 'Number of signers: 1', 'Signer #1 certificate DN: CN=Android Debug', ('Signer #1 certificate SHA-256 digest: ' + $Matches[1])) }
}
$fakeAapt = {
    param([string]$Phase, [string]$Path)
    $text = [IO.File]::ReadAllText($Path)
    if ($text -cnotmatch 'pkg=([a-z.]+);vc=([0-9]+);') { return [pscustomobject]@{ ExitCode = 1; Lines = @('ERROR: dump failed') } }
    $lines = @("package: name='$($Matches[1])' versionCode='$($Matches[2])' versionName='0.0.1-dev' platformBuildVersionName='16'")
    if ($text.Contains('launcher=main')) { $lines += "launchable-activity: name='com.visualworkbench.android.MainActivity'  label='' icon=''" }
    return [pscustomobject]@{ ExitCode = 0; Lines = $lines }
}

$base = Join-Path ([IO.Path]::GetTempPath()) ('vw-normal-delivery-fixtures-' + [guid]::NewGuid().ToString('N'))
$script:scenario = 0
function Invoke-Scenario {
    param($Phone, [string]$ApkText, [string]$Sha256, [string]$Model = 'SM-S918U', [string]$DeviceModel = 'SM-S918U', [string]$ApkName)
    $script:scenario++
    $root = Join-Path $base ('root-' + $script:scenario)
    [void][IO.Directory]::CreateDirectory($root)
    if (-not $ApkName) { $ApkName = 'input-' + $script:scenario + '.apk' }
    $apkDirectory = Join-Path $base ('input-' + $script:scenario)
    [void][IO.Directory]::CreateDirectory($apkDirectory)
    $apk = Join-Path $apkDirectory $ApkName
    [IO.File]::WriteAllText($apk, $ApkText, $utf8)
    if (-not $Sha256) { $Sha256 = Get-TextSha256 $ApkText }
    $deviceModel = $DeviceModel; $fakeSerial = $serial
    $select = { [pscustomobject]@{ Model = $deviceModel; Serial = $fakeSerial; AdbPath = 'fake-adb' } }.GetNewClosure()
    $receipt = Invoke-VwNormalDelivery -ProjectRoot $root -ApkPath $apk -ApkSha256 $Sha256 -ExpectedModel $Model -SelectDevice $select -Adb (New-FakeAdb $Phone) -ApkSigner $fakeSigner -Aapt $fakeAapt -AssertAdbServer (New-FakeServer $Phone) -ForegroundAttempts 2 -ForegroundDelaySeconds 0 -InstallSettleSeconds 0 -ProcessCheckDelaySeconds 0 6>$null
    $receiptFile = Join-Path $root ('.local/android-normal-delivery-' + $receipt.run_id + '.json')
    Check (Test-Path -LiteralPath $receiptFile -PathType Leaf) 'receipt written under .local'
    $json = [IO.File]::ReadAllText($receiptFile)
    $null = $json | ConvertFrom-Json
    Check ($json.IndexOf($serial, [StringComparison]::OrdinalIgnoreCase) -lt 0) 'receipt omits device serial'
    Check ($json.IndexOf($root.Replace('\', '\\'), [StringComparison]::OrdinalIgnoreCase) -lt 0 -and $json.IndexOf($base.Replace('\', '\\'), [StringComparison]::OrdinalIgnoreCase) -lt 0) 'receipt omits local absolute paths'
    foreach ($call in $Phone.calls) {
        $tokens = @($call)
        Check (-not ($tokens -ccontains '-d') -and -not ($tokens -ccontains 'uninstall') -and -not ($tokens -ccontains 'clear') -and -not ($tokens -ccontains '-k') -and -not ($tokens -ccontains 'force-stop')) 'no downgrade, uninstall, data clear or force-stop'
        Check (-not (($tokens -join ' ') -cmatch '^shell pm list packages')) 'profile scope never uses per-user pm list'
        if ($tokens[0] -like 'install*') { Check ($tokens[1] -ceq '-r' -and $tokens[2] -ceq '--user' -and $tokens[3] -ceq '0') 'install uses -r --user 0 only' }
    }
    return [pscustomobject]@{ Receipt = $receipt; Root = $root; Apk = $apk; Json = $json }
}
function Get-InstallCalls($Phone) {
    # Each call is a string[]; return the outer array as one object.
    $calls = New-Object Collections.Generic.List[object]
    foreach ($call in $Phone.calls) { if (@($call)[0] -like 'install*') { $calls.Add($call) } }
    return ,$calls.ToArray()
}

$oldText = New-ApkText 'old'
$newText = New-ApkText 'new'
try {
    # 1. Happy path.
    $phone = New-FakePhone $oldText
    $run = Invoke-Scenario $phone $newText
    $r = $run.Receipt
    Check ($r.status -ceq 'delivered') 'happy path delivered'
    Check ($phone.files['base.apk'] -ceq $newText) 'new bytes installed'
    Check ($r.readback.matches -and $r.readback.sha256 -ceq (Get-TextSha256 $newText)) 'installed bytes read back'
    Check ($r.launch.status_ok -and $r.launch.foreground -and $r.launch.process_state_after_launch -ceq 'alive') 'normal launch in foreground with live process'
    Check ($r.input.signer_sha256 -ceq $signerA -and $r.installed_before.signer_sha256 -ceq $signerA) 'signers recorded'
    $backupBase = Join-Path $run.Root ('.local/android-normal-backups/' + $r.run_id + '/base.apk')
    Check ((Test-Path -LiteralPath $backupBase) -and (Get-VwFileSha256 $backupBase) -ceq (Get-TextSha256 $oldText)) 'previous base APK backed up with hash'
    Check (Test-Path -LiteralPath (Join-Path $run.Root ('.local/android-normal-backups/' + $r.run_id + '/backup.json'))) 'backup manifest written'
    Check (-not (Test-Path -LiteralPath (Join-Path $run.Root ('.local/android-normal-backups/' + $r.run_id + '/readback')))) 'readback copy disposed'
    Check ((Get-InstallCalls $phone).Count -eq 1 -and -not $r.rollback.attempted) 'single install, no rollback'

    # 2. Input hash mismatch: refused before any tool or device access.
    $phone = New-FakePhone $oldText
    $r = (Invoke-Scenario $phone $newText ('00' * 32)).Receipt
    Check ($r.status -ceq 'refused' -and $r.failure_code -ceq 'apk_sha256_mismatch' -and $phone.calls.Count -eq 0) 'APK hash mismatch refused without device access'

    # 3. Model not authorized / selected device differs.
    $phone = New-FakePhone $oldText
    $r = (Invoke-Scenario $phone $newText -Model 'IN2019').Receipt
    Check ($r.failure_code -ceq 'model_not_authorized' -and $phone.calls.Count -eq 0) 'unauthorized model refused'
    $phone = New-FakePhone $oldText
    $r = (Invoke-Scenario $phone $newText -DeviceModel 'IN2019').Receipt
    Check ($r.failure_code -ceq 'device_model_mismatch' -and $phone.calls.Count -eq 0) 'selected device model mismatch refused'

    # 4. Signer mismatch: backup taken, no install.
    $phone = New-FakePhone (New-ApkText 'old' -Signer $signerB)
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.failure_code -ceq 'signer_mismatch' -and (Get-InstallCalls $phone).Count -eq 0 -and $r.backup.complete) 'signer mismatch refused before install'

    # 5. Not installed for user 0.
    $phone = New-FakePhone $oldText; $phone.packageUsers['0'] = $false
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.failure_code -ceq 'package_not_installed_for_owner' -and (Get-InstallCalls $phone).Count -eq 0) 'missing owner install refused'

    # 6. Installed in another profile.
    $phone = New-FakePhone $oldText
    $phone.users = @('Users:', '	UserInfo{0:Owner:c13} running', '	UserInfo{150:Secure Folder:1030}'); $phone.packageUsers['150'] = $true
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.failure_code -ceq 'package_installed_in_other_profile' -and (Get-InstallCalls $phone).Count -eq 0 -and $r.device_scope.other_profiles_with_package -eq 1) 'other profile install refused'
    $phone = New-FakePhone $oldText
    $phone.users = @('Users:', '	UserInfo{0:Owner:c13} running', '	UserInfo{150:Secure Folder:1030}')
    Check ((Invoke-Scenario $phone $newText).Receipt.status -ceq 'delivered') 'other profile without the package is preserved and allowed'

    # 7. Owner user not current; ambiguous profiles.
    $phone = New-FakePhone $oldText; $phone.currentUser = '150'
    Check ((Invoke-Scenario $phone $newText).Receipt.failure_code -ceq 'owner_user_not_current') 'non-owner current user refused'
    $phone = New-FakePhone $oldText; $phone.users = @('Error: could not list users')
    Check ((Invoke-Scenario $phone $newText).Receipt.failure_code -ceq 'profile_inventory') 'ambiguous profile inventory refused'

    # 8. Wrong package (isolated HIL APK), missing launcher, unsigned input.
    $phone = New-FakePhone $oldText
    $r = (Invoke-Scenario $phone (New-ApkText 'hil' -Package 'com.visualworkbench.android.hil')).Receipt
    Check ($r.failure_code -ceq 'apk_package_not_normal' -and $phone.calls.Count -eq 0) 'HIL package APK refused'
    $phone = New-FakePhone $oldText
    Check ((Invoke-Scenario $phone (New-ApkText 'nl' -NoLauncher)).Receipt.failure_code -ceq 'apk_launcher_activity') 'APK without normal launcher refused'
    $phone = New-FakePhone $oldText
    Check ((Invoke-Scenario $phone 'FAKEAPK;unsigned').Receipt.failure_code -ceq 'apk_signature_unverified') 'unsigned APK refused'

    # 9. Version downgrade refused; newer version flags rollback downgrade risk.
    $phone = New-FakePhone (New-ApkText 'old' -VersionCode 2)
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.failure_code -ceq 'version_downgrade_refused' -and (Get-InstallCalls $phone).Count -eq 0) 'version downgrade refused'
    $phone = New-FakePhone $oldText
    $r = (Invoke-Scenario $phone (New-ApkText 'newer' -VersionCode 2)).Receipt
    Check ($r.status -ceq 'delivered' -and $r.rollback.may_require_downgrade) 'newer version delivered with rollback downgrade risk recorded'

    # 10. Install refused by Android: state unchanged, no rollback install.
    $phone = New-FakePhone $oldText; $phone.installModes[0] = 'failure'
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'install-failed-unchanged' -and $r.failure_code -ceq 'install_failed_update_incompatible' -and -not $r.rollback.needed -and (Get-InstallCalls $phone).Count -eq 1 -and $phone.files['base.apk'] -ceq $oldText) 'refused install leaves old bytes without rollback'

    # 11. Readback mismatch: rolled back to the backup.
    $phone = New-FakePhone $oldText; $phone.installModes[0] = 'corrupt'; $phone.installModes.Add('success')
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'rolled-back' -and $r.failure_code -ceq 'readback_sha256_mismatch' -and $r.rollback.succeeded -and $phone.files['base.apk'] -ceq $oldText) 'readback mismatch rolled back'
    $calls = Get-InstallCalls $phone
    Check ($calls.Count -eq 2 -and @($calls[1])[0] -ceq 'install' -and [IO.Path]::GetFileName(@($calls[1])[4]) -ceq 'base.apk') 'rollback reinstalls the backed-up base APK'

    # 12. Split installation: backup includes splits, rollback uses install-multiple.
    $splitText = 'FAKESPLIT;config.xxhdpi;signer=' + $signerA
    $phone = New-FakePhone $oldText; $phone.files['split_config.xxhdpi.apk'] = $splitText
    $phone.installModes[0] = 'corrupt'; $phone.installModes.Add('success')
    $run = Invoke-Scenario $phone $newText
    $r = $run.Receipt
    Check (@($r.installed_before.files).Count -eq 2 -and (Get-VwFileSha256 (Join-Path $run.Root ('.local/android-normal-backups/' + $r.run_id + '/split_config.xxhdpi.apk'))) -ceq (Get-TextSha256 $splitText)) 'split APK backed up with hash'
    $calls = Get-InstallCalls $phone
    Check ($r.status -ceq 'rolled-back' -and @($calls[1])[0] -ceq 'install-multiple' -and @($calls[1]).Count -eq 6 -and $phone.files.Count -eq 2 -and $phone.files['split_config.xxhdpi.apk'] -ceq $splitText) 'split install rolled back with install-multiple'

    # 13. Launch failure or timeout after verified install: never rolled back
    # (the new build may already have migrated app data).
    $phone = New-FakePhone $oldText; $phone.launchMode = 'error'
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'delivered-launch-inconclusive' -and $r.failure_code -ceq 'launch_failed' -and -not $r.rollback.attempted -and (Get-InstallCalls $phone).Count -eq 1 -and $phone.files['base.apk'] -ceq $newText) 'launch failure is inconclusive without rollback'
    $phone = New-FakePhone $oldText; $phone.launchMode = 'timeout'
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'delivered-launch-inconclusive' -and $r.failure_code -ceq 'launch_timeout' -and -not $r.rollback.attempted -and $phone.files['base.apk'] -ceq $newText) 'slow cold start timeout is inconclusive without rollback'
    $phone = New-FakePhone $oldText; $phone.processAlive = $false
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'delivered-process-exited' -and $r.launch.foreground -and $r.launch.process_state_after_launch -ceq 'absent' -and -not $r.rollback.attempted -and $phone.files['base.apk'] -ceq $newText) 'process exit after launch reported without rollback'

    # 14. Foreground not observed: verified bytes kept, inconclusive.
    $phone = New-FakePhone $oldText; $phone.foreground = $false
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'delivered-foreground-inconclusive' -and $r.launch.attempts -eq 2 -and -not $r.rollback.attempted -and $phone.files['base.apk'] -ceq $newText) 'foreground contention is inconclusive without rollback'

    # 15. Rollback itself refused: reported, backup retained.
    $phone = New-FakePhone $oldText; $phone.installModes[0] = 'corrupt'; $phone.installModes.Add('failure')
    $run = Invoke-Scenario $phone $newText
    $r = $run.Receipt
    Check ($r.status -ceq 'rollback-failed' -and $r.rollback.attempted -and -not $r.rollback.succeeded -and $r.rollback.failure_code -ceq 'install_failed_update_incompatible' -and $r.backup.retained -and (Test-Path -LiteralPath (Join-Path $run.Root ('.local/android-normal-backups/' + $r.run_id + '/base.apk')))) 'failed rollback reported with retained backup'

    # 17. Package section absent / profile rows inconsistent with pm list users.
    $phone = New-FakePhone $oldText; $phone.packageAbsent = $true
    Check ((Invoke-Scenario $phone $newText).Receipt.failure_code -ceq 'package_user_states') 'absent package section refused'
    $phone = New-FakePhone $oldText
    $phone.users = @('Users:', '	UserInfo{0:Owner:c13} running', '	UserInfo{150:Secure Folder:1030}'); $phone.dumpIds = @('0')
    Check ((Invoke-Scenario $phone $newText).Receipt.failure_code -ceq 'package_user_states_inconsistent') 'missing profile row refused'

    # 18. Byte-identical delivery: distinct status, nothing installed.
    $phone = New-FakePhone $newText
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'already-installed' -and $r.install.skipped_identical -and -not $r.install.attempted -and (Get-InstallCalls $phone).Count -eq 0 -and $r.readback.matches -and $r.launch.foreground) 'identical bytes reported as already installed'

    # 19. adb install timeout: settled delayed re-reads decide the outcome.
    $phone = New-FakePhone $oldText; $phone.installModes[0] = 'timeout-unchanged'
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'install-timeout-unchanged' -and $r.install.timed_out -and $r.install.settled_after_timeout -ceq 'previous-bytes' -and -not $r.rollback.needed -and (Get-InstallCalls $phone).Count -eq 1) 'install timeout with previous bytes is distinct from refused install'
    $phone = New-FakePhone $oldText; $phone.installModes[0] = 'timeout-installed'
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'delivered' -and $r.install.timed_out -and $r.install.settled_after_timeout -ceq 'delivered-bytes' -and $r.readback.matches) 'install timeout that settled on the delivered bytes continues verification'

    # 20. adb server re-verified before install and before rollback.
    $phone = New-FakePhone $oldText; $phone.serverChecks.Add($false)
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'refused' -and $r.failure_code -ceq 'adb_server_unavailable' -and (Get-InstallCalls $phone).Count -eq 0) 'missing adb server refuses before install'
    $phone = New-FakePhone $oldText; $phone.installModes[0] = 'corrupt'; $phone.serverChecks.Add($true); $phone.serverChecks.Add($false)
    $r = (Invoke-Scenario $phone $newText).Receipt
    Check ($r.status -ceq 'rollback-failed' -and $r.rollback.failure_code -ceq 'adb_server_unavailable' -and (Get-InstallCalls $phone).Count -eq 1) 'missing adb server before rollback is reported, not bypassed'

    # 21. Privacy refusal keeps the outcome in a minimal redacted receipt.
    $phone = New-FakePhone $oldText
    $run = Invoke-Scenario $phone $newText -ApkName ($serial + '.apk')
    $minimal = $run.Json | ConvertFrom-Json
    Check ($run.Receipt.status -ceq 'delivered' -and $minimal.status -ceq 'delivered' -and $minimal.receipt_redacted -eq $true -and $minimal.redaction_reason -ceq 'privacy_check') 'privacy refusal writes a minimal receipt with the status'

    # 16. Concurrent delivery lock.
    $phone = New-FakePhone $oldText
    $lockRoot = Join-Path $base 'locked'
    [void][IO.Directory]::CreateDirectory((Join-Path $lockRoot '.local'))
    $held = [IO.File]::Open((Join-Path $lockRoot '.local/android-normal-delivery.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    try {
        $apk = Join-Path $base 'locked.apk'; [IO.File]::WriteAllText($apk, $newText, $utf8)
        $select = { [pscustomobject]@{ Model = 'SM-S918U'; Serial = $serial } }.GetNewClosure()
        $r = Invoke-VwNormalDelivery -ProjectRoot $lockRoot -ApkPath $apk -ApkSha256 (Get-TextSha256 $newText) -ExpectedModel 'SM-S918U' -SelectDevice $select -Adb (New-FakeAdb $phone) -ApkSigner $fakeSigner -Aapt $fakeAapt -AssertAdbServer (New-FakeServer $phone) 6>$null
        Check ($r.failure_code -ceq 'delivery_lock_held' -and $phone.calls.Count -eq 0) 'concurrent delivery refused'
    } finally { $held.Dispose() }

    # ---------- Entry points ----------
    $entry = Join-Path $repoRoot 'tools/deliver-android-normal.ps1'
    $tokens = $null; $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile($entry, [ref]$tokens, [ref]$errors)
    Check ($errors.Count -eq 0) 'entry script parses'
    # Windows PowerShell turns redirected native stderr into errors under Stop.
    $ErrorActionPreference = 'Continue'
    try { $output = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $entry -ApkPath (Join-Path $base 'none.apk') -ApkSha256 ('00' * 32) -ExpectedModel 'SM-S918U' 2>&1 }
    finally { $ErrorActionPreference = 'Stop' }
    Check ($LASTEXITCODE -ne 0 -and (($output | Out-String) -match 'build\.ps1 deliver-android-normal')) 'entry script refuses without -Execute'
    $buildText = Get-Content -Raw -LiteralPath (Join-Path $repoRoot 'build.ps1')
    Check ($buildText.Contains("'deliver-android-normal'") -and $buildText.Contains('tools/tests/android-normal-delivery-fixtures.ps1') -and $buildText.Contains('tools/deliver-android-normal.ps1')) 'build.ps1 exposes delivery and runs these fixtures'
} finally {
    if ($base.StartsWith([IO.Path]::GetTempPath(), [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $base)) { Remove-Item -LiteralPath $base -Recurse -Force }
}
Write-Host "Normal app delivery fixtures: PASS ($script:checks checks; $script:scenario delivery scenarios with fake adb/apksigner/aapt2). No adb commands executed."
