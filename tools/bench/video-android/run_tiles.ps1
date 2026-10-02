param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-f]{32}$')][string]$RunId,
    [ValidateSet('smoke','full')][string]$Profile='full'
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/process.psm1')
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
$check=Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/video-pc/build_receipt.py','check','android') -WorkingDirectory $projectRoot -Phase 'tiles-apk-build-binding' -TimeoutSeconds 30
if ($check.ExitCode -ne 0){throw 'Tile receiver APK is stale'}
$record=Get-Content -Raw -LiteralPath (Join-Path $projectRoot ('.local/video-run-'+$RunId+'.json')) | ConvertFrom-Json
$media=[IO.Path]::GetFullPath($record.media_directory)
$expected=[IO.Path]::GetFullPath((Join-Path ([IO.Path]::GetTempPath()) ('VW-video-'+$RunId)))
if ($record.run_id -cne $RunId -or $media -cne $expected -or $record.media_disposed){throw 'Media ownership mismatch'}
$device=Get-VwAndroidDevice -Root $projectRoot -ExpectedModel 'SM-S918U'
$id=[guid]::NewGuid().ToString('N')
$package='com.visualworkbench.videobench'
$apk=Join-Path $PSScriptRoot 'build/outputs/apk/debug/video-bench-debug.apk'
$output=Join-Path $projectRoot ('.local/video-tiles-'+$id)
[IO.Directory]::CreateDirectory($output) | Out-Null
$completed=$false
$cleanup=$false
$removedMappings=0
try {
    Invoke-VwAdb -Device $device -Arguments @('install','-r',$apk) -Phase 'tiles-receiver-install' -TimeoutSeconds 120 | Out-Null
    foreach ($percent in @(10,25)) {
        $fixture='tiles-'+$percent
        $source=Join-Path (Join-Path $media ('raster-'+$percent)) ($fixture+'.vwt')
        if (-not (Test-Path -LiteralPath $source -PathType Leaf)){throw 'No generated dirty-tile recording exists'}
        $folder=Join-Path $output $fixture
        $server=$null
        $mapping=$null
        $mappingCreated=$false
        try {
            $script=Join-Path $projectRoot 'tools/bench/video-pc/serve_tiles.py'
            $server=Start-Process -FilePath 'python.exe' -ArgumentList @(('"'+$script+'"'),('"'+$source+'"'),('"'+$folder+'"'),'--profile',$Profile) -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru -RedirectStandardOutput (Join-Path $output ($fixture+'-stdout.log')) -RedirectStandardError (Join-Path $output ($fixture+'-stderr.log'))
            $null=$server.Handle
            $watch=[Diagnostics.Stopwatch]::StartNew()
            while (-not (Test-Path -LiteralPath (Join-Path $folder 'ready.json'))) {
                if ($server.HasExited -or $watch.Elapsed.TotalSeconds -gt 10){throw 'Tile server readiness failed'}
                Start-Sleep -Milliseconds 250
            }
            $ready=Get-Content -Raw -LiteralPath (Join-Path $folder 'ready.json') | ConvertFrom-Json
            if ($ready.port -lt 1024 -or $ready.port -gt 65535){throw 'Invalid loopback port'}
            $mapping='tcp:'+$ready.port
            Invoke-VwAdb -Device $device -Arguments @('reverse','--no-rebind',$mapping,$mapping) -Phase ('tiles-'+$percent+'-owned-reverse') -TimeoutSeconds 15 | Out-Null
            $mappingCreated=$true
            Invoke-VwAdb -Device $device -Arguments @('shell','am','start','-W','-n',($package+'/.VideoActivity'),'--es','run_id',$id,'--es','mode','tiles','--es','fixture',$fixture,'--ei','port',([string]$ready.port)) -Phase ('tiles-'+$percent+'-start') -TimeoutSeconds 15 | Out-Null
            $foreground=Invoke-VwAdb -Device $device -Arguments @('shell','dumpsys','activity','activities') -Phase ('tiles-'+$percent+'-foreground') -TimeoutSeconds 15 -Capture
            $resumed=@($foreground.Lines | Where-Object {$_ -match '(?:mResumedActivity|topResumedActivity)'}) -join "`n"
            if ($resumed -notmatch 'com\.visualworkbench\.videobench/\.VideoActivity'){throw 'Tile receiver is not foreground; inspect lifecycle'}
            $name='result-'+$id+'-'+$fixture+'.json'
            $watch.Restart()
            $receipt=$null
            while ($watch.Elapsed.TotalSeconds -lt 90) {
                $command='run-as '+$package+" sh -c 'if [ -f files/"+$name+' ]; then cat files/'+$name+"; else echo PENDING; fi'"
                $candidate=Invoke-VwAdb -Device $device -Arguments @('shell',$command) -Phase ('tiles-'+$percent+'-receipt') -TimeoutSeconds 10 -Capture
                $value=($candidate.Lines -join "`n").Trim()
                if ($value -ne 'PENDING'){$receipt=$value | ConvertFrom-Json;break}
                if ($server.HasExited -and $server.ExitCode -ne 0){throw 'Tile server exited with failure; inspect its phase log'}
                Write-Host ('USB tile '+$percent+'% run progressing: '+[int]$watch.Elapsed.TotalSeconds+'s / 90s')
                Start-Sleep -Seconds 2
            }
            if (-not $receipt){throw 'Tile receiver result timed out'}
            $receipt | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $output ($fixture+'-phone.json')) -Encoding UTF8
            if (-not $server.WaitForExit(10000) -or $server.ExitCode -ne 0){throw 'Tile server did not complete cleanly'}
            $result=Get-Content -Raw -LiteralPath (Join-Path $folder 'result.json') | ConvertFrom-Json
            $count=if ($Profile -eq 'full'){120}else{10}
            if (-not $receipt.completed -or -not $result.completed -or $receipt.measured_frames -ne $count -or $result.measured_frames -ne $count -or $result.recording_sha256 -cne (Get-TransportHash $source)){throw 'Tile transfer census/binding failed'}
            Write-Host ('S23 USB '+$percent+'% dirty tiles: '+[math]::Round($result.frames_per_second,2)+' frames/s delivered, decoded and posted (recorded source)')
        } finally {
            try {
                if ($mappingCreated){Invoke-VwAdb -Device $device -Arguments @('reverse','--remove',$mapping) -Phase ('tiles-'+$percent+'-reverse-cleanup') -TimeoutSeconds 15 | Out-Null; $removedMappings++}
            } finally {
                if ($server){try {if (-not $server.HasExited){$server.Kill();[void]$server.WaitForExit(5000)}} finally {$server.Dispose()}}
            }
        }
    }
    $completed=$true
} finally {
    try {
        Invoke-VwAdb -Device $device -Arguments @('shell','am','force-stop',$package) -Phase 'tiles-owned-receiver-stop' -TimeoutSeconds 15 | Out-Null
        Invoke-VwAdb -Device $device -Arguments @('shell','run-as',$package,'rm','-f',('files/result-'+$id+'-tiles-10.json'),('files/result-'+$id+'-tiles-25.json')) -Phase 'tiles-private-receipt-cleanup' -TimeoutSeconds 15 | Out-Null
        $cleanup=$true
    } catch {Write-Warning 'Owned tile receiver cleanup could not be confirmed'}
    [ordered]@{schema=1;pc_run_id=$RunId;run_id=$id;device_model=$device.Model;apk_sha256=(Get-TransportHash $apk);completed=$completed;owned_reverse_mappings_removed=$removedMappings;owned_device_cleanup_confirmed=$cleanup;screenshots_created=0} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'binding.json') -Encoding UTF8
}
if (-not $cleanup){throw 'Tile receiver cleanup incomplete'}
