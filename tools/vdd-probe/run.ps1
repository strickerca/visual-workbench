param([ValidateSet('inventory','normal','watchdog')][string]$Scenario='inventory', [switch]$OwnerReady)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$projectRoot=(Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
if($Scenario -ne 'inventory' -and -not $OwnerReady) { throw 'Reserve this desktop and dedicated SudoVDA driver before passing -OwnerReady. Its watchdog is shared.' }
$executable=Join-Path $projectRoot 'target/release/vdd-probe.exe'
if(-not (Test-Path -LiteralPath $executable)) { throw 'Run build.ps1 build-vdd-probe first' }
$binding=Invoke-VwProcess -FilePath python.exe -ArgumentList @('tools/vdd-probe/build_receipt.py','check') -WorkingDirectory $projectRoot -Phase 't008-probe-build-binding' -TimeoutSeconds 30
if($binding.ExitCode -ne 0) { throw 'Probe build binding failed' }
$arguments=@($Scenario)
if($OwnerReady) { $arguments += '--owner-ready' }
$result=Invoke-VwProcess -FilePath $executable -ArgumentList $arguments -WorkingDirectory $projectRoot -Phase "t008-vdd-$Scenario" -TimeoutSeconds 90 -Capture
$receipt=[ordered]@{schema=1;scenario=$Scenario;binary_sha256=(Get-TransportHash $executable);exit_code=$result.ExitCode;elapsed_seconds=$result.ElapsedSeconds;log=[IO.Path]::GetFileName($result.LogPath);output=@($result.Lines)}
$receipt | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $projectRoot ".local/t008-probe-$Scenario.json") -Encoding UTF8
$result.Lines | Write-Output
if($result.ExitCode -ne 0) { throw 'Virtual display probe failed; inspect the retained text receipt' }
