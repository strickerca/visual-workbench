param([ValidateSet('smoke','full')][string]$Profile='full')
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
Import-Module (Join-Path $projectRoot 'tools/process.psm1')
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
$check=Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/video-pc/build_receipt.py','check','pc') -WorkingDirectory $projectRoot -Phase 'video-pc-build-binding' -TimeoutSeconds 30
if ($check.ExitCode -ne 0){throw 'Native video build is stale'}
$id=[guid]::NewGuid().ToString('N')
$directory=[IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ('VW-video-'+$id)))
$record=Join-Path $projectRoot ('.local/video-run-'+$id+'.json')
$exe=Join-Path $projectRoot 'target/release/video-pc.exe'
$buildInputs=(Get-Content -Raw -Encoding UTF8 -LiteralPath (Join-Path $projectRoot '.local/video-build-pc.json') | ConvertFrom-Json).sha256
$metadata=[ordered]@{schema=1;run_id=$id;media_directory=$directory;profile=$Profile;pc_sha256=(Get-TransportHash $exe);build_inputs_sha256=$buildInputs;runner_sha256=(Get-TransportHash $PSCommandPath);pc_exit_code=$null;media_disposed=$false}
$metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $record -Encoding UTF8
if (Test-Path -LiteralPath $directory){throw 'Video run must use a fresh media directory'}
[IO.Directory]::CreateDirectory($directory) | Out-Null
$reports=@()
$workers=@()
$failed=$false
foreach($phase in @('portrait','4k','raster-10','raster-25','raster-4k')) {
    $output=Join-Path $directory $phase
    $result=Invoke-VwProcess -FilePath $exe -ArgumentList @($output,$Profile,$phase) -WorkingDirectory $projectRoot -Phase ('video-pc-'+$Profile+'-'+$phase) -TimeoutSeconds 180 -RedactValues @($projectRoot,$env:USERPROFILE)
    $workers+=@{phase=$phase;exit_code=$result.ExitCode}
    $reportPath=Join-Path $output 'report.json'
    if(Test-Path -LiteralPath $reportPath){
        $report=Get-Content -Raw -Encoding UTF8 -LiteralPath $reportPath | ConvertFrom-Json
        $reports+=@($report.results)
        if(-not $report.completed){$failed=$true}
    } else {$failed=$true; $reports+=@{phase=$phase;error='Worker did not finalize a phase report'}}
    if($result.ExitCode -ne 0){$failed=$true}
    # Save after every worker so later native faults cannot erase prior results.
    @{schema=1;completed=($phase -eq 'raster-4k' -and -not $failed);profile=$Profile;isolated_profile_processes=$true;worker_exits=$workers;results=$reports} | ConvertTo-Json -Depth 18 | Set-Content -LiteralPath (Join-Path $directory 'report.json') -Encoding UTF8
}
$metadata.pc_exit_code=if($failed){1}else{0}
$metadata | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $record -Encoding UTF8
Write-Host ('Video run ID: '+$id+'. Owned media is outside Git; inspect it and dispose after decoder use.')
if ($failed){throw 'A native video phase failed; retain its numeric report and inspect the specific phase'}
