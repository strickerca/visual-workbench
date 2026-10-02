param([ValidatePattern('^[a-z0-9-]+$')][string]$FixtureSet='t009-v2')
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
$verified=Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/image/fetch_vips.py') -WorkingDirectory $projectRoot -Phase 'image-cli-runtime-binding' -TimeoutSeconds 180
if ($verified.ExitCode -ne 0){throw 'libvips runtime binding failed'}
$tool=Get-Content -Raw -LiteralPath (Join-Path $projectRoot '.local/t009-vips-tool.json') | ConvertFrom-Json
if ((Get-TransportHash $tool.executable) -cne $tool.executable_sha256){throw 'libvips executable binding changed'}
$runId=[guid]::NewGuid().ToString('N')
$receipt='.local/image-pc-'+$runId+'.json'
$result=Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/image/pc.py','--vips',$tool.executable,'--fixtures',('fixtures/generated/'+$FixtureSet),'--receipt',$receipt) -WorkingDirectory $projectRoot -Phase 'image-pc' -TimeoutSeconds 600
if ($result.ExitCode -ne 0){throw 'Image PC measurement failed; retained receipt identifies completed operations'}
Write-Host ('Image PC measurements: '+$receipt)
