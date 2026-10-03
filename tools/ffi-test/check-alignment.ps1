param([Parameter(Mandatory=$true)][string]$Library,[Parameter(Mandatory=$true)][string]$Objdump)
$ErrorActionPreference='Stop'
if (-not (Test-Path -LiteralPath $Library -PathType Leaf)) { throw 'Native Android library is missing' }
if (-not (Test-Path -LiteralPath $Objdump -PathType Leaf)) { throw 'Pinned NDK llvm-objdump is missing' }
$output = & $Objdump -p $Library 2>&1
if ($LASTEXITCODE -ne 0) { throw 'llvm-objdump failed' }
$loads = @($output | Where-Object { $_ -match '^\s*LOAD\s' })
if ($loads.Count -eq 0) { throw 'No ELF LOAD segments found' }
foreach ($line in $loads) {
    if ($line -notmatch 'align\s+2\*\*(\d+)') { throw 'Unrecognized ELF LOAD alignment' }
    if ([int]$Matches[1] -lt 14) { throw 'Android ELF LOAD alignment is below 16 KiB' }
}
Write-Output ('Android 16 KiB LOAD alignment: PASS; segments=' + $loads.Count)
