Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Reuse the shared module so importing device helpers does not remove the
# caller's Invoke-VwProcess export from its scope.
Import-Module (Join-Path $PSScriptRoot 'process.psm1')

function Invoke-VwAdb {
    param(
        [Parameter(Mandatory = $true)]$Device,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [Parameter(Mandatory = $true)][string]$Phase,
        [int]$TimeoutSeconds = 120,
        [switch]$Capture
    )
    $result = Invoke-VwProcess -FilePath $Device.AdbPath -ArgumentList (@('-s', $Device.Serial) + $Arguments) -WorkingDirectory $Device.Root -Phase $Phase -TimeoutSeconds $TimeoutSeconds -RedactValues @($Device.Serial, $Device.Root, $env:USERPROFILE) -Capture:$Capture -SensitiveCapture:$Capture
    if ($result.ExitCode -ne 0) { throw "$Phase failed (exit $($result.ExitCode))" }
    return $result
}

function Select-VwAndroidTransport {
    param([string[]]$Lines, [string]$ExpectedModel)
    $devices = @($Lines | ForEach-Object {
        if ($_ -match '^(\S+)\s+(device|offline|unauthorized)\b') {
            $serial = $Matches[1]; $state = $Matches[2]
            $model = if ($_ -match '\bmodel:(\S+)') { $Matches[1] -replace '_','-' } else { '' }
            [pscustomobject]@{ Serial = $serial; State = $state; Model = $model }
        }
    })
    if ($ExpectedModel) { $devices = @($devices | Where-Object { $_.Model -ceq $ExpectedModel }) }
    if ($devices.Count -ne 1) { throw 'HIL requires exactly one matching authorized physical device; no fallback is allowed' }
    if ($devices[0].State -ne 'device') { throw 'HIL prerequisite: selected device is not online/authorized' }
    if ($devices[0].Serial -like 'emulator-*') { throw 'HIL acceptance requires physical hardware, not an emulator' }
    return $devices[0]
}

function Get-VwAndroidDevice {
    param([Parameter(Mandatory = $true)][string]$Root, [string]$ExpectedModel = $env:VW_ANDROID_EXPECTED_MODEL)
    if (-not $ExpectedModel) {
        $selectionFile = Join-Path $Root '.local/android-target.json'
        if (Test-Path -LiteralPath $selectionFile -PathType Leaf) {
            $ExpectedModel = (Get-Content -Raw -LiteralPath $selectionFile | ConvertFrom-Json).model
        }
    }
    $sdkRoot = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } elseif ($env:ANDROID_SDK_ROOT) { $env:ANDROID_SDK_ROOT } else { Join-Path $env:LOCALAPPDATA 'Android\Sdk' }
    $adbPath = Join-Path $sdkRoot 'platform-tools\adb.exe'
    if (-not (Test-Path -LiteralPath $adbPath -PathType Leaf)) {
        $adbCommand = Get-Command adb.exe -ErrorAction SilentlyContinue
        if (-not $adbCommand) { throw 'HIL prerequisite: adb is unavailable' }
        $adbPath = $adbCommand.Source
    }
    $listing = Invoke-VwProcess -FilePath $adbPath -ArgumentList @('devices', '-l') -WorkingDirectory $Root -Phase 'hil-device-selection' -TimeoutSeconds 30 -SensitiveCapture
    if ($listing.ExitCode -ne 0) { throw 'HIL prerequisite: adb device listing failed' }
    $selected = Select-VwAndroidTransport -Lines $listing.Lines -ExpectedModel $ExpectedModel
    if ($env:VW_ANDROID_EXPECTED_SERIAL -and $selected.Serial -cne $env:VW_ANDROID_EXPECTED_SERIAL) { throw 'HIL device changed after selection; rerun device selection before testing' }
    $device = [pscustomobject]@{ Serial = $selected.Serial; AdbPath = $adbPath; Root = $Root; Model = '' }
    $qemu = Invoke-VwAdb -Device $device -Arguments @('shell', 'getprop', 'ro.kernel.qemu') -Phase 'hil-physical-device' -TimeoutSeconds 30 -Capture
    if (($qemu.Lines -join '').Trim() -eq '1') { throw 'HIL acceptance requires physical hardware, not an emulator' }
    $model = Invoke-VwAdb -Device $device -Arguments @('shell', 'getprop', 'ro.product.model') -Phase 'hil-device-model' -TimeoutSeconds 30 -Capture
    $device.Model = ($model.Lines -join ' ').Trim()
    if ($ExpectedModel -and $device.Model -cne $ExpectedModel) { throw 'Selected device model differs from authorization; refusing further access' }
    Write-Host "Selected physical device model: $($device.Model)"
    return $device
}

function Start-VwAndroidStarter {
    param(
        [Parameter(Mandatory = $true)]$Device,
        [ValidatePattern('^[a-z][a-z0-9-]*$')][string]$PhasePrefix = 'hil-starter',
        [ValidateRange(1, 3)][int]$Attempts = 3
    )
    for ($attempt = 1; $attempt -le $Attempts; $attempt++) {
        $launch = Invoke-VwAdb -Device $Device -Arguments @('shell', 'am', 'start', '-W', '-n', 'com.visualworkbench.android/.MainActivity') -Phase "$PhasePrefix-launch-$attempt" -TimeoutSeconds 15 -Capture
        if (($launch.Lines -join "`n") -notmatch '(?m)^Status:\s*ok\s*$') { throw 'Starter activity launch did not report Status: ok; check installation before retrying' }
        $foreground = Invoke-VwAdb -Device $Device -Arguments @('shell', 'dumpsys', 'activity', 'activities') -Phase "$PhasePrefix-foreground-$attempt" -TimeoutSeconds 15 -Capture
        $resumed = @($foreground.Lines | Where-Object { $_ -match '(?:mResumedActivity|topResumedActivity)' }) -join "`n"
        if ($resumed -match 'com\.visualworkbench\.android/\.MainActivity') {
            Write-Host "Starter foreground: PASS (attempt $attempt)."
            return
        }
        Write-Host "Starter lost focus on the shared phone (attempt $attempt/$Attempts); no other app was stopped."
        if ($attempt -lt $Attempts) { Start-Sleep -Seconds 2 }
    }
    throw 'Starter could not retain foreground during bounded checks on the shared phone; test is inconclusive until focus is available'
}

function Invoke-VwAndroidCargoTest {
    param(
        [Parameter(Mandatory = $true)]$Device,
        [Parameter(Mandatory = $true)][ValidatePattern('^[a-z][a-z0-9-]*$')][string]$Crate,
        [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600
    )
    # cargo-ndk 4.1.2 consumes Cargo's artifact JSON and overrides the target
    # runner. Export its compiler environment, then let Cargo use our configured
    # runner directly. ndk-env omits the private linker wrapper environment;
    # restore those two values from the same pinned arm64/API 29 configuration.
    $export = Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList @('+1.99.0', 'ndk-env', '-t', 'arm64-v8a', '--platform', '29', '--json') -WorkingDirectory $Device.Root -Phase 'hil-ndk-environment' -TimeoutSeconds 30 -SensitiveCapture
    if ($export.ExitCode -ne 0) { throw "Android compiler environment export failed (exit $($export.ExitCode))" }
    $variables = ($export.Lines -join "`n") | ConvertFrom-Json
    if (-not $variables.CLANG_PATH -or -not $variables.CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER) { throw 'Android compiler environment export omitted the linker configuration' }
    $ndkPrefix = [IO.Path]::GetFullPath($env:ANDROID_NDK_HOME).TrimEnd('\') + '\'
    if (-not [IO.Path]::GetFullPath($variables.CLANG_PATH).StartsWith($ndkPrefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Android compiler export differs from the selected pinned NDK' }
    $changes = [ordered]@{}
    foreach ($property in $variables.PSObject.Properties) {
        if ($property.Name -ne 'CARGO_TARGET_AARCH64_LINUX_ANDROID_RUNNER') { $changes[$property.Name] = [string]$property.Value }
    }
    $changes['CARGO_TARGET_AARCH64_LINUX_ANDROID_RUNNER'] = $null
    $changes['_CARGO_NDK_LINK_CLANG'] = [string]$variables.CLANG_PATH
    $changes['_CARGO_NDK_LINK_TARGET'] = '--target=aarch64-linux-android29'
    $changes['_CARGO_NDK_LDFLAGS'] = $null
    $changes['VW_ANDROID_EXPECTED_SERIAL'] = $Device.Serial
    $previous = @{}
    try {
        foreach ($name in $changes.Keys) {
            $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
            [Environment]::SetEnvironmentVariable($name, $changes[$name], 'Process')
        }
        $testRun = Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList @('+1.99.0', 'test', '--target', 'aarch64-linux-android', '-p', $Crate, '--locked', '--message-format=json-render-diagnostics', '--', '--test-threads=1') -WorkingDirectory $Device.Root -Phase 'hil-rust-cargo-test' -TimeoutSeconds $TimeoutSeconds -SensitiveCapture
        if ($testRun.ExitCode -ne 0) { throw "Rust Android Cargo test failed (exit $($testRun.ExitCode)); inspect the phase receipts" }
        $summaries = [regex]::Matches(($testRun.Lines -join "`n"), 'Cargo Android target runner: PASS \((\d+) actual tests\)')
        $passedTests = 0
        foreach ($summary in $summaries) { $passedTests += [int]$summary.Groups[1].Value }
        if ($passedTests -eq 0) { throw 'Cargo did not exercise the configured Android runner with actual passing tests; HIL remains pending' }
        return $passedTests
    } finally {
        foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    }
}

function Invoke-VwAndroidRustExecutable {
    param(
        [Parameter(Mandatory = $true)]$Device,
        [Parameter(Mandatory = $true)][string]$Executable,
        [string[]]$Arguments = @(),
        [ValidateSet('test', 'hello')][string]$Mode = 'test',
        [ValidateRange(1, 86400)][int]$TimeoutSeconds = 600
    )
    $file = Get-Item -LiteralPath $Executable -ErrorAction Stop
    if ($file.PSIsContainer) { throw 'Android Rust runner requires an executable file' }
    $stream = [IO.File]::OpenRead($file.FullName)
    try {
        $header = New-Object byte[] 20
        if ($stream.Read($header, 0, 20) -ne 20 -or $header[0] -ne 127 -or $header[1] -ne 69 -or $header[2] -ne 76 -or $header[3] -ne 70 -or $header[4] -ne 2 -or $header[5] -ne 1 -or $header[18] -ne 183 -or $header[19] -ne 0) { throw 'Android Rust runner requires a little-endian arm64 ELF executable' }
    } finally { $stream.Dispose() }
    if ($Mode -eq 'hello' -and $Arguments.Count -ne 0) { throw 'Android hello validation does not accept test arguments' }
    foreach ($argument in $Arguments) { if ($argument -match "['`r`n]") { throw 'Android test arguments cannot contain quotes or line breaks' } }
    $ownedDeviceDirectory = '/data/local/tmp/vw-tests/' + [guid]::NewGuid().ToString('N')
    $devicePath = $ownedDeviceDirectory + '/' + $Mode
    $validatedCount = 0
    $validationSucceeded = $false
    $cleanupConfirmed = $false
    $executableHash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    try {
        Invoke-VwAdb -Device $Device -Arguments @('shell', 'mkdir', '-p', $ownedDeviceDirectory) -Phase 'hil-rust-directory' | Out-Null
        $fixtureDirectory = Join-Path $Device.Root 'fixtures\traces'
        $syntheticManifest = Join-Path $fixtureDirectory 'SYNTHETIC.json'
        if ($Mode -eq 'test' -and (Test-Path -LiteralPath $syntheticManifest -PathType Leaf)) {
            $fixtureManifest = Get-Content -Raw -LiteralPath $syntheticManifest | ConvertFrom-Json
            if ($fixtureManifest.synthetic -ne $true) { throw 'Rust HIL transfers synthetic fixtures only' }
            Invoke-VwAdb -Device $Device -Arguments @('shell', 'mkdir', '-p', ($ownedDeviceDirectory + '/traces')) -Phase 'hil-rust-fixture-directory' | Out-Null
            foreach ($fixture in $fixtureManifest.files) {
                if ($fixture.path -notmatch '^[A-Za-z0-9_.-]+$' -or $fixture.path -in @('.', '..')) { throw 'Synthetic HIL fixture path must be one safe file name' }
                $fixturePath = Join-Path $fixtureDirectory $fixture.path
                if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) { throw 'Synthetic HIL fixture is missing' }
                if ((Get-FileHash -LiteralPath $fixturePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $fixture.sha256) { throw 'Synthetic HIL fixture hash differs' }
                Invoke-VwAdb -Device $Device -Arguments @('push', $fixturePath, ($ownedDeviceDirectory + '/traces/' + $fixture.path)) -Phase 'hil-rust-fixtures' | Out-Null
            }
        } elseif ($Mode -eq 'test') { Write-Host 'HIL fixtures: none transferred; synthetic manifest absent. Private and owner traces remain on the PC.' }
        Invoke-VwAdb -Device $Device -Arguments @('push', $file.FullName, $devicePath) -Phase 'hil-rust-transfer' | Out-Null
        Invoke-VwAdb -Device $Device -Arguments @('shell', 'chmod', '700', $devicePath) -Phase 'hil-rust-permissions' | Out-Null
        $remoteArguments = ($Arguments | ForEach-Object { "'" + $_ + "'" }) -join ' '
        $remoteCommand = 'cd ' + $ownedDeviceDirectory + ' && ' + $devicePath + ' ' + $remoteArguments
        $testRun = Invoke-VwAdb -Device $Device -Arguments @('shell', $remoteCommand) -Phase 'hil-rust-execution' -TimeoutSeconds $TimeoutSeconds -Capture
        if ($Mode -eq 'hello') {
            if (($testRun.Lines -join "`n").Trim() -cne 'Visual Workbench toolchain smoke ABI v1 (android/aarch64)') { throw 'Android hello did not emit the exact ABI v1 android/aarch64 marker' }
            $validatedCount = 1
            $validationSucceeded = $true
            Write-Host 'Android hello ABI runtime: PASS (1 exact ABI v1 android/aarch64 marker)'
            return $validatedCount
        }
        $summaries = [regex]::Matches(($testRun.Lines -join "`n"), 'test result: ok\. (\d+) passed; 0 failed;')
        $passedTests = 0
        foreach ($summary in $summaries) { $passedTests += [int]$summary.Groups[1].Value }
        if ($summaries.Count -eq 0 -or $passedTests -eq 0) { throw 'Rust HIL ran zero actual passing tests or emitted no passing summary; acceptance remains pending' }
        $validatedCount = $passedTests
        $validationSucceeded = $true
        Write-Host "Android Rust executable: PASS ($passedTests actual tests)"
        return $passedTests
    } finally {
        if ($ownedDeviceDirectory -match '^/data/local/tmp/vw-tests/[0-9a-f]{32}$') {
            try {
                Invoke-VwAdb -Device $Device -Arguments @('shell', 'rm', '-rf', $ownedDeviceDirectory) -Phase 'hil-owned-test-cleanup' | Out-Null
                Invoke-VwAdb -Device $Device -Arguments @('shell', ('[ ! -e ' + $ownedDeviceDirectory + ' ]')) -Phase 'hil-owned-test-cleanup-verify' -Capture | Out-Null
                $cleanupConfirmed = $true
            } catch { Write-Warning 'HIL owned device test cleanup could not be confirmed' }
        }
        $receiptDirectory = Join-Path ([IO.Path]::GetTempPath()) 'VisualWorkbench-setup-text-logs'
        [IO.Directory]::CreateDirectory($receiptDirectory) | Out-Null
        $receiptName = 'android-' + $Mode + '-runtime-' + [IO.Path]::GetFileName($ownedDeviceDirectory) + '.json'
        [ordered]@{ mode = $Mode; device_model = $Device.Model; executable_sha256 = $executableHash; validated_count = $validatedCount; validation_succeeded = $validationSucceeded; owned_device_directory = $ownedDeviceDirectory; owned_device_cleanup_confirmed = $cleanupConfirmed; screenshots_created = 0 } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $receiptDirectory $receiptName) -Encoding UTF8
        Write-Host "Android $Mode text receipt: $receiptName"
        if ($validationSucceeded -and -not $cleanupConfirmed) { throw 'Android validation passed, but owned device cleanup could not be confirmed' }
    }
}

Export-ModuleMember -Function Get-VwAndroidDevice, Invoke-VwAdb, Start-VwAndroidStarter, Invoke-VwAndroidCargoTest, Invoke-VwAndroidRustExecutable
