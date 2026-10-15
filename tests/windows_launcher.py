"""Check first-run discovery from a normal shell with no Codex on PATH. No CLI execution."""
import json
import os
from pathlib import Path
import shutil
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

        def configure(label, binary):
            data = root / label
            result = subprocess.run([str(powershell), '-NoProfile', '-ExecutionPolicy', 'Bypass',
                                     '-File', str(ROOT / 'install/windows.ps1'), '-Action', 'Configure',
                                     '-DataDirectory', str(data), '-CodexHome', str(home), '-Model', 'fixture'],
                                    env=env, capture_output=True, text=True, timeout=30)
            assert result.returncode == 0, result.stderr
            settings = json.loads((data / 'core.json').read_text(encoding='utf-8'))
            assert Path(settings['CLOUDROOM_CODEX_BINARY']) == binary
            assert len(settings['CLOUDROOM_TOKEN']) == 64

        bundled = local / 'OpenAI/Codex/bin/test-version/codex.exe'
        bundled.parent.mkdir(parents=True)
        shutil.copy2(sys.executable, bundled)  # Discovery only; this file is never executed.
        configure('desktop-runtime', bundled)
        bundled.unlink()
        npm = roaming / 'npm/node_modules/@openai/codex/vendor/windows/codex.exe'
        npm.parent.mkdir(parents=True)
        shutil.copy2(sys.executable, npm)
        configure('npm-runtime', npm)
        print('PASS: first-run Codex desktop/npm discovery with a normal shell PATH; private settings generated without executing Codex')


if __name__ == '__main__':
    main()
