param(
    [Parameter(Position = 0)]
    [ValidateSet('build-remote-helpers', 'test-remote-helpers', 'build-remote-integration', 'build-core', 'build-raster', 'test-raster', 'build-network', 'test-network', 'build-pairing', 'build-ffi', 'test-ffi', 'test-editor', 'lint-editor', 'test-shared', 'test-apps', 'test-mcp', 'build-ai', 'test-ai', 'build-managed-apps', 'package-managed-apps', 'package-background-apps', 'package-android-app', 'package-desktop-app', 'assemble-release', 'test-paint-integration', 'build-android', 'build-stroke-core', 'build-stroke', 'build-pen-probe', 'build-pen-inject', 'build-transport', 'build-video-pc', 'build-video-android', 'build-image-android', 'build-vdd-probe', 'build-desktop', 'build-desktop-distribution', 'test-all', 'lint-all', 'license-check', 'hil-test', 'run-desktop', 'doctor')]
    [string]$Command = 'doctor',
    [Parameter(Position = 1)][ValidateSet('app', 'rust', 'stroke', 'pen', 'pen-owner', 'win-pen', 'transport', 'pairing', 'shared-ffi', 'video-pc', 'video-android', 'video-tiles', 'image-pc', 'image-android', 'vdd')][string]$HilMode = 'app',
    [Parameter(Position = 2)][ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
    [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600,
    [switch]$OwnerReady,
    [ValidateSet('normal', 'no-refresh', 'guards')][string]$WinPenScenario = 'normal',
    [ValidatePattern('^[0-9a-f]{32}$')][string]$VideoRunId,
    [ValidateSet('inventory','normal','watchdog')][string]$VddScenario='inventory',
    [string]$RemoteEditorCatalog,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$RemoteEditorCatalogSha256,
    [string]$NativeReuseReceipt,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$NativeReuseReceiptSha256,
    [switch]$PairingTests,
    [switch]$ProjectFlowTests,
    [string]$BackgroundInputs,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$BackgroundInputsSha256,
    [string]$AndroidPackageInputs,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$AndroidPackageInputsSha256,
    [string]$ReleaseManifest,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$ReleaseManifestSha256,
    [string]$ReleaseDestination,
    [string]$ReleasePythonExecutable,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$ReleasePythonSha256
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
Import-Module (Join-Path $projectRoot 'tools\process.psm1') -Force
. (Join-Path $projectRoot 'tools/mcp-build.ps1')
. (Join-Path $projectRoot 'tools/ffi-test/core-unit.ps1')
$nativeReuseLeases = [Collections.Generic.List[IO.FileStream]]::new()

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
    param([string]$Phase, [string[]]$Tasks, [switch]$Incremental)
    $catalogArguments = @(Get-RemoteEditorCatalogArguments $RemoteEditorCatalog $RemoteEditorCatalogSha256 $projectRoot)
    $gradleArguments = $Tasks + $catalogArguments + @('--console=plain', '--no-daemon', '--no-configuration-cache', '--no-build-cache', '--no-parallel')
    if (-not $Incremental) { $gradleArguments += '--rerun-tasks' }
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

# The build owner supplies a reviewed current-source applicability receipt.
# Its outer caller holds the exact source/tool closure; retain these payload
# leases here too so reuse cannot race an output replacement during packaging.
function Assert-NativeReuseReceipt {
    if (-not $NativeReuseReceipt -or -not $NativeReuseReceiptSha256) { throw 'Managed packaging requires the exact root-admitted native reuse receipt' }
    function Pin-ReusedFile([string]$Relative, [string]$Expected, [long]$Bytes = -1) {
        if ([IO.Path]::IsPathRooted($Relative) -or $Relative.Contains(':') -or $Expected -cnotmatch '^[0-9a-f]{64}$') { throw 'Invalid native reuse descriptor' }
        $path = [IO.Path]::GetFullPath((Join-Path $projectRoot $Relative))
        if (-not $path.StartsWith($projectRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Native reuse escaped checkout' }
        $item = Get-Item -LiteralPath $path
        while ($null -ne $item) {
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected native reuse input' }
            $item = if ($item -is [IO.DirectoryInfo]) { $item.Parent } else { $item.Directory }
        }
        $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            if (($Bytes -ge 0 -and $stream.Length -ne $Bytes) -or [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() -cne $Expected) { throw 'Reused native input changed' }
            $stream.Position = 0; $nativeReuseLeases.Add($stream); return $stream
        } catch { $stream.Dispose(); throw }
    }
    if ($NativeReuseReceipt -cnotmatch '^\.local/[A-Za-z0-9][A-Za-z0-9_.-]*\.json$') { throw 'Native reuse receipt requires an exact local name' }
    $stream = Pin-ReusedFile $NativeReuseReceipt $NativeReuseReceiptSha256
    if ($stream.Length -le 0 -or $stream.Length -gt 4MB) { throw 'Native reuse receipt size refused' }
    $reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false, $true), $true, 4096, $true)
    try { $proof = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable -Depth 32 } finally { $reader.Dispose(); $stream.Position = 0 }
    if ($proof.schema -ne 1 -or $proof.operation -cne 'M4_R162_NORMAL_NATIVE_REUSE' -or $proof.status -cne 'source-reviewed-and-admitted' -or $proof.root_reviewed -ne $true -or $proof.source_closure_count -le 0) { throw 'Root-admitted current native source applicability required' }
    $sourceEvidence = @{}
    foreach ($key in @('source_map', 'source_closure', 'source_admission', 'managed_compile', 'mcp_server_inventory')) {
        $descriptor = $proof[$key]
        if ($key -cne 'mcp_server_inventory' -and $descriptor.name -cnotmatch '^[A-Za-z0-9][A-Za-z0-9_.-]*\.json$') { throw 'Local reuse evidence name refused' }
        $relative = if ($key -ceq 'mcp_server_inventory') { $descriptor.path } else { '.local/' + $descriptor.name }
        $pinned = Pin-ReusedFile $relative $descriptor.sha256
        if ($key -cin @('source_map', 'source_closure', 'source_admission')) {
            if ($pinned.Length -le 0 -or $pinned.Length -gt 4MB) { throw 'Source evidence size refused' }
            $reader = [IO.StreamReader]::new($pinned, [Text.UTF8Encoding]::new($false, $true), $true, 4096, $true)
            try { $sourceEvidence[$key] = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable -Depth 32 }
            finally { $reader.Dispose(); $pinned.Position = 0 }
        }
    }
    $admitted = $sourceEvidence.source_admission
    if ($sourceEvidence.source_map.Count -ne $proof.source_count -or
        $sourceEvidence.source_closure.Count -ne $proof.source_closure_count -or
        $admitted.root_reviewed -ne $true -or $admitted.status -cne 'source-reviewed-and-admitted' -or
        $admitted.source_count -ne $proof.source_count -or $admitted.source_closure_count -ne $proof.source_closure_count -or
        $admitted.source_map.sha256 -cne $proof.source_map.sha256 -or
        $admitted.source_closure.sha256 -cne $proof.source_closure.sha256) { throw 'Actual source inventory and admission joins required' }
    $names = @('target/debug/vw_core.dll', 'target/debug/vw_host.dll', 'target/debug/vw-capture-helper.exe', 'target/debug/vw-capture-stream-helper.exe', 'target/debug/vw-connection-helper.exe', 'target/debug/vw-hevc-helper.exe', 'target/debug/vw-input-helper.exe', 'target/debug/vw-editor-probe.exe', 'target/debug/vw-paint-drawing-helper.exe', 'target/android-jni/arm64-v8a/libvw_core.so', 'target/debug/vw-bindgen.exe', 'target/debug/vw-mcp-host.exe', 'target/debug/vw-mcp-package.exe', 'target/debug/vw-codex-host.exe')
    if ($proof.files.Count -ne $names.Count -or @($names | Where-Object { -not $proof.files.ContainsKey($_) }).Count -ne 0) { throw 'Exact native/binding-tool/MCP-server reuse set required' }
    foreach ($name in $names) { $row = $proof.files[$name]; [void](Pin-ReusedFile $name $row.sha256 $row.bytes) }
}

# The outer root-admitted caller holds the source/tool closure and only the
# needed unchanged native/binding payloads. Retain the small evidence leases
# through the existing finally block; do not inventory or rebuild that cohort.
function Read-AndroidPackageInputs {
    if (-not $AndroidPackageInputs -or -not $AndroidPackageInputsSha256) {
        throw 'Android-only packaging requires its paired admitted input path and SHA256'
    }
    function Read-AndroidEvidence([string]$Relative, [string]$Digest) {
        if ($Relative -cnotmatch '^\.local/[A-Za-z0-9][A-Za-z0-9_.-]*\.json$' -or
            $Digest -cnotmatch '^[a-f0-9]{64}$') { throw 'Exact local Android evidence descriptor required' }
        $path = Join-Path $projectRoot $Relative
        $item = Get-Item -LiteralPath $path
        while ($null -ne $item) {
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected Android evidence refused' }
            $item = if ($item -is [IO.DirectoryInfo]) { $item.Parent } else { $item.Directory }
        }
        $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        try {
            if ($stream.Length -le 0 -or $stream.Length -gt 1MB -or
                [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() -cne $Digest) {
                throw 'Android evidence changed or exceeded bounds'
            }
            $stream.Position = 0
            $reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false, $true), $true, 4096, $true)
            try { $value = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable -Depth 32 }
            finally { $reader.Dispose(); $stream.Position = 0 }
            $nativeReuseLeases.Add($stream)
            return $value
        } catch { $stream.Dispose(); throw }
    }
    function Same-AndroidEvidence($First, $Second, [string]$Location = 'name') {
        return $First[$Location] -ceq $Second[$Location] -and
            $First.sha256 -cmatch '^[a-f0-9]{64}$' -and $First.sha256 -ceq $Second.sha256
    }
    $cfg = Read-AndroidEvidence $AndroidPackageInputs $AndroidPackageInputsSha256
    if ($cfg.schema -ne 1 -or $cfg.operation -cne 'M4_R165_ANDROID_PACKAGING_INPUTS' -or
        $cfg.status -cne 'root-admitted-one-normal-android-package' -or $cfg.root_reviewed -ne $true -or
        $cfg.execution_authorized -ne $true -or $cfg.owner_build_hold -ne $false -or $cfg.root -cne $projectRoot -or
        $cfg.case_id -cnotmatch '^[a-f0-9]{32}$' -or $cfg.source_count -le 0 -or
        $cfg.source_closure_count -lt $cfg.source_count -or $cfg.outer_source_tool_closure_held -ne $true -or
        $cfg.predecessor_capture.path -cnotmatch '^[A-Za-z]:\\' -or $cfg.predecessor_capture.sha256 -cnotmatch '^[a-f0-9]{64}$' -or
        $cfg.python.path -cnotmatch '^[A-Za-z]:\\.+\\python\.exe$' -or $cfg.python.sha256 -cnotmatch '^[a-f0-9]{64}$') {
        throw 'Current admitted Android source, pinned outer executor and released build hold required'
    }
    foreach ($key in @('source_map', 'source_closure', 'source_admission', 'native_license_applicability', 'predecessor_build', 'predecessor_license_carry')) {
        if ($cfg[$key].name -cnotmatch '^[A-Za-z0-9][A-Za-z0-9_.-]*\.json$' -or
            $cfg[$key].sha256 -cnotmatch '^[a-f0-9]{64}$') { throw 'Exact local Android source and provenance descriptors required' }
    }
    $admission = Read-AndroidEvidence ('.local/' + $cfg.source_admission.name) $cfg.source_admission.sha256
    $applicable = Read-AndroidEvidence ('.local/' + $cfg.native_license_applicability.name) $cfg.native_license_applicability.sha256
    if ($admission.schema -ne 1 -or $admission.operation -cne 'M4_R165_ANDROID_SOURCE_ADMISSION' -or
        $admission.status -cne 'source-reviewed-and-admitted' -or $admission.root_reviewed -ne $true -or
        $admission.root -cne $projectRoot -or $admission.case_id -cne $cfg.case_id -or
        $admission.scope -cnotin @('android-only-with-build-entrypoint', 'paint-managed-with-build-entrypoint') -or
        $admission.source_count -ne $cfg.source_count -or $admission.source_closure_count -ne $cfg.source_closure_count -or
        -not (Same-AndroidEvidence $admission.source_map $cfg.source_map) -or
        -not (Same-AndroidEvidence $admission.source_closure $cfg.source_closure)) {
        throw 'Current Android source admission and input joins differ'
    }
    if ($applicable.schema -ne 1 -or $applicable.operation -cne 'M4_R165_ANDROID_NATIVE_LICENSE_APPLICABILITY' -or
        $applicable.status -cne 'reviewed-prior-native-and-license-evidence-only' -or $applicable.root_reviewed -ne $true -or
        $applicable.case_id -cne $cfg.case_id -or $applicable.producer_case_id -cnotmatch '^[a-f0-9]{32}$' -or
        $applicable.producer_case_id -ceq $cfg.case_id -or $applicable.unchanged_native_inputs -ne $true -or
        $applicable.unchanged_uniffi_and_binding_inputs -ne $true -or $applicable.unchanged_dependency_license_inputs -ne $true -or
        $applicable.unchanged_native_and_binding_tools -ne $true -or $applicable.outer_needed_payload_leases_held -ne $true -or
        $applicable.prior_evidence_is_not_a_fresh_test_run -ne $true) {
        throw 'Explicit unchanged native/binding/licensing applicability required'
    }
    foreach ($key in @('source_map', 'source_closure', 'source_admission', 'predecessor_build', 'predecessor_license_carry')) {
        if (-not (Same-AndroidEvidence $applicable[$key] $cfg[$key])) { throw 'Android applicability provenance join differs' }
    }
    if (-not (Same-AndroidEvidence $applicable.predecessor_capture $cfg.predecessor_capture 'path')) {
        throw 'Actual predecessor native/archive provenance join differs'
    }
    return $cfg
}


# Source/tool/consumer ownership belongs to the root-admitted outer caller.
# This mode never accepts the old native-reuse receipt: its native producers run
# before binding generation and both normal packages are made from those outputs.
function Read-BackgroundInputs {
    if (-not $BackgroundInputs -or -not $BackgroundInputsSha256 -or
        $BackgroundInputs -cnotmatch '^\.local/[A-Za-z0-9][A-Za-z0-9_.-]*\.json$') {
        throw 'Exact root-admitted background packaging inputs required'
    }
    $path = Join-Path $projectRoot $BackgroundInputs
    $item = Get-Item -LiteralPath $path
    while ($null -ne $item) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected background input' }
        $item = if ($item -is [IO.DirectoryInfo]) { $item.Parent } else { $item.Directory }
    }
    $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($stream.Length -le 0 -or $stream.Length -gt 4MB -or
            [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() -cne $BackgroundInputsSha256) {
            throw 'Background packaging inputs changed or exceed bounds'
        }
        $stream.Position = 0
        $reader = [IO.StreamReader]::new($stream, [Text.UTF8Encoding]::new($false, $true), $true, 4096, $true)
        try { $cfg = $reader.ReadToEnd() | ConvertFrom-Json -AsHashtable -Depth 100 }
        finally { $reader.Dispose(); $stream.Position = 0 }
        if ($cfg.schema -ne 1 -or $cfg.operation -cne 'M4_R164_BACKGROUND_PACKAGING_INPUTS' -or
            $cfg.root_reviewed -ne $true -or $cfg.case_id -cnotmatch '^[a-f0-9]{32}$' -or
            $cfg.source_count -le 0 -or $cfg.source_closure_count -lt $cfg.source_count -or
            (@($cfg.changed_native_crates | Sort-Object) -join ',') -cne 'vw-ffi,vw-raster' -or
            @($cfg.native_regression_tests).Count -ne 2 -or @($cfg.managed_regression_tests).Count -ne 0) {
            throw 'Admitted native-only background source/test scope required'
        }
        $seen = @{}
        foreach ($test in $cfg.native_regression_tests) {
            $key = $test.crate + '/' + $test.target
            if ($key -cnotin @('vw-raster/display', 'vw-ffi/background') -or $seen.ContainsKey($key) -or
                @($test.methods).Count -le 0 -or @($test.methods | Sort-Object -Unique).Count -ne @($test.methods).Count -or
                @($test.extra_features).Count -ne 0 -or (@($test.arguments) -join ',') -cne '--test-threads=1') {
                throw 'Only the two exact new background integration targets are admitted'
            }
            foreach ($method in $test.methods) {
                if ($method -cnotmatch '^[A-Za-z][A-Za-z0-9_]*$') { throw 'Plain exact native regression method required' }
            }
            $seen[$key] = $true
        }
        $nativeReport = Join-Path $projectRoot ('.local/m4-r164-background-native-tests-' + $cfg.case_id + '.json')
        if (Test-Path -LiteralPath $nativeReport) { throw 'Fresh native regression result path required' }
        if ($cfg.ContainsKey('license_carry_receipt') -and $null -ne $cfg.license_carry_receipt) {
            $carry = $cfg.license_carry_receipt
            if ($carry.name -cnotmatch '^m4-r164-background-license-carry-[a-f0-9]{32}\.json$' -or
                $carry.sha256 -cnotmatch '^[a-f0-9]{64}$') { throw 'Exact prior licensing carry descriptor required' }
            $carryPath = Join-Path $projectRoot ('.local/' + $carry.name)
            $carryItem = Get-Item -LiteralPath $carryPath
            while ($null -ne $carryItem) {
                if (($carryItem.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected licensing carry refused' }
                $carryItem = if ($carryItem -is [IO.DirectoryInfo]) { $carryItem.Parent } else { $carryItem.Directory }
            }
            $carryStream = [IO.File]::Open($carryPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
            try {
                if ($carryStream.Length -le 0 -or $carryStream.Length -gt 4MB -or
                    [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($carryStream)).ToLowerInvariant() -cne $carry.sha256) {
                    throw 'Licensing carry changed or exceeded bounds'
                }
                $carryStream.Position = 0
                $carryReader = [IO.StreamReader]::new($carryStream, [Text.UTF8Encoding]::new($false, $true), $true, 4096, $true)
                try { $carryProof = $carryReader.ReadToEnd() | ConvertFrom-Json -AsHashtable -Depth 100 }
                finally { $carryReader.Dispose(); $carryStream.Position = 0 }
                $licenses = @('license-offline-policy','license-npm-policy','license-cargo-deny-version','license-rust','license-gradle')
                if ($carryProof.operation -cne 'M4_R164_BACKGROUND_LICENSE_CARRY' -or
                    $carryProof.status -cne 'reviewed-prior-licensing-evidence-only' -or $carryProof.root_reviewed -ne $true -or
                    $carryProof.case_id -cne $cfg.case_id -or $carryProof.source_map.sha256 -cne $cfg.source_map.sha256 -or
                    $carryProof.source_closure.sha256 -cne $cfg.source_closure.sha256 -or
                    $carryProof.source_admission.sha256 -cne $cfg.source_admission.sha256 -or
                    (@($carryProof.carried_phase_receipts | ForEach-Object {$_.phase}) -join ',') -cne ($licenses -join ',') -or
                    @($licenses | Where-Object {$_ -cin @($cfg.expected_phase_names)}).Count -ne 0) {
                    throw 'Exact prior licensing provenance and fresh-phase exclusion required'
                }
                $nativeReuseLeases.Add($carryStream)
            } catch { $carryStream.Dispose(); throw }
        }
        $nativeReuseLeases.Add($stream)
        return $cfg
    } catch { $stream.Dispose(); throw }
}

function Run-BackgroundNativeRegression {
    param($Test)
    # Apply /DEBUG:NONE only to this integration executable. A global RUSTFLAGS
    # change would invalidate unrelated native outputs and Android compilations.
    if ($env:CARGO_BUILD_TARGET) { throw 'Background integration tests require the pinned Windows host' }
    $phase = 'background-native-' + $Test.crate + '-' + $Test.target
    $build = Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList @('+1.99.0', 'rustc', '--locked', '-p', $Test.crate,
        '--test', $Test.target, '--profile', 'test', '--message-format=json', '--', '-C', 'link-arg=/DEBUG:NONE') `
        -WorkingDirectory $projectRoot -Phase ($phase + '-build') -TimeoutSeconds $TimeoutSeconds -Capture -CleanCompilerTelemetry
    if ($build.ExitCode -ne 0) { throw 'Background regression executable build failed; retain its process receipt' }
    $found = $null; $finished = 0; $capturedBytes = 0L
    $normalRoot = $projectRoot.Replace('\', '/')
    $expectedManifest = '/core/crates/' + $Test.crate + '/Cargo.toml'
    $expectedSource = '/core/crates/' + $Test.crate + '/tests/' + $Test.target + '.rs'
    foreach ($line in $build.Lines) {
        $capturedBytes += [Text.Encoding]::UTF8.GetByteCount($line)
        if ($capturedBytes -gt 64MB -or $line.Length -gt 524288) { throw 'Background compiler JSON bound exceeded' }
        if (-not $line.StartsWith('{')) { continue }
        try { $record = $line | ConvertFrom-Json -ErrorAction Stop } catch { throw 'Malformed background compiler JSON' }
        if ($record.reason -ceq 'build-finished') {
            if ($record.success -isnot [bool] -or -not $record.success) { throw 'Background executable did not compile' }
            $finished++; continue
        }
        if ($record.reason -cne 'compiler-artifact' -or $record.target.name -cne $Test.target) { continue }
        if ($null -ne $found -or $record.profile.test -isnot [bool] -or -not $record.profile.test -or
            @($record.target.kind).Count -ne 1 -or $record.target.kind[0] -cne 'test' -or
            @($record.target.crate_types).Count -ne 1 -or $record.target.crate_types[0] -cne 'bin' -or
            $record.target.edition -cne '2024') { throw 'Unexpected integration compiler artifact' }
        foreach ($pair in @(@($record.manifest_path, $expectedManifest), @($record.target.src_path, $expectedSource))) {
            $value = ([string]$pair[0]).Replace('\', '/')
            if ($value -cne ('[redacted]' + $pair[1]) -and
                -not [string]::Equals($value, $normalRoot + $pair[1], [StringComparison]::OrdinalIgnoreCase)) {
                throw 'Background regression source binding differs'
            }
        }
        $exe = ([string]$record.executable).Replace('\', '/')
        $prefix = if ($exe.StartsWith('[redacted]/', [StringComparison]::Ordinal)) { '[redacted]' } else { $normalRoot }
        $tail = '/target/debug/deps/'
        if (-not $exe.StartsWith($prefix + $tail, [StringComparison]::OrdinalIgnoreCase)) { throw 'Integration executable escapes expected target' }
        $name = $exe.Substring(($prefix + $tail).Length)
        if ($name -cnotmatch ('^' + [regex]::Escape($Test.target) + '-[0-9a-f]{16}\.exe$') -or
            @($record.filenames | Where-Object { ([string]$_).Replace('\', '/') -ceq $exe }).Count -ne 1) {
            throw 'Integration executable missing from exact compiler artifact'
        }
        $found = Join-Path $projectRoot ('target/debug/deps/' + $name)
    }
    if ($finished -ne 1 -or $null -eq $found) { throw 'Missing successful exact background compiler artifact' }
    $item = Get-Item -LiteralPath $found
    while ($null -ne $item) {
        if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected background test executable' }
        $item = if ($item -is [IO.DirectoryInfo]) { $item.Parent } else { $item.Directory }
    }
    $lease = [IO.File]::Open($found, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $before = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($lease)).ToLowerInvariant(); $lease.Position = 0
        $run = Invoke-VwProcess -FilePath $found -ArgumentList @('--test-threads=1') -WorkingDirectory $projectRoot `
            -Phase ($phase + '-test') -TimeoutSeconds ([Math]::Min($TimeoutSeconds, 240)) -Capture
        $run.Lines | Write-Host
        if ($run.ExitCode -ne 0) { throw 'Background native regression execution failed; retain its process receipt' }
        $count = Assert-VwCoreUnitResult -Lines $run.Lines -RequiredTests @($Test.methods)
        if ($count -ne @($Test.methods).Count) { throw 'Background regression executed an unadmitted test census' }
        $lease.Position = 0
        if ([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($lease)).ToLowerInvariant() -cne $before) { throw 'Background executable changed during execution' }
        return [ordered]@{crate=$Test.crate;target=$Test.target;methods=@($Test.methods);tests=$count;passed=$true;
            arguments=@('--test-threads=1');extra_features=@();
            executable=[ordered]@{path=('target/debug/deps/' + [IO.Path]::GetFileName($found));sha256=$before;bytes=$lease.Length};
            build_phase=($phase + '-build');test_phase=($phase + '-test')}
    } finally { $lease.Dispose() }
}

function Save-BackgroundNativeResults {
    param($Config, [object[]]$Results)
    $path = Join-Path $projectRoot ('.local/m4-r164-background-native-tests-' + $Config.case_id + '.json')
    $value = [ordered]@{schema=1;operation='M4_R164_BACKGROUND_NATIVE_REGRESSIONS';case_id=$Config.case_id;status='passed';
        inputs=[ordered]@{name=[IO.Path]::GetFileName($BackgroundInputs);sha256=$BackgroundInputsSha256};
        source_map=$Config.source_map;source_closure=$Config.source_closure;source_admission=$Config.source_admission;
        targets=$Results;tests=(@($Results | ForEach-Object {$_.tests}) | Measure-Object -Sum).Sum;
        historical_suites_rerun=$false;device_actions=$false;physical_acceptance=$false}
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($value | ConvertTo-Json -Depth 20) + "`n")
    $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes); $stream.Flush($true) } finally { $stream.Dispose() }
}

try {
    $releaseParameterNames = @('ReleaseManifest', 'ReleaseManifestSha256', 'ReleaseDestination', 'ReleasePythonExecutable', 'ReleasePythonSha256')
    foreach ($releaseName in $releaseParameterNames) {
        if ($Command -ne 'assemble-release' -and $PSBoundParameters.ContainsKey($releaseName)) {
            throw 'Release assembly inputs are limited to assemble-release.'
        }
    }
    if ($PairingTests -and $ProjectFlowTests) { throw 'Select one focused managed test batch' }
    if ($ProjectFlowTests -and $Command -ne 'package-managed-apps') { throw 'Focused project-flow tests require managed packaging' }
    if ($PairingTests -and $Command -ne 'package-managed-apps') { throw 'Focused pairing tests require managed packaging' }
    if ($Command -ne 'package-managed-apps' -and ($NativeReuseReceipt -or $NativeReuseReceiptSha256)) { throw 'Native reuse is limited to managed packaging' }
    if ($Command -ne 'package-background-apps' -and ($BackgroundInputs -or $BackgroundInputsSha256)) { throw 'Background inputs are limited to background packaging' }
    if ($Command -cnotin @('package-android-app', 'package-desktop-app', 'test-paint-integration') -and ($AndroidPackageInputs -or $AndroidPackageInputsSha256)) { throw 'App package inputs are limited to incremental normal app integration' }
    switch ($Command) {
        'assemble-release' {
            if ($PSVersionTable.PSVersion.Major -lt 7) { throw 'Release assembly requires PowerShell 7 (pwsh).' }
            foreach ($releaseName in $releaseParameterNames) {
                if (-not $PSBoundParameters.ContainsKey($releaseName) -or [string]::IsNullOrWhiteSpace([string]$PSBoundParameters[$releaseName])) {
                    throw 'Release assembly requires explicit manifest, manifest SHA256, destination, Python executable and Python SHA256.'
                }
            }
            if ($TimeoutSeconds -gt 3600) { throw 'Release assembly timeout must be between 1 and 3600 seconds.' }
            & (Join-Path $projectRoot 'tools/release/package-windows.ps1') `
                -Root $projectRoot -Manifest $ReleaseManifest -ManifestSha256 $ReleaseManifestSha256 `
                -Destination $ReleaseDestination -PythonExecutable $ReleasePythonExecutable `
                -PythonSha256 $ReleasePythonSha256 -TimeoutSeconds $TimeoutSeconds
        }
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
        'build-managed-apps' {
            Run-LicenseGate
            Run-Gradle 'build-managed-apps' @(':desktop:compileKotlin', ':android:compileDebugKotlin', ':android:compileDebugAndroidTestKotlin')
        }
        'package-managed-apps' {
            Assert-NativeReuseReceipt
            if (-not $RemoteEditorCatalog -or -not $RemoteEditorCatalogSha256) { throw 'Paint managed packaging requires its root-admitted catalog pair' }
            if (Test-Path -LiteralPath (Join-Path $projectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev')) { throw 'Preserve and retire the previous application image before managed packaging; a fresh image path is required' }
            Run-Step 'desktop-build-begin' 'python.exe' @('tools/desktop-test/reports.py','begin','--root',$projectRoot)
            $desktopBuild = Get-Content -LiteralPath (Join-Path $projectRoot '.local/desktop-test-build-start.json') -Raw | ConvertFrom-Json
            if ($desktopBuild.nonce -cnotmatch '^[0-9a-f]{32}$') { throw 'Desktop build receipt refused' }
            Run-LicenseGate
            if ($PairingTests) {
                Run-Gradle 'pairing-managed-tests' @(':shared:desktopTest', '--tests', 'com.visualworkbench.shared.PeerLabelStoreTest',
                    ':desktop:test', '--tests', 'com.visualworkbench.desktop.DesktopPeerLabelsTest',
                    ':android:testDebugUnitTest', '--tests', 'com.visualworkbench.android.session.QrLuminanceTest') -Incremental
            }
            if ($ProjectFlowTests) {
                Run-Gradle 'project-flow-managed-tests' @(':desktop:test', '--tests', 'com.visualworkbench.desktop.ConnectionDraftTest',
                    '--tests', 'com.visualworkbench.desktop.DesktopBackgroundDisplayTest',
                    ':android:testDebugUnitTest', '--tests', 'com.visualworkbench.android.editor.ProjectReceiveCompletionTest') -Incremental
            }
            Run-Gradle 'package-managed-android' @(':android:assembleDebug', ':android:assembleDebugAndroidTest') -Incremental
            Run-Step 'verify-android-native-packaging' 'python.exe' @('tools/ffi-test/check_apk.py', 'apps/android/build/outputs/apk/debug/android-debug.apk')
            $mcpResources = Build-McpServerResources $desktopBuild.nonce -ReuseNative
            Run-Gradle 'package-managed-desktop' @(':desktop:createDistributable',
                ('-PvwNativeDir=' + (Join-Path $projectRoot 'target/debug')),
                ('-PvwMcpServerResources=' + $mcpResources),
                '-PvwDesktopSmokeConsole=true', ('-PvwDesktopBuildId=' + $desktopBuild.nonce)) -Incremental
            Complete-McpApplicationImage $desktopBuild.nonce
            Run-Step 'desktop-build-record' 'python.exe' @('tools/desktop-test/reports.py','record','--root',$projectRoot)
        }
        'package-background-apps' {
            $background = Read-BackgroundInputs
            if (-not $RemoteEditorCatalog -or -not $RemoteEditorCatalogSha256 -or
                $RemoteEditorCatalog -cne $background.catalog.path -or $RemoteEditorCatalogSha256 -cne $background.catalog.sha256) {
                throw 'Exact root-admitted Paint catalog pair required'
            }
            if (Test-Path -LiteralPath (Join-Path $projectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev')) {
                throw 'Preserve and retire the previous application image before background packaging'
            }
            Run-Step 'desktop-build-begin' 'python.exe' @('tools/desktop-test/reports.py','begin','--root',$projectRoot)
            $desktopBuild = Get-Content -LiteralPath (Join-Path $projectRoot '.local/desktop-test-build-start.json') -Raw | ConvertFrom-Json
            if ($desktopBuild.nonce -cnotmatch '^[0-9a-f]{32}$') { throw 'Fresh desktop build nonce required' }
            if ($background.ContainsKey('license_carry_receipt') -and $null -ne $background.license_carry_receipt) {
                Write-Host 'Prior licensing evidence is separately admitted; this attempt creates a fresh desktop nonce and runs format, lint, all new regressions and remaining producers.'
            } else { Run-LicenseGate }
            Run-Cargo 'background-format-native' @('fmt','-p','vw-raster','-p','vw-ffi','--','--check')
            Run-Cargo 'background-lint-native' @('clippy','--locked','-p','vw-raster','-p','vw-ffi','--all-targets',
                '--features','vw-ffi/bindgen,vw-ffi/fixtures','--','-D','warnings')
            $nativeResults = @(foreach ($test in $background.native_regression_tests) { Run-BackgroundNativeRegression $test })
            Save-BackgroundNativeResults $background $nativeResults
            Build-NativeWindows
            Build-NativeAndroid
            Run-Gradle 'background-shared-bindings' @(':shared:generateCoreBindings', ':shared:generateHostBindings',
                ':shared:checkCommonMainImports', ':shared:checkImportGuardFixtures', ':shared:checkAndroidNativeAlignment') -Incremental
            Run-Gradle 'package-background-android' @(':android:assembleDebug', ':android:assembleDebugAndroidTest') -Incremental
            Run-Step 'verify-android-native-packaging' 'python.exe' @('tools/ffi-test/check_apk.py','apps/android/build/outputs/apk/debug/android-debug.apk')
            # vw-raster is a dependency of vw-package and therefore these MCP
            # servers. Do not reuse the old server binary applicability.
            $mcpResources = Build-McpServerResources $desktopBuild.nonce
            Run-Gradle 'package-background-desktop' @(':desktop:createDistributable',
                ('-PvwNativeDir=' + (Join-Path $projectRoot 'target/debug')),
                ('-PvwMcpServerResources=' + $mcpResources),
                '-PvwDesktopSmokeConsole=true', ('-PvwDesktopBuildId=' + $desktopBuild.nonce)) -Incremental
            Complete-McpApplicationImage $desktopBuild.nonce
            Run-Step 'desktop-build-record' 'python.exe' @('tools/desktop-test/reports.py','record','--root',$projectRoot)
        }
        'package-android-app' {
            $android = Read-AndroidPackageInputs
            if ($RemoteEditorCatalog -or $RemoteEditorCatalogSha256) { throw 'Android-only packaging does not accept unrelated catalog overrides' }
            foreach ($name in @('ORG_GRADLE_PROJECT_vwAndroidHil', 'ORG_GRADLE_PROJECT_vwRemoteIntegration', 'ORG_GRADLE_PROJECT_vwNativeDir', 'ORG_GRADLE_PROJECT_vwAndroidNativeDir')) {
                if ([Environment]::GetEnvironmentVariable($name, 'Process')) { throw 'Android-only packaging requires the admitted normal native source graph' }
            }
            Run-Gradle 'package-android-app' @(':android:assembleDebug', '--max-workers=2') -Incremental
            Run-Step 'verify-android-native-packaging' $android.python.path @('tools/ffi-test/check_apk.py', 'apps/android/build/outputs/apk/debug/android-debug.apk')
        }
        'test-paint-integration' {
            $null = Read-AndroidPackageInputs
            if ($RemoteEditorCatalog -or $RemoteEditorCatalogSha256) { throw 'Focused managed tests do not accept catalog overrides' }
            Run-Gradle 'test-paint-integration' @(':shared:desktopTest',
                '--tests', 'com.visualworkbench.shared.RemoteEditorPaletteTest.revoked_grant_does_not_adopt_delayed_dispatch_failure',
                '--tests', 'com.visualworkbench.shared.RemoteEditorPaletteTest.replacement_editor_does_not_adopt_same_binding_delayed_dispatch_failure',
                ':desktop:test', '--tests', 'com.visualworkbench.desktop.remote.RemoteWindowSelectionTest',
                '--max-workers=2') -Incremental
        }
        'package-desktop-app' {
            $app = Read-AndroidPackageInputs
            if ($app.desktop.outer_reuse_resources_held -ne $true -or
                $app.desktop.mcp_resources.path -cnotmatch '^[A-Za-z]:\\' -or
                $app.desktop.mcp_resources.inventory_sha256 -cnotmatch '^[a-f0-9]{64}$' -or
                $app.desktop.previous_image_preserved_and_retired -ne $true -or
                $RemoteEditorCatalog -cne $app.desktop.catalog.path -or
                $RemoteEditorCatalogSha256 -cne $app.desktop.catalog.sha256) {
                throw 'Exact admitted desktop resources, catalog and previous-image preservation required'
            }
            foreach ($name in @('ORG_GRADLE_PROJECT_vwAndroidHil', 'ORG_GRADLE_PROJECT_vwRemoteIntegration',
                'ORG_GRADLE_PROJECT_vwNativeDir', 'ORG_GRADLE_PROJECT_vwMcpServerResources',
                'ORG_GRADLE_PROJECT_vwDesktopBuildId', 'ORG_GRADLE_PROJECT_vwRemoteEditorCatalog',
                'ORG_GRADLE_PROJECT_vwRemoteEditorCatalogSha256', 'VW_MCP_APP_IMAGE_MANIFEST')) {
                if ([Environment]::GetEnvironmentVariable($name, 'Process')) { throw 'Desktop packaging requires the admitted normal source graph' }
            }
            $image = Join-Path $projectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev'
            if (Test-Path -LiteralPath $image) { throw 'Preserve and retire the previous app image before packaging' }
            $inventory = Join-Path $app.desktop.mcp_resources.path 'vw-mcp-server.sha256'
            if ((Get-FileHash -LiteralPath $inventory -Algorithm SHA256).Hash.ToLowerInvariant() -cne
                $app.desktop.mcp_resources.inventory_sha256) { throw 'Reused MCP resource inventory differs' }
            Run-Step 'desktop-build-begin' $app.python.path @('tools/desktop-test/reports.py', 'begin', '--root', $projectRoot)
            $desktopBuild = Get-Content -LiteralPath (Join-Path $projectRoot '.local/desktop-test-build-start.json') -Raw | ConvertFrom-Json
            if ($desktopBuild.nonce -cnotmatch '^[0-9a-f]{32}$') { throw 'Fresh desktop build nonce required' }
            $bridgeWork = Join-Path $projectRoot ('.local/mcp-build/' + $desktopBuild.nonce)
            if (Test-Path -LiteralPath $bridgeWork) { throw 'Final bridge requires fresh owned staging' }
            [void][IO.Directory]::CreateDirectory($bridgeWork)
            # The app image changed; its bound bridge must be regenerated. The
            # unchanged native host and staged MCP server are reused separately.
            Run-Gradle 'package-desktop-app' @(':desktop:createDistributable', '--max-workers=2',
                ('-PvwNativeDir=' + (Join-Path $projectRoot 'target/debug')),
                ('-PvwMcpServerResources=' + $app.desktop.mcp_resources.path),
                '-PvwDesktopSmokeConsole=true', ('-PvwDesktopBuildId=' + $desktopBuild.nonce)) -Incremental
            Complete-McpApplicationImage $desktopBuild.nonce
            Run-Step 'desktop-build-record' $app.python.path @('tools/desktop-test/reports.py', 'record', '--root', $projectRoot)
        }
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
} finally {
    foreach ($stream in $nativeReuseLeases) { $stream.Dispose() }
}
