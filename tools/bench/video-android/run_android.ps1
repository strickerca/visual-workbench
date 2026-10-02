param([Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{32}$')][string]$RunId)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/process.psm1')
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
$check=Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/video-pc/build_receipt.py','check','android') -WorkingDirectory $projectRoot -Phase 'video-decoder-build-binding' -TimeoutSeconds 30
if ($check.ExitCode -ne 0){throw 'Decoder APK is stale'}
$record=Get-Content -Raw -LiteralPath (Join-Path $projectRoot ('.local/video-run-'+$RunId+'.json')) | ConvertFrom-Json
$media=[IO.Path]::GetFullPath($record.media_directory)
$expected=[IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ('VW-video-'+$RunId)))
if ($record.run_id -cne $RunId -or $media -cne $expected -or $record.media_disposed){throw 'Media ownership mismatch'}
$device=Get-VwAndroidDevice -Root $projectRoot -ExpectedModel 'SM-S918U'
$decoderRunId=[guid]::NewGuid().ToString('N')
$package='com.visualworkbench.videobench'
$remote='/sdcard/Android/data/'+$package+'/files/vw-video-'+$decoderRunId
$apk=Join-Path $PSScriptRoot 'build/outputs/apk/debug/video-bench-debug.apk'
$output=Join-Path $projectRoot ('.local/video-decoder-'+$RunId+'-'+$decoderRunId)
if (Test-Path -LiteralPath $output){throw 'Decoder output must be a fresh run'}
[IO.Directory]::CreateDirectory($output) | Out-Null
$cleanup=$false
$completed=$false
try {
    Invoke-VwAdb -Device $device -Arguments @('install','-r',$apk) -Phase 'video-decoder-install' -TimeoutSeconds 120 | Out-Null
    Invoke-VwAdb -Device $device -Arguments @('shell','am','start','-W','-n',($package+'/.VideoActivity'),'--es','run_id',$decoderRunId,'--es','mode','prepare') -Phase 'video-decoder-prepare' -TimeoutSeconds 15 | Out-Null
    $ready=Invoke-VwAdb -Device $device -Arguments @('shell','run-as',$package,'cat',('files/ready-'+$decoderRunId+'.json')) -Phase 'video-decoder-storage-ready' -TimeoutSeconds 15 -Capture
    if (-not (($ready.Lines -join "`n") | ConvertFrom-Json).ready){throw 'Decoder storage readiness failed'}
    foreach ($fixture in @('portrait','4k')) {
        $source=Join-Path $media $fixture
        $index=Get-Content -Raw -LiteralPath (Join-Path $source 'index.json') | ConvertFrom-Json
        if (-not $index.completed -or $index.stream_file -cne 'stream.h265'){throw 'Native encoder did not complete this fixture'}
        Invoke-VwAdb -Device $device -Arguments @('shell','mkdir',($remote+'/'+$fixture)) -Phase ('video-'+$fixture+'-directory') | Out-Null
        foreach ($name in @('index.json','stream.h265')) {
            Invoke-VwAdb -Device $device -Arguments @('push',(Join-Path $source $name),($remote+'/'+$fixture+'/'+$name)) -Phase ('video-'+$fixture+'-'+$name.Replace('.','-')+'-transfer') -TimeoutSeconds 60 | Out-Null
        }
        Invoke-VwAdb -Device $device -Arguments @('shell','am','start','-W','-n',($package+'/.VideoActivity'),'--es','run_id',$decoderRunId,'--es','mode','decode','--es','fixture',$fixture) -Phase ('video-'+$fixture+'-decode-start') -TimeoutSeconds 15 | Out-Null
        $foreground=Invoke-VwAdb -Device $device -Arguments @('shell','dumpsys','activity','activities') -Phase ('video-'+$fixture+'-foreground') -TimeoutSeconds 15 -Capture
        $resumed=@($foreground.Lines | Where-Object {$_ -match '(?:mResumedActivity|topResumedActivity)'}) -join "`n"
        if ($resumed -notmatch 'com\.visualworkbench\.videobench/\.VideoActivity'){throw 'Decoder did not retain foreground; inspect lifecycle, do not wait on launcher'}
        $resultName='result-'+$decoderRunId+'-'+$fixture+'.json'
        $watch=[Diagnostics.Stopwatch]::StartNew()
        $receipt=$null
        while ($watch.Elapsed.TotalSeconds -lt 90) {
            $pollCommand='run-as '+$package+" sh -c 'if [ -f files/"+$resultName+' ]; then cat files/'+$resultName+"; else echo PENDING; fi'"
            $candidate=Invoke-VwAdb -Device $device -Arguments @('shell',$pollCommand) -Phase ('video-'+$fixture+'-result-poll') -TimeoutSeconds 10 -Capture
            $value=($candidate.Lines -join "`n").Trim()
            if ($value -ne 'PENDING'){$receipt=$value | ConvertFrom-Json; break}
            Write-Host ('Decoder '+$fixture+' progressing: '+[int]$watch.Elapsed.TotalSeconds+'s / 90s')
            Start-Sleep -Seconds 2
        }
        if (-not $receipt){throw 'Decoder result timed out; inspect project app process/lifecycle before retry'}
        $receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $output ($fixture+'.json')) -Encoding UTF8
        if (-not $receipt.completed -or $receipt.input_frames -ne $index.samples.Count -or $receipt.output_frames -ne $index.samples.Count -or -not $receipt.all_render_callbacks_observed){throw 'Decoder did not account for every expected decoded/rendered frame'}
        Write-Host ('S23 '+$fixture+' decode PASS: '+$receipt.output_frames+' frames; queue/output p95 '+[math]::Round($receipt.queue_to_output_ms.p95,3)+' ms')
    }
    $completed=$true
} finally {
    # Stop only this diagnostic app before removing its exact UUID-owned files.
    try {
        Invoke-VwAdb -Device $device -Arguments @('shell','am','force-stop',$package) -Phase 'video-decoder-owned-stop' -TimeoutSeconds 15 | Out-Null
        if ($remote -cne ('/sdcard/Android/data/'+$package+'/files/vw-video-'+$decoderRunId)){throw 'Invalid remote cleanup target'}
        Invoke-VwAdb -Device $device -Arguments @('shell','rm','-rf',$remote) -Phase 'video-decoder-media-cleanup' -TimeoutSeconds 15 | Out-Null
        Invoke-VwAdb -Device $device -Arguments @('shell',('[ ! -e '+$remote+' ]')) -Phase 'video-decoder-media-cleanup-verify' -TimeoutSeconds 15 -Capture | Out-Null
        Invoke-VwAdb -Device $device -Arguments @('shell','run-as',$package,'rm','-f',('files/ready-'+$decoderRunId+'.json'),('files/result-'+$decoderRunId+'-portrait.json'),('files/result-'+$decoderRunId+'-4k.json')) -Phase 'video-decoder-private-receipts-cleanup' -TimeoutSeconds 15 | Out-Null
        $cleanup=$true
    } catch { Write-Warning 'Owned decoder cleanup could not be confirmed' }
    [ordered]@{schema=1;pc_run_id=$RunId;decoder_run_id=$decoderRunId;device_model=$device.Model;apk_sha256=(Get-TransportHash $apk);completed=$completed;owned_device_cleanup_confirmed=$cleanup;screenshots_created=0} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'binding.json') -Encoding UTF8
}
if (-not $cleanup){throw 'Decoder cleanup incomplete'}
