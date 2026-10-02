param([switch]$RequireBuildReady)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
$source = Invoke-VwProcess -FilePath python.exe -ArgumentList @('drivers/sudovda/check_source.py') -WorkingDirectory $projectRoot -Phase 't008-source-binding' -TimeoutSeconds 60 -Capture
if($source.ExitCode -ne 0) { throw 'Vendored source binding failed' }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$vs = & $vswhere -latest -products '*' -property installationPath
if($LASTEXITCODE -ne 0 -or -not $vs) { throw 'Visual Studio Build Tools not found' }
$compilerDirectories = @(Get-ChildItem -LiteralPath (Join-Path $vs 'VC/Tools/MSVC') -Directory | Sort-Object Name -Descending)
if($compilerDirectories.Count -eq 0) { throw 'MSVC toolchain not found' }
$compiler = $compilerDirectories[0]
$spectre = Test-Path -LiteralPath (Join-Path $compiler.FullName 'lib/spectre/x64/libcmt.lib')
$packages = Get-Content -LiteralPath (Join-Path $projectRoot '.local/cache/nuget/receipt.json') -Raw | ConvertFrom-Json
$packageResults = @()
foreach($package in $packages) {
    if($package.Version -ne '10.0.28000.2526' -or $package.Package -notin @('Microsoft.Windows.WDK.x64','Microsoft.Windows.SDK.CPP','Microsoft.Windows.SDK.CPP.x64')) { throw 'Unexpected cached package' }
    $path = Join-Path $projectRoot ('.local/cache/nuget/' + $package.Package + '.' + $package.Version + '.nupkg')
    if((Get-TransportHash $path) -ne $package.SHA256) { throw 'NuGet cache hash mismatch' }
    $packageResults += [ordered]@{name=$package.Package;version=$package.Version;sha256=$package.SHA256}
}
if(@($packageResults.name | Sort-Object -Unique).Count -ne 3) { throw 'Three matching cached packages required' }
$secureBoot=$null; $secureBootError=$null
try { $secureBoot=[bool](Confirm-SecureBootUEFI) } catch { $secureBootError=('0x{0:x8}' -f ($_.Exception.HResult -band 0xffffffffL)) }
$hvci=$null; $hvciError=$null
try { $guard=Get-CimInstance -Namespace 'root/Microsoft/Windows/DeviceGuard' -ClassName Win32_DeviceGuard; $hvci=(@($guard.SecurityServicesRunning) -contains 2) } catch { $hvciError=('0x{0:x8}' -f ($_.Exception.HResult -band 0xffffffffL)) }
$restoreCount=$null; $restoreError=$null
try { $restoreCount=@(Get-ComputerRestorePoint -ErrorAction Stop).Count } catch { $restoreError=('0x{0:x8}' -f ($_.Exception.HResult -band 0xffffffffL)) }
$sourceManifest=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'upstream.json') -Raw | ConvertFrom-Json
$result=[ordered]@{
    schema=1; task='T0.08'; captured_utc=[DateTime]::UtcNow.ToString('o'); source_commit=$sourceManifest.commit
    source_files_verified=@($sourceManifest.files.PSObject.Properties).Count; complete_vendor=$sourceManifest.complete_vendor
    msvc=$compiler.Name; spectre_x64_present=$spectre; packages=$packageResults
    secure_boot=$secureBoot; secure_boot_error=$secureBootError; hvci_running=$hvci; hvci_error=$hvciError
    restore_point_count=$restoreCount; restore_point_error=$restoreError; machine_changes_performed=$false
}
$result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $projectRoot '.local/t008-preflight.json') -Encoding UTF8
$result | ConvertTo-Json -Depth 6
if($RequireBuildReady -and (-not $spectre -or -not $sourceManifest.complete_vendor)) {
    throw 'Driver build blocked: install C++ Spectre-mitigated libraries for x64/x86 (Latest MSVC) and resolve the recorded EDID provenance gap. No compiler, signing, or installation was run.'
}
