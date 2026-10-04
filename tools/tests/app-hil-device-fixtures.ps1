Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot '../app-test/device.psm1') -Force
$checks = 0
function Check([bool]$Passed) { if (-not $Passed) { throw 'App HIL device-scope fixture failed.' }; $script:checks++ }
Check ((Get-VwAppHilProfileCount @('Users:', ' UserInfo{0:Owner:13} running')) -eq 1)
Check ((Get-VwAppHilProfileCount @('Users:', ' UserInfo{0:Owner:13} running', ' UserInfo{150:Private:1030}')) -eq 2)
foreach ($invalid in @(
    @('Users:'),
    @('Users:', ' UserInfo{150:Private:1030}'),
    @('Users:', ' UserInfo{0:Owner:13}', ' UserInfo{0:Other:13}'),
    @('permission denied', ' UserInfo{0:Owner:13}'),
    @('Users:', ' UserInfo{0:Owner:13}', 'unrecognized output'),
    @('Users:', ' UserInfo{00:Owner:13}')
)) {
    $refused = $false
    try { Get-VwAppHilProfileCount -Lines $invalid | Out-Null } catch { $refused = $true }
    Check $refused
}
$package = 'com.visualworkbench.android.hil'
Check (Test-VwAppHilPackageAbsent $package @('Unable to find package: com.visualworkbench.android.hil'))
Check (Test-VwAppHilPackageAbsent 'com.visualworkbench.android.hil.test' @('', ' Unable to find package: com.visualworkbench.android.hil.test '))
foreach ($invalid in @(
    @(''),
    @('Permission Denial'),
    @('Unable to find package: com.visualworkbench.android.hil.test'),
    @('Unable to find package: com.visualworkbench.android.hil', 'Package exists in another profile'),
    @('Packages:', ' Package [com.visualworkbench.android.hil] (abc)', ' User 150: installed=true')
)) { Check (-not (Test-VwAppHilPackageAbsent $package $invalid)) }
$refused = $false
try { Test-VwAppHilPackageAbsent 'another.app' @('Unable to find package: another.app') | Out-Null } catch { $refused = $true }
Check $refused
Write-Host "App HIL owner-profile/global-package fixtures: PASS ($checks checks); no device commands."
