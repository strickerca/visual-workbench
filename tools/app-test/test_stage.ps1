#requires -Version 7.2
# Synthetic host file-sharing fixtures only. No SDK, adb, app, or native code.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $IsWindows){throw 'Windows file-share fixtures require Windows.'}
Import-Module (Join-Path $PSScriptRoot 'stage.psm1')
$parent=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$fixture=Join-Path $parent ('VisualWorkbench-app-stage-'+[Guid]::NewGuid().ToString('N'))
$handle=$null;$created=$false
try{
    if(Test-Path -LiteralPath $fixture){throw 'Fixture collision.'}
    New-Item -ItemType Directory -Path $fixture -ErrorAction Stop|Out-Null;$created=$true
    $source=Join-Path $fixture 'input.apk';$bytes=[Text.Encoding]::UTF8.GetBytes('synthetic immutable APK fixture only')
    [IO.File]::WriteAllBytes($source,$bytes)
    $hash=(Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash.ToLowerInvariant()
    $copy=Copy-VwAppApk -Source $source -Directory $fixture -Name 'main.apk' -ExpectedSha256 $hash
    $handle=$copy.Handle
    [IO.File]::WriteAllText($source,'other build output')
    if((Get-FileHash -LiteralPath $copy.Path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $hash){throw 'Staged bytes changed with original.'}
    foreach($operation in @('write','delete','replace')){
        $refused=$false
        try{
            switch($operation){
                'write' {[IO.File]::WriteAllText($copy.Path,'replacement')}
                'delete' {[IO.File]::Delete($copy.Path)}
                'replace' {[IO.File]::Move($source,$copy.Path,$true)}
            }
        }catch [IO.IOException]{$refused=$true}
        catch [UnauthorizedAccessException]{$refused=$true}
        if(-not $refused){throw 'Locked APK admitted mutation.'}
    }
    $collision=$false
    try{Copy-VwAppApk -Source $copy.Path -Directory $fixture -Name 'main.apk' -ExpectedSha256 $hash|Out-Null}catch [IO.IOException]{$collision=$true}
    if(-not $collision){throw 'Staging overwrote a preexisting file.'}
    [IO.File]::WriteAllBytes($source,$bytes)
    $mismatch=$false
    try{Copy-VwAppApk -Source $source -Directory $fixture -Name 'test.apk' -ExpectedSha256 ('0'*64)|Out-Null}catch{$mismatch=$true}
    if(-not $mismatch){throw 'Unverified copy admitted.'}
    $probe=[IO.File]::Open((Join-Path $fixture 'test.apk'),[IO.FileMode]::Open,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None);$probe.Dispose()
    Write-Output 'APP_HIL_STAGING_FIXTURES_PASS: immutable-copy, write-delete-replace-denial, no-clobber, changed-source-refusal, failed-handle-cleanup'
}finally{
    if($null -ne $handle){$handle.Dispose()}
    if($created -and (Test-Path -LiteralPath $fixture)){
        $resolved=[IO.Path]::GetFullPath($fixture)
        if([IO.Path]::GetDirectoryName($resolved) -cne $parent -or [IO.Path]::GetFileName($resolved) -notmatch '^VisualWorkbench-app-stage-[a-f0-9]{32}$'){throw 'Fixture cleanup containment refused.'}
        $items=@(Get-Item -LiteralPath $resolved -Force)+@(Get-ChildItem -LiteralPath $resolved -Force)
        foreach($item in $items){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Fixture cleanup redirect refused.'}}
        foreach($item in $items|Where-Object{-not $_.PSIsContainer}){Remove-Item -LiteralPath $item.FullName -Force}
        [IO.Directory]::Delete($resolved,$false)
    }
}
