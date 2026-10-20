#requires -Version 7.2
# Windows fixtures: source validation and two short owned PowerShell children.
# No jpackage, installer, application, capture, provider or device is run.
[CmdletBinding()]
param([string]$Root = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'MSI wrapper fixtures require Windows.' }
$Root = [IO.Path]::GetFullPath($Root)
$wrapper = Join-Path $Root 'tools/release/build-msi.ps1'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($wrapper, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw 'MSI wrapper syntax failed.' }
foreach ($name in @('Get-MsiAbsolutePath', 'Assert-MsiRelativePath', 'Assert-MsiPythonLayout')) {
    $definitions = @($ast.FindAll({ param($node)
        $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name
    }, $true))
    if ($definitions.Count -ne 1) { throw 'Expected exactly one production bootstrap function.' }
    . ([ScriptBlock]::Create($definitions[0].Extent.Text))
}
foreach ($path in @((Join-Path $Root 'build.ps1'), (Join-Path $Root 'tools/process.psm1'), $PSCommandPath)) {
    $tokens = $null; $errors = $null
    [void][Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
    if ($errors.Count -ne 0) { throw 'MSI dependency syntax failed.' }
}
$script:cases = 0
function Check-Case([string]$Name, [scriptblock]$Body, [bool]$Reject) {
    $failed = $false
    try { & $Body | Out-Null } catch { $failed = $true }
    if ($failed -ne $Reject) { throw ('MSI wrapper fixture failed: ' + $Name) }
    $script:cases++; Write-Host ('PASS MSI wrapper fixture: ' + $Name)
}
function Names([string[]]$Values) {
    $set = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($value in $Values) { [void]$set.Add($value) }
    return ,$set
}

$owned = Join-Path ([IO.Path]::GetTempPath()) ('vw-msi-wrapper-' + [guid]::NewGuid().ToString('N'))
if (Test-Path -LiteralPath $owned) { throw 'Fixture requires fresh storage.' }
[void][IO.Directory]::CreateDirectory($owned)
$pythonRoot = Join-Path $owned 'python'
[void][IO.Directory]::CreateDirectory($pythonRoot)
$base = @{ root = $pythonRoot; version = '3.14.3'; roles = @{ python = 'python.exe' } }
$saved = @{}
foreach ($name in @('VW_MSI_INHERITED', 'JAVA_TOOL_OPTIONS')) {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
try {
    $standard = Names @('python.exe', 'python314.dll', 'Lib/os.py')
    Check-Case 'complete base layout' { Assert-MsiPythonLayout $base $standard } $false
    Check-Case 'confined standard-library archive' {
        Assert-MsiPythonLayout $base (Names @('python.exe', 'python314.dll', 'python314.zip'))
    } $false
    Check-Case 'alternate _pth refused before interpreter' {
        Assert-MsiPythonLayout $base (Names @('python.exe', 'python314.dll', 'Lib/os.py', 'python314._pth'))
    } $true
    Check-Case 'nested virtual environment refused before interpreter' {
        Assert-MsiPythonLayout $base (Names @('python.exe', 'python314.dll', 'Lib/os.py', 'nested/pyvenv.cfg'))
    } $true
    Check-Case 'nested executable refused' {
        $value = $base.Clone(); $value.roles = @{ python = 'bin/python.exe' }
        Assert-MsiPythonLayout $value (Names @('bin/python.exe', 'python314.dll', 'Lib/os.py'))
    } $true
    Check-Case 'PATH separator in root refused' {
        $value = $base.Clone(); $value.root = $pythonRoot + ';outside'
        Assert-MsiPythonLayout $value $standard
    } $true
    Check-Case 'PATH separator in role refused' {
        $value = $base.Clone(); $value.roles = @{ python = 'bin;outside/python.exe' }
        Assert-MsiPythonLayout $value $standard
    } $true
    Check-Case 'missing runtime DLL refused' { Assert-MsiPythonLayout $base (Names @('python.exe', 'Lib/os.py')) } $true
    Check-Case 'missing standard library refused' { Assert-MsiPythonLayout $base (Names @('python.exe', 'python314.dll')) } $true
    Check-Case 'unsupported Python refused' {
        $value = $base.Clone(); $value.version = '3.9.0'; Assert-MsiPythonLayout $value $standard
    } $true
    $venv = Join-Path $owned 'pyvenv.cfg'
    $stream = [IO.File]::Open($venv, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    $stream.Dispose()
    try { Check-Case 'parent virtual environment refused' { Assert-MsiPythonLayout $base $standard } $true }
    finally { [IO.File]::Delete($venv) }

    Import-Module (Join-Path $Root 'tools/process.psm1') -Force
    $pwsh = Join-Path $PSHOME 'pwsh.exe'
    $windows = [Environment]::GetFolderPath([Environment+SpecialFolder]::Windows)
    $path = $PSHOME + ';' + (Join-Path $windows 'System32')
    $selected = [ordered]@{ SystemRoot = $windows; WINDIR = $windows; ComSpec = (Join-Path $windows 'System32/cmd.exe');
        PATH = $path; VW_MSI_SELECTED = 'exact' }
    [Environment]::SetEnvironmentVariable('VW_MSI_INHERITED', 'fixture-present', 'Process')
    [Environment]::SetEnvironmentVariable('JAVA_TOOL_OPTIONS', '-Dvw.msi.fixture=inherited', 'Process')
    $probe = @'
if ([Environment]::GetEnvironmentVariable('VW_MSI_INHERITED','Process') -or
    [Environment]::GetEnvironmentVariable('JAVA_TOOL_OPTIONS','Process') -or
    [Environment]::GetEnvironmentVariable('VW_MSI_SELECTED','Process') -cne 'exact') { exit 31 }
Write-Output 'VW_MSI_ENVIRONMENT_PASS'
'@
    Check-Case 'actual child receives explicit environment and joins' {
        $result = Invoke-VwProcess -FilePath $pwsh -ArgumentList @('-NoLogo', '-NoProfile', '-NonInteractive', '-Command', $probe) `
            -WorkingDirectory $Root -Phase 'msi-environment-fixture' -TimeoutSeconds 30 -Capture -Environment $selected
        $proof = Get-Content -LiteralPath ($result.LogPath + '.json') -Raw | ConvertFrom-Json -AsHashtable
        if ($result.ExitCode -ne 0 -or @($result.Lines).Count -ne 1 -or $result.Lines[0] -cne 'VW_MSI_ENVIRONMENT_PASS' -or
            $proof.parent_exit_code -ne 0 -or $proof.contained_in_windows_job -ne $true -or
            $proof.process_tree_cleanup_confirmed -ne $true -or $proof.output_streams_completed -ne $true) {
            throw 'Explicit environment or owned process proof failed.'
        }
        if ($env:VW_MSI_INHERITED -cne 'fixture-present' -or $env:JAVA_TOOL_OPTIONS -cne '-Dvw.msi.fixture=inherited') {
            throw 'Child environment changed parent state.'
        }
    } $false
    $legacy = @'
if ([Environment]::GetEnvironmentVariable('VW_MSI_INHERITED','Process') -cne 'fixture-present' -or
    [Environment]::GetEnvironmentVariable('JAVA_TOOL_OPTIONS','Process') -cne '-Dvw.msi.fixture=inherited') { exit 32 }
Write-Output 'VW_MSI_LEGACY_PASS'
'@
    Check-Case 'existing callers retain inherited environment' {
        $result = Invoke-VwProcess -FilePath $pwsh -ArgumentList @('-NoLogo', '-NoProfile', '-NonInteractive', '-Command', $legacy) `
            -WorkingDirectory $Root -Phase 'msi-legacy-environment-fixture' -TimeoutSeconds 30 -Capture
        $proof = Get-Content -LiteralPath ($result.LogPath + '.json') -Raw | ConvertFrom-Json -AsHashtable
        if ($result.ExitCode -ne 0 -or @($result.Lines).Count -ne 1 -or $result.Lines[0] -cne 'VW_MSI_LEGACY_PASS' -or
            $proof.parent_exit_code -ne 0 -or $proof.contained_in_windows_job -ne $true -or
            $proof.process_tree_cleanup_confirmed -ne $true -or $proof.output_streams_completed -ne $true) {
            throw 'Existing caller environment or cleanup changed.'
        }
    } $false
    Check-Case 'null explicit environment refused before process' {
        Invoke-VwProcess -FilePath $pwsh -WorkingDirectory $Root -Environment $null
    } $true
    Check-Case 'invalid environment key refused before process' {
        Invoke-VwProcess -FilePath $pwsh -WorkingDirectory $Root -Environment @{ 'INVALID=NAME' = 'fixture' }
    } $true
    Check-Case 'NUL environment value refused before process' {
        Invoke-VwProcess -FilePath $pwsh -WorkingDirectory $Root -Environment @{ VW_MSI_SELECTED = ('a' + [char]0 + 'b') }
    } $true
    Write-Host ("MSI wrapper fixtures: {0} passed; two owned PowerShell children; no MSI/app/device execution." -f $script:cases)
} finally {
    foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
    # Delete only known empty fixture directories; retain any unexpected content.
    [IO.Directory]::Delete($pythonRoot, $false)
    [IO.Directory]::Delete($owned, $false)
}
