@echo off
setlocal
set "CloudBeePowerShell=%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe"
if exist "%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe" set "CloudBeePowerShell=%SystemRoot%\Sysnative\WindowsPowerShell\v1.0\powershell.exe"
"%CloudBeePowerShell%" -NoProfile -ExecutionPolicy Bypass -File "%~dp0install\windows.ps1" -Action Gui
if errorlevel 1 (
    echo.
    echo Cloud Bee could not start. See the error above.
    pause
    exit /b 1
)
