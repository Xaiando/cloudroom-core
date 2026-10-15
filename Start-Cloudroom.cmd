@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0install\windows.ps1" -Action Gui
if errorlevel 1 (
    echo.
    echo Cloudroom could not start. See the error above.
    pause
    exit /b 1
)
