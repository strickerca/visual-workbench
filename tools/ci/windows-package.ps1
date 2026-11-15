# Builds the Windows desktop app as a ready-to-run folder (an "app image": the
# program, its own Java runtime, the native libraries and the local agent
# server) on a clean build machine. No installer and no WiX are involved; the
# folder is what build.ps1 build-desktop-distribution produces on the laptop,
# with one difference: the core libraries are built optimised.
#
# Nothing here runs the app. A package from this script has been built, not
# tried: the first start on a real PC is still the owner's check.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
Set-Location $projectRoot
$TimeoutSeconds = 3600

# Hosted runners hand out TEMP in 8.3 short form (same fix as windows-rust.ps1).
foreach ($name in 'TEMP', 'TMP') {
    $value = [Environment]::GetEnvironmentVariable($name)
    if ($value -and (Test-Path -LiteralPath $value)) {
        [Environment]::SetEnvironmentVariable($name, (Get-Item -LiteralPath $value).FullName)
    }
}

# tools/mcp-build.ps1 is written against build.ps1's Run-Step and Run-Cargo.
# These are the plain equivalents: run, show the output, stop on failure.
# Output goes to the host so that it never becomes a function's return value.
function Run-Step {
    param([string]$Phase, [string]$Executable, [string[]]$Arguments, [string]$Directory = $projectRoot)
    Write-Host "`n=== $Phase ==="
    Push-Location -LiteralPath $Directory
    try {
        & $Executable @Arguments | Out-Host
        if ($LASTEXITCODE -ne 0) { throw "$Phase failed with exit code $LASTEXITCODE" }
    } finally { Pop-Location }
}
function Run-Cargo {
    param([string]$Phase, [string[]]$Arguments)
    Run-Step $Phase 'cargo' $Arguments
}
. (Join-Path $projectRoot 'tools/mcp-build.ps1')

# 1. The pinned Node runtime that ships inside the app, checked against the
#    hash recorded in tools/licenses/node-runtime.json before it is unpacked.
$pin = Get-Content -LiteralPath (Join-Path $projectRoot 'tools/licenses/node-runtime.json') -Raw | ConvertFrom-Json
$nodeName = "node-v$($pin.version)-win-x64"
$toolchains = Join-Path $projectRoot '.local/toolchains'
$nodeDirectory = Join-Path $toolchains $nodeName
if (-not (Test-Path -LiteralPath (Join-Path $nodeDirectory 'node.exe'))) {
    Write-Host "`n=== fetch $nodeName ==="
    [void][IO.Directory]::CreateDirectory($toolchains)
    $archive = Join-Path $toolchains "$nodeName.zip"
    Invoke-WebRequest -Uri "https://nodejs.org/dist/v$($pin.version)/$nodeName.zip" -OutFile $archive
    $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $pin.archive_sha256) { throw "Node archive hash differs from the pinned one ($actual)" }
    Expand-Archive -LiteralPath $archive -DestinationPath $toolchains
    Remove-Item -LiteralPath $archive
}

# 2. The laptop installs the agent server's packages from a local cache with
#    the network off. A clean machine has no cache, so fill it once from the
#    lock file (every package is checked against the hash in the lock file).
$prime = Join-Path $projectRoot '.local/npm-prime'
if (Test-Path -LiteralPath $prime) { Remove-Item -LiteralPath $prime -Recurse -Force }
[void][IO.Directory]::CreateDirectory($prime)
foreach ($name in 'package.json', 'package-lock.json') {
    Copy-Item -LiteralPath (Join-Path $projectRoot "mcp/$name") -Destination (Join-Path $prime $name)
}
Run-Step 'fill npm cache' (Join-Path $nodeDirectory 'node.exe') @(
    (Join-Path $nodeDirectory 'node_modules/npm/bin/npm-cli.js'), 'ci', '--ignore-scripts', '--no-audit', '--no-fund',
    '--cache', (Join-Path $projectRoot '.local/npm-cache')) $prime
Remove-Item -LiteralPath $prime -Recurse -Force

# 3. Native libraries and helper programs, optimised.
Run-Cargo 'windows natives (optimised)' @('build', '--release', '--locked', '-p', 'vw-ffi', '-p', 'vw-host-ffi',
    '-p', 'vw-capture', '-p', 'vw-remote-host', '--features', 'vw-ffi/bindgen,vw-ffi/fixtures')

# 4. The local agent server, staged exactly as on the laptop.
$nonce = [guid]::NewGuid().ToString('N')
$mcpResources = Build-McpServerResources $nonce

# 5. The app image, then the agent bridge that is bound to that exact image.
Run-Step 'desktop app image' (Join-Path $projectRoot 'apps\gradlew.bat') @(
    '-p', 'apps', ':desktop:createDistributable',
    ('-PvwNativeDir=' + (Join-Path $projectRoot 'target\release')),
    ('-PvwMcpServerResources=' + $mcpResources),
    ('-PvwDesktopBuildId=' + $nonce),
    '--console=plain', '--no-daemon', '--no-parallel', '--max-workers=2')
Complete-McpApplicationImage $nonce

# 6. What was built.
$image = Join-Path $projectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev'
$files = @(Get-ChildItem -LiteralPath $image -Recurse -File)
$megabytes = [math]::Round((($files | Measure-Object -Property Length -Sum).Sum) / 1MB)
Write-Host "`n=== result ==="
Write-Host "desktop app image: $($files.Count) files, $megabytes MB"
foreach ($name in 'VisualWorkbenchDev.exe', 'vw-mcp.exe') {
    $path = Join-Path $image $name
    if (-not (Test-Path -LiteralPath $path)) { throw "The app image has no $name" }
    Write-Host "$name sha256 $((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())"
}
