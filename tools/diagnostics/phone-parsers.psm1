Set-StrictMode -Version Latest
function ConvertFrom-VwPenListing([string]$Text) {
    $rows = @()
    foreach ($block in [regex]::Split($Text, '(?m)^add device ')) {
        if ($block -notmatch '\bBTN_TOOL_PEN\b') { continue }
        $axes = [ordered]@{}
        foreach ($axis in @('ABS_PRESSURE', 'ABS_DISTANCE', 'ABS_TILT_X', 'ABS_TILT_Y')) {
            $present = $block -match ("\b" + $axis + "\b")
            $row = [ordered]@{ present = $present; minimum = $null; maximum = $null }
            if ($block -match ("(?m)\b" + $axis + "\b[^\r\n]*min\s+(-?\d+),\s*max\s+(-?\d+)")) {
                $row.minimum = [int]$Matches[1]; $row.maximum = [int]$Matches[2]
            }
            $axes[$axis] = $row
        }
        $rows += [ordered]@{ pen_index = $rows.Count + 1; axes = $axes }
    }
    return ,$rows
}
function ConvertFrom-VwDisplayListing([string]$Text) {
    $rows = @()
    $seen = @{}
    foreach ($m in [regex]::Matches($Text, 'width=(\d+),\s*height=(\d+),\s*(?:fps|refreshRate)=([0-9.]+)')) {
        $key = $m.Value
        if ($seen.ContainsKey($key)) { continue }
        $seen[$key] = $true
        $rows += [ordered]@{ width = [int]$m.Groups[1].Value; height = [int]$m.Groups[2].Value; refresh_hz = [double]::Parse($m.Groups[3].Value, [Globalization.CultureInfo]::InvariantCulture) }
    }
    return ,$rows
}
function ConvertFrom-VwCodecListing([string]$Text) {
    $names = @([regex]::Matches($Text, '<MediaCodec\b[^>]*\bname="((?:c2\.|OMX\.)[A-Za-z0-9_.-]+)"') |
        ForEach-Object { $_.Groups[1].Value } |
        Where-Object { $_ -match '(?:avc|h264|hevc|h265|vp9|av1)' -and $_ -match '(?:decoder|encoder)' } |
        Sort-Object -Unique)
    return ,$names
}
Export-ModuleMember -Function ConvertFrom-VwPenListing, ConvertFrom-VwDisplayListing, ConvertFrom-VwCodecListing
