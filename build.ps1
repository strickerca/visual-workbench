param(
    [Parameter(Position = 0)]
    [ValidateSet('build-core', 'build-android', 'build-desktop', 'test-all', 'lint-all', 'license-check', 'hil-test', 'run-desktop', 'doctor')]
    [string]$Command = 'doctor',
    [Parameter(Position = 1)][ValidateSet('app', 'rust')][string]$HilMode = 'app',
    [Parameter(Position = 2)][ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
Import-Module (Join-Path $projectRoot 'tools\process.psm1') -Force

function Run-Step {
    param([string]$Phase, [string]$Executable, [string[]]$Arguments, [string]$Directory = $projectRoot, [int]$Limit = $TimeoutSeconds, [int]$ExitGraceSeconds = 5)
    $result = Invoke-VwProcess -FilePath $Executable -ArgumentList $Arguments -WorkingDirectory $Directory -Phase $Phase -TimeoutSeconds $Limit -ParentExitGraceSeconds $ExitGraceSeconds -RedactValues @($projectRoot, $env:USERPROFILE)
    if ($result.ExitCode -ne 0) { throw "$Phase failed (exit $($result.ExitCode)); see its retained external text log" }
}

function Run-Cargo {
    param([string]$Phase, [string[]]$Arguments)
    $result = Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList (@('+1.99.0') + $Arguments) -WorkingDirectory $projectRoot -Phase $Phase -TimeoutSeconds $TimeoutSeconds -CleanCompilerTelemetry
    if ($result.ExitCode -ne 0) { throw "$Phase failed (exit $($result.ExitCode)); see its retained external text log" }
}

function Run-Gradle {
    param([string]$Phase, [string[]]$Tasks)
    $gradleArguments = $Tasks + @('--console=plain', '--no-daemon', '--no-configuration-cache', '--no-build-cache', '--no-parallel', '--rerun-tasks')
    Run-Step $Phase (Join-Path $projectRoot 'apps\gradlew.bat') $gradleArguments (Join-Path $projectRoot 'apps') -ExitGraceSeconds 20
}

function Run-LicenseGate {
    Run-Step 'license-offline-policy' 'python.exe' @('tools/check_setup.py', 'all')
    $version = Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList @('+1.99.0', 'deny', '--version') -WorkingDirectory $projectRoot -Phase 'license-cargo-deny-version' -TimeoutSeconds 30 -Capture
    if ($version.ExitCode -ne 0 -or ($version.Lines -join "`n") -notmatch '\b0\.20\.2\b') { throw 'cargo-deny must match the pinned version 0.20.2' }
    Run-Cargo 'license-rust' @('deny', 'check', 'licenses', 'sources', 'bans')
    Run-Gradle 'license-gradle' @('checkDependencyLicenses')
}

try {
    switch ($Command) {
        'doctor' { Run-Step 'doctor' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/doctor.ps1', '-Strict') }
        'license-check' { Run-LicenseGate }
        'build-core' { Run-LicenseGate; Run-Cargo 'build-core' @('build', '--workspace', '--locked') }
        'build-android' { Run-LicenseGate; Run-Gradle 'build-android' @(':android:assembleDebug') }
        'build-desktop' { Run-LicenseGate; Run-Gradle 'build-desktop' @(':desktop:packageUberJarForCurrentOS') }
        'run-desktop' { Run-LicenseGate; Run-Gradle 'run-desktop' @(':desktop:run') }
        'test-all' {
            Run-Step 'test-tools' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/tests', '-v')
            Run-Cargo 'test-rust' @('test', '--workspace', '--locked')
            Run-Gradle 'test-kotlin' @(':shared:allTests', ':android:testDebugUnitTest', ':desktop:test')
        }
        'lint-all' {
            Run-Step 'lint-setup' 'python.exe' @('tools/check_setup.py', 'all')
            Run-Step 'lint-plan' 'python.exe' @('tools/check_plan_coverage.py')
            Run-Cargo 'lint-rust-format' @('fmt', '--all', '--', '--check')
            Run-Cargo 'lint-rust-clippy' @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
            Run-Step 'lint-secrets' 'gitleaks.exe' @('dir', '.', '--config', '.gitleaks.toml', '--redact=100', '--no-banner')
            Run-Gradle 'lint-kotlin' @(':android:lintDebug', ':shared:check', ':desktop:check')
        }
        'hil-test' {
            $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/hil-test.ps1', '-Mode', $HilMode, '-TimeoutSeconds', "$TimeoutSeconds")
            if ($HilMode -eq 'rust') {
                if (-not $Crate) { throw 'Usage: build.ps1 hil-test rust <crate>' }
                $arguments += @('-Crate', $Crate)
            }
            Run-Step 'hil-test' 'powershell.exe' $arguments -Limit ($TimeoutSeconds + 600)
        }
    }
    exit 0
} catch {
    Write-Error $_
    exit 1
}
