#requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Root,
    [Parameter(Mandatory)][string]$ToolchainManifest,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-f]{64}$')][string]$ToolchainSha256,
    [Parameter(Mandatory)][string]$WorkDirectory,
    [string]$Catalog,
    [ValidatePattern('^[0-9a-f]{64}$')][string]$CatalogSha256,
    [ValidateRange(1, 3600)][int]$TimeoutSeconds = 600
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'MSI production requires Windows.' }

function Get-MsiAbsolutePath([string]$Value) {
    if ($Value -cnotmatch '^[A-Za-z]:\\' -or $Value.Substring(2).Contains(':') -or
        $Value.Contains([char]0) -or $Value.Contains(';') -or $Value -match '[\\/]\.\.?([\\/]|$)') {
        throw 'MSI paths must be explicit ordinary absolute local paths.'
    }
    return [IO.Path]::GetFullPath($Value)
}
function Assert-MsiRelativePath([string]$Value) {
    if ($Value -cnotmatch '^[ -~]{1,512}$' -or $Value -match '[\\:<>"|?*]' -or
        $Value.StartsWith('/') -or @($Value.Split('/') | Where-Object {
            $_ -in @('', '.', '..') -or $_.EndsWith(' ') -or $_.EndsWith('.')
        }).Count -gt 0) { throw 'MSI relative path refused.' }
}
function Read-MsiJson([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or $item.Length -gt 8MB) { throw 'MSI JSON size/type refused.' }
    return [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::new($false, $true)) | ConvertFrom-Json -AsHashtable
}
function Write-MsiJson([string]$Path, [System.Collections.IDictionary]$Value) {
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes(($Value | ConvertTo-Json -Depth 40) + "`n")
    if ($bytes.Length -gt 8MB) { throw 'MSI receipt size refused.' }
    $stream = [IO.File]::Open($Path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
    try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}

$Root = Get-MsiAbsolutePath $Root
$WorkDirectory = Get-MsiAbsolutePath $WorkDirectory
$scriptRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
if (-not [StringComparer]::OrdinalIgnoreCase.Equals($Root.TrimEnd('\'), $scriptRoot.TrimEnd('\'))) {
    throw 'MSI root must contain this reviewed wrapper.'
}
Assert-MsiRelativePath $ToolchainManifest
if ([bool]$Catalog -ne [bool]$CatalogSha256) { throw 'MSI catalog requires an exact path/hash pair.' }
if ($Catalog) { Assert-MsiRelativePath $Catalog }
if (Test-Path -LiteralPath $WorkDirectory) { throw 'MSI work directory must be fresh; preserve earlier work.' }

Import-Module (Join-Path $Root 'tools/desktop-test/lease.psm1') -Force
Import-Module (Join-Path $Root 'tools/process.psm1') -Force
$lease = [VwDesktopLease]::new()
$pinned = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
function Pin-MsiPath([string]$Path, [bool]$Directory) {
    $Path = Get-MsiAbsolutePath $Path
    $key = $Path.TrimEnd('\')
    if ($pinned.Contains($key)) { return }
    $chain = [Collections.Generic.List[string]]::new()
    $cursor = if ($Directory) { [IO.DirectoryInfo]::new($Path).Parent } else { [IO.FileInfo]::new($Path).Directory }
    while ($null -ne $cursor -and -not $pinned.Contains($cursor.FullName.TrimEnd('\'))) {
        $chain.Add($cursor.FullName); $cursor = $cursor.Parent
    }
    for ($i = $chain.Count - 1; $i -ge 0; $i--) {
        $lease.Pin($chain[$i], $true); [void]$pinned.Add($chain[$i].TrimEnd('\'))
    }
    $lease.Pin($Path, $Directory); [void]$pinned.Add($key)
}
function Assert-MsiFile([string]$Path, [System.Collections.IDictionary]$Expected) {
    Pin-MsiPath $Path $false
    $item = Get-Item -LiteralPath $Path -Force
    if ($Expected.bytes -isnot [long] -and $Expected.bytes -isnot [int]) { throw 'MSI file size descriptor refused.' }
    if ($item.Length -ne $Expected.bytes -or $Expected.sha256 -cnotmatch '^[0-9a-f]{64}$' -or
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Expected.sha256) {
        throw 'MSI pinned file binding changed.'
    }
}
function Assert-MsiPythonLayout([System.Collections.IDictionary]$Python,
    [Collections.Generic.HashSet[string]]$Names) {
    $pythonRoot = Get-MsiAbsolutePath $Python.root
    $version = [regex]::Match([string]$Python.version, '^3\.(1[0-9]|[2-9][0-9])\.[0-9A-Za-z.+_-]{1,80}$')
    if ($Python.version -isnot [string] -or -not $version.Success) {
        throw 'MSI Python version refused.'
    }
    $minor = $version.Groups[1].Value
    # This bootstrap profile is a complete base Windows Python installation.
    # Refuse alternate path initialization before any Python code executes.
    if ($Python.roles.python -cne 'python.exe' -or -not $Names.Contains('python.exe') -or
        -not $Names.Contains(('python3' + $minor + '.dll')) -or
        (-not $Names.Contains('Lib/os.py') -and -not $Names.Contains(('python3' + $minor + '.zip')))) {
        throw 'MSI Python base layout refused.'
    }
    foreach ($name in $Names) {
        if ([IO.Path]::GetFileName($name) -ieq 'pyvenv.cfg' -or $name.EndsWith('._pth', [StringComparison]::OrdinalIgnoreCase)) {
            throw 'MSI Python alternate search path refused.'
        }
    }
    $parent = [IO.DirectoryInfo]::new($pythonRoot).Parent
    if ($null -ne $parent -and (Test-Path -LiteralPath (Join-Path $parent.FullName 'pyvenv.cfg'))) {
        throw 'MSI Python parent virtual environment refused.'
    }
}
function Assert-MsiPythonInventory([System.Collections.IDictionary]$Python) {
    $pythonRoot = Get-MsiAbsolutePath $Python.root
    Pin-MsiPath $pythonRoot $true
    $expected = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    if ($Python.files.Count -lt 2 -or $Python.files.Count -gt 24000) { throw 'MSI Python inventory count refused.' }
    foreach ($file in $Python.files) {
        Assert-MsiRelativePath $file.path
        if (-not $expected.Add($file.path)) { throw 'MSI Python inventory alias refused.' }
        Assert-MsiFile (Join-Path $pythonRoot $file.path) $file
    }
    Assert-MsiPythonLayout $Python $expected
    # Bootstrap code runs only after every declared Python file is held. Check
    # the complete observed namespace as well; these directory leases do not
    # exclude transient child creation by another process with write access.
    $queue = [Collections.Generic.Queue[string]]::new(); $queue.Enqueue($pythonRoot)
    $found = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $count = 0
    while ($queue.Count -gt 0) {
        foreach ($item in Get-ChildItem -LiteralPath $queue.Dequeue() -Force) {
            $count++; if ($count -gt 32768) { throw 'MSI Python namespace bound refused.' }
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'MSI Python redirect refused.' }
            if ($item.PSIsContainer) { Pin-MsiPath $item.FullName $true; $queue.Enqueue($item.FullName) }
            else {
                $relative = [IO.Path]::GetRelativePath($pythonRoot, $item.FullName).Replace('\', '/')
                if (-not $found.Add($relative) -or -not $expected.Contains($relative)) { throw 'MSI Python namespace differs.' }
            }
        }
    }
    if (-not $found.SetEquals($expected) -or $expected.Contains('pyvenv.cfg')) { throw 'MSI Python closure refused.' }
    Assert-MsiRelativePath $Python.roles.python
    if ([IO.Path]::GetFileName($Python.roles.python) -cne 'python.exe' -or
        -not $expected.Contains($Python.roles.python)) { throw 'MSI Python role refused.' }
    return Join-Path $pythonRoot $Python.roles.python
}

try {
    $manifestPath = Join-Path $Root $ToolchainManifest
    Pin-MsiPath $manifestPath $false
    if ((Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ToolchainSha256) {
        throw 'MSI toolchain manifest hash differs.'
    }
    $toolchain = Read-MsiJson $manifestPath
    if ($toolchain.schema -cne 'vw-msi-toolchain-v1') { throw 'MSI toolchain schema refused.' }
    $python = Assert-MsiPythonInventory $toolchain.python
    $backend = Join-Path $Root 'tools/release/msi.py'
    foreach ($path in @($backend, $PSCommandPath, (Join-Path $Root 'tools/release/assemble.py'),
        (Join-Path $Root 'tools/desktop-test/reports.py'), (Join-Path $Root 'tools/desktop-test/personal_package.py'),
        (Join-Path $Root 'tools/ffi-test/check_apk.py'), (Join-Path $Root 'tools/process.psm1'),
        (Join-Path $Root 'tools/desktop-test/lease.psm1'))) { Pin-MsiPath $path $false }

    # OS paths come from the Windows special-folder API, never PATH/ComSpec or
    # inherited JVM options. System DLL/.NET/OS provenance remains a host gate.
    $windows = Get-MsiAbsolutePath ([Environment]::GetFolderPath([Environment+SpecialFolder]::Windows))
    $system32 = Join-Path $windows 'System32'
    Pin-MsiPath $system32 $true
    $osEnvironment = [ordered]@{ SystemRoot = $windows; WINDIR = $windows; ComSpec = (Join-Path $system32 'cmd.exe') }
    $pythonEnvironment = [ordered]@{ SystemRoot = $windows; WINDIR = $windows; ComSpec = $osEnvironment.ComSpec;
        PATH = ((Split-Path -Parent $python) + ';' + $system32) }
    function Invoke-MsiBackend([string[]]$Arguments, [string]$Phase) {
        $result = Invoke-VwProcess -FilePath $python -ArgumentList (@('-I', '-S', '-B', $backend) + $Arguments) `
            -Environment $pythonEnvironment -WorkingDirectory $Root -Phase $Phase -TimeoutSeconds 120 `
            -RedactValues @($Root, $WorkDirectory, $toolchain.python.root, $toolchain.jdk.root, $toolchain.wix.root)
        $proof = Read-MsiJson ($result.LogPath + '.json')
        if ($result.ExitCode -ne 0 -or $proof.exit_code -ne 0 -or $proof.parent_exit_code -ne 0 -or
            $proof.contained_in_windows_job -ne $true -or $proof.process_tree_cleanup_confirmed -ne $true -or
            $proof.output_streams_completed -ne $true -or $proof.failure_reason -or
            $proof.surviving_processes_before_cleanup.Count -ne 0) { throw 'MSI backend or process cleanup refused.' }
    }

    $arguments = @('plan', '--root', $Root, '--toolchain', $ToolchainManifest,
        '--toolchain-sha256', $ToolchainSha256, '--work', $WorkDirectory, '--timeout-seconds', [string]$TimeoutSeconds)
    if ($Catalog) { $arguments += @('--catalog', $Catalog, '--catalog-sha256', $CatalogSha256) }
    Invoke-MsiBackend $arguments 'desktop-msi-plan'
    $planPath = Join-Path $WorkDirectory 'plan.json'
    Pin-MsiPath $planPath $false
    $planSha = (Get-FileHash -LiteralPath $planPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $plan = Read-MsiJson $planPath
    foreach ($inputPath in $plan.lease_inputs) { Pin-MsiPath $inputPath.path $inputPath.directory }
    Invoke-MsiBackend @('check', '--root', $Root, '--plan', $planPath, '--plan-sha256', $planSha) 'desktop-msi-check'

    $childEnvironment = [ordered]@{}
    foreach ($entry in $plan.minimal_env.GetEnumerator()) { $childEnvironment.Add($entry.Key, [string]$entry.Value) }
    foreach ($entry in $osEnvironment.GetEnumerator()) { $childEnvironment.Add($entry.Key, $entry.Value) }
    $childEnvironment.PATH += ';' + $system32
    $result = Invoke-VwProcess -FilePath $plan.executable -ArgumentList @($plan.argv) -Environment $childEnvironment `
        -WorkingDirectory $plan.working_dir -Phase 'desktop-msi' -TimeoutSeconds $plan.timeout_seconds `
        -RedactValues @($Root, $WorkDirectory, $toolchain.python.root, $toolchain.jdk.root, $toolchain.wix.root)
    $runner = Read-MsiJson ($result.LogPath + '.json')
    $processPath = Join-Path $WorkDirectory 'process.json'
    Write-MsiJson $processPath ([ordered]@{ schema = 'vw-msi-process-v1'; run_nonce = $plan.run_nonce;
        plan_sha256 = $planSha; executable = $plan.executable; argv = @($plan.argv); working_dir = $plan.working_dir;
        minimal_env = $plan.minimal_env; os_env = [ordered]@{ SystemRoot = $windows; WINDIR = $windows;
            ComSpec = $osEnvironment.ComSpec; PATH = $childEnvironment.PATH }; runner = $runner })
    Pin-MsiPath $processPath $false
    if ($result.ExitCode -ne 0) { throw 'MSI build failed; retain the work and bounded process log.' }
    Invoke-MsiBackend @('record', '--root', $Root, '--plan', $planPath, '--plan-sha256', $planSha,
        '--process', $processPath) 'desktop-msi-record'
    Write-Host 'MSI build receipt recorded; payload, installation, signatures and product acceptance remain separate.'
} finally {
    # Failed owned work is retained. No recursive path cleanup or install runs.
    $lease.Dispose()
}
