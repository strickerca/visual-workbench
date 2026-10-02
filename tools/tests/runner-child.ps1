param([ValidateSet('ok','sleep','orphan')][string]$Mode)
if ($Mode -eq 'ok') { Write-Output 'synthetic-success'; exit 0 }
if ($Mode -eq 'sleep') { Start-Sleep -Seconds 60; exit 0 }
$arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '" -Mode sleep'
$child = Start-Process powershell.exe -ArgumentList $arguments -WindowStyle Hidden -PassThru
Write-Output ("owned-child-pid=" + $child.Id)
exit 0
