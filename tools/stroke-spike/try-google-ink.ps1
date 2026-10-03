param([ValidateRange(30, 14400)][int]$TimeoutSeconds = 900, [switch]$MsvcCxx20, [switch]$ShortOutputBase, [switch]$MsvcMathDefines)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
# Preparation records this exact task-owned directory after checking upstream
# tag and executable digest. It contains no product source or owner material.
$location = Join-Path $projectRoot '.local/t010-google-ink-location.txt'
$attemptRoot = (Resolve-Path -LiteralPath ([IO.File]::ReadAllText($location))).Path
$temporaryRoot = (Resolve-Path -LiteralPath $env:TEMP).Path.TrimEnd('\') + '\'
if (-not $attemptRoot.StartsWith($temporaryRoot, [StringComparison]::OrdinalIgnoreCase) -or
    (Split-Path -Leaf $attemptRoot) -notmatch '^vw-google-ink-[0-9a-f]{32}$') { throw 'Unexpected attempt directory' }
$source = Join-Path $attemptRoot 'source'
$bazel = Join-Path $attemptRoot 'bazel.exe'
$expectedCommit = '994e034c9a6e0747386360275058feb7dcac7762'
$head = (& git.exe -C $source rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $head -cne $expectedCommit) { throw 'Pinned Google Ink source mismatch' }
if ((Get-FileHash -LiteralPath $bazel -Algorithm SHA256).Hash -cne '6D9FB21E806CF4F4E61BFA2BC865DF4900FFDC1E9EA90CA1016BA70367EF0DE4') { throw 'Pinned Bazel digest mismatch' }
if ((& git.exe -C $source diff HEAD --name-only) -or $LASTEXITCODE -ne 0) { throw 'Pinned upstream tracked source modified' }
$untracked = @(& git.exe -C $source ls-files --others --exclude-standard)
if ($LASTEXITCODE -ne 0 -or @($untracked | Where-Object { $_ -ne 'MODULE.bazel.lock' }).Count -gt 0) { throw 'Unexpected untracked upstream source' }
$variant = if ($MsvcMathDefines) { 'msvc-math-defines' } elseif ($ShortOutputBase) { 'msvc-short-output' } elseif ($MsvcCxx20) { 'msvc-cxx20' } else { 'upstream' }
$startupArguments = @('--batch', '--host_jvm_args=-Xmx2048m', ('--output_user_root=' + (Join-Path $attemptRoot 'output')))
if ($ShortOutputBase) {
    if (-not $MsvcCxx20) { throw 'The short-path retry requires the diagnosed C++20 correction' }
    # Keep MSVC response files below MAX_PATH without changing system policy.
    $shortLocation = Join-Path $projectRoot '.local/t010-google-ink-short-output.txt'
    if (-not (Test-Path -LiteralPath $shortLocation)) {
        $shortDirectory = Join-Path $temporaryRoot ('vwgi-' + [guid]::NewGuid().ToString('N').Substring(0,12))
        New-Item -ItemType Directory -Path $shortDirectory | Out-Null
        [IO.File]::WriteAllText($shortLocation, $shortDirectory, [Text.UTF8Encoding]::new($false))
    }
    $shortDirectory = (Resolve-Path -LiteralPath ([IO.File]::ReadAllText($shortLocation))).Path
    if ((Split-Path -Parent $shortDirectory).TrimEnd('\') -cne $temporaryRoot.TrimEnd('\') -or
        (Split-Path -Leaf $shortDirectory) -notmatch '^vwgi-[0-9a-f]{12}$') { throw 'Unexpected short output base' }
    $startupArguments += '--output_base=' + $shortDirectory
}
$arguments = $startupArguments + @('build', '--jobs=2', '--local_ram_resources=2048', '--color=no', '--curses=no', '//ink/strokes:in_progress_stroke')
if ($MsvcCxx20) { $arguments += @('--cxxopt=/std:c++20', '--host_cxxopt=/std:c++20') }
if ($MsvcMathDefines) {
    if (-not $MsvcCxx20 -or -not $ShortOutputBase) { throw 'Math-definition retry requires C++20 and short output' }
    $arguments += @('--cxxopt=/D_USE_MATH_DEFINES', '--host_cxxopt=/D_USE_MATH_DEFINES')
}
$result = Invoke-VwProcess -FilePath $bazel -ArgumentList $arguments -WorkingDirectory $source -Phase "google-ink-windows-$variant" -TimeoutSeconds $TimeoutSeconds -RedactValues @($attemptRoot,$projectRoot,$env:USERPROFILE)
$sourceChanges = @(& git.exe -C $source status --porcelain)
if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect upstream source after attempt' }
$receipt = [ordered]@{ schema=1;source_tag='jetpack-1.0.0';source_commit=$expectedCommit;bazel_version='7.7.1';
    bazel_sha256='6d9fb21e806cf4f4e61bfa2bc865df4900ffdc1e9ea90ca1016ba70367ef0de4';target='//ink/strokes:in_progress_stroke';
    platform='Windows x64';variant=$variant;workers=2;limit_seconds=$TimeoutSeconds;elapsed_seconds=[math]::Round($result.ElapsedSeconds,3);
    log_file=[IO.Path]::GetFileName($result.LogPath);exit_code=$result.ExitCode;passed=($result.ExitCode -eq 0);
    source_modified=($sourceChanges.Count -gt 0);driver_or_security_changes=$false;D6_decided=$false }
[IO.File]::WriteAllText((Join-Path $projectRoot '.local/t010-google-ink-receipt.json'), ($receipt | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $projectRoot ".local/t010-google-ink-$variant-receipt.json"), ($receipt | ConvertTo-Json), [Text.UTF8Encoding]::new($false))
if ($result.ExitCode -ne 0) { throw 'Google Ink Windows feasibility build failed; retain its bounded text log and report the specific upstream/toolchain failure' }
Write-Host 'Google Ink Windows core target built; D6 still requires the owner comparison and device latency.'
