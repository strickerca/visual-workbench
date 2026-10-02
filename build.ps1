param(
    [Parameter(Position = 0)]
    [ValidateSet('build-core', 'build-android', 'build-pen-probe', 'build-pen-inject', 'build-desktop', 'test-all', 'lint-all', 'license-check', 'hil-test', 'run-desktop', 'doctor')]
    [string]$Command = 'doctor',
    [Parameter(Position = 1)][ValidateSet('app', 'rust', 'pen', 'pen-owner', 'win-pen')][string]$HilMode = 'app',
    [Parameter(Position = 2)][ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
    [switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$WinPenScenario = 'normal'
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
        'build-pen-probe' { Run-LicenseGate; Run-Gradle 'build-pen-probe' @(':pen-probe:assembleDebug', ':pen-probe:assembleDebugAndroidTest') }
        'build-pen-inject' {
            Run-LicenseGate
            Run-Cargo 'build-pen-inject' @('build', '--locked', '-p', 'vw-pen-harness', '-p', 'vw-pen-inject')
            Run-Step 'bind-pen-inject-build' 'python.exe' @('tools/pen-inject/build_receipt.py', 'record')
        }
        'build-desktop' { Run-LicenseGate; Run-Gradle 'build-desktop' @(':desktop:packageUberJarForCurrentOS') }
        'run-desktop' { Run-LicenseGate; Run-Gradle 'run-desktop' @(':desktop:run') }
        'test-all' {
            Run-Step 'test-tools' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/tests', '-v')
            Run-Cargo 'test-rust' @('test', '--workspace', '--locked')
            Run-Gradle 'test-kotlin' @(':shared:allTests', ':android:testDebugUnitTest', ':desktop:test', ':pen-probe:testDebugUnitTest')
        }
        'lint-all' {
            Run-Step 'lint-setup' 'python.exe' @('tools/check_setup.py', 'all')
            Run-Step 'lint-plan' 'python.exe' @('tools/check_plan_coverage.py')
            Run-Cargo 'lint-rust-format' @('fmt', '--all', '--', '--check')
            Run-Cargo 'lint-rust-clippy' @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
            Run-Step 'lint-secrets' 'gitleaks.exe' @('dir', '.', '--config', '.gitleaks.toml', '--redact=100', '--no-banner')
            Run-Gradle 'lint-kotlin' @(':android:lintDebug', ':shared:check', ':desktop:check', ':pen-probe:lintDebug')
        }
        'hil-test' {
            $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/hil-test.ps1', '-Mode', $HilMode, '-TimeoutSeconds', "$TimeoutSeconds")
            if ($HilMode -eq 'win-pen') {
                if (-not $OwnerReady) { throw 'Windows input HIL requires -OwnerReady after the owner reserves the desktop for testing' }
                $arguments += @('-OwnerReady', '-WinPenScenario', $WinPenScenario)
            }
            if ($HilMode -eq 'rust') {
                if (-not $Crate) { throw 'Usage: build.ps1 hil-test rust <crate>' }
                $arguments += @('-Crate', $Crate)
            }
            # Leave the child enough time to run its bounded build, replay and
            # device cleanup. An outer deadline must not preempt its finally block.
            $hilBudget = if ($HilMode -in @('pen', 'pen-owner', 'win-pen')) { $TimeoutSeconds * 6 + 600 } else { $TimeoutSeconds + 600 }
            Run-Step 'hil-test' 'powershell.exe' $arguments -Limit $hilBudget
        }
    }
    exit 0
} catch {
    Write-Error $_
    exit 1
}
