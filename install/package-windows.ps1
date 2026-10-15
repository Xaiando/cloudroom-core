[CmdletBinding()]
param(
    [string]$CoreBinary,
    [string]$DesktopBinary,
    [string]$Destination
)
$ErrorActionPreference = 'Stop'
$sourceRoot = Split-Path -Parent $PSScriptRoot
if (!$CoreBinary) { $CoreBinary = Join-Path $sourceRoot 'target\release\cloudroom.exe' }
if (!$DesktopBinary) { $DesktopBinary = Join-Path $sourceRoot 'integrations\windows\publish\Cloudroom.Desktop.exe' }
if (!$Destination) { $Destination = Join-Path $sourceRoot 'private\releases\Cloudroom-Windows-alpha' }
if (!(Test-Path -LiteralPath $CoreBinary) -or !(Test-Path -LiteralPath $DesktopBinary)) { throw 'Build release Core and publish the standalone desktop app first.' }
if ((Test-Path -LiteralPath $Destination) -or (Test-Path -LiteralPath ($Destination + '.zip'))) { throw 'Choose a new Destination; packages are never overwritten.' }
[void][System.IO.Directory]::CreateDirectory($Destination)
[void][System.IO.Directory]::CreateDirectory((Join-Path $Destination 'install'))
Copy-Item -LiteralPath $CoreBinary -Destination (Join-Path $Destination 'cloudroom.exe')
Copy-Item -LiteralPath $DesktopBinary -Destination (Join-Path $Destination 'Cloudroom.Desktop.exe')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'LICENSE') -Destination (Join-Path $Destination 'LICENSE')
Copy-Item -LiteralPath (Join-Path $sourceRoot 'install\windows.ps1') -Destination (Join-Path $Destination 'install\windows.ps1')
$guide = @'
# Cloudroom for Windows (alpha)

This portable package includes the native Core runtime and desktop GUI.
Requires Windows 10/11 x64 and a signed-in native codex.exe available on PATH.
No .NET installation is required for the bundled desktop app.

Double-click Start-Cloudroom.cmd. First use creates private runtime settings
under %LOCALAPPDATA%\CloudroomCore, then starts Core and the GUI. If Codex is
not on PATH or its model is not configured, use PowerShell:

  .\install\windows.ps1 -Action Configure -CodexBinary C:\path\codex.exe -Model MODEL_ID
  .\install\windows.ps1 -Action Gui

Closing the GUI leaves agents running. Stop Core with:

  .\install\windows.ps1 -Action Stop

This is a single-user, localhost-only alpha. Agents share your Windows account
and run without approval prompts. Terminals, attachments, other harnesses,
command guards and Linux isolation are not ported. Back up the history directory
while Core is stopped. Never share the generated core.json or account tokens.

Source and full setup guide: https://github.com/Xaiando/cloudroom-core
'@
[System.IO.File]::WriteAllText((Join-Path $Destination 'README.md'), $guide)
$start = '@echo off' + "`r`n" + 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0install\windows.ps1" -Action Gui' + "`r`n" + 'if errorlevel 1 pause' + "`r`n"
[System.IO.File]::WriteAllText((Join-Path $Destination 'Start-Cloudroom.cmd'), $start)
Compress-Archive -LiteralPath $Destination -DestinationPath ($Destination + '.zip')
Write-Output ($Destination + '.zip')
