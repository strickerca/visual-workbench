# Read-only device facts; no activity launch, reset or foreground change.
param([string]$OutFile)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $root 'tools/android-device.psm1')
Import-Module (Join-Path $PSScriptRoot 'phone-parsers.psm1')
$device = Get-VwAndroidDevice -Root $root
function ReadPhone([string[]]$Arguments, [string]$Phase) {
    try {
        $r = Invoke-VwAdb -Device $device -Arguments $Arguments -Phase $Phase -TimeoutSeconds 30 -Capture
        return [pscustomobject]@{ status = 'readable'; text = ($r.Lines -join [Environment]::NewLine) }
    } catch { return [pscustomobject]@{ status = 'untestable: bounded device probe failed'; text = '' } }
}
function Property([string]$Name) {
    $r = ReadPhone @('shell','getprop',$Name) ('t003-prop-' + $Name.Replace('.','-'))
    $text = $r.text.Trim()
    if ($r.status -ne 'readable') { return $r.status }
    if ($text -match '^[A-Za-z0-9 ._()+/-]{1,100}$') { return $text }
    return 'untestable: property absent or outside allowlist'
}
$o = [ordered]@{
    schema_version = 2; collected_utc = [DateTime]::UtcNow.ToString('o'); read_only = $true
    shared_device = $true; foreground_changed = $false
    model = Property 'ro.product.model'; manufacturer = Property 'ro.product.manufacturer'
    android_release = Property 'ro.build.version.release'; api_level = Property 'ro.build.version.sdk'
    build_id = Property 'ro.build.id'; security_patch = Property 'ro.build.version.security_patch'
    one_ui = Property 'ro.build.version.oneui'; chipset = Property 'ro.board.platform'
    target_s23 = 'untestable: target S23 Ultra not connected; connected IN2019 is authorized for startup only'
}
$size = ReadPhone @('shell','wm','size') 't003-phone-size'
$o.display_sizes = @([regex]::Matches($size.text, '(Physical|Override) size:\s*(\d+)x(\d+)') | ForEach-Object { [ordered]@{ type = $_.Groups[1].Value; width = [int]$_.Groups[2].Value; height = [int]$_.Groups[3].Value } })
$density = ReadPhone @('shell','wm','density') 't003-phone-density'
$o.display_densities = @([regex]::Matches($density.text, '(Physical|Override) density:\s*(\d+)') | ForEach-Object { [ordered]@{ type = $_.Groups[1].Value; dpi = [int]$_.Groups[2].Value } })
$display = ReadPhone @('shell','dumpsys','display') 't003-phone-display'
$o.display_modes = ConvertFrom-VwDisplayListing $display.text
$o.display_probe_status = $display.status
$inputProbe = ReadPhone @('shell','getevent','-lp') 't003-phone-input'
$o.pen_devices = ConvertFrom-VwPenListing $inputProbe.text
$o.pen_probe_status = $inputProbe.status
$o.pen_behavior = 'untestable: no physical S Pen traces; static axes do not establish pressure, hover, tilt or rate behavior'
$codec = ReadPhone @('shell','cat /vendor/etc/media_codecs*.xml 2>/dev/null') 't003-phone-codecs'
$o.codec_vendor_declarations = ConvertFrom-VwCodecListing $codec.text
$o.codec_probe_status = $codec.status
$o.codec_execution = 'untestable: XML names do not establish hardware acceleration, low-latency support or 4:4:4 output; MediaCodec capability/processing probes pending'
$battery = ReadPhone @('shell','dumpsys','battery') 't003-phone-battery'
$o.battery = [ordered]@{ temperature_c = $null; level_percent = $null; ten_minute_baseline_recorded = $false; status = $battery.status }
if ($battery.text -match '(?m)^\s*temperature:\s*(-?\d+)\s*$') { $o.battery.temperature_c = [int]$Matches[1] / 10.0 }
if ($battery.text -match '(?m)^\s*level:\s*(\d+)\s*$') { $o.battery.level_percent = [int]$Matches[1] }
$package = ReadPhone @('shell','dumpsys','package','com.visualworkbench.android') 't003-phone-app-version'
$o.app_version = if ($package.text -match '(?m)^\s*versionName=([A-Za-z0-9._-]+)\s*$') { $Matches[1] } else { 'untestable: starter not installed or version unavailable' }
$json = $o | ConvertTo-Json -Depth 12
if ($OutFile) { [IO.File]::WriteAllText([IO.Path]::GetFullPath($OutFile), $json + [Environment]::NewLine, (New-Object Text.UTF8Encoding($false))) }
Write-Output $json
