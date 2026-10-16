"""Exercise first-run configuration and Core startup in Windows PowerShell 5.1."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def main():
    assert sys.platform == 'win32'
    powershell = Path(os.environ['SystemRoot']) / 'System32/WindowsPowerShell/v1.0/powershell.exe'
    with tempfile.TemporaryDirectory(prefix='cloudroom launcher ') as folder:
        root = Path(folder)
        local, roaming, profile = root / 'local', root / 'roaming', root / 'user'
        home = profile / '.codex'
        home.mkdir(parents=True)
        roaming.mkdir()
        env = dict(os.environ, LOCALAPPDATA=str(local), APPDATA=str(roaming), USERPROFILE=str(profile),
                   PATH=str(powershell.parent) + os.pathsep + str(Path(os.environ['SystemRoot']) / 'System32'))
        env.pop('CODEX_HOME', None)
        # Let Windows PowerShell use its own built-in modules rather than inheriting pwsh's.
        env.pop('PSModulePath', None)

        # A shortcut launched by a 32-bit host must still select native PowerShell,
        # which can inspect the 64-bit Core executable for ownership checks.
        desktop = root / 'desktop-entry'
        (desktop / 'install').mkdir(parents=True)
        shutil.copy2(ROOT / 'Start-Cloud-Bee.cmd', desktop / 'Start-Cloud-Bee.cmd')
        (desktop / 'install/windows.ps1').write_text('''param([string]$Action)
if ($Action -ne 'Gui') { throw 'Expected Gui action' }
[Environment]::Is64BitProcess.ToString() | Set-Content (Join-Path $PSScriptRoot '../architecture.txt')
''')
        for host in ('System32', 'SysWOW64'):
            cmd = Path(os.environ['SystemRoot']) / host / 'cmd.exe'
            if cmd.exists():
                result = subprocess.run([str(cmd), '/d', '/c', str(desktop / 'Start-Cloud-Bee.cmd')],
                                        env=env, stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=20)
                assert result.returncode == 0 and (desktop / 'architecture.txt').read_text().strip() == 'True', (host, result.stdout, result.stderr)

        def launch(action, data, *args, poison_proxy=False):
            # File handles avoid a background Windows child keeping Python's capture
            # pipes open through its inherited standard input handle.
            output = root / 'powershell-output.txt'
            script = ROOT / 'install/windows.ps1'
            if poison_proxy:
                assert action == 'Api' and not args
                script = root / 'proxy-check.ps1'
                script.write_text('''Add-Type @'
using System;
using System.Net;
public sealed class LauncherPoisonProxy : IWebProxy {
    public ICredentials Credentials { get; set; }
    public bool IsBypassed(Uri uri) { return false; }
    public Uri GetProxy(Uri uri) { throw new InvalidOperationException("Local API attempted to use a proxy"); }
}
'@
[System.Net.WebRequest]::DefaultWebProxy = New-Object LauncherPoisonProxy
& 'REPLACE_LAUNCHER' -Action Api -DataDirectory 'REPLACE_DATA'
'''.replace('REPLACE_LAUNCHER', str(ROOT / 'install/windows.ps1').replace("'", "''"))
                     .replace('REPLACE_DATA', str(data).replace("'", "''")), encoding='utf-8')
            with output.open('w', encoding='utf-8') as stream:
                command = [str(powershell), '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(script)]
                if not poison_proxy:
                    command += ['-Action', action, '-DataDirectory', str(data), *map(str, args)]
                result = subprocess.run(command,
                                        env=env, stdin=subprocess.DEVNULL, stdout=stream, stderr=stream,
                                        cwd=os.environ['SystemRoot'], timeout=40)
            return result.returncode, output.read_text(encoding='utf-8', errors='replace')

        def configure(label, binary, port=9840):
            data = root / label
            code, output = launch('Configure', data, '-CodexHome', home, '-Model', 'fixture', '-Port', port)
            assert code == 0, output
            settings = json.loads((data / 'core.json').read_text(encoding='utf-8'))
            assert Path(settings['CLOUDROOM_CODEX_BINARY']).samefile(binary), (settings['CLOUDROOM_CODEX_BINARY'], str(binary))
            assert len(settings['CLOUDROOM_TOKEN']) == 64
            return data

        bundled = local / 'OpenAI/Codex/bin/test-version/codex.exe'
        bundled.parent.mkdir(parents=True)
        shutil.copy2(sys.executable, bundled)  # Discovery only; this file is never executed.
        configure('desktop-runtime', bundled)
        bundled.unlink()
        npm = roaming / 'npm/node_modules/@openai/codex/vendor/windows/codex.exe'
        npm.parent.mkdir(parents=True)
        shutil.copy2(sys.executable, npm)
        configure('npm-runtime', npm)
        core = (Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'target/debug/cloudroom.exe').resolve()
        with socket.socket() as listener:
            listener.bind(('127.0.0.1', 0))
            port = listener.getsockname()[1]
        data = configure('startup-runtime', npm, port)
        saved = json.loads((data / 'core.json').read_text(encoding='utf-8'))
        npm.unlink()
        updated = local / 'OpenAI/Codex/bin/updated-version/codex.exe'
        updated.parent.mkdir(parents=True)
        shutil.copy2(sys.executable, updated)
        try:
            code, output = launch('Start', data, '-CoreBinary', core)
            assert code == 0 and 'Core ready' in output and 'Rediscovered installed Codex' in output, output
            repaired = json.loads((data / 'core.json').read_text(encoding='utf-8'))
            assert Path(repaired['CLOUDROOM_CODEX_BINARY']).samefile(updated), (repaired['CLOUDROOM_CODEX_BINARY'], str(updated))
            assert {k: v for k, v in repaired.items() if k != 'CLOUDROOM_CODEX_BINARY'} == {k: v for k, v in saved.items() if k != 'CLOUDROOM_CODEX_BINARY'}, 'Rediscovery changed other runtime settings'
            code, output = launch('Api', data)
            assert code == 0 and 'True' in output, output
            code, output = launch('Api', data, poison_proxy=True)
            assert code == 0 and 'True' in output, output
            profile = dict(harness='local', provider='local', model='launcher-蜂',
                           endpoint='http://127.0.0.1:9/v1/chat/completions', api_key='')
            code, output = launch('Api', data, '-Method', 'POST', '-ApiPath', '/v1/bee/profiles',
                                  '-Body', json.dumps(profile, ensure_ascii=False))
            assert code == 0 and 'True' in output, output
            profiles = json.loads((Path(repaired['CLOUDROOM_STATE_DIR']) / 'bee/profiles.json').read_text(encoding='utf-8'))
            assert profile in profiles, 'Launcher changed the UTF-8 JSON request body'
            stub = root / 'gui-stub.cmd'
            stub.write_text('@echo off\r\n> "%~dp0gui-started.txt" echo started\r\n')
            marker = root / 'gui-started.txt'

            def open_gui(settings, warning=False):
                marker.unlink(missing_ok=True)
                code, output = launch('Gui', settings, '-DesktopBinary', stub, '-CoreBinary', core)
                deadline = time.monotonic() + 5
                while not marker.exists() and time.monotonic() < deadline:
                    time.sleep(.05)
                assert code == 0 and marker.exists(), output
                assert ('No existing service was stopped' in ' '.join(output.split())) == warning, output
                return output

            # Reopen against the existing Core both with and without its launcher record.
            open_gui(data)
            record = data / 'process.json'
            ownership = record.read_bytes()
            record.unlink()
            try:
                open_gui(data)
                assert not record.exists(), 'Reopening GUI stole ownership of an existing Core'
            finally:
                record.write_bytes(ownership)
            # A second runtime cannot bind the first one's port. Its launch error
            # must include the actual Core error, while leaving the first running.
            conflict = configure('conflict-runtime', updated, port)
            code, output = launch('Start', conflict, '-CoreBinary', core)
            assert code != 0 and 'Core exited during startup.' in output and 'Log:' in output, output
            assert '10048' in output or 'address' in output.lower(), output
            settings = json.loads((conflict / 'core.json').read_text(encoding='utf-8'))
            assert settings['CLOUDROOM_TOKEN'] not in output
            # Wrong credentials must still open connection settings, without killing
            # the existing service, replacing its token or taking over its PID record.
            output = open_gui(conflict, warning=True)
            assert 'Core returned HTTP 401' in output and settings['CLOUDROOM_TOKEN'] not in output
            assert not (conflict / 'process.json').exists()
            assert record.read_bytes() == ownership
            code, output = launch('Api', data)
            assert code == 0 and 'True' in output, output
        finally:
            code, output = launch('Stop', data)
            assert code == 0, output
        print('PASS: clean-shell discovery/rediscovery, Core start/API/stop, proxy-free local requests, GUI reopen with tracked/untracked Core, occupied-port connection screen without takeover, actionable bind errors; no Codex execution')


if __name__ == '__main__':
    main()
