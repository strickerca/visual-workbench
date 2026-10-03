#requires -Version 7.2
[CmdletBinding()]
param([Parameter(Mandatory)][string]$ProjectRoot,[Parameter(Mandatory)][string]$PrivateDirectory,
    [Parameter(Mandatory)][string]$RunId,[ValidateRange(15,300)][int]$TimeoutSeconds=90)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$result=$null
try {
    if(-not $IsWindows -or $RunId -cnotmatch '^[a-f0-9]{32}$'){throw 'Capture scope refused.'}
    $ProjectRoot=[IO.Path]::GetFullPath($ProjectRoot).TrimEnd('\')
    $PrivateDirectory=[IO.Path]::GetFullPath($PrivateDirectory).TrimEnd('\')
    $expected=Join-Path ([IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')) ('VisualWorkbench-desktop-startup-'+$RunId)
    if($PrivateDirectory -cne $expected){throw 'Private capture scope refused.'}
    foreach($path in @($ProjectRoot,$PrivateDirectory)){
        $item=Get-Item -LiteralPath $path -Force
        while($null -ne $item){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Redirected capture scope refused.'};$item=$item.Parent}
    }
    $marker=Join-Path $PrivateDirectory '.owner-v1';$owner=Get-Item -LiteralPath $marker -Force
    if($owner.PSIsContainer -or ($owner.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $owner.Length -gt 128 -or
        [IO.File]::ReadAllText($marker) -cne ("VW desktop startup v1`n"+$RunId+"`n")){throw 'Private owner refused.'}
    Import-Module (Join-Path $ProjectRoot 'tools/process.psm1')
    Import-Module (Join-Path $PSScriptRoot 'capture.psm1')
    $launcher=Join-Path $ProjectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev/VisualWorkbenchDev.exe'
    $result=Invoke-VwBoundedDesktopChild -Executable $launcher -Arguments @('--startup-smoke') -WorkingDirectory $ProjectRoot -TimeoutSeconds $TimeoutSeconds
    function Save-Bytes([string]$Name,[byte[]]$Value){
        if($Name -notin @('startup.txt','process.json') -or $Value.Length -gt 1MB){throw 'Capture output refused.'}
        $stream=[IO.File]::Open((Join-Path $PrivateDirectory $Name),[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try{$stream.Write($Value);$stream.Flush($true)}finally{$stream.Dispose()}
    }
    if($null -ne $result.Bytes){Save-Bytes 'startup.txt' $result.Bytes}
    Save-Bytes 'process.json' ([Text.UTF8Encoding]::new($false).GetBytes(($result.Proof|ConvertTo-Json -Depth 6)))
    Write-Host "[desktop-capture] END exit=$($result.ExitCode)"
    exit $result.ExitCode
}catch{Write-Host '[desktop-capture] FAILED';exit 1}
finally{if($null -ne $result -and $null -ne $result.Bytes){[Array]::Clear($result.Bytes,0,$result.Bytes.Length)}}
