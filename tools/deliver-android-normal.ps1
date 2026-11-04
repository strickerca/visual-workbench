[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ApkPath,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ApkSha256,
    [Parameter(Mandatory = $true)][ValidateSet('SM-S918U')][string]$ExpectedModel,
    [switch]$Execute
)
# Reviewed same-signer update of the NORMAL Visual Workbench app for owner
# user 0. Reach it through: build.ps1 deliver-android-normal. Windows
# PowerShell 5.1 compatible. Decision logic lives in android-normal-delivery.psm1.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $Execute) { throw 'Normal app delivery changes the phone; run it through build.ps1 deliver-android-normal.' }
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Normal app delivery requires Windows.' }
$projectRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
Import-Module (Join-Path $PSScriptRoot 'process.psm1')
Import-Module (Join-Path $PSScriptRoot 'android-device.psm1')
Import-Module (Join-Path $PSScriptRoot 'android-normal-delivery.psm1')

# Same SDK resolution as tools/doctor.ps1 and tools/android-device.psm1.
$sdkRoot = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } elseif ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$adbPath = Join-Path $sdkRoot 'platform-tools\adb.exe'
$apkSignerPath = Join-Path $sdkRoot 'build-tools\36.1.0\apksigner.bat'
$aaptPath = Join-Path $sdkRoot 'build-tools\36.1.0\aapt2.exe'
foreach ($tool in @($adbPath, $apkSignerPath, $aaptPath)) {
    if (-not (Test-Path -LiteralPath $tool -PathType Leaf)) { throw 'Normal delivery requires Android platform-tools and build-tools 36.1.0 (see build.ps1 doctor).' }
    Assert-VwPlainPath $tool
}
$redact = @($projectRoot, $env:USERPROFILE)

$selectDevice = {
    $null = Assert-VwExistingAdbServer -AdbPath $adbPath -Root $projectRoot
    # Explicit model selection; exactly one matching physical device, no fallback.
    $selected = Get-VwAndroidDevice -Root $projectRoot -ExpectedModel $ExpectedModel
    if ([IO.Path]::GetFullPath($selected.AdbPath) -ine [IO.Path]::GetFullPath($adbPath)) { throw 'Device selection used a different adb client.' }
    return $selected
}.GetNewClosure()
$adb = {
    param($Device, [string]$Phase, [string[]]$Arguments, [int]$Seconds)
    $result = Invoke-VwProcess -FilePath $Device.AdbPath -ArgumentList (@('-s', $Device.Serial) + $Arguments) -WorkingDirectory $projectRoot -Phase $Phase -TimeoutSeconds $Seconds -RedactValues (@($Device.Serial) + $redact) -SensitiveCapture
    return [pscustomobject]@{ ExitCode = $result.ExitCode; Lines = $result.Lines }
}.GetNewClosure()
$apkSigner = {
    param([string]$Phase, [string]$Path)
    $result = Invoke-VwProcess -FilePath $apkSignerPath -ArgumentList @('verify', '--verbose', '--print-certs', $Path) -WorkingDirectory $projectRoot -Phase $Phase -TimeoutSeconds 120 -RedactValues $redact -SensitiveCapture
    return [pscustomobject]@{ ExitCode = $result.ExitCode; Lines = $result.Lines }
}.GetNewClosure()
$aapt = {
    param([string]$Phase, [string]$Path)
    $result = Invoke-VwProcess -FilePath $aaptPath -ArgumentList @('dump', 'badging', $Path) -WorkingDirectory $projectRoot -Phase $Phase -TimeoutSeconds 120 -RedactValues $redact -SensitiveCapture
    return [pscustomobject]@{ ExitCode = $result.ExitCode; Lines = $result.Lines }
}.GetNewClosure()

$assertAdbServer = { Assert-VwExistingAdbServer -AdbPath $adbPath -Root $projectRoot }.GetNewClosure()

$receipt = Invoke-VwNormalDelivery -ProjectRoot $projectRoot -ApkPath $ApkPath -ApkSha256 $ApkSha256 -ExpectedModel $ExpectedModel -SelectDevice $selectDevice -Adb $adb -ApkSigner $apkSigner -Aapt $aapt -AssertAdbServer $assertAdbServer
switch ($receipt.status) {
    'delivered' { Write-Host "Normal app delivery on ${ExpectedModel}: PASS (same signer, owner user 0, installed bytes read back, normal launch in foreground, process alive). Physical S Pen, performance and owner acceptance remain separate." }
    'already-installed' { Write-Host 'The installed normal app already has these exact bytes; nothing was installed. Normal launch observed.' }
    'delivered-foreground-inconclusive' { Write-Host 'Normal app installed and read back, but foreground was not observed on the shared phone; launch result is inconclusive. No other app was stopped.' }
    'delivered-launch-inconclusive' { Write-Host ("Normal app installed and read back; launch did not complete cleanly (" + $receipt.failure_code + "). The new build may have run, so it was NOT rolled back automatically. Diagnose on the phone; the backup is under .local/android-normal-backups.") }
    'delivered-process-exited' { Write-Host 'Normal app installed, read back and shown, but its process exited shortly after launch (possible crash). Not rolled back; diagnose on the phone.' }
    'install-timeout-unchanged' { Write-Host 'adb install timed out; settled re-reads show the previous bytes still installed. A device-side session may still finish: re-check before retrying.' }
    'rolled-back' { Write-Host 'Normal app delivery FAILED after install and before launch; the previous installed APK bytes were restored and verified.' }
    'rollback-failed' { Write-Host 'Normal app delivery FAILED and rollback could not be verified. Keep the backup under .local/android-normal-backups; do not uninstall the app.' }
    default { Write-Host ("Normal app delivery refused in phase '" + $receipt.failure_phase + "' (" + $receipt.failure_code + '); the installed app was not changed by this run.') }
}
if ($receipt.status -cnotin @('delivered', 'already-installed')) { exit 1 }
exit 0
