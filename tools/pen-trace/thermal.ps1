param(
    [Parameter(Mandatory = $true)][string]$OutFile,
    [Parameter(Mandatory = $true)][switch]$OwnerReady,
    [ValidateRange(30, 600)][int]$DurationSeconds = 600
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $projectRoot 'tools/process.psm1') -Force
Import-Module (Join-Path $projectRoot 'tools/android-device.psm1') -Force
if (-not $OwnerReady) { throw 'Confirm the owner is ready to draw before starting the thermal session' }
$outputPath = [IO.Path]::GetFullPath($OutFile)
if (Test-Path -LiteralPath $outputPath) { throw 'Choose a new output file; existing evidence is preserved' }
$device = Get-VwAndroidDevice -Root $projectRoot
$rows = [Collections.Generic.List[object]]::new()
$watch = [Diagnostics.Stopwatch]::StartNew()
$complete = $false
try {
    for ($target = 0; $target -le $DurationSeconds; $target += 30) {
        while ($watch.Elapsed.TotalSeconds -lt $target) {
            Write-Host ("Thermal session: {0:N0}/{1}s; owner drawing, next sample at {2}s" -f $watch.Elapsed.TotalSeconds, $DurationSeconds, $target)
            Start-Sleep -Seconds ([Math]::Min(15, [Math]::Ceiling($target - $watch.Elapsed.TotalSeconds)))
        }
        $sampleAt = $watch.Elapsed.TotalSeconds
        $battery = Invoke-VwAdb -Device $device -Arguments @('shell', 'dumpsys', 'battery') -Phase 'pen-thermal-battery' -TimeoutSeconds 15 -Capture
        $focus = Invoke-VwAdb -Device $device -Arguments @('shell', 'dumpsys', 'activity', 'activities') -Phase 'pen-thermal-focus' -TimeoutSeconds 15 -Capture
        $data = $battery.Lines -join "`n"
        if ($data -notmatch '(?m)^\s*temperature:\s*(-?[0-9]+)\s*$') { throw 'Battery temperature not reported' }
        $temperature = [int]$Matches[1] / 10.0
        $foreground = @($focus.Lines | Where-Object { $_ -match '(?:mResumedActivity|topResumedActivity).*com\.visualworkbench\.penprobe/' }).Count -gt 0
        $charging = $data -match '(?m)^\s*(?:AC|USB|Wireless|Dock) powered:\s*true\s*$'
        $rows.Add([pscustomobject]@{ scheduled_seconds = $target; elapsed_seconds = [Math]::Round($sampleAt, 3); temperature_c = $temperature; probe_foreground = $foreground; powered = $charging })
        Write-Host "Thermal sample: ${temperature} C; probe foreground=$foreground; powered=$charging"
        if (-not $foreground) { throw 'INCONCLUSIVE: probe backgrounded during thermal session; no app was relaunched' }
    }
    $complete = $DurationSeconds -eq 600 -and $rows.Count -eq 21
} finally {
    $report = [ordered]@{ format = 'pen-thermal-v1'; device_model = $device.Model; owner_ready = [bool]$OwnerReady;
        requested_seconds = $DurationSeconds; completed_ten_minutes = $complete;
        foreground_check = 'sampled_every_30s_not_continuous'; drawing_continuity = 'requires_owner_confirmation';
        start_c = $(if ($rows.Count) { $rows[0].temperature_c } else { $null });
        max_c = $(if ($rows.Count) { ($rows | Measure-Object temperature_c -Maximum).Maximum } else { $null });
        end_c = $(if ($rows.Count) { $rows[$rows.Count - 1].temperature_c } else { $null }); samples = @($rows.ToArray()) }
    [IO.File]::WriteAllText($outputPath, ($report | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
}
