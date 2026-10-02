Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$module = Import-Module (Join-Path $PSScriptRoot '../android-device.psm1') -PassThru
& $module {
    $samsung = 'synthetic-samsung device product:fixture model:SM_S918U transport_id:1'
    $reserved = 'synthetic-reserved device product:fixture model:IN2019 transport_id:2'
    $selected = Select-VwAndroidTransport -Lines @($samsung, $reserved) -ExpectedModel 'SM-S918U'
    if ($selected.Serial -cne 'synthetic-samsung') { throw 'Wrong transport selected' }
    foreach ($rows in @(@($reserved), @($samsung, $samsung), @('synthetic-samsung offline'), @('emulator-5554 device model:SM_S918U'))) {
        $rejected = $false
        try { Select-VwAndroidTransport -Lines $rows -ExpectedModel 'SM-S918U' | Out-Null }
        catch { $rejected = $true }
        if (-not $rejected) { throw 'Unsafe device selection was not rejected' }
    }
    Write-Host 'Device selection: PASS (authorized model with reserved peer; missing, duplicate, offline and emulator rejected). No adb commands executed.'
}
