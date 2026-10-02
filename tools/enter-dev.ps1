# Dot-source this file to configure the current shell; no persistent settings change.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } elseif ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
$ndk = Join-Path $sdk 'ndk\30.0.16248370'
if (-not (Test-Path -LiteralPath (Join-Path $ndk 'source.properties'))) {
    throw 'Pinned Android NDK is missing; run tools/doctor.ps1 before building.'
}
$env:ANDROID_HOME = $sdk
$env:ANDROID_SDK_ROOT = $sdk
$env:ANDROID_NDK_HOME = $ndk
$paths = @((Join-Path $sdk 'platform-tools'), (Join-Path $sdk 'cmdline-tools\latest\bin'))
foreach ($path in $paths) {
    if (($env:Path -split ';') -notcontains $path) { $env:Path = $path + ';' + $env:Path }
}
Write-Host 'Visual Workbench SDK/NDK environment configured for this shell.'
