param(
    [Parameter(Mandatory = $true, Position = 0)][string]$Executable,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$TestArguments = @()
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Import-Module (Join-Path $PSScriptRoot 'android-device.psm1') -Force
try {
    $device = Get-VwAndroidDevice -Root $projectRoot
    $arguments = @($TestArguments)
    if (-not ($arguments | Where-Object { $_ -match '^--test-threads(?:=|$)' })) { $arguments += '--test-threads=1' }
    $count = Invoke-VwAndroidRustExecutable -Device $device -Executable $Executable -Arguments $arguments -TimeoutSeconds 600
    Write-Host "Cargo Android target runner: PASS ($count actual tests); S23 Ultra/S Pen acceptance remains pending."
    exit 0
} catch {
    Write-Error $_
    exit 1
}
