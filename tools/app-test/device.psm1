Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-VwAppHilProfileCount {
    param([Parameter(Mandatory = $true)][string[]]$Lines)
    $nonempty = @($Lines | Where-Object { $_.Trim() })
    if ($nonempty.Count -lt 2 -or $nonempty[0].Trim() -cne 'Users:') { throw 'Profile inventory refused.' }
    $ids = @()
    foreach ($line in $nonempty | Select-Object -Skip 1) {
        if ($line -cnotmatch '^\s*UserInfo\{([0-9]{1,10}):[^\r\n]*:[0-9a-fA-F]+\}(?:\s+running)?\s*$') { throw 'Profile inventory refused.' }
        $ids += $Matches[1]
    }
    if ($ids.Count -gt 24 -or @($ids | Select-Object -Unique).Count -ne $ids.Count -or @($ids | Where-Object { $_ -ceq '0' }).Count -ne 1) { throw 'Owner profile inventory refused.' }
    return $ids.Count
}

function Test-VwAppHilPackageAbsent {
    param([Parameter(Mandatory = $true)][string]$Package, [AllowEmptyCollection()][string[]]$Lines)
    if ($Package -cnotin @('com.visualworkbench.android.hil', 'com.visualworkbench.android.hil.test')) { throw 'Package outside HIL scope.' }
    $nonempty = @($Lines | Where-Object { $_.Trim() })
    # AOSP DumpHelper checks the global package settings before printing this
    # exact response. Empty output, permission errors and existing package dumps
    # never establish absence in another profile.
    return $nonempty.Count -eq 1 -and $nonempty[0].Trim() -ceq ('Unable to find package: ' + $Package)
}

Export-ModuleMember -Function Get-VwAppHilProfileCount, Test-VwAppHilPackageAbsent
