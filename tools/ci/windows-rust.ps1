# Compiles, lints and tests the Rust workspace on Windows. This is the only
# place the Windows-only code (capture, input injection, UI Automation) is
# built outside the owner's laptop. No device, GPU session or Paint is needed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $root

# Hosted runners hand out TEMP in 8.3 short form (a RUNNER~1 path component). The
# path-pinning tests compare against the canonical long path, so give them one.
foreach ($name in 'TEMP', 'TMP') {
    $value = [Environment]::GetEnvironmentVariable($name)
    if ($value -and (Test-Path -LiteralPath $value)) {
        [Environment]::SetEnvironmentVariable($name, (Get-Item -LiteralPath $value).FullName)
    }
}
Write-Host "TEMP=$env:TEMP"

function Step([string]$Name, [scriptblock]$Body) {
    Write-Host "`n=== $Name ==="
    & $Body
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE" }
}

Step 'clippy' { cargo clippy --workspace --all-targets --locked -- -D warnings }
Step 'workspace tests' { cargo test --workspace --exclude vw-sim --locked -- --test-threads=2 }
foreach ($fixture in 'device-selection-fixtures.ps1', 'app-hil-device-fixtures.ps1',
                     'android-normal-delivery-fixtures.ps1', 'store-crash-receipt-fixtures.ps1') {
    Step "fixture $fixture" {
        powershell.exe -NoProfile -ExecutionPolicy Bypass -File (Join-Path $root "tools\tests\$fixture")
    }
}
