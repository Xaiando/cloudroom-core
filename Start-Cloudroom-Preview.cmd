@echo off
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0install\original-desktop.ps1" -Action Gui
if errorlevel 1 pause
