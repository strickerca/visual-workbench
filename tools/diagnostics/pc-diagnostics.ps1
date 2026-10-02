# Read-only allowlisted facts; raw command output stays in memory.
param([string]$OutFile, [ValidateSet('True','False')][string]$OwnerSecureBootAnswer)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $root 'tools/process.psm1')
function RegistryValue([string]$Path, [string]$Name) {
    try { return Get-ItemPropertyValue -LiteralPath $Path -Name $Name -ErrorAction Stop } catch { return $null }
}
function SafeText([string]$Text) {
    foreach ($value in @($env:USERPROFILE,$env:USERNAME,$env:COMPUTERNAME)) {
        if ($value) { $Text = [regex]::Replace($Text, [regex]::Escape($value), '[redacted]', 'IgnoreCase') }
    }
    return $Text
}
function VersionParts([string]$Text) { return ,@($Text.Split('.') | ForEach-Object { if ($_ -match '^\d+$') { [int]$_ } }) }
function ReadCommand([string]$Exe, [string[]]$Arguments, [string]$Phase) {
    $result = Invoke-VwProcess -FilePath $Exe -ArgumentList $Arguments -WorkingDirectory $root -Phase $Phase -TimeoutSeconds 30 -SensitiveCapture
    return [pscustomobject]@{ code = $result.ExitCode; text = ($result.Lines -join [Environment]::NewLine) }
}
$cs = Get-CimInstance Win32_ComputerSystem
$cpu = Get-CimInstance Win32_Processor | Select-Object -First 1
$os = Get-CimInstance Win32_OperatingSystem
$disk = Get-CimInstance Win32_LogicalDisk -Filter "DeviceID='C:'"
$versionKey = 'HKLM:\SOFTWARE\Microsoft\Windows NT\CurrentVersion'
$o = [ordered]@{
    schema_version = 2; collected_utc = [DateTime]::UtcNow.ToString('o'); read_only = $true
    pc = [ordered]@{ manufacturer = SafeText $cs.Manufacturer; model = SafeText $cs.Model; cpu = SafeText $cpu.Name; cores = $cpu.NumberOfCores; logical_processors = $cpu.NumberOfLogicalProcessors; ram_bytes = [long]$cs.TotalPhysicalMemory }
    windows = [ordered]@{ edition = SafeText $os.Caption; display_version = RegistryValue $versionKey 'DisplayVersion'; build = [int]$os.BuildNumber; update_build_revision = RegistryValue $versionKey 'UBR' }
    disk_c = [ordered]@{ free_bytes = [long]$disk.FreeSpace; total_bytes = [long]$disk.Size; storage_warning_waived_by_owner = $true }
}
$configured = RegistryValue 'HKLM:\SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity' 'Enabled'
try {
    $guard = Get-CimInstance -Namespace root\Microsoft\Windows\DeviceGuard -ClassName Win32_DeviceGuard -ErrorAction Stop
    $o.memory_integrity = [ordered]@{ running = (@($guard.SecurityServicesRunning) -contains 2); configured_registry = $configured; method = 'Win32_DeviceGuard.SecurityServicesRunning contains 2' }
} catch { $o.memory_integrity = [ordered]@{ running = $null; configured_registry = $configured; status = 'untestable: DeviceGuard query unavailable; configured is not running evidence' } }
try { $o.secure_boot = [ordered]@{ enabled = [bool](Confirm-SecureBootUEFI -ErrorAction Stop); method = 'Confirm-SecureBootUEFI in diagnostic process' } }
catch { $o.secure_boot = [ordered]@{ enabled = $null; method = 'untestable: process lacks firmware read permission' } }
if ($OwnerSecureBootAnswer) {
    $o.secure_boot = [ordered]@{ enabled = ($OwnerSecureBootAnswer -eq 'True'); method = 'Owner-confirmed administrator PowerShell Confirm-SecureBootUEFI'; confirmed_date = (Get-Date).ToString('yyyy-MM-dd') }
}
$sac = RegistryValue 'HKLM:\SYSTEM\CurrentControlSet\Control\CI\Policy' 'VerifiedAndReputablePolicyState'
$o.smart_app_control = switch ($sac) { 0 {'off'} 1 {'on'} 2 {'evaluation'} default {'untestable: policy value absent'} }
$dev = RegistryValue 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock' 'AllowDevelopmentWithoutDevLicense'
$o.developer_mode = if ($null -eq $dev) { 'untestable: registry value absent' } else { $dev -eq 1 }
$boot = ReadCommand (Join-Path $env:SystemRoot 'System32/bcdedit.exe') @('/enum','{current}') 't003-boot-policy'
$o.test_signing = if ($boot.code -ne 0) { 'untestable: bcdedit unavailable without elevation' } elseif ($boot.text -match '(?im)^testsigning\s+(Yes|No)\s*$') { $Matches[1] -eq 'Yes' } else { 'untestable: no explicit test-signing field returned' }
$o.graphics = @(Get-CimInstance Win32_VideoController | ForEach-Object { [ordered]@{ name = SafeText $_.Name; driver_version_components = VersionParts $_.DriverVersion } })
$apps = @(Get-ItemProperty @('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*','HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*') -ErrorAction SilentlyContinue)
$o.creative_apps_classic = @($apps | Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -match 'Photoshop|Illustrator|Krita|Affinity|GIMP|Clip Studio|Photopea' } | ForEach-Object { [ordered]@{ name = SafeText $_.DisplayName; version_components = if ($_.PSObject.Properties['DisplayVersion']) { VersionParts $_.DisplayVersion } else { @() } } })
try {
    $store = @(Get-AppxPackage -ErrorAction Stop)
    $o.creative_apps_store = @($store | Where-Object Name -match 'Affinity|Krita|GIMP|Photoshop' | ForEach-Object { [ordered]@{ name = SafeText $_.Name; version_components = VersionParts "$($_.Version)" } })
    $o.paint = @($store | Where-Object Name -eq 'Microsoft.Paint' | ForEach-Object { VersionParts "$($_.Version)" })
    $o.codec_extensions = [ordered]@{}
    foreach ($codec in @('HEIF','HEVC','AV1','Webp','Raw')) { $o.codec_extensions[$codec] = [bool]@($store | Where-Object Name -like "Microsoft.$($codec)*Extension*").Count }
} catch { $o.creative_apps_store = 'untestable: Store package enumeration unavailable'; $o.paint = $null; $o.codec_extensions = 'untestable: Store package enumeration unavailable' }
$o.documents_cloud_synced = [bool]([Environment]::GetFolderPath('MyDocuments') -match 'OneDrive|Dropbox|Google Drive|iCloud')
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA 'Android/Sdk' }
$adb = ReadCommand (Join-Path $sdk 'platform-tools/adb.exe') @('version') 't003-adb-version'
$o.adb = if ($adb.code -eq 0 -and $adb.text -match '(?m)^Version (\d+\.\d+\.\d+)') { $Matches[1] } else { 'untestable: no parsed platform-tools version' }
$wireless = ReadCommand (Join-Path $env:SystemRoot 'System32/netsh.exe') @('wlan','show','interfaces') 't003-wifi'
$wifi = [ordered]@{ band_ghz = $null; radio = $null; channel = $null; status = 'untestable: no active numeric Wi-Fi fields; disconnected or Location access denied' }
if ($wireless.text -match '(?im)^\s*Band\s*:\s*(2\.4|5|6)\s*GHz') { $wifi.band_ghz = [double]::Parse($Matches[1], [Globalization.CultureInfo]::InvariantCulture); $wifi.status = 'verified active interface fields' }
if ($wireless.text -match '(?im)^\s*Radio type\s*:\s*(802\.11[a-z0-9]+)\s*$') { $wifi.radio = $Matches[1] }
if ($wireless.text -match '(?im)^\s*Channel\s*:\s*(\d{1,3})\s*$') { $wifi.channel = [int]$Matches[1] }
$o.wifi = $wifi
$o.usb_controllers = @(Get-CimInstance Win32_USBController | ForEach-Object { SafeText $_.Name } | Sort-Object -Unique)
$json = $o | ConvertTo-Json -Depth 12
if ($OutFile) { [IO.File]::WriteAllText([IO.Path]::GetFullPath($OutFile), $json + [Environment]::NewLine, (New-Object Text.UTF8Encoding($false))) }
Write-Output $json
