param(
    [Parameter(Mandatory=$true)][string]$WindowTitleSubstring,
    [Parameter(Mandatory=$true)][string]$OutFile,
    [ValidateRange(1,60)][int]$TimeoutSeconds = 15
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $root 'tools/process.psm1')
$binary = Join-Path $root 'target/debug/diag-win.exe'
$result = Invoke-VwProcess -FilePath $binary -ArgumentList @('--screenshot',$WindowTitleSubstring,[IO.Path]::GetFullPath($OutFile)) -WorkingDirectory $root -Phase selected-window-capture -TimeoutSeconds $TimeoutSeconds
exit $result.ExitCode
