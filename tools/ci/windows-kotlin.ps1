# Builds the desktop app's Kotlin code on Windows against the Windows natives
# and the Windows dependency lock, then runs its tests. The Linux Kotlin job
# checks the same code with the Linux dependency set; this is the build the
# owner's laptop actually uses. No device, GPU session or Paint is needed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $root

function Step([string]$Name, [scriptblock]$Body) {
    Write-Host "`n=== $Name ==="
    & $Body
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE" }
}

# The same native set build.ps1 builds before any desktop Gradle task.
Step 'windows natives' {
    cargo build --locked -p vw-ffi -p vw-host-ffi -p vw-capture -p vw-remote-host --features 'vw-ffi/bindgen,vw-ffi/fixtures'
}
Step 'desktop compile and tests' {
    & (Join-Path $root 'apps\gradlew.bat') -p apps ':desktop:compileKotlin' ':desktop:compileTestKotlin' ':desktop:test' `
        '--console=plain' '--no-daemon' '--no-parallel' '--max-workers=2' '--continue'
}
