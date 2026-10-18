# Orchestration source only in this batch. This script assembles existing files.
# Root owns the separate, reviewed Gradle/MSI build and runtime staging hooks.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$Manifest,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{64}$')][string]$ManifestSha256,
    [Parameter(Mandatory)][string]$Destination,
    [Parameter(Mandatory)][string]$PythonExecutable,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{64}$')][string]$PythonSha256,
    [ValidateRange(1, 3600)][int]$TimeoutSeconds = 600
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Windows assembly orchestration requires Windows.' }

function Get-OrdinaryAbsolutePath([string]$Value) {
    if ($Value -cnotmatch '^[A-Za-z]:\\' -or $Value.Substring(2).Contains(':') -or
        $Value.Contains([char]0) -or $Value -match '[\\/]\.\.?([\\/]|$)') {
        throw 'An explicit ordinary absolute local path is required.'
    }
    return [IO.Path]::GetFullPath($Value)
}

$Root = Get-OrdinaryAbsolutePath $Root
$Destination = Get-OrdinaryAbsolutePath $Destination
$PythonExecutable = Get-OrdinaryAbsolutePath $PythonExecutable
if ($Manifest -notmatch '^[ -~]{1,512}$' -or $Manifest -match '[\\:<>"|?*]' -or
    $Manifest.StartsWith('/') -or @($Manifest.Split('/') | Where-Object {
        $_ -in @('', '.', '..') -or $_.EndsWith(' ') -or $_.EndsWith('.')
    }).Count -gt 0) { throw 'An exact safe root-relative manifest path is required.' }

# Use the already admitted desktop lease implementation. No interpreter search,
# shell command string, download, Gradle invocation or package execution occurs.
$scriptRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if (-not [StringComparer]::OrdinalIgnoreCase.Equals($scriptRoot.TrimEnd('\'), $Root.TrimEnd('\'))) {
    throw 'Assembly root must be the checkout containing this reviewed script.'
}
Import-Module (Join-Path $Root 'tools\desktop-test\lease.psm1') -Force
Import-Module (Join-Path $Root 'tools\process.psm1') -Force
$lease = [VwDesktopLease]::new()
$pinnedDirectories = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
try {
    $files = @($PythonExecutable, (Join-Path $Root 'tools\release\assemble.py'),
        (Join-Path $Root 'tools\desktop-test\reports.py'),
        (Join-Path $Root 'tools\desktop-test\personal_package.py'),
        (Join-Path $Root 'tools\ffi-test\check_apk.py'))
    foreach ($file in $files) {
        $chain = [Collections.Generic.List[string]]::new()
        $cursor = [IO.FileInfo]::new($file).Directory
        while ($null -ne $cursor -and -not $pinnedDirectories.Contains($cursor.FullName)) {
            $chain.Add($cursor.FullName)
            $cursor = $cursor.Parent
        }
        for ($index = $chain.Count - 1; $index -ge 0; $index--) {
            $lease.Pin($chain[$index], $true)
            [void]$pinnedDirectories.Add($chain[$index])
        }
        $lease.Pin($file, $false)
    }
    $pythonHash = (Get-FileHash -LiteralPath $PythonExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($pythonHash -cne $PythonSha256) { throw 'Explicit Python interpreter hash mismatch.' }
    $result = Invoke-VwProcess -FilePath $PythonExecutable -ArgumentList @(
        '-I', '-B', (Join-Path $Root 'tools\release\assemble.py'),
        '--root', $Root, '--manifest', $Manifest, '--manifest-sha256', $ManifestSha256,
        '--destination', $Destination
    ) -WorkingDirectory $Root -Phase 'release-assembly' -TimeoutSeconds $TimeoutSeconds `
        -RedactValues @($Root, $Destination, $PythonExecutable, $env:USERPROFILE)
    if ($result.ExitCode -ne 0) { throw 'Release assembly refused; retain the bounded process receipt.' }
} finally {
    $lease.Dispose()
}
