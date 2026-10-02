param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$jarDirectory = Join-Path $PSScriptRoot 'apps\desktop\build\compose\jars'
$applications = @(Get-ChildItem -LiteralPath $jarDirectory -Filter '*.jar' -File -ErrorAction SilentlyContinue)
if ($applications.Count -ne 1) {
    throw 'Build the desktop starter first: powershell -NoProfile -ExecutionPolicy Bypass -File .\build.ps1 build-desktop'
}
$application = $applications[0].FullName
$java = if ($env:JAVA_HOME) { Join-Path $env:JAVA_HOME 'bin\java.exe' } else { (Get-Command java.exe -ErrorAction Stop).Source }
if (-not (Test-Path -LiteralPath $java -PathType Leaf)) { throw 'The selected JDK 21 is unavailable; run build.ps1 doctor' }
$runtime = Join-Path $PSScriptRoot '.local\runtime'
[IO.Directory]::CreateDirectory($runtime) | Out-Null
$credentials = @{}
try {
    foreach ($entry in [Environment]::GetEnvironmentVariables('Process').GetEnumerator()) {
        if ($entry.Key -match '(?i)(TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY)') {
            $credentials[$entry.Key] = $entry.Value
            [Environment]::SetEnvironmentVariable($entry.Key, $null, 'Process')
        }
    }
    $process = Start-Process -FilePath $java -ArgumentList @('-Dsun.java2d.dpiaware=true', '-Dskiko.renderApi=DIRECT3D', '-jar', ('"' + $application + '"')) -WorkingDirectory $PSScriptRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $runtime 'desktop.stdout.log') `
        -RedirectStandardError (Join-Path $runtime 'desktop.stderr.log')
} finally {
    foreach ($name in $credentials.Keys) { [Environment]::SetEnvironmentVariable($name, $credentials[$name], 'Process') }
}
@{
    pid = $process.Id
    application_jar_sha256 = (Get-FileHash -LiteralPath $application -Algorithm SHA256).Hash.ToLowerInvariant()
    uses_owner_installed_jdk = $true
    launched_at = (Get-Date).ToString('o')
} | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $runtime 'desktop-launch.json') -Encoding UTF8
Write-Host "Visual Workbench development starter launched (PID $($process.Id))."
