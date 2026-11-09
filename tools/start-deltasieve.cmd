@echo off
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0start-deltasieve.ps1" %*
set "result=%errorlevel%"
if not "%result%"=="0" pause
exit /b %result%
