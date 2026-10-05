param([string]$ModulePath=(Join-Path $PSScriptRoot 'core-unit.ps1'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. $ModulePath
$fixtureRoot='C:\fixture-workbench'
$base=@{reason='compiler-artifact';manifest_path='[redacted]/core/crates/vw-ffi/Cargo.toml';target=@{name='vw_core';kind=@('cdylib','rlib');crate_types=@('cdylib','rlib');edition='2024';src_path='[redacted]/core/crates/vw-ffi/src/lib.rs'};profile=@{test=$true};executable='[redacted]/target/debug/deps/vw_core-0123456789abcdef.exe';filenames=@('[redacted]/target/debug/deps/vw_core-0123456789abcdef.exe')}
$complete='{"reason":"build-finished","success":true}'
$passed=0
function Copy-CoreUnitRecord {($base|ConvertTo-Json -Depth 8)|ConvertFrom-Json}
function Assert-CoreUnitRefused([string]$Name,[string[]]$Lines){$refused=$false;try{Resolve-VwCoreUnitArtifact -Lines $Lines -ProjectRoot $fixtureRoot|Out-Null}catch{$refused=$true};if(-not $refused){throw $Name};$script:passed++}
function Assert-CoreUnitAdmitted([string]$Name,[string[]]$Lines){$actual=Resolve-VwCoreUnitArtifact -Lines $Lines -ProjectRoot $fixtureRoot;if($actual -cne (Join-Path $fixtureRoot 'target/debug/deps/vw_core-0123456789abcdef.exe')){throw $Name};$script:passed++}
$valid=$base|ConvertTo-Json -Compress -Depth 8
Assert-CoreUnitAdmitted 'exact-redacted-artifact' @($valid,$complete)
$r=Copy-CoreUnitRecord;$r.manifest_path=$r.manifest_path.Replace('[redacted]',$fixtureRoot.Replace('\','/'));$r.target.src_path=$r.target.src_path.Replace('[redacted]',$fixtureRoot.Replace('\','/'));$r.executable=$r.executable.Replace('[redacted]',$fixtureRoot.Replace('\','/'));$r.filenames=@($r.executable)
Assert-CoreUnitAdmitted 'exact-unredacted-artifact' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
Assert-CoreUnitAdmitted 'cargo-progress-is-not-an-artifact' @('Compiling vw-ffi',$valid,$complete)
Assert-CoreUnitRefused 'missing-artifact' @($complete)
Assert-CoreUnitRefused 'duplicate-artifact' @($valid,$valid,$complete)
Assert-CoreUnitRefused 'missing-finish' @($valid)
Assert-CoreUnitRefused 'duplicate-finish' @($valid,$complete,$complete)
Assert-CoreUnitRefused 'failed-finish' @($valid,'{"reason":"build-finished","success":false}')
Assert-CoreUnitRefused 'nonboolean-finish' @($valid,'{"reason":"build-finished","success":"true"}')
Assert-CoreUnitRefused 'malformed-json' @('{broken',$valid,$complete)
Assert-CoreUnitRefused 'oversize-line' @(('x'*524289),$valid,$complete)
$r=Copy-CoreUnitRecord;$r.target.name='vw_host';Assert-CoreUnitRefused 'wrong-target' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.profile.test=$false;Assert-CoreUnitRefused 'ordinary-library' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.profile.test='true';Assert-CoreUnitRefused 'nonboolean-test' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.manifest_path='[redacted]/host-win/crates/vw-host-ffi/Cargo.toml';Assert-CoreUnitRefused 'wrong-manifest' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.target.src_path='[redacted]/core/crates/vw-ffi/src/bin/bindgen.rs';Assert-CoreUnitRefused 'wrong-source' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.target.edition='2021';Assert-CoreUnitRefused 'wrong-edition' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.target.kind=@('bin');Assert-CoreUnitRefused 'binary-target' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.target.crate_types=@('rlib','rlib');Assert-CoreUnitRefused 'duplicate-types' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
foreach($name in @('vw_core-old.exe','vw_core-0123456789abcdef.exe/../other.exe','vw_core-0123456789abcdef.exe:stream','vw_core-0123456789ABCDEF.exe')){$r=Copy-CoreUnitRecord;$r.executable='[redacted]/target/debug/deps/'+$name;$r.filenames=@($r.executable);Assert-CoreUnitRefused 'invalid-executable-name' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)}
$r=Copy-CoreUnitRecord;$r.executable='[redacted]/target/other/vw_core-0123456789abcdef.exe';$r.filenames=@($r.executable);Assert-CoreUnitRefused 'outside-deps' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.filenames=@();Assert-CoreUnitRefused 'unlisted-executable' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
$r=Copy-CoreUnitRecord;$r.filenames=@($r.executable,$r.executable);Assert-CoreUnitRefused 'duplicate-listed-executable' @(($r|ConvertTo-Json -Compress -Depth 8),$complete)
if($passed -ne 26){throw "Unexpected parser case census: $passed"}
Write-Host "core-unit-parser: $passed cases passed; no native/compiler execution"
