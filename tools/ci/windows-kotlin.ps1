# Builds the desktop app's Kotlin code on Windows against the Windows natives
# and the Windows dependency lock, then runs its tests. The Linux Kotlin job
# checks the same code with the Linux dependency set; this is the build the
# owner's laptop actually uses. No device, GPU session or Paint is needed.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
Set-Location $root

# Hosted runners hand out TEMP in 8.3 short form. Tests that pin files by their
# canonical path need the long form (same fix as windows-rust.ps1).
foreach ($name in 'TEMP', 'TMP') {
    $value = [Environment]::GetEnvironmentVariable($name)
    if ($value -and (Test-Path -LiteralPath $value)) {
        [Environment]::SetEnvironmentVariable($name, (Get-Item -LiteralPath $value).FullName)
    }
}

# After a test run, list each failed test with the first line of its message,
# so a failure can be read from the end of the log.
function Show-FailedTests([string]$ResultsDirectory) {
    if (-not (Test-Path -LiteralPath $ResultsDirectory)) { return }
    $failed = 0
    foreach ($file in Get-ChildItem -LiteralPath $ResultsDirectory -Filter '*.xml') {
        $document = [xml](Get-Content -LiteralPath $file.FullName -Raw)
        foreach ($problem in $document.SelectNodes('//testcase/failure | //testcase/error')) {
            $failed++
            $case = $problem.ParentNode
            $message = ($problem.GetAttribute('message') -split "`n")[0]
            if ($message.Length -gt 200) { $message = $message.Substring(0, 200) }
            Write-Host "FAILED $($case.GetAttribute('classname')).$($case.GetAttribute('name')): $($problem.GetAttribute('type')) $message"
        }
    }
    Write-Host "failed tests: $failed"
}

function Step([string]$Name, [scriptblock]$Body) {
    Write-Host "`n=== $Name ==="
    & $Body
    if ($LASTEXITCODE -ne 0) { throw "$Name failed with exit code $LASTEXITCODE" }
}

# The same native set build.ps1 builds before any desktop Gradle task.
Step 'windows natives' {
    cargo build --locked -p vw-ffi -p vw-host-ffi -p vw-capture -p vw-remote-host --features 'vw-ffi/bindgen,vw-ffi/fixtures'
}
try {
    Step 'desktop compile and tests' {
        & (Join-Path $root 'apps\gradlew.bat') -p apps ':desktop:compileKotlin' ':desktop:compileTestKotlin' ':desktop:test' `
            '--console=plain' '--no-daemon' '--no-parallel' '--max-workers=2' '--continue'
    }
} finally {
    Write-Host "`n=== failed desktop tests ==="
    Show-FailedTests (Join-Path $root 'apps\desktop\build\test-results\test')
}
