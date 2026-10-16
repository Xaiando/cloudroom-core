# Native local runtime. Works in Windows PowerShell 5.1 and PowerShell 7.
[CmdletBinding()]
param(
    [ValidateSet('Check', 'Configure', 'Run', 'Start', 'Status', 'Stop', 'Api', 'Gui', 'Resolve')]
    [string]$Action = 'Check',
    [string]$DataDirectory = (Join-Path $env:LOCALAPPDATA 'CloudroomCore'),
    [string]$Repository,
    [string]$Model,
    [string]$CodexBinary,
    [string]$CodexHome = (Join-Path $env:USERPROFILE '.codex'),
    [string]$CoreBinary,
    [string]$DesktopBinary,
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

function Find-GrokBinary {
    $native = Join-Path $env:USERPROFILE '.grok\bin\grok.exe'
    if (Test-Path -LiteralPath $native) { return (Resolve-Path -LiteralPath $native).Path }
    $command = Get-Command grok.exe -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($command) { return $command.Source }
    return $null
}

function Find-QwenBinary {
    foreach ($name in @('qwen.exe', 'qwen.cmd')) {
        $command = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($command -and $command.Source -notlike '*.ps1') { return $command.Source }
    }
    $npm = Join-Path $env:APPDATA 'npm\qwen.cmd'
    if (Test-Path -LiteralPath $npm) { return (Resolve-Path -LiteralPath $npm).Path }
    return $null
}

function Invoke-CoreApi($Config, $Verb, $Path, $JsonBody, $TimeoutSeconds = 15) {
    if (!$Path.StartsWith('/v1/') -or $Path.Contains('://')) { throw 'ApiPath must start with /v1/.' }
    $baseUri = [Uri]('http://' + $Config.CLOUDROOM_LISTEN)
    if (!$baseUri.IsLoopback -or $baseUri.UserInfo -or $baseUri.AbsolutePath -ne '/' -or $baseUri.Query -or $baseUri.Fragment) {
        throw 'Core settings must use a localhost address.'
    }
    # Match the desktop client: local credentials must never use a system proxy
    # or follow a redirect. HttpClient also works in Windows PowerShell 5.1.
    Add-Type -AssemblyName System.Net.Http
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.UseProxy = $false
    $handler.AllowAutoRedirect = $false
    $http = [System.Net.Http.HttpClient]::new($handler)
    $http.Timeout = [TimeSpan]::FromSeconds($TimeoutSeconds)
    $http.MaxResponseContentBufferSize = 32 * 1024 * 1024
    $request = $null
    $response = $null
    try {
        $request = [System.Net.Http.HttpRequestMessage]::new([System.Net.Http.HttpMethod]::new($Verb), [Uri]::new($baseUri, $Path))
        $request.Headers.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new('Bearer', [string]$Config.CLOUDROOM_TOKEN)
        if ($JsonBody) { $request.Content = [System.Net.Http.StringContent]::new($JsonBody, [System.Text.Encoding]::UTF8, 'application/json') }
        $response = $http.SendAsync($request).GetAwaiter().GetResult()
        if (!$response.IsSuccessStatusCode) { throw ('Core returned HTTP ' + [int]$response.StatusCode + '.') }
        $response.Content.ReadAsStringAsync().GetAwaiter().GetResult() | ConvertFrom-Json
    } finally {
        if ($response) { $response.Dispose() }
        if ($request) { $request.Dispose() }
        $http.Dispose()
    }
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

function Get-PhysicalRuntimePath([string]$Path) {
    if (!(Test-Path -LiteralPath $Path)) { return [IO.Path]::GetFullPath($Path) }
    if (!('CloudBee.RuntimePaths' -as [type])) {
        Add-Type @'
using System;
using System.IO;
using System.Text;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
namespace CloudBee {
public static class RuntimePaths {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFile(string path, uint access, uint share, IntPtr security, uint creation, uint flags, IntPtr template);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern uint GetFinalPathNameByHandle(SafeFileHandle handle, StringBuilder path, uint length, uint flags);
    public static string Resolve(string path) {
        using (var handle = CreateFile(path, 0, 7, IntPtr.Zero, 3, 0x02000000, IntPtr.Zero)) {
            if (handle.IsInvalid) throw new IOException("Cannot resolve the runtime settings path.");
            var buffer = new StringBuilder(32768);
            uint length = GetFinalPathNameByHandle(handle, buffer, 32768, 0);
            if (length == 0 || length >= 32768) throw new IOException("Cannot resolve the runtime settings path.");
            string resolved = buffer.ToString();
            if (resolved.StartsWith(@"\\?\UNC\")) return @"\\" + resolved.Substring(8);
            return resolved.StartsWith(@"\\?\") ? resolved.Substring(4) : resolved;
        }
    }
}}
'@
    }
    [CloudBee.RuntimePaths]::Resolve($Path)
}

function Resolve-CoreSettingsPath {
    $candidates = @($configPath)
    # MSIX can redirect the same AppData string to a different physical file.
    # Explicit data directories always win; discovery only applies to defaults.
    if (!$script:PSBoundParameters.ContainsKey('DataDirectory')) {
        $packages = Join-Path $env:LOCALAPPDATA 'Packages'
        if (Test-Path -LiteralPath $packages) {
            foreach ($package in @(Get-ChildItem -LiteralPath $packages -Directory -Filter 'OpenAI.Codex_*')) {
                $candidates += Join-Path $package.FullName 'LocalCache\Local\CloudroomCore\core.json'
            }
        }
    }
    $existing = @()
    foreach ($candidate in $candidates) {
        if (!(Test-Path -LiteralPath $candidate -PathType Leaf)) { continue }
        $physical = Get-PhysicalRuntimePath $candidate
        if ($existing -contains $physical) { continue }
        $existing += $physical
        try {
            $candidateConfig = Get-Content -LiteralPath $physical -Raw | ConvertFrom-Json
            if ((Invoke-CoreApi $candidateConfig 'GET' '/v1/ready' $null 2).ready) { return $physical }
        } catch { }
    }
    if ($existing.Count -gt 0) { return $existing[0] }
    return $configPath
}

if ($Action -notin @('Check', 'Configure')) {
    $configPath = Resolve-CoreSettingsPath
    $DataDirectory = Split-Path -Parent $configPath
    $pidPath = Join-Path $DataDirectory 'process.json'
}
if ($Action -eq 'Resolve') { Write-Output $configPath; return }

if ($Action -eq 'Gui') {
    if (!$DesktopBinary) {
        $DesktopBinary = Join-Path $sourceRoot 'Cloudroom.Desktop.exe'
        if (!(Test-Path -LiteralPath $DesktopBinary)) { $DesktopBinary = Join-Path $sourceRoot 'integrations\windows\publish\Cloudroom.Desktop.exe' }
    }
    if (!(Test-Path -LiteralPath $desktopBinary)) { throw 'Publish the desktop app first. See integrations/windows/README.md.' }
    Write-Output "Using Core settings: $configPath"
    if (!(Test-Path -LiteralPath $configPath)) {
        & $PSCommandPath -Action Configure -DataDirectory $DataDirectory -Repository $Repository -Model $Model -CodexBinary $CodexBinary -CodexHome $CodexHome -Port $Port
    }
    if (!(Get-OwnedProcess)) {
        $guiConfig = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
        $alreadyReady = $false
        $probeFailure = ''
        try { $alreadyReady = (Invoke-CoreApi $guiConfig 'GET' '/v1/ready' $null).ready } catch { $probeFailure = $_.Exception.Message.Replace([string]$guiConfig.CLOUDROOM_TOKEN, '[redacted]') }
        if (!$alreadyReady) {
            $endpoint = [Uri]('http://' + $guiConfig.CLOUDROOM_LISTEN)
            $probe = New-Object System.Net.Sockets.TcpClient
            try { $portInUse = $probe.ConnectAsync($endpoint.Host, $endpoint.Port).Wait(500) -and $probe.Connected } catch { $portInUse = $false } finally { $probe.Dispose() }
            if ($portInUse) {
                Write-Warning ('An existing process is listening at ' + $guiConfig.CLOUDROOM_LISTEN + '. Opening Cloud Bee to show its connection status. No existing service was stopped. ' + $probeFailure)
            } else {
                & $PSCommandPath -Action Start -DataDirectory $DataDirectory -CoreBinary $CoreBinary
            }
        }
    }
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
    $grokBinary = Find-GrokBinary
    [pscustomobject]@{ Tool = 'grok'; Available = [bool]$grokBinary; Path = $grokBinary }
    $qwenBinary = Find-QwenBinary
    [pscustomobject]@{ Tool = 'qwen'; Available = [bool]$qwenBinary; Path = $qwenBinary }
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
    $grokBinary = Find-GrokBinary
    if ($grokBinary) {
        $grokHome = Join-Path $env:USERPROFILE '.grok'
        if (Test-Path -LiteralPath $grokHome) {
            $config.CLOUDROOM_GROK_BINARY = $grokBinary
            $config.CLOUDROOM_GROK_HOME = (Resolve-Path -LiteralPath $grokHome).Path
            $config.CLOUDROOM_GROK_MODEL = 'grok-4.7'
        }
    }
    $qwenBinary = Find-QwenBinary
    if ($qwenBinary) {
        $qwenHome = Join-Path $env:USERPROFILE '.qwen'
        if (!(Test-Path -LiteralPath $qwenHome)) { [void][System.IO.Directory]::CreateDirectory($qwenHome) }
        $config.CLOUDROOM_QWEN_BINARY = $qwenBinary
        $config.CLOUDROOM_QWEN_HOME = (Resolve-Path -LiteralPath $qwenHome).Path
        $config.CLOUDROOM_QWEN_MODEL = 'qwen3-coder-plus'
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
if (!(Test-Path -LiteralPath ([string]$config.CLOUDROOM_CODEX_BINARY) -PathType Leaf)) {
    $rediscoveredCodex = Find-CodexBinary
    if (!$rediscoveredCodex) { throw 'The saved Codex executable is missing and no installed codex.exe was found. Configure Cloud Bee with -Action Configure -CodexBinary C:\path\codex.exe -Model MODEL_ID.' }
    $config.CLOUDROOM_CODEX_BINARY = $rediscoveredCodex
    $pendingConfig = $configPath + '.pending'
    [System.IO.File]::WriteAllText($pendingConfig, ($config | ConvertTo-Json), [System.Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $pendingConfig -Destination $configPath -Force
    Write-Output 'Rediscovered installed Codex executable. Existing runtime settings retained.'
}
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
