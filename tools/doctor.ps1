param([switch]$Strict)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $PSScriptRoot 'process.psm1') -Force
$script:missing = New-Object System.Collections.Generic.List[string]

function Check-Tool {
    param([string]$Name, [string]$Command, [string[]]$Arguments, [string]$Expected)
    try {
        $result = Invoke-VwProcess -FilePath $Command -ArgumentList $Arguments -WorkingDirectory $projectRoot -Phase ('doctor-' + $Name) -TimeoutSeconds 30 -Capture
        $text = $result.Lines -join "`n"
        if ($result.ExitCode -ne 0 -or ($Expected -and $text -notmatch $Expected)) {
            $script:missing.Add($Name)
            Write-Host "$Name : MISSING OR DIFFERENT VERSION"
        } else {
            $display = @($result.Lines | Where-Object { $_ -and (-not $Expected -or $_ -match $Expected) }) | Select-Object -First 2
            Write-Host "$Name : $($display -join ' / ')"
        }
    } catch {
        $script:missing.Add($Name)
        Write-Host "$Name : UNAVAILABLE"
    }
}

Check-Tool 'Git' 'git.exe' @('--version') '^git version '
Check-Tool 'Rust' 'rustc.exe' @('+1.99.0', '--version') '^rustc 1\.99\.0\b'
Check-Tool 'cargo-ndk' 'cargo.exe' @('+1.99.0', 'install', '--list') '(?m)^cargo-ndk v4\.1\.2:\s*$'
Check-Tool 'cargo-deny' 'cargo.exe' @('+1.99.0', 'deny', '--version') '\b0\.20\.2\b'
Check-Tool 'Python' 'python.exe' @('--version') '^Python 3\.(?:1[1-9]|[2-9][0-9])\.'
$java = if ($env:JAVA_HOME -and (Test-Path -LiteralPath (Join-Path $env:JAVA_HOME 'bin\java.exe'))) { Join-Path $env:JAVA_HOME 'bin\java.exe' } else { 'java.exe' }
Check-Tool 'JDK' $java @('-version') 'version "21\.'
Check-Tool 'gitleaks' 'gitleaks.exe' @('version') '\d+\.\d+\.\d+'

$sdkRoot = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } elseif ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
foreach ($item in @(
    @{Name = 'Android SDK 37.0'; File = 'platforms\android-37.0\android.jar'},
    @{Name = 'Android build-tools 36.1.0'; File = 'build-tools\36.1.0\aapt2.exe'},
    @{Name = 'Android NDK 30.0.16248370'; File = 'ndk\30.0.16248370\source.properties'},
    @{Name = 'Android platform-tools'; File = 'platform-tools\adb.exe'}
)) {
    if (Test-Path -LiteralPath (Join-Path $sdkRoot $item.File) -PathType Leaf) {
        Write-Host "$($item.Name) : PRESENT"
    } else {
        $script:missing.Add($item.Name)
        Write-Host "$($item.Name) : MISSING"
    }
}
$adbPath = Join-Path $sdkRoot 'platform-tools\adb.exe'
if (Test-Path -LiteralPath $adbPath -PathType Leaf) {
    Check-Tool 'adb' $adbPath @('version') '^Android Debug Bridge version '
}
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (Test-Path -LiteralPath $vswhere) {
    $msvc = Invoke-VwProcess -FilePath $vswhere -ArgumentList @('-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationVersion') -WorkingDirectory $projectRoot -Phase 'doctor-msvc' -TimeoutSeconds 30 -Capture
    if ($msvc.ExitCode -eq 0 -and @($msvc.Lines | Where-Object { $_ }).Count -gt 0) {
        Write-Host "MSVC : PRESENT ($($msvc.Lines -join ' '))"
    } else { $script:missing.Add('MSVC'); Write-Host 'MSVC : MISSING' }
} else { $script:missing.Add('MSVC'); Write-Host 'MSVC : MISSING' }

Write-Host 'Doctor is read-only; it does not change security settings, drivers, firewall, or the phone.'
Write-Host 'Phone hardware acceptance and WDK driver tooling are separate Phase 0 checks.'
Write-Host "Missing or incompatible checks: $($script:missing.Count)"
if ($Strict -and $script:missing.Count -gt 0) { exit 1 }
exit 0
