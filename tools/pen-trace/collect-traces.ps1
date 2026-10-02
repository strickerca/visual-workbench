param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
$device = Get-VwAndroidDevice -Root $projectRoot
$destination = Join-Path $projectRoot ('fixtures-private/pen-traces-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $destination | Out-Null
$listing = Invoke-VwAdb -Device $device -Arguments @('shell', 'run-as', 'com.visualworkbench.penprobe', 'ls', 'files/traces') -Phase 'pen-list-traces' -TimeoutSeconds 15 -Capture
$names = @($listing.Lines | Where-Object { $_ -match '^[a-z0-9][a-z0-9-]{0,110}\.json$' })
if ($names.Count -eq 0 -or $names.Count -gt 200) { throw 'Expected 1..200 saved probe traces' }
foreach ($name in $names) {
    # App-private data is pulled only to ignored storage for owner/privacy review.
    $pull = Invoke-VwAdb -Device $device -Arguments @('exec-out', 'run-as', 'com.visualworkbench.penprobe', 'cat', "files/traces/$name") -Phase 'pen-pull-trace' -TimeoutSeconds 30 -Capture
    [IO.File]::WriteAllText((Join-Path $destination $name), ($pull.Lines -join "`n"), [Text.UTF8Encoding]::new($false))
}
$check = Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/pen-trace/trace_tool.py', 'validate', $destination) -WorkingDirectory $projectRoot -Phase 'pen-validate-collected' -TimeoutSeconds 30
if ($check.ExitCode -ne 0) { throw 'Collected trace validation failed; preserve private originals for diagnosis' }
Write-Host "Collected $($names.Count) traces privately: $destination. Review shapes and metadata before copying to fixtures/traces."
