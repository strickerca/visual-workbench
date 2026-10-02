@echo off
rem Cargo resolves this executable relative to .cargo/config.toml's project root.
rem The wrapper keeps the PowerShell script path independent of Cargo's test cwd.
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0android-rust-runner.ps1" %*
exit /b %ERRORLEVEL%
