param(
    [Parameter(Position = 0)]
    [ValidateSet('build-remote-helpers', 'test-remote-helpers', 'build-remote-integration', 'build-core', 'build-raster', 'test-raster', 'build-network', 'test-network', 'build-pairing', 'build-ffi', 'test-ffi', 'test-editor', 'lint-editor', 'test-shared', 'test-apps', 'test-mcp', 'build-ai', 'test-ai', 'build-android', 'build-stroke-core', 'build-stroke', 'build-pen-probe', 'build-pen-inject', 'build-transport', 'build-video-pc', 'build-video-android', 'build-image-android', 'build-vdd-probe', 'build-desktop', 'build-desktop-distribution', 'test-all', 'lint-all', 'license-check', 'hil-test', 'run-desktop', 'doctor')]
    [string]$Command = 'doctor',
    [Parameter(Position = 1)][ValidateSet('app', 'rust', 'stroke', 'pen', 'pen-owner', 'win-pen', 'transport', 'pairing', 'shared-ffi', 'video-pc', 'video-android', 'video-tiles', 'image-pc', 'image-android', 'vdd')][string]$HilMode = 'app',
    [Parameter(Position = 2)][ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
    [switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$WinPenScenario = 'normal',
    [ValidatePattern('^[0-9a-f]{32}$')][string]$VideoRunId,
    [ValidateSet('inventory','normal','watchdog')][string]$VddScenario='inventory',
    [string]$RemoteEditorCatalog,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$RemoteEditorCatalogSha256
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
Import-Module (Join-Path $projectRoot 'tools\process.psm1') -Force
. (Join-Path $projectRoot 'tools/mcp-build.ps1')
. (Join-Path $projectRoot 'tools/ffi-test/core-unit.ps1')

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

# Build-owner asset admission only. No product runtime path override exists.
function Get-RemoteEditorCatalogArguments {
    param([string]$Path,[string]$Digest,[string]$ProjectRoot)
    if ([bool]$Path -ne [bool]$Digest) { throw 'Root-admitted editor catalog path and SHA256 must be supplied together' }
    if (-not $Path) { return @() }
    if ($Digest -cnotmatch '^[0-9a-f]{64}$') { throw 'Invalid root-admitted editor catalog SHA256' }
    if ($Path.IndexOf([char]0) -ge 0 -or ([IO.Path]::IsPathRooted($Path) -and -not [IO.Path]::IsPathFullyQualified($Path))) { throw 'Ambiguous editor catalog path' }
    $absolute=[IO.Path]::GetFullPath($Path,$ProjectRoot)
    if ($absolute -cnotmatch '^[A-Za-z]:\\' -or $absolute.Substring(2).Contains(':')) { throw 'Editor catalog requires an ordinary absolute local path' }
    return @(('-PvwRemoteEditorCatalog=' + $absolute), ('-PvwRemoteEditorCatalogSha256=' + $Digest))
}

function Run-Gradle {
    param([string]$Phase, [string[]]$Tasks)
    $catalogArguments = @(Get-RemoteEditorCatalogArguments $RemoteEditorCatalog $RemoteEditorCatalogSha256 $projectRoot)
    $gradleArguments = $Tasks + $catalogArguments + @('--console=plain', '--no-daemon', '--no-configuration-cache', '--no-build-cache', '--no-parallel', '--rerun-tasks')
    Run-Step $Phase (Join-Path $projectRoot 'apps\gradlew.bat') $gradleArguments (Join-Path $projectRoot 'apps') -ExitGraceSeconds 20
}

function Run-LicenseGate {
    Run-Step 'license-offline-policy' 'python.exe' @('tools/check_setup.py', 'all')
    Run-Step 'license-npm-policy' 'python.exe' @('tools/check_npm_licenses.py')
    $version = Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList @('+1.99.0', 'deny', '--version') -WorkingDirectory $projectRoot -Phase 'license-cargo-deny-version' -TimeoutSeconds 30 -Capture
    if ($version.ExitCode -ne 0 -or ($version.Lines -join "`n") -notmatch '\b0\.20\.2\b') { throw 'cargo-deny must match the pinned version 0.20.2' }
    Run-Cargo 'license-rust' @('deny', 'check', 'licenses', 'sources', 'bans')
    Run-Gradle 'license-gradle' @('checkDependencyLicenses')
}

function Build-NativeWindows {
    Run-Cargo 'build-native-windows' @('build', '--locked', '-p', 'vw-ffi', '-p', 'vw-host-ffi', '-p', 'vw-capture', '-p', 'vw-remote-host', '--features', 'vw-ffi/bindgen,vw-ffi/fixtures')
}

function Build-NativeAndroid {
    . (Join-Path $projectRoot 'tools/enter-dev.ps1')
    Run-Cargo 'build-native-android' @('ndk', '-t', 'arm64-v8a', '--platform', '29', '-o', 'target/android-jni', 'build', '--locked', '-p', 'vw-ffi')
}

try {
    switch ($Command) {
        'doctor' { Run-Step 'doctor' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/doctor.ps1', '-Strict') }
        'license-check' { Run-LicenseGate }
        { $_ -in @('test-editor','lint-editor') } {
            Run-LicenseGate
            $editorPackages = @('-p','vw-mask','-p','vw-ai','-p','vw-ai-provider','-p','vw-instructions','-p','vw-semantics','-p','vw-package','-p','vw-ink','-p','vw-ops','-p','vw-ffi','-p','vw-ai-platform','-p','vw-raster','-p','vw-net','-p','vw-capture','-p','vw-codec-os','-p','vw-host-ffi','-p','vw-mcp-native')
            if ($Command -eq 'test-editor') { Run-Cargo 'test-editor-native' (@('test','--locked','--no-fail-fast') + $editorPackages + @('--features','vw-ffi/fixtures','--','--test-threads=2')) }
            Run-Cargo 'lint-editor-native' (@('clippy','--locked','--keep-going') + $editorPackages + @('--all-targets','--features','vw-ffi/fixtures','--','-D','warnings'))
            Run-Cargo 'format-editor-native' (@('fmt') + $editorPackages + @('--','--check'))
        }
        'build-remote-helpers' {
            Run-LicenseGate
            Run-Cargo 'build-remote-helpers' @('build','--locked','-p','vw-remote','-p','vw-remote-host','--all-targets')
        }
        'test-remote-helpers' {
            Run-LicenseGate
            Run-Cargo 'test-remote-helpers' @('test','--locked','-p','vw-remote','-p','vw-remote-host','-p','vw-capture','--all-targets','--','--test-threads=2')
            Run-Cargo 'lint-remote-helpers' @('clippy','--locked','-p','vw-remote','-p','vw-remote-host','-p','vw-capture','--all-targets','--','-D','warnings')
            Run-Cargo 'format-remote-helpers' @('fmt','-p','vw-remote','-p','vw-remote-host','-p','vw-capture','--','--check')
        }
        'build-raster' {
            Run-LicenseGate
            Run-Cargo 'build-raster' @('build', '--locked', '-p', 'vw-raster', '--all-targets')
        }
        'test-raster' {
            Run-Cargo 'test-raster' @('test', '--locked', '-p', 'vw-raster')
            Run-Cargo 'lint-raster' @('clippy', '--locked', '-p', 'vw-raster', '--all-targets', '--', '-D', 'warnings')
            Run-Cargo 'format-raster' @('fmt', '-p', 'vw-raster', '--', '--check')
        }
        'build-network' {
            Run-LicenseGate
            Run-Cargo 'build-network' @('build', '--locked', '-p', 'vw-net', '-p', 'vw-sim', '-p', 'vw-host-win', '-p', 'vw-pair-cli', '--features', 'vw-net/mdns', '--all-targets')
        }
        'test-network' {
            Run-Cargo 'test-network-core' @('test', '--locked', '-p', 'vw-net', '-p', 'vw-host-win', '--features', 'vw-net/mdns', '--', '--nocapture')
            Run-Cargo 'test-recovered-outbox' @('test', '--locked', '-p', 'vw-ops', '--test', 'recovered_outbox')
            Run-Cargo 'test-sync-storage' @('test', '--locked', '-p', 'vw-store', '--test', 'sync_recovery', '--test', 'authenticated_checkpoint', '--', '--nocapture')
            # Keep optimized compilation separate from the 10-minute simulation
            # budget. Debug validation of 20 seeds exceeds that projected target.
            Run-Cargo 'build-network-simulation' @('test', '--no-run', '--release', '--locked', '-p', 'vw-sim', '--lib')
            Run-Cargo 'test-network-simulation' @('test', '--release', '--locked', '-p', 'vw-sim', '--lib', '--', '--nocapture')
            Run-Cargo 'lint-network' @('clippy', '--locked', '-p', 'vw-net', '-p', 'vw-sim', '-p', 'vw-host-win', '-p', 'vw-pair-cli', '--features', 'vw-net/mdns', '--all-targets', '--', '-D', 'warnings')
            Run-Cargo 'format-network' @('fmt', '-p', 'vw-proto', '-p', 'vw-ops', '-p', 'vw-store', '-p', 'vw-net', '-p', 'vw-sim', '-p', 'vw-host-win', '-p', 'vw-pair-cli', '--', '--check')
        }
        'build-pairing' {
            Run-LicenseGate
            Run-Step 'test-pairing-build-binding' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/pair-cli/tests', '-v')
            Run-Step 'snapshot-pairing-source' 'python.exe' @('tools/pair-cli/build_receipt.py', 'begin')
            Run-Cargo 'build-pairing-windows' @('build', '--locked', '-p', 'vw-pair-cli')
            . (Join-Path $projectRoot 'tools/enter-dev.ps1')
            Run-Cargo 'build-pairing-android' @('ndk', '-t', 'arm64-v8a', '--platform', '29', 'build', '--locked', '-p', 'vw-pair-cli')
            Run-Step 'bind-pairing-build' 'python.exe' @('tools/pair-cli/build_receipt.py', 'record')
        }
        'build-remote-integration' {
            # Root alone runs this serialized opt-in build. NativeWindows builds
            # both DLLs, connection/capture/HEVC/input helpers and owned harness.
            Run-LicenseGate
            Build-NativeWindows
            Build-NativeAndroid
            Run-Step 'remote-integration-ffi-golden' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/ffi-test/generate-golden.ps1')
            Run-Gradle 'remote-integration-shared-bindings' @(':shared:generateCoreBindings', ':shared:generateHostBindings', ':shared:checkCommonMainImports', ':shared:checkImportGuardFixtures', ':shared:checkAndroidNativeAlignment')
            Run-Gradle 'remote-integration-artifacts' @(':desktop:remoteIntegrationClasspath', ':android:assembleDebug', ':android:assembleDebugAndroidTest', '-PvwAndroidHil=true', '-PvwRemoteIntegration=true', ('-PvwNativeDir=' + (Join-Path $projectRoot 'target/debug')))
            Run-Step 'remote-integration-native-packaging' 'python.exe' @('tools/ffi-test/check_apk.py', 'apps/android/build-remote-integration/outputs/apk/debug/android-debug.apk')
            Run-Step 'remote-integration-entrypoint-preflight' 'python.exe' @('tools/remote-edit/android_entrypoint_preflight.py', '--apk-test', 'apps/android/build-remote-integration/outputs/apk/androidTest/debug/android-debug-androidTest.apk', '--contract', 'tools/remote-edit/remote_instrumentation_contract.json', '--selection', 'com.visualworkbench.android.remote.RemoteNormalPathInstrumentedTest')
            # Inventory, explicit route selection and device execution are
            # separate owner-run steps; this build never installs or launches.
        }
        'build-ffi' {
            Run-LicenseGate
            Build-NativeWindows
            Build-NativeAndroid
            Run-Step 'prepare-ffi-golden' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/ffi-test/generate-golden.ps1')
            Run-Gradle 'build-shared-bindings' @(':shared:generateCoreBindings', ':shared:generateHostBindings', ':shared:checkCommonMainImports', ':shared:checkImportGuardFixtures', ':shared:checkAndroidNativeAlignment')
        }
        'test-ffi' {
            # A final-target link argument preserves other packages/bins and Android.
            Run-Step 'test-ffi-core-census' 'pwsh.exe' @('-NoProfile', '-File', 'tools/ffi-test/test_core_unit_census.ps1')
            Invoke-VwCoreUnitTest -ProjectRoot $projectRoot -Limit $TimeoutSeconds
            Run-Cargo 'test-ffi-integration' @('test', '--locked', '-p', 'vw-ffi', '--test', '*', '--', '--test-threads=2')
            Run-Cargo 'test-ffi-host' @('test', '--locked', '-p', 'vw-host-ffi', '--all-targets', '--', '--test-threads=2')
            Run-Cargo 'lint-ffi-native' @('clippy', '--locked', '-p', 'vw-ffi', '-p', 'vw-host-ffi', '--all-targets', '--features', 'vw-ffi/bindgen,vw-ffi/fixtures', '--', '-D', 'warnings')
            Run-Cargo 'format-ffi-native' @('fmt', '-p', 'vw-ffi', '-p', 'vw-host-ffi', '--', '--check')
        }
        'test-shared' {
            Run-Gradle 'test-shared-kotlin' @(':shared:desktopTest')
        }
        'test-mcp' { Run-LicenseGate; Test-McpRuntime }
        'test-apps' {
            Run-Gradle 'test-apps-kotlin' @(':shared:desktopTest', ':desktop:test', ':android:testDebugUnitTest', '--continue')
        }
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
        'build-android' {
            Run-LicenseGate
            Build-NativeWindows
            Build-NativeAndroid
            Run-Step 'test-apk-verifier' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/ffi-test', '-p', 'test_apk.py', '-v')
            Run-Gradle 'build-android' @(':android:assembleDebug', ':android:assembleDebugAndroidTest')
            Run-Step 'verify-android-native-packaging' 'python.exe' @('tools/ffi-test/check_apk.py', 'apps/android/build/outputs/apk/debug/android-debug.apk')
        }
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
        'build-desktop' { Run-LicenseGate; Build-NativeWindows; Run-Gradle 'build-desktop' @(':desktop:packageUberJarForCurrentOS') }
        'build-desktop-distribution' {
            # The fresh nonce invalidates any prior receipt even when a new
            # build later fails. Source is checked again after all compilation.
            Run-Step 'desktop-build-begin' 'python.exe' @('tools/desktop-test/reports.py','begin','--root',$projectRoot)
            $desktopBuild = Get-Content -LiteralPath (Join-Path $projectRoot '.local/desktop-test-build-start.json') -Raw | ConvertFrom-Json
            if ($desktopBuild.nonce -cnotmatch '^[0-9a-f]{32}$') { throw 'Desktop build receipt refused' }
            Run-LicenseGate
            Build-NativeWindows
            $mcpResources = Build-McpServerResources $desktopBuild.nonce
            Run-Gradle 'build-desktop-distribution' @(':desktop:createDistributable',
                ('-PvwNativeDir=' + (Join-Path $projectRoot 'target/debug')),
                ('-PvwMcpServerResources=' + $mcpResources),
                '-PvwDesktopSmokeConsole=true', ('-PvwDesktopBuildId=' + $desktopBuild.nonce))
            Complete-McpApplicationImage $desktopBuild.nonce
            Run-Step 'desktop-build-record' 'python.exe' @('tools/desktop-test/reports.py','record','--root',$projectRoot)
        }
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
        'run-desktop' { Run-LicenseGate; Build-NativeWindows; Run-Gradle 'run-desktop' @(':desktop:run') }
        'test-all' {
            Run-Step 'test-ai-verifier' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/ai-spike/tests', '-p', 'test_*.py', '-v')
            Run-Step 'test-tools' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/tests', '-v')
            Run-Step 'test-desktop-reports' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/desktop-test', '-p', 'test_reports.py', '-v')
            Test-McpRuntime
            Run-Step 'test-device-selection' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/tests/device-selection-fixtures.ps1')
            Run-Step 'test-app-device-scope' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/tests/app-hil-device-fixtures.ps1')
            Run-Step 'test-store-crash-receipt' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/tests/store-crash-receipt-fixtures.ps1')
            Run-Cargo 'test-rust' @('test', '--workspace', '--exclude', 'vw-sim', '--locked', '--', '--test-threads=2')
            # Run the complete 10,000-seed simulator in its measured optimized
            # configuration, with compilation on its own progress budget.
            Run-Cargo 'build-rust-simulation' @('test', '--no-run', '--release', '--locked', '-p', 'vw-sim')
            Run-Cargo 'test-rust-simulation' @('test', '--release', '--locked', '-p', 'vw-sim', '--', '--nocapture', '--test-threads=2')
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
            if ($HilMode -eq 'app') {
                if ($TimeoutSeconds -lt 60 -or $TimeoutSeconds -gt 3600) { throw 'App HIL phase timeout must be between 60 and 3600 seconds' }
                Run-LicenseGate
                . (Join-Path $projectRoot 'tools/enter-dev.ps1')
                Run-Step 'test-app-hil-reports' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/app-test', '-p', 'test_*.py', '-v') -Limit 120
                Run-Step 'test-app-hil-staging' 'pwsh.exe' @('-NoProfile', '-File', 'tools/app-test/test_stage.ps1') -Limit 60
                Run-Step 'test-app-device-scope' 'powershell.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/tests/app-hil-device-fixtures.ps1') -Limit 60
                Run-Step 'test-app-hil-native-inventory' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/ffi-test', '-p', 'test_apk.py', '-v') -Limit 120
            }
            if ($HilMode -eq 'shared-ffi') {
                if ($TimeoutSeconds -lt 60 -or $TimeoutSeconds -gt 3600) { throw 'Shared FFI phase timeout must be between 60 and 3600 seconds' }
                Run-LicenseGate
                . (Join-Path $projectRoot 'tools/enter-dev.ps1')
                # Run synthetic parser/package-inventory fixtures before any
                # device work. Native inputs and reviewed goldens must already
                # exist; this mode never builds native code or regenerates them.
                Run-Step 'test-shared-ffi-reports' 'python.exe' @('-m', 'unittest', 'discover', '-s', 'tools/ffi-test', '-p', 'test_shared_reports.py', '-v') -Limit 120
                Run-Step 'test-shared-ffi-package-inventory' 'pwsh.exe' @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', 'tools/ffi-test/test_package_inventory.ps1') -Limit 120
            }
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
            $hilBudget = if ($HilMode -eq 'app') { $TimeoutSeconds * 2 + 2100 } elseif ($HilMode -eq 'shared-ffi') {
                # Match tools/hil-test.ps1: three Gradle phases, 300s task
                # inventory, 1800s auxiliary work, 300s inner cleanup, then
                # another 300s for the dispatcher's process-tree cleanup.
                $TimeoutSeconds * 3 + 300 + 1800 + 600
            } elseif ($HilMode -in @('pen', 'pen-owner', 'win-pen')) { $TimeoutSeconds * 6 + 600 } else { $TimeoutSeconds + 600 }
            $hilShell = if ($HilMode -in @('app', 'shared-ffi')) { 'pwsh.exe' } else { 'powershell.exe' }
            Run-Step 'hil-test' $hilShell $arguments -Limit $hilBudget
        }
    }
    exit 0
} catch {
    Write-Error $_
    exit 1
}
