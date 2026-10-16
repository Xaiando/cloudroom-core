# Create a per-user launcher; no administrator privileges are needed.
[CmdletBinding()]
param(
    [string]$DestinationDirectory = [Environment]::GetFolderPath('DesktopDirectory')
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This launcher requires native Windows.' }
$sourceRoot = Split-Path -Parent $PSScriptRoot
$launcher = Join-Path $sourceRoot 'Start-Cloud-Bee.cmd'
if (!(Test-Path -LiteralPath $launcher)) { throw "Launcher not found: $launcher" }
if (!(Test-Path -LiteralPath $DestinationDirectory -PathType Container)) { throw 'Shortcut destination must be an existing directory.' }
$shortcutPath = Join-Path $DestinationDirectory 'Cloud Bee.lnk'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
if ((Test-Path -LiteralPath $shortcutPath) -and $shortcut.TargetPath -ne $launcher) {
    throw "A shortcut to another application already exists at $shortcutPath. Choose another DestinationDirectory."
}
$shortcut.TargetPath = $launcher
$shortcut.WorkingDirectory = $sourceRoot
$shortcut.Description = 'Start Cloud Bee and its native Windows Core'
$shortcut.WindowStyle = 1
foreach ($icon in @(
    (Join-Path $sourceRoot 'Cloudroom.Desktop.exe'),
    (Join-Path $sourceRoot 'integrations\windows\publish\Cloudroom.Desktop.exe')
)) {
    if (Test-Path -LiteralPath $icon) { $shortcut.IconLocation = $icon + ',0'; break }
}
$shortcut.Save()
Write-Output "Created desktop launcher: $shortcutPath"
