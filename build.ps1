param(
    [Parameter(Position = 0)]
    [ValidateSet('build-core', 'build-ai', 'test-ai', 'build-android', 'build-stroke-core', 'build-stroke', 'build-pen-probe', 'build-pen-inject', 'build-transport', 'build-video-pc', 'build-video-android', 'build-image-android', 'build-vdd-probe', 'build-desktop', 'test-all', 'lint-all', 'license-check', 'hil-test', 'run-desktop', 'doctor')]
    [string]$Command = 'doctor',
    [Parameter(Position = 1)][ValidateSet('app', 'rust', 'stroke', 'pen', 'pen-owner', 'win-pen', 'transport', 'video-pc', 'video-android', 'video-tiles', 'image-pc', 'image-android', 'vdd')][string]$HilMode = 'app',
    [Parameter(Position = 2)][ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
    [switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$WinPenScenario = 'normal',
    [ValidatePattern('^[0-9a-f]{32}$')][string]$VideoRunId,
    [ValidateSet('inventory','normal','watchdog')][string]$VddScenario='inventory'
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
        'build-ai' {
            Run-LicenseGate
            Run-Cargo 'build-ai-spike' @('build', '--locked', '-p', 'vw-ai-spike')
        }
        'test-ai' {
            Run-Step 'test-ai-verifier' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/ai-spike/tests', '-p', 'test_*.py', '-v')
            Run-Cargo 'test-ai-spike' @('test', '--locked', '-p', 'vw-ai-spike')
            Run-Cargo 'lint-ai-spike' @('clippy', '--locked', '-p', 'vw-ai-spike', '--all-targets', '--', '-D', 'warnings')
            Run-Cargo 'format-ai-spike' @('fmt', '-p', 'vw-ai-spike', '--', '--check')
        }
        'build-core' { Run-LicenseGate; Run-Cargo 'build-core' @('build', '--workspace', '--locked') }
        'build-android' { Run-LicenseGate; Run-Gradle 'build-android' @(':android:assembleDebug') }
        'build-stroke-core' {
            Run-LicenseGate
            Run-Cargo 'build-stroke-windows' @('build', '--release', '--locked', '-p', 'vw-stroke-spike', '-p', 'vw-stroke-jni')
        }
        'build-stroke' {
            Run-LicenseGate
            Run-Cargo 'build-stroke-windows' @('build', '--release', '--locked', '-p', 'vw-stroke-spike', '-p', 'vw-stroke-jni')
            . (Join-Path $projectRoot 'tools/enter-dev.ps1')
            Run-Cargo 'build-stroke-android-native' @('ndk', '-t', 'arm64-v8a', '--platform', '29', 'build', '--release', '--locked', '-p', 'vw-stroke-spike', '-p', 'vw-stroke-jni')
            $nativeDirectory = Join-Path $projectRoot 'tools/stroke-spike/android/build/generated/jniLibs/arm64-v8a'
            New-Item -ItemType Directory -Force -Path $nativeDirectory | Out-Null
            Copy-Item -LiteralPath (Join-Path $projectRoot 'target/aarch64-linux-android/release/libvw_stroke_jni.so') -Destination $nativeDirectory
            Run-Gradle 'build-stroke-android-app' @(':stroke-spike:assembleDebug', ':stroke-spike:assembleDebugAndroidTest', ':stroke-spike:lintDebug')
            Run-Step 'bind-stroke-build' 'python.exe' @('tools/stroke-spike/build_receipt.py', 'record')
        }
        'build-pen-probe' { Run-LicenseGate; Run-Gradle 'build-pen-probe' @(':pen-probe:assembleDebug', ':pen-probe:assembleDebugAndroidTest') }
        'build-pen-inject' {
            Run-LicenseGate
            Run-Cargo 'build-pen-inject' @('build', '--locked', '-p', 'vw-pen-harness', '-p', 'vw-pen-inject')
            Run-Step 'bind-pen-inject-build' 'python.exe' @('tools/pen-inject/build_receipt.py', 'record')
        }
        'build-desktop' { Run-LicenseGate; Run-Gradle 'build-desktop' @(':desktop:packageUberJarForCurrentOS') }
        'build-video-pc' {
            Run-LicenseGate
            Run-Cargo 'build-video-pc' @('build', '--release', '--locked', '-p', 'vw-video-bench')
            Run-Step 'bind-video-pc-build' 'python.exe' @('tools/bench/video-pc/build_receipt.py', 'record', 'pc')
        }
        'build-vdd-probe' {
            Run-LicenseGate
            Run-Step 'vdd-source-binding' 'python.exe' @('drivers/sudovda/check_source.py')
            Run-Cargo 'build-vdd-probe' @('build', '--release', '--locked', '-p', 'vw-vdd-probe')
            Run-Step 'bind-vdd-probe-build' 'python.exe' @('tools/vdd-probe/build_receipt.py', 'record')
        }
        'build-video-android' {
            Run-LicenseGate
            Run-Gradle 'build-video-android' @(':video-bench:assembleDebug')
            Run-Step 'bind-video-android-build' 'python.exe' @('tools/bench/video-pc/build_receipt.py', 'record', 'android')
        }
        'build-image-android' {
            Run-LicenseGate
            Run-Gradle 'build-image-android' @(':image-bench:assembleDebug', ':image-bench:lintDebug')
            Run-Step 'bind-image-android-build' 'python.exe' @('tools/bench/image/build_receipt.py', 'record')
        }
        'build-transport' {
            Run-LicenseGate
            Run-Cargo 'build-transport-host' @('build', '--release', '--locked', '-p', 'vw-transport-bench')
            . (Join-Path $projectRoot 'tools/enter-dev.ps1')
            Run-Cargo 'build-transport-android' @('ndk', '-t', 'arm64-v8a', '--platform', '29', 'build', '--release', '--locked', '-p', 'vw-transport-bench')
            Run-Step 'bind-transport-build' 'python.exe' @('tools/bench/transport/build_receipt.py', 'record')
        }
        'run-desktop' { Run-LicenseGate; Run-Gradle 'run-desktop' @(':desktop:run') }
        'test-all' {
            Run-Step 'test-ai-verifier' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/ai-spike/tests', '-p', 'test_*.py', '-v')
            Run-Step 'test-tools' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/tests', '-v')
            Run-Step 'test-device-selection' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/tests/device-selection-fixtures.ps1')
            Run-Step 'test-store-crash-receipt' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/tests/store-crash-receipt-fixtures.ps1')
            Run-Cargo 'test-rust' @('test', '--workspace', '--locked')
            Run-Gradle 'test-kotlin' @(':shared:allTests', ':android:testDebugUnitTest', ':desktop:test', ':pen-probe:testDebugUnitTest')
        }
        'lint-all' {
            Run-Step 'lint-setup' 'python.exe' @('tools/check_setup.py', 'all')
            Run-Step 'lint-plan' 'python.exe' @('tools/check_plan_coverage.py')
            Run-Cargo 'lint-rust-format' @('fmt', '--all', '--', '--check')
            Run-Cargo 'lint-rust-clippy' @('clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings')
            Run-Step 'lint-secrets' 'gitleaks.exe' @('dir', '.', '--config', '.gitleaks.toml', '--redact=100', '--no-banner')
            Run-Gradle 'lint-kotlin' @(':android:lintDebug', ':shared:check', ':desktop:check', ':pen-probe:lintDebug', ':video-bench:lintDebug', ':image-bench:lintDebug')
        }
        'hil-test' {
            $arguments = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/hil-test.ps1', '-Mode', $HilMode, '-TimeoutSeconds', "$TimeoutSeconds")
            if ($HilMode -eq 'vdd') {
                $arguments += @('-VddScenario', $VddScenario)
                if ($OwnerReady) { $arguments += '-OwnerReady' }
            }
            if ($HilMode -in @('video-android', 'video-tiles')) {
                if (-not $VideoRunId) { throw 'Decoder HIL requires -VideoRunId from the native capture run' }
                $arguments += @('-VideoRunId', $VideoRunId)
            }
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
