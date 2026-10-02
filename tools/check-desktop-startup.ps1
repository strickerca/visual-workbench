param([Parameter(Mandatory = $true)][int]$ApplicationPid, [int]$TimeoutSeconds = 45)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$runtime = Join-Path $root '.local\runtime'
if (-not ('VwWindowReadiness' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class VwWindowReadiness {
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
}
'@
}
$watch = [Diagnostics.Stopwatch]::StartNew()
$lastProgress = 0
$lastScaleObservation = 'No complete display-scale marker observed'
while ($watch.Elapsed.TotalSeconds -lt $TimeoutSeconds) {
    $application = Get-Process -Id $ApplicationPid -ErrorAction Stop
    $application.Refresh()
    $stdoutFile = Join-Path $runtime 'desktop.stdout.log'
    $stdout = ''
    if (Test-Path -LiteralPath $stdoutFile) {
        $stream = [IO.File]::Open($stdoutFile, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        $reader = New-Object IO.StreamReader -ArgumentList (, $stream)
        try { $stdout = $reader.ReadToEnd() } finally { $reader.Dispose() }
    }
    $markers = [regex]::Matches($stdout, 'VW_DESKTOP_READY composeDensity=([0-9.]+) awtScaleX=([0-9.]+) awtScaleY=([0-9.]+)')
    if ($markers.Count -gt 0 -and $stdout -match 'VW_NATIVE_LIBRARIES_LOADED count=2' -and $application.MainWindowHandle -ne [IntPtr]::Zero -and [VwWindowReadiness]::IsWindowVisible($application.MainWindowHandle)) {
        $marker = $markers[$markers.Count - 1]
        $dpi = [VwWindowReadiness]::GetDpiForWindow($application.MainWindowHandle)
        $scale = $dpi / 96.0
        $density = [double]::Parse($marker.Groups[1].Value, [Globalization.CultureInfo]::InvariantCulture)
        $awtX = [double]::Parse($marker.Groups[2].Value, [Globalization.CultureInfo]::InvariantCulture)
        $awtY = [double]::Parse($marker.Groups[3].Value, [Globalization.CultureInfo]::InvariantCulture)
        $lastScaleObservation = "Windows DPI=$dpi scale=$scale; Compose=$density; AWT=$awtX,$awtY"
        if ($dpi -gt 0 -and [math]::Abs($density - $scale) -le 0.01 -and [math]::Abs($awtX - $scale) -le 0.01 -and [math]::Abs($awtY - $scale) -le 0.01) {
        [ordered]@{ visible_window = $true; native_libraries_loaded = 2; windows_window_dpi = $dpi; windows_scale = $scale; compose_density = $density; awt_scale_x = $awtX; awt_scale_y = $awtY; elapsed_seconds = [math]::Round($watch.Elapsed.TotalSeconds, 3); screenshot_count = 0; visual_sharpness_manually_verified = $false } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runtime 'desktop-readiness.json') -Encoding UTF8
        Write-Host "Desktop startup: PASS; visible window, two native libraries loaded, Windows DPI=$dpi, Compose/AWT scale=$scale."
        exit 0
        }
    }
    if ($watch.Elapsed.TotalSeconds - $lastProgress -ge 15) {
        Write-Host "Desktop startup: waiting for native loading, Compose marker and visible window ($([int]$watch.Elapsed.TotalSeconds)s)."
        $lastProgress = $watch.Elapsed.TotalSeconds
    }
    Start-Sleep -Milliseconds 500
}
throw "Desktop startup timed out ($lastScaleObservation); inspect .local/runtime desktop stdout/stderr and app process before retrying"
