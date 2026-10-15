# Native local runtime. Works in Windows PowerShell 5.1 and PowerShell 7.
[CmdletBinding()]
param(
    [ValidateSet('Check', 'Configure', 'Run', 'Start', 'Status', 'Stop', 'Api', 'Gui')]
    [string]$Action = 'Check',
    [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'CloudroomCore'),
    [string]$Repository,
    [string]$Model,
    [string]$CodexBinary,
    [string]$CodexHome = (Join-Path $env:USERPROFILE '.codex'),
    [string]$CoreBinary,
    [ValidateRange(1024, 65535)][int]$Port = 9840,
    [ValidateSet('GET', 'POST', 'DELETE')][string]$Method = 'GET',
    [string]$ApiPath = '/v1/ready',
    [string]$Body
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'This launcher requires native Windows.' }
$sourceRoot = Split-Path -Parent $PSScriptRoot
if (!$CoreBinary) {
    $CoreBinary = Join-Path $sourceRoot 'cloudroom.exe'
    if (!(Test-Path -LiteralPath $CoreBinary)) { $CoreBinary = Join-Path $sourceRoot 'target\release\cloudroom.exe' }
    if (!(Test-Path -LiteralPath $CoreBinary)) { $CoreBinary = Join-Path $sourceRoot 'target\debug\cloudroom.exe' }
}
$configPath = Join-Path $DataDirectory 'core.json'
$pidPath = Join-Path $DataDirectory 'process.json'

function Find-CodexBinary {
    # Codex desktop adds its CLI to its own environment, which an ordinary PowerShell
    # window does not inherit. Prefer a native executable and ignore npm .cmd shims.
    $command = Get-Command codex.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    foreach ($directory in @(
        (Join-Path $env:LOCALAPPDATA 'OpenAI\Codex\bin'),
        (Join-Path $env:APPDATA 'npm\node_modules\@openai')
    )) {
        if (Test-Path -LiteralPath $directory) {
            $binary = Get-ChildItem -LiteralPath $directory -Filter codex.exe -Recurse -File -ErrorAction SilentlyContinue |
                Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
            if ($binary) { return $binary.FullName }
        }
    }
    # Microsoft Store installations live outside the user's ordinary application folder.
    if (Get-Command Get-AppxPackage -ErrorAction SilentlyContinue) {
        foreach ($package in @(Get-AppxPackage -Name '*Codex*' -ErrorAction SilentlyContinue)) {
            if ($package.InstallLocation) {
                $binary = Get-ChildItem -LiteralPath $package.InstallLocation -Filter codex.exe -Recurse -File -ErrorAction SilentlyContinue |
                    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
                if ($binary) { return $binary.FullName }
            }
        }
    }
    return $null
}

function Invoke-CoreApi($Config, $Verb, $Path, $JsonBody) {
    if (!$Path.StartsWith('/v1/') -or $Path.Contains('://')) { throw 'ApiPath must start with /v1/.' }
    $request = @{
        Uri = ('http://' + $Config.CLOUDROOM_LISTEN + $Path)
        Method = $Verb
        Headers = @{ Authorization = ('Bearer ' + $Config.CLOUDROOM_TOKEN) }
        TimeoutSec = 15
    }
    if ($JsonBody) { $request.Body = [System.Text.Encoding]::UTF8.GetBytes($JsonBody); $request.ContentType = 'application/json' }
    Invoke-RestMethod @request
}

function Get-OwnedProcess {
    if (!(Test-Path -LiteralPath $pidPath)) { return $null }
    $saved = Get-Content -LiteralPath $pidPath -Raw | ConvertFrom-Json
    $process = Get-Process -Id $saved.Id -ErrorAction SilentlyContinue
    if ($process -and $process.StartTime.ToUniversalTime().Ticks.ToString() -eq $saved.StartTicks -and $process.Path -eq $saved.Binary) { return $process }
    return $null
}

function Get-StartupDetails($Config) {
    $logPath = Join-Path $DataDirectory 'core.stderr.log'
    $details = ''
    if (Test-Path -LiteralPath $logPath) {
        $details = (@(Get-Content -LiteralPath $logPath -Tail 12) -join [Environment]::NewLine).Trim()
        $details = $details.Replace([string]$Config.CLOUDROOM_TOKEN, '[redacted]')
        if ($details.Length -gt 3000) { $details = $details.Substring($details.Length - 3000) }
    }
    if (!$details) { $details = 'Core did not write an error message.' }
    return ($details + [Environment]::NewLine + 'Log: ' + $logPath)
}

if ($Action -eq 'Gui') {
    $desktopBinary = Join-Path $sourceRoot 'Cloudroom.Desktop.exe'
    if (!(Test-Path -LiteralPath $desktopBinary)) { $desktopBinary = Join-Path $sourceRoot 'integrations\windows\publish\Cloudroom.Desktop.exe' }
    if (!(Test-Path -LiteralPath $desktopBinary)) { throw 'Publish the desktop app first. See integrations/windows/README.md.' }
    Write-Output "Using Core settings: $configPath"
    if (!(Test-Path -LiteralPath $configPath)) {
        & $PSCommandPath -Action Configure -DataDirectory $DataDirectory -Repository $Repository -Model $Model -CodexBinary $CodexBinary -CodexHome $CodexHome -Port $Port
    }
    if (!(Get-OwnedProcess)) { & $PSCommandPath -Action Start -DataDirectory $DataDirectory -CoreBinary $CoreBinary }
    Start-Process -FilePath $desktopBinary -ArgumentList @('--config', ('"' + $configPath + '"'))
    return
}

if ($Action -eq 'Check') {
    foreach ($tool in @('git', 'cargo', 'rustc')) {
        $command = Get-Command $tool -ErrorAction SilentlyContinue
        [pscustomobject]@{ Tool = $tool; Available = [bool]$command; Path = $(if ($command) { $command.Source } else { '' }) }
    }
    $nativeCodex = Find-CodexBinary
    [pscustomobject]@{ Tool = 'codex'; Available = [bool]$nativeCodex; Path = $nativeCodex }
    [pscustomobject]@{ Tool = 'cloudroom'; Available = (Test-Path -LiteralPath $CoreBinary); Path = $CoreBinary }
    return
}

if ($Action -eq 'Configure') {
    if (Test-Path -LiteralPath $configPath) { throw "Configuration already exists at $configPath. Edit it locally to change settings." }
    if (!$CodexBinary) { $CodexBinary = Find-CodexBinary }
    if (!$CodexBinary) { throw 'Could not find native codex.exe. Install Codex, or pass -CodexBinary C:\path\codex.exe. The launcher checks PATH, the Codex desktop bundle, npm packages, and Microsoft Store installations.' }
    $CodexBinary = (Resolve-Path -LiteralPath $CodexBinary).Path
    if ([System.IO.Path]::GetExtension($CodexBinary) -ne '.exe') { throw 'Use a native codex.exe, not an npm .cmd shim.' }
    $CodexHome = (Resolve-Path -LiteralPath $CodexHome).Path
    if (!$Model) {
        $modelFile = Join-Path $CodexHome 'config.toml'
        if (Test-Path -LiteralPath $modelFile) {
            $match = Select-String -LiteralPath $modelFile -Pattern '^model\s*=\s*"([^"]+)"' | Select-Object -First 1
            if ($match) { $Model = $match.Matches[0].Groups[1].Value }
        }
    }
    if (!$Model) { throw 'Supply -Model with a model supported by your Codex login.' }
    if (!$Repository) { $Repository = Join-Path $DataDirectory 'workspace' }
    if ((Test-Path -LiteralPath $DataDirectory) -and (Get-ChildItem -LiteralPath $DataDirectory -Force | Select-Object -First 1)) {
        throw 'Choose a new or empty DataDirectory; Configure changes its ACL.'
    }
    [void][System.IO.Directory]::CreateDirectory($DataDirectory)
    # Protect the new runtime root before writing the bearer token. Agent processes still
    # share this user's identity; this ACL only protects against other ordinary accounts.
    $identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().User
    $acl = New-Object System.Security.AccessControl.DirectorySecurity
    $acl.SetOwner($identity)
    $acl.SetAccessRuleProtection($true, $false)
    foreach ($sid in @($identity, [System.Security.Principal.SecurityIdentifier]::new('S-1-5-18'))) {
        $rule = [System.Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow')
        [void]$acl.AddAccessRule($rule)
    }
    # Apply directly through .NET; first-run setup must also work in clean shells
    # where PowerShell's optional security module has not been loaded.
    if ($PSVersionTable.PSEdition -eq 'Desktop') {
        [System.IO.Directory]::SetAccessControl($DataDirectory, $acl)
    } else {
        [System.IO.FileSystemAclExtensions]::SetAccessControl([System.IO.DirectoryInfo]::new($DataDirectory), $acl)
    }
    [void][System.IO.Directory]::CreateDirectory($Repository)
    [void][System.IO.Directory]::CreateDirectory((Join-Path $DataDirectory 'history'))
    $Repository = (Resolve-Path -LiteralPath $Repository).Path
    $random = New-Object byte[] 32
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($random) } finally { $rng.Dispose() }
    $token = -join ($random | ForEach-Object { $_.ToString('x2') })
    $config = [ordered]@{
        CLOUDROOM_WINDOWS_LOCAL = '1'
        CLOUDROOM_LOCAL_HISTORY = '1'
        CLOUDROOM_TOKEN = $token
        CLOUDROOM_LISTEN = "127.0.0.1:$Port"
        CLOUDROOM_STORE = 'windows-local'
        CLOUDROOM_STATE_DIR = (Join-Path $DataDirectory 'history')
        CLOUDROOM_ACCOUNT_HOME = $env:USERPROFILE
        CLOUDROOM_REPOSITORY = $Repository
        CLOUDROOM_CODEX_BINARY = $CodexBinary
        CLOUDROOM_CODEX_HOME = $CodexHome
        CLOUDROOM_CODEX_MODEL = $Model
    }
    [System.IO.File]::WriteAllText($configPath, ($config | ConvertTo-Json), [System.Text.UTF8Encoding]::new($false))
    Write-Output "Configured native Windows Core. Private settings: $configPath"
    return
}

if (!(Test-Path -LiteralPath $configPath)) { throw 'Run -Action Configure first.' }
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
if ($config.CLOUDROOM_WINDOWS_LOCAL -ne '1' -or $config.CLOUDROOM_LISTEN -notmatch '^127\.0\.0\.1:\d+$') { throw 'This launcher requires Windows local mode on IPv4 loopback.' }
if ($Action -eq 'Api') { Invoke-CoreApi $config $Method $ApiPath $Body; return }
if ($Action -eq 'Status') {
    $owned = Get-OwnedProcess
    [pscustomobject]@{ Running = [bool]$owned; ProcessId = $(if ($owned) { $owned.Id } else { $null }); Url = ('http://' + $config.CLOUDROOM_LISTEN) }
    if ($owned) { Invoke-CoreApi $config 'GET' '/v1/ready' $null }
    return
}
if ($Action -eq 'Stop') {
    $owned = Get-OwnedProcess
    if ($owned) { Stop-Process -InputObject $owned; $owned.WaitForExit(); Write-Output 'Core stopped. Running turns recover as interrupted on restart.' }
    else { Write-Output 'No launcher-owned Core is running.' }
    return
}
if (!(Test-Path -LiteralPath $CoreBinary)) { throw 'Build Core first with cargo build --locked --release.' }
if (Get-OwnedProcess) { throw 'Core is already running. Use -Action Status.' }
$CoreBinary = (Resolve-Path -LiteralPath $CoreBinary).Path
# Load settings only for the launched process, then restore this shell's environment.
$previous = @{}
try {
    foreach ($entry in (Get-ChildItem Env: | Where-Object { $_.Name.StartsWith('CLOUDROOM_') })) {
        $previous[$entry.Name] = $entry.Value
        [Environment]::SetEnvironmentVariable($entry.Name, $null, 'Process')
    }
    foreach ($property in $config.PSObject.Properties) {
        if (!$property.Name.StartsWith('CLOUDROOM_')) { throw 'Unexpected configuration key.' }
        if (!$previous.ContainsKey($property.Name)) { $previous[$property.Name] = $null }
        [Environment]::SetEnvironmentVariable($property.Name, [string]$property.Value, 'Process')
    }
    if ($Action -eq 'Run') { & $CoreBinary; if ($LASTEXITCODE -ne 0) { throw "Core exited with code $LASTEXITCODE" }; return }
    $child = Start-Process -FilePath $CoreBinary -WorkingDirectory $sourceRoot -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput (Join-Path $DataDirectory 'core.stdout.log') -RedirectStandardError (Join-Path $DataDirectory 'core.stderr.log')
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(20)
        do {
            $child.Refresh()
            if ($child.HasExited) {
                $child.WaitForExit()
                throw ('Core exited during startup.' + [Environment]::NewLine + (Get-StartupDetails $config))
            }
            try { $ready = (Invoke-CoreApi $config 'GET' '/v1/ready' $null).ready } catch { $ready = $false }
            if (!$ready) { Start-Sleep -Milliseconds 200 }
        } while (!$ready -and [DateTime]::UtcNow -lt $deadline)
        if (!$ready) { throw ('Core did not become ready within 20 seconds.' + [Environment]::NewLine + (Get-StartupDetails $config)) }
        $saved = @{ Id = $child.Id; StartTicks = $child.StartTime.ToUniversalTime().Ticks.ToString(); Binary = $CoreBinary }
        [System.IO.File]::WriteAllText($pidPath, ($saved | ConvertTo-Json))
        Write-Output ('Core ready at http://' + $config.CLOUDROOM_LISTEN + ' (PID ' + $child.Id + ').')
    } catch {
        if (!$child.HasExited) { $child.Kill(); $child.WaitForExit() }
        throw
    }
} finally {
    foreach ($name in $previous.Keys) { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
}
