#requires -Version 7.2
# Synthetic ownership fixture only. Parent validation lane runs this explicitly.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $IsWindows){throw 'Windows file-sharing fixture required.'}
Import-Module (Join-Path $PSScriptRoot 'lease.psm1')
$nonce=[Guid]::NewGuid().ToString('N')
$parent=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\')
$directory=Join-Path $parent ('VisualWorkbench-desktop-lease-fixture-'+$nonce)
$created=$false;$lease=$null
try{
    New-Item -ItemType Directory -Path $directory -ErrorAction Stop | Out-Null;$created=$true
    $inventory=@{}
    foreach($name in @('a.bin','b.bin','c.bin','d.bin')){[IO.File]::WriteAllText((Join-Path $directory $name),'synthetic');$inventory[$name]=@{}}
    $lease=Lock-VwDesktopDistribution -Directory $directory -Inventory $inventory
    $target=Join-Path $directory 'a.bin'
    foreach($operation in @('write','delete','rename')){
        $refused=$false
        try{switch($operation){
            'write'{[IO.File]::WriteAllText($target,'wrong')}
            'delete'{[IO.File]::Delete($target)}
            'rename'{[IO.File]::Move($target,(Join-Path $directory 'moved.bin'))}
        }}catch [IO.IOException]{$refused=$true}catch [UnauthorizedAccessException]{$refused=$true}
        if(-not $refused){throw 'A pinned runtime mutation succeeded.'}
    }
    $lease.Dispose();$lease=$null
    [IO.File]::WriteAllText($target,'released')
    if([IO.File]::ReadAllText($target) -cne 'released'){throw 'Released lease remained unavailable.'}
    $invalid=@{'a.bin'=@{};'b.bin'=@{};'c.bin'=@{};'../outside.bin'=@{}}
    $refused=$false
    try{$unexpected=Lock-VwDesktopDistribution -Directory $directory -Inventory $invalid;$unexpected.Dispose()}
    catch{$refused=$true}
    if(-not $refused){throw 'Traversal was accepted.'}
    # A failure after some pins must release all earlier pins.
    $missing=@{'a.bin'=@{};'b.bin'=@{};'c.bin'=@{};'missing.bin'=@{}}
    $refused=$false
    try{$unexpected=Lock-VwDesktopDistribution -Directory $directory -Inventory $missing;$unexpected.Dispose()}
    catch{$refused=$true}
    if(-not $refused){throw 'Missing runtime member was accepted.'}
    [IO.File]::WriteAllText($target,'after failed acquisition')
    Write-Host 'DESKTOP_LEASE_FIXTURES_OK:write_delete_rename_traversal_release_failure_cleanup'
}finally{
    if($null -ne $lease){$lease.Dispose()}
    if($created){
        $actual=[IO.Path]::GetFullPath($directory)
        if($actual -cne (Join-Path $parent ('VisualWorkbench-desktop-lease-fixture-'+$nonce))){throw 'Fixture containment changed.'}
        foreach($child in Get-ChildItem -LiteralPath $directory -Force){
            if($child.PSIsContainer -or ($child.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $child.Name -cnotin @('a.bin','b.bin','c.bin','d.bin','moved.bin')){throw 'Unknown fixture child preserved.'}
            Remove-Item -LiteralPath $child.FullName
        }
        Remove-Item -LiteralPath $directory
    }
}
