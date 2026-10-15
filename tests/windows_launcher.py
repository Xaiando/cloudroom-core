"""Exercise first-run configuration and Core startup in Windows PowerShell 5.1."""
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile

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

        def launch(action, data, *args):
            # File handles avoid a background Windows child keeping Python's capture
            # pipes open through its inherited standard input handle.
            output = root / 'powershell-output.txt'
            with output.open('w', encoding='utf-8') as stream:
                result = subprocess.run([str(powershell), '-NoProfile', '-ExecutionPolicy', 'Bypass',
                                         '-File', str(ROOT / 'install/windows.ps1'), '-Action', action,
                                         '-DataDirectory', str(data), *map(str, args)],
                                        env=env, stdin=subprocess.DEVNULL, stdout=stream, stderr=stream,
                                        cwd=os.environ['SystemRoot'], timeout=40)
            return result.returncode, output.read_text(encoding='utf-8', errors='replace')

        def configure(label, binary, port=9840):
            data = root / label
            code, output = launch('Configure', data, '-CodexHome', home, '-Model', 'fixture', '-Port', port)
            assert code == 0, output
            settings = json.loads((data / 'core.json').read_text(encoding='utf-8'))
            assert Path(settings['CLOUDROOM_CODEX_BINARY']) == binary
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
        try:
            code, output = launch('Start', data, '-CoreBinary', core)
            assert code == 0 and 'Core ready' in output, output
            code, output = launch('Api', data)
            assert code == 0 and 'True' in output, output
            # A second runtime cannot bind the first one's port. Its launch error
            # must include the actual Core error, while leaving the first running.
            conflict = configure('conflict-runtime', npm, port)
            code, output = launch('Start', conflict, '-CoreBinary', core)
            assert code != 0 and 'Core exited during startup.' in output and 'Log:' in output, output
            assert '10048' in output or 'address' in output.lower(), output
            settings = json.loads((conflict / 'core.json').read_text(encoding='utf-8'))
            assert settings['CLOUDROOM_TOKEN'] not in output
            code, output = launch('Api', data)
            assert code == 0 and 'True' in output, output
        finally:
            code, output = launch('Stop', data)
            assert code == 0, output
        print('PASS: clean-shell Codex discovery, Core start/API/stop, and actionable port-conflict error; no Codex execution')


if __name__ == '__main__':
    main()
