param([ValidatePattern('^[a-z0-9-]+$')][string]$FixtureSet='t009-v2')
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$projectRoot=Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $PSScriptRoot))
. (Join-Path $projectRoot 'tools/enter-dev.ps1')
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
. (Join-Path $projectRoot 'tools/bench/transport/common.ps1')
$check=Invoke-VwProcess -FilePath 'python.exe' -ArgumentList @('tools/bench/image/build_receipt.py','check') -WorkingDirectory $projectRoot -Phase 'image-apk-binding' -TimeoutSeconds 30
if ($check.ExitCode -ne 0){throw 'Image APK stale'}
$fixtures=Join-Path $projectRoot ('fixtures/generated/'+$FixtureSet)
$manifest=Get-Content -Raw -LiteralPath (Join-Path $fixtures 'manifest.json') | ConvertFrom-Json
$jpeg=Join-Path $fixtures '200mp.jpg'
$jpegRecord=@($manifest.files | Where-Object {$_.path -ceq '200mp.jpg'})
if ($jpegRecord.Count -ne 1 -or (Get-TransportHash $jpeg) -cne $jpegRecord[0].sha256){throw 'JPEG fixture binding differs'}
$device=Get-VwAndroidDevice -Root $projectRoot -ExpectedModel 'SM-S918U'
$runId=[guid]::NewGuid().ToString('N')
$package='com.visualworkbench.imagebench'
$remote='/sdcard/Android/data/'+$package+'/files/vw-image-'+$runId
$apk=Join-Path $PSScriptRoot 'build/outputs/apk/debug/image-bench-debug.apk'
$output=Join-Path $projectRoot ('.local/image-android-'+$runId)
[IO.Directory]::CreateDirectory($output) | Out-Null
$cleanup=$false
$completed=$false

function Invoke-ImageMode {
    param([string]$Mode,[string]$Hash,[int]$Limit=180)
    # Each mode gets a fresh app process, so HEIF decode memory excludes generation buffers.
    Invoke-VwAdb -Device $device -Arguments @('shell','am','force-stop',$package) -Phase ('image-'+$Mode+'-fresh-process') -TimeoutSeconds 15 | Out-Null
    $arguments=@('shell','am','start','-W','-n',($package+'/.ImageActivity'),'--es','run_id',$runId,'--es','mode',$Mode)
    if ($Hash){$arguments+=@('--es','sha256',$Hash)}
    Invoke-VwAdb -Device $device -Arguments $arguments -Phase ('image-'+$Mode+'-start') -TimeoutSeconds 15 | Out-Null
    $foreground=Invoke-VwAdb -Device $device -Arguments @('shell','dumpsys','activity','activities') -Phase ('image-'+$Mode+'-foreground') -TimeoutSeconds 15 -Capture
    $resumed=@($foreground.Lines | Where-Object {$_ -match '(?:mResumedActivity|topResumedActivity)'}) -join "`n"
    if ($resumed -notmatch 'com\.visualworkbench\.imagebench/\.ImageActivity'){throw 'Image diagnostic foreground lost; inspect lifecycle before retry'}
    $name='result-'+$runId+'-'+$Mode+'.json'
    $watch=[Diagnostics.Stopwatch]::StartNew()
    while ($watch.Elapsed.TotalSeconds -lt $Limit) {
        $command='run-as '+$package+" sh -c 'if [ -f files/"+$name+' ]; then cat files/'+$name+"; else echo PENDING; fi'"
        $poll=Invoke-VwAdb -Device $device -Arguments @('shell',$command) -Phase ('image-'+$Mode+'-poll') -TimeoutSeconds 30 -Capture
        $value=($poll.Lines -join "`n").Trim()
        if ($value -ne 'PENDING') {
            $receipt=$value | ConvertFrom-Json
            if ($receipt.run_id -cne $runId -or $receipt.mode -cne $Mode){throw 'Image receipt identity mismatch'}
            $receipt | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $output ($Mode+'.json')) -Encoding UTF8
            return $receipt
        }
        $progress=Invoke-VwAdb -Device $device -Arguments @('shell','run-as',$package,'cat',('files/progress-'+$runId+'.json')) -Phase ('image-'+$Mode+'-progress') -TimeoutSeconds 10 -Capture
        $state=($progress.Lines -join "`n") | ConvertFrom-Json
        Write-Host ('S23 image '+$Mode+': '+$state.phase+'; '+[int]$watch.Elapsed.TotalSeconds+'s / '+$Limit+'s')
        Start-Sleep -Seconds 2
    }
    Invoke-VwAdb -Device $device -Arguments @('shell','pidof',$package) -Phase 'image-timeout-process-check' -TimeoutSeconds 10 -Capture | Out-Null
    throw ('Image '+$Mode+' exceeded bounded deadline; phase progress retained')
}

try {
    Invoke-VwAdb -Device $device -Arguments @('install','-r',$apk) -Phase 'image-install' -TimeoutSeconds 120 | Out-Null
    $prepared=Invoke-ImageMode -Mode 'prepare' -Limit 30
    if (-not $prepared.completed){throw 'Image storage preparation failed'}
    Invoke-VwAdb -Device $device -Arguments @('push',$jpeg,($remote+'/200mp.jpg')) -Phase 'image-jpeg-transfer' -TimeoutSeconds 120 | Out-Null
    $jpegResult=Invoke-ImageMode -Mode 'jpeg' -Hash $jpegRecord[0].sha256
    if (-not $jpegResult.completed -or $jpegResult.measurements.Count -ne 12){throw 'JPEG region/sample measurements incomplete'}
    $generated=Invoke-ImageMode -Mode 'generate'
    if (-not $generated.completed){throw '200 MP HEIF generation failed or memory guard rejected it; retained actual receipt'}
    $heif=Join-Path $fixtures ('200mp-'+$runId+'.heic')
    Invoke-VwAdb -Device $device -Arguments @('pull',($remote+'/200mp.heic'),$heif) -Phase 'image-heif-transfer' -TimeoutSeconds 120 | Out-Null
    if ((Get-TransportHash $heif) -cne $generated.output_sha256){throw 'HEIF pull hash differs'}
    $heifResult=Invoke-ImageMode -Mode 'heif' -Hash $generated.output_sha256
    if (-not $heifResult.completed -or $heifResult.measurements.Count -lt 3){throw 'HEIF target measurements incomplete'}
    $completed=$true
} finally {
    try {
        Invoke-VwAdb -Device $device -Arguments @('shell','am','force-stop',$package) -Phase 'image-owned-stop' -TimeoutSeconds 15 | Out-Null
        if ($remote -cne ('/sdcard/Android/data/'+$package+'/files/vw-image-'+$runId)){throw 'Image cleanup containment failed'}
        Invoke-VwAdb -Device $device -Arguments @('shell','rm','-rf',$remote) -Phase 'image-owned-files-cleanup' -TimeoutSeconds 15 | Out-Null
        Invoke-VwAdb -Device $device -Arguments @('shell',('[ ! -e '+$remote+' ]')) -Phase 'image-owned-cleanup-verify' -TimeoutSeconds 15 -Capture | Out-Null
        $remove=@('shell','run-as',$package,'rm','-f',('files/progress-'+$runId+'.json'))
        foreach ($mode in @('prepare','jpeg','generate','heif')){$remove+=('files/result-'+$runId+'-'+$mode+'.json')}
        Invoke-VwAdb -Device $device -Arguments $remove -Phase 'image-private-receipts-cleanup' -TimeoutSeconds 15 | Out-Null
        $cleanup=$true
    } catch { Write-Warning 'Image diagnostic owned cleanup could not be confirmed' }
    [ordered]@{schema=1;run_id=$runId;device_model=$device.Model;apk_sha256=(Get-TransportHash $apk);jpeg_sha256=$jpegRecord[0].sha256;completed=$completed;device_files_disposed=$cleanup;screenshots_created=0} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $output 'binding.json') -Encoding UTF8
}
if (-not $cleanup){throw 'Image diagnostic cleanup incomplete'}
Write-Host ('S23 image measurement receipts: .local/image-android-'+$runId)
