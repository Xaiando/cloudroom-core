[CmdletBinding()]
param(
    [ValidateSet('Setup', 'Build', 'Gui')][string]$Action = 'Gui',
    [string]$DesktopDirectory = (Join-Path (Split-Path -Parent $PSScriptRoot) 'private\upstream-gui'),
    [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'CloudroomDesktopPreview'),
    [string]$UserDataDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$launcher = Join-Path $DesktopDirectory 'gui\install\windows.ps1'
if (!(Test-Path -LiteralPath $launcher)) {
    if ($Action -ne 'Setup') { throw 'Set up the original desktop first: powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\install\original-desktop.ps1 -Action Setup' }
    if (Test-Path -LiteralPath $DesktopDirectory) { throw 'The destination exists but is not the Windows desktop checkout. Choose another DesktopDirectory.' }
    git clone --depth 1 --branch windows-native https://github.com/Xaiando/cloudroom-gui.git $DesktopDirectory
    if ($LASTEXITCODE) { throw 'Desktop checkout failed.' }
}
& $launcher -Action $Action -DataDirectory $DataDirectory -UserDataDirectory $UserDataDirectory
if ($Action -eq 'Setup') { & $launcher -Action Build -DataDirectory $DataDirectory }
