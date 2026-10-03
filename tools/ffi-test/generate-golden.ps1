param([ValidateRange(1, 600)][int]$TimeoutSeconds = 300)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
Import-Module (Join-Path $repository 'tools/process.psm1') -Force
$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$name = 'VisualWorkbench-ffi-golden-' + [Guid]::NewGuid().ToString('N')
$owned = Join-Path $temporaryParent $name
$fixtures = Join-Path $PSScriptRoot 'fixtures'
$created = $false
$cleanup = $false
function Get-GoldenHash([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($algorithm.ComputeHash($stream)).Replace('-', '').ToLowerInvariant() }
    finally { $algorithm.Dispose(); $stream.Dispose() }
}
try {
    if (Test-Path -LiteralPath $owned) { throw 'Golden work directory already exists' }
    [IO.Directory]::CreateDirectory($owned) | Out-Null
    $created = $true
    $output = Join-Path $owned 'expected'
    [IO.Directory]::CreateDirectory($output) | Out-Null
    $run = Invoke-VwProcess -FilePath (Join-Path $repository 'target/debug/vw-ffi-golden.exe') -ArgumentList @($output, (Join-Path $owned 'project')) -WorkingDirectory $repository -Phase 'generate-ffi-golden' -TimeoutSeconds $TimeoutSeconds -RedactValues @($repository, $owned, $env:USERPROFILE)
    if ($run.ExitCode -ne 0) { throw 'Rust golden generator failed; retained its bounded receipt' }
    [IO.Directory]::CreateDirectory($fixtures) | Out-Null
    foreach ($file in @('ffi-golden.json', 'ffi-golden.properties')) {
        $source = Join-Path $output $file
        $destination = Join-Path $fixtures $file
        if (-not (Test-Path -LiteralPath $source -PathType Leaf) -or (Get-Item -LiteralPath $source).Length -gt 1048576) { throw 'Golden fixture missing or oversized' }
        $digest = Get-GoldenHash $source
        if (Test-Path -LiteralPath $destination) {
            if ((Get-GoldenHash $destination) -cne $digest) { throw 'Rust golden changed; preserve the committed fixture and investigate' }
        } else {
            # No-clobber publication; the golden is textual synthetic test data.
            [IO.File]::Copy($source, $destination, $false)
        }
        Write-Host ('FFI golden verified: ' + $file + ' sha256=' + $digest.ToLowerInvariant())
    }
} finally {
    if ($created) {
        $resolved = [IO.Path]::GetFullPath($owned)
        if ([IO.Path]::GetDirectoryName($resolved) -cne $temporaryParent -or [IO.Path]::GetFileName($resolved) -cne $name) { throw 'Golden cleanup containment failed' }
        $entries = @((Get-Item -LiteralPath $resolved -Force)) + @(Get-ChildItem -LiteralPath $resolved -Recurse -Force)
        foreach ($entry in $entries) {
            if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or -not ([IO.Path]::GetFullPath($entry.FullName) -eq $resolved -or [IO.Path]::GetFullPath($entry.FullName).StartsWith($resolved + '\', [StringComparison]::OrdinalIgnoreCase))) { throw 'Golden cleanup refused a redirected path' }
        }
        Remove-Item -LiteralPath $resolved -Recurse -Force
        $cleanup = -not (Test-Path -LiteralPath $resolved)
        if (-not $cleanup) { throw 'Golden cleanup is unconfirmed' }
    }
    Write-Host ('FFI golden private project disposal confirmed: ' + $cleanup)
}
