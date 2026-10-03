# Read-only staging handles remain owned by the caller through verification/install.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Assert-AppStagePlain([string]$Path,[bool]$Directory) {
    $item=Get-Item -LiteralPath ([IO.Path]::GetFullPath($Path)) -Force
    if($item.PSIsContainer -ne $Directory){throw 'APK staging path type refused.'}
    $cursor=$item
    while($null -ne $cursor){
        if(($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'APK staging redirect refused.'}
        $cursor=if($cursor -is [IO.DirectoryInfo]){$cursor.Parent}else{$cursor.Directory}
    }
}
function Copy-VwAppApk {
    [CmdletBinding()]
    param([Parameter(Mandatory)][string]$Source,[Parameter(Mandatory)][string]$Directory,
          [Parameter(Mandatory)][ValidateSet('main.apk','test.apk')][string]$Name,
          [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{64}$')][string]$ExpectedSha256)
    $Source=[IO.Path]::GetFullPath($Source);$Directory=[IO.Path]::GetFullPath($Directory)
    Assert-AppStagePlain $Source $false;Assert-AppStagePlain $Directory $true
    $destination=Join-Path $Directory $Name
    $sourceStream=$null;$output=$null;$locked=$null
    try{
        # Hold the mutable AGP source against concurrent writes/deletion while
        # copying, then bind the private read-only handle to its expected bytes.
        $sourceStream=[IO.File]::Open($Source,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        if($sourceStream.Length -lt 22 -or $sourceStream.Length -gt 2GB){throw 'APK staging size refused.'}
        $output=[IO.File]::Open($destination,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        $sourceStream.CopyTo($output,1048576);$output.Flush($true);$output.Dispose();$output=$null
        $sourceStream.Dispose();$sourceStream=$null
        Assert-AppStagePlain $destination $false
        # The short close/open interval precedes all verification. A substitution
        # there fails the hash below. No write/delete sharing is granted afterward.
        $locked=[IO.File]::Open($destination,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        if($locked.Length -lt 22 -or $locked.Length -gt 2GB){throw 'Locked APK staging size refused.'}
        $hash=[Security.Cryptography.SHA256]::Create()
        try{$actual=[Convert]::ToHexString($hash.ComputeHash($locked)).ToLowerInvariant()}finally{$hash.Dispose()}
        if($actual -cne $ExpectedSha256){throw 'APK staging source changed.'}
        $locked.Position=0
        $result=[pscustomobject]@{Path=$destination;Sha256=$actual;Handle=$locked}
        $locked=$null
        return $result
    }finally{
        if($null -ne $sourceStream){$sourceStream.Dispose()};if($null -ne $output){$output.Dispose()};if($null -ne $locked){$locked.Dispose()}
    }
}
Export-ModuleMember -Function Copy-VwAppApk
