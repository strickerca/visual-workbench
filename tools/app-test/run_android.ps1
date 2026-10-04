#requires -Version 7.2
[CmdletBinding()]
param(
    [string]$ProjectRoot = (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)),
    [ValidateSet('IN2019', 'SM-S918U')][string]$ExpectedModel = 'IN2019',
    [ValidateRange(60,3600)][int]$TimeoutSeconds = 1200,
    [switch]$Execute
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $Execute) { throw 'Source-only by default. The central validation owner must pass -Execute.' }
if (-not $IsWindows) { throw 'App HIL requires Windows.' }
$ProjectRoot = [IO.Path]::GetFullPath($ProjectRoot)
Import-Module (Join-Path $ProjectRoot 'tools/process.psm1')
Import-Module (Join-Path $ProjectRoot 'tools/android-device.psm1')
Import-Module (Join-Path $PSScriptRoot 'stage.psm1')
Import-Module (Join-Path $PSScriptRoot 'device.psm1')
$packages = @('com.visualworkbench.android.hil', 'com.visualworkbench.android.hil.test')
$runId = [Guid]::NewGuid().ToString('N')
$temporaryParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$owned = Join-Path $temporaryParent ('VisualWorkbench-app-hil-' + $runId)
$parser = Join-Path $ProjectRoot 'tools/app-test/reports.py'
$state = Join-Path $owned 'state.json'
$device = $null; $artifacts = $null; $apks = $null; $lock = $null
$created = $false; $processesClean = $true; $assertionsPassed = $false
$attempted = @{}; $installed = @{}; $absent = @{}; $apkLocks = @{}
$phase = 'setup'
$receipt = [ordered]@{
    schema=1; kind='isolated-android-app-hil'; run_id=$runId; model=$ExpectedModel
    status='failed'; failure_phase='setup'; failure_code=$null; assertions=$null; artifacts=$null; process_runs=@()
    source_sha256=$null; instrumentation_observation=$null
    isolated_application=$true; normal_app_tested=$false; normal_app_modified=$false
    device_scope=[ordered]@{owner_user=0;profile_count=$null;global_package_absence_verified=$false;other_profiles_modified=$false;cleanup_owner_user_only=$true}
    cleanup=[ordered]@{process_trees=$false; owned_packages=$false; private_work_disposed=$false}
}
function Assert-PlainPath([string]$Path, [switch]$Directory) {
    $item = Get-Item -LiteralPath ([IO.Path]::GetFullPath($Path)) -Force
    if ($item.PSIsContainer -ne [bool]$Directory) { throw 'Path type refused.' }
    $cursor=$item
    while ($null -ne $cursor) {
        if (($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Redirected path refused.' }
        $cursor=if($cursor -is [IO.DirectoryInfo]){$cursor.Parent}else{$cursor.Directory}
    }
}
function Save-Private([string]$Name, [string]$Text, [int]$Limit=1048576) {
    if ($Name -cne '.owner-v1' -and $Name -notmatch '^[a-z][a-z0-9.-]+$') { throw 'Private filename refused.' }
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes($Text)
    if ($bytes.Length -gt $Limit) { throw 'Private output exceeded its limit.' }
    $stream=[IO.File]::Open((Join-Path $owned $Name),[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try{$stream.Write($bytes);$stream.Flush($true)}finally{$stream.Dispose()}
}
function Run-Owned([string]$Name,[string]$Executable,[string[]]$Arguments,[int]$Seconds,[string]$WorkingDirectory=$ProjectRoot) {
    $script:phase=$Name
    $redact=@($ProjectRoot,$owned,$env:USERPROFILE)
    if($null -ne $device){$redact+=$device.Serial}
    try {$result=Invoke-VwProcess -FilePath $Executable -ArgumentList $Arguments -WorkingDirectory $WorkingDirectory -Phase $Name -TimeoutSeconds $Seconds -ParentExitGraceSeconds 20 -RedactValues $redact -SensitiveCapture}
    catch {$script:processesClean=$false;throw}
    try {$proof=Get-Content -Raw -LiteralPath ($result.LogPath+'.json')|ConvertFrom-Json}
    catch {$script:processesClean=$false;throw}
    $clean=$proof.contained_in_windows_job -eq $true -and $proof.process_tree_cleanup_confirmed -eq $true -and $proof.output_streams_completed -eq $true
    $script:processesClean=$script:processesClean -and $clean
    $script:receipt.process_runs += [ordered]@{phase=$Name;exit_code=$result.ExitCode;elapsed_seconds=$proof.elapsed_seconds;process_tree_cleanup_confirmed=$clean}
    if($result.ExitCode -ne 0 -or -not $clean){
        foreach($line in $result.Lines){if($line -cmatch '^APP_HIL_REPORT_REJECTED:([a-z_]{1,80})$'){$script:receipt.failure_code=$Matches[1]}}
        throw 'Bounded phase or process cleanup failed.'
    }
    return $result
}
function Run-Parser([string]$Name,[string[]]$Arguments) {
    Run-Owned $Name 'python.exe' (@($parser)+$Arguments+@('--root',$ProjectRoot,'--state',$state)) 120 | Out-Null
}
function Adb([string]$Name,[string[]]$Arguments,[int]$Seconds=30) {
    Run-Owned $Name $device.AdbPath (@('-s',$device.Serial)+$Arguments) $Seconds
}
function Read-ServerBytes([IO.Stream]$Stream,[int]$Count) {
    $bytes=[byte[]]::new($Count);$offset=0
    while($offset -lt $Count){$n=$Stream.Read($bytes,$offset,$Count-$offset);if($n -eq 0){throw 'Existing adb server closed.'};$offset+=$n}
    [Text.Encoding]::ASCII.GetString($bytes)
}
function Assert-ExistingAdbServer([string]$AdbPath) {
    if($env:ADB_SERVER_SOCKET -or ($env:ANDROID_ADB_SERVER_PORT -and $env:ANDROID_ADB_SERVER_PORT -ne '5037')){throw 'Nondefault adb server refused.'}
    $v=Run-Owned 'app-hil-adb-client' $AdbPath @('version') 15
    $m=[regex]::Matches(($v.Lines -join "`n"),'(?m)^Android Debug Bridge version 1\.0\.(\d+)\s*$')
    if($m.Count -ne 1){throw 'adb client version unavailable.'}
    $client=[Net.Sockets.TcpClient]::new()
    try{
        $task=$client.ConnectAsync('127.0.0.1',5037);if(-not $task.Wait(1500)){throw 'Existing adb server unavailable.'};$task.GetAwaiter().GetResult()
        $stream=$client.GetStream();$stream.ReadTimeout=1500;$stream.WriteTimeout=1500
        $request=[Text.Encoding]::ASCII.GetBytes('000chost:version');$stream.Write($request)
        if((Read-ServerBytes $stream 4) -cne 'OKAY'){throw 'Existing adb server refused.'}
        $length=Read-ServerBytes $stream 4
        if($length -notmatch '^[0-9a-fA-F]{4}$'){throw 'adb response refused.'}
        $count=[Convert]::ToInt32($length,16);if($count -lt 1 -or $count -gt 8){throw 'adb response refused.'}
        $version=Read-ServerBytes $stream $count
        if($version -notmatch '^[0-9a-fA-F]{1,8}$' -or [Convert]::ToInt32($version,16) -ne [int]$m[0].Groups[1].Value){throw 'adb mismatch; automatic restart refused.'}
    }finally{$client.Dispose()}
}
function Package-Path([string]$Package) {
    if($Package -cnotin $packages){throw 'Package outside HIL scope.'}
    $v=Adb 'app-hil-package-inventory' @('shell','pm','list','packages','--user','0',$Package)
    $lines=@($v.Lines|Where-Object{$_.Trim()})
    foreach($line in $lines){if($line -cnotin @('package:com.visualworkbench.android.hil','package:com.visualworkbench.android.hil.test')){throw 'Ambiguous package inventory.'}}
    if(('package:'+$Package) -cnotin $lines){return $null}
    if(@($lines|Where-Object{$_ -ceq ('package:'+$Package)}).Count -ne 1){throw 'Duplicate package inventory.'}
    $v=Adb 'app-hil-package-path' @('shell','pm','path','--user','0',$Package)
    $lines=@($v.Lines|Where-Object{$_.Trim()})
    if($lines.Count -ne 1 -or $lines[0] -notmatch '^package:(/data/app/[A-Za-z0-9/_=+~.-]+/base\.apk)$'){throw 'Split or unknown installed package.'}
    return $Matches[1]
}
function Owned-Package([string]$Package,[string]$ExpectedHash) {
    $path=Package-Path $Package
    if($null -eq $path){return $false}
    $v=Adb 'app-hil-installed-apk-binding' @('shell','sha256sum',$path) 60
    $lines=@($v.Lines|Where-Object{$_.Trim()})
    return $lines.Count -eq 1 -and $lines[0] -match '^([0-9a-f]{64})\s+' -and $Matches[1] -ceq $ExpectedHash
}
function Assert-GlobalPackageAbsent([string]$Package) {
    $v=Adb 'app-hil-global-package-absence' @('shell','dumpsys','package',$Package)
    if(-not (Test-VwAppHilPackageAbsent -Package $Package -Lines $v.Lines)){throw 'Existing or unknown global HIL package preserved; refusing run.'}
}
function Cleanup-Packages {
    $ok=$true
    foreach($key in @('test','main')){
        $package=if($key -ceq 'main'){$packages[0]}else{$packages[1]}
        if(-not $attempted.ContainsKey($package)){continue}
        try{
            if($null -eq (Package-Path $package)){continue}
            # An unknown/failed install is not ownership proof, even if the
            # matching APK appeared after the last absence check. Preserve it.
            if($absent[$package] -ne $true -or -not $installed.ContainsKey($package) -or $null -eq $artifacts){$ok=$false;continue}
            $artifact=$artifacts.artifacts.$key
            # Matching installed bytes bind the previously verified signer too.
            if(-not (Owned-Package $package $artifact.sha256)){$ok=$false;continue}
            Adb 'app-hil-stop-owned-package' @('shell','am','force-stop','--user','0',$package) | Out-Null
            $v=Adb 'app-hil-remove-owned-package' @('uninstall','--user','0',$package) 60
            if(($v.Lines -join "`n").Trim() -cne 'Success' -or $null -ne (Package-Path $package)){$ok=$false}
        }catch{$ok=$false}
    }
    return $ok
}
try {
    Assert-PlainPath $ProjectRoot -Directory
    $local=Join-Path $ProjectRoot '.local';if(-not(Test-Path -LiteralPath $local)){[IO.Directory]::CreateDirectory($local)|Out-Null};Assert-PlainPath $local -Directory
    $lockPath=Join-Path $local 'app-hil.lock';if(Test-Path -LiteralPath $lockPath){Assert-PlainPath $lockPath}
    $lock=[IO.File]::Open($lockPath,[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    Assert-PlainPath $temporaryParent -Directory
    if(Test-Path -LiteralPath $owned){throw 'Private work collision.'}
    [IO.Directory]::CreateDirectory($owned)|Out-Null;$created=$true;Assert-PlainPath $owned -Directory
    Save-Private '.owner-v1' $runId
    Run-Parser 'app-hil-bind-source' @('begin')
    $receipt.source_sha256=(Get-Content -Raw -LiteralPath $state|ConvertFrom-Json).source_sha256
    $gradle=@(':android:assembleDebug',':android:assembleDebugAndroidTest','-PvwAndroidHil=true',('-PvwNativeDir='+(Join-Path $ProjectRoot 'target/debug')),('-PvwAndroidNativeDir='+(Join-Path $ProjectRoot 'target/android-jni')),'--console=plain','--no-daemon','--no-parallel','--max-workers=2','--no-configuration-cache','--no-build-cache','--rerun-tasks')
    Run-Owned 'app-hil-assemble-isolated' (Join-Path $ProjectRoot 'apps/gradlew.bat') $gradle $TimeoutSeconds (Join-Path $ProjectRoot 'apps') | Out-Null
    Run-Parser 'app-hil-discover-apks' @('discover','--output',(Join-Path $owned 'source-apks.json'))
    $sources=Get-Content -Raw -LiteralPath (Join-Path $owned 'source-apks.json')|ConvertFrom-Json
    $staged=[ordered]@{}
    foreach($key in @('main','test')){
        $candidate=Copy-VwAppApk -Source $sources.$key.path -Directory $owned -Name ($key+'.apk') -ExpectedSha256 $sources.$key.sha256
        $apkLocks[$key]=$candidate.Handle
        $staged[$key]=[ordered]@{path=$candidate.Path;sha256=$candidate.Sha256}
    }
    Save-Private 'apks.json' ($staged|ConvertTo-Json -Depth 8)
    $apks=Get-Content -Raw -LiteralPath (Join-Path $owned 'apks.json')|ConvertFrom-Json
    $sdk=if($env:ANDROID_HOME){$env:ANDROID_HOME}elseif($env:ANDROID_SDK_ROOT){$env:ANDROID_SDK_ROOT}else{Join-Path $env:LOCALAPPDATA 'Android/Sdk'}
    $analyzer=Join-Path $sdk 'cmdline-tools/latest/bin/apkanalyzer.bat'
    $signer=Join-Path $sdk 'build-tools/36.1.0/apksigner.bat'
    Assert-PlainPath $analyzer;Assert-PlainPath $signer
    foreach($key in @('main','test')){
        $v=Run-Owned ('app-hil-manifest-'+$key) $analyzer @('manifest','print',$apks.$key.path) 120
        Save-Private ($key+'.xml') ($v.Lines -join "`n")
        $v=Run-Owned ('app-hil-signature-'+$key) $signer @('verify','--verbose','--print-certs',$apks.$key.path) 120
        Save-Private ($key+'.cert.txt') ($v.Lines -join "`n") 262144
    }
    Run-Parser 'app-hil-verify-apks' @('verify','--private',$owned,'--output',(Join-Path $owned 'artifacts.json'))
    $artifacts=Get-Content -Raw -LiteralPath (Join-Path $owned 'artifacts.json')|ConvertFrom-Json
    $receipt.artifacts=$artifacts.artifacts
    $adb=Join-Path $sdk 'platform-tools/adb.exe';Assert-PlainPath $adb
    Assert-ExistingAdbServer $adb
    $phase='app-hil-device-selection'
    $device=Get-VwAndroidDevice -Root $ProjectRoot -ExpectedModel $ExpectedModel
    $users=Adb 'app-hil-owner-users' @('shell','pm','list','users')
    $receipt.device_scope.profile_count=Get-VwAppHilProfileCount -Lines $users.Lines
    $current=Adb 'app-hil-current-user' @('shell','am','get-current-user')
    if(($current.Lines -join "`n").Trim() -cne '0'){throw 'Owner user must be current.'}
    foreach($package in $packages){Assert-GlobalPackageAbsent $package;if($null -ne (Package-Path $package)){throw 'Preexisting HIL package preserved; refusing run.'};$absent[$package]=$true}
    $receipt.device_scope.global_package_absence_verified=$true
    foreach($key in @('main','test')){
        $package=if($key -ceq 'main'){$packages[0]}else{$packages[1]}
        Assert-GlobalPackageAbsent $package
        if($null -ne (Package-Path $package)){throw 'HIL package appeared before install; preserving it.'}
        $attempted[$package]=$true
        $v=Adb ('app-hil-install-'+$key) @('install','--user','0','-t',$apks.$key.path) 120
        if(($v.Lines -join "`n") -notmatch '(?m)^Success\s*$'){throw 'Install success not confirmed.'}
        $installed[$package]=$true
        if(-not(Owned-Package $package $artifacts.artifacts.$key.sha256)){throw 'Installed HIL APK not bound.'}
    }
    $command=@($device.AdbPath,'-s',$device.Serial,'shell','am','instrument','--user','0','-w','-r',($packages[1]+'/androidx.test.runner.AndroidJUnitRunner'))
    Save-Private 'command.json' ($command|ConvertTo-Json -Compress) 65536
    Run-Owned 'app-hil-instrumentation' 'python.exe' @((Join-Path $ProjectRoot 'tools/app-test/capture.py'),'--command',(Join-Path $owned 'command.json'),'--output',(Join-Path $owned 'instrumentation.txt'),'--seconds',[string]$TimeoutSeconds) ($TimeoutSeconds+30) | Out-Null
    Run-Parser 'app-hil-collect' @('collect','--private',$owned,'--output',(Join-Path $owned 'assertions.json'))
    $receipt.assertions=Get-Content -Raw -LiteralPath (Join-Path $owned 'assertions.json')|ConvertFrom-Json
    $assertionsPassed=$receipt.assertions.assertions_passed -eq $true
} catch {
    $receipt.failure_phase=$phase
    Write-Host "Isolated app HIL failed in $phase; private details are not published."
} finally {
    if(-not $assertionsPassed -and (Test-Path -LiteralPath (Join-Path $owned 'instrumentation.txt'))){
        try{
            Run-Parser 'app-hil-private-diagnosis' @('diagnose','--private',$owned,'--output',(Join-Path $owned 'diagnosis.json'))
            $receipt.instrumentation_observation=Get-Content -Raw -LiteralPath (Join-Path $owned 'diagnosis.json')|ConvertFrom-Json
        }catch{ }
    }
    $receipt.cleanup.owned_packages=Cleanup-Packages
    foreach($handle in $apkLocks.Values){try{$handle.Dispose()}catch{$processesClean=$false}}
    if($created){
        try{
            $resolved=[IO.Path]::GetFullPath($owned)
            if([IO.Path]::GetDirectoryName($resolved) -cne $temporaryParent -or [IO.Path]::GetFileName($resolved) -cne ('VisualWorkbench-app-hil-'+$runId)){throw 'Cleanup containment refused.'}
            Assert-PlainPath $resolved -Directory
            $marker=Join-Path $resolved '.owner-v1';Assert-PlainPath $marker
            if((Get-Content -Raw -LiteralPath $marker) -cne $runId){throw 'Cleanup owner refused.'}
            foreach($entry in Get-ChildItem -LiteralPath $resolved -Recurse -Force){
                if(($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or -not [IO.Path]::GetFullPath($entry.FullName).StartsWith($resolved+'\',[StringComparison]::OrdinalIgnoreCase)){throw 'Cleanup redirected child refused.'}
            }
            Remove-Item -LiteralPath $resolved -Recurse -Force
            $receipt.cleanup.private_work_disposed=-not(Test-Path -LiteralPath $resolved)
        }catch{$receipt.cleanup.private_work_disposed=$false}
    }else{$receipt.cleanup.private_work_disposed=$true}
    $receipt.cleanup.process_trees=$processesClean
    if($assertionsPassed -and $processesClean -and $receipt.cleanup.owned_packages -and $receipt.cleanup.private_work_disposed){$receipt.status='passed';$receipt.failure_phase=$null}
    if($null -ne $lock){
        try{
            $destination=Join-Path $ProjectRoot ('.local/app-hil-'+$runId+'.json')
            $stream=[IO.File]::Open($destination,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
            try{$bytes=[Text.Encoding]::UTF8.GetBytes(($receipt|ConvertTo-Json -Depth 32));$stream.Write($bytes);$stream.Flush($true)}finally{$stream.Dispose()}
            Write-Host ('Isolated app HIL receipt: app-hil-'+$runId+'.json; status='+$receipt.status)
        }finally{$lock.Dispose()}
    }
}
if($receipt.status -ne 'passed'){exit 1}
Write-Host "Isolated app instrumentation on ${ExpectedModel}: PASS. Normal app, physical S Pen and performance acceptance remain pending."
exit 0
