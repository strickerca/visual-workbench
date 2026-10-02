param(
    [Parameter(Mandatory = $true)][string]$FilePath,
    [string[]]$ArgumentList = @(),
    [string]$WorkingDirectory,
    [string]$Phase = 'command',
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600
)
$ErrorActionPreference = 'Stop'
if (-not $WorkingDirectory) { $WorkingDirectory = Split-Path -Parent $PSScriptRoot }
Import-Module (Join-Path $PSScriptRoot 'process.psm1') -Force
try {
    $result = Invoke-VwProcess -FilePath $FilePath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory -Phase $Phase -TimeoutSeconds $TimeoutSeconds
    exit $result.ExitCode
} catch {
    Write-Error $_
    exit 1
}
