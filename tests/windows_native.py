"""Native Windows HTTP, recovery and Job Object checks. Uses fixtures, never model inference."""
import ctypes
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parents[1]


def until(check, label, timeout=30):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.05)
    raise AssertionError(f"Timed out: {label}")


class ProcessHandle:
    """Retain a native handle so PID reuse cannot make a cleanup assertion pass."""
    def __init__(self, pid):
        self.kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        self.kernel.OpenProcess.restype = ctypes.c_void_p
        self.kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
        self.kernel.CloseHandle.argtypes = [ctypes.c_void_p]
        self.handle = self.kernel.OpenProcess(0x100000, False, pid)
        assert self.handle, f"Could not retain process {pid}"

    def wait(self):
        try:
            assert self.kernel.WaitForSingleObject(self.handle, 10000) == 0, "Descendant survived Job cleanup"
        finally:
            self.kernel.CloseHandle(self.handle)


def main():
    assert sys.platform == 'win32', 'This test requires native Windows'
    binary = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'target/debug/cloudroom.exe'
    with tempfile.TemporaryDirectory(prefix='cloudroom windows spaces ') as folder:
        root = Path(folder)
        home, repository = root / 'home', root / 'repository'
        for path in [home, repository, home / '.codex']:
            path.mkdir(parents=True, exist_ok=True)
        wrapper = root / 'codex fixture.cmd'
        # Return ordinary drive paths, as a CLI may do even when Core supplies \\?\ paths.
        wrapper.write_text(f'@echo off\r\nset "CODEX_HOME=%CODEX_HOME:~4%"\r\n"{sys.executable}" -u "{ROOT / "tests/core_fixture.py"}" %*\r\n', encoding='utf-8')
        with socket.socket() as free:
            free.bind(('127.0.0.1', 0))
            port = free.getsockname()[1]
        token = secrets.token_hex(32)
        env = {k: v for k, v in os.environ.items() if not k.startswith('CLOUDROOM_')}
        env.update(CLOUDROOM_TOKEN=token, CLOUDROOM_LISTEN=f'127.0.0.1:{port}',
                   CLOUDROOM_WINDOWS_LOCAL='1', CLOUDROOM_LOCAL_HISTORY='1',
                   CLOUDROOM_STORE='windows-test', CLOUDROOM_STATE_DIR=str(root / 'state'),
                   CLOUDROOM_ACCOUNT_HOME=str(home), CLOUDROOM_REPOSITORY=str(repository),
                   CLOUDROOM_CODEX_BINARY=str(wrapper), CLOUDROOM_CODEX_HOME=str(home / '.codex'),
                   CLOUDROOM_CODEX_MODEL='fixture')

        # Configuration must fail closed before binding a socket or creating state.
        for changes, message in [
            ({'CLOUDROOM_WINDOWS_LOCAL': '0'}, 'CLOUDROOM_WINDOWS_LOCAL=1'),
            ({'CLOUDROOM_LISTEN': f'0.0.0.0:{port}', 'CLOUDROOM_ALLOW_NON_LOOPBACK_HTTP': '1'}, 'requires loopback'),
            ({'CLOUDROOM_STORAGE_POLICY': str(root / 'linux-policy.json')}, 'Linux storage policy'),
        ]:
            result = subprocess.run([str(binary)], env={**env, **changes}, capture_output=True, timeout=10,
                                    creationflags=subprocess.CREATE_NO_WINDOW)
            assert result.returncode != 0 and message in result.stderr.decode(errors='replace'), result.stderr

        def api(method, path, body=None, auth=True, expected=200):
            headers = {'Content-Type': 'application/json'}
            if auth:
                headers['Authorization'] = f'Bearer {token}'
            request = Request(f'http://127.0.0.1:{port}{path}',
                              data=json.dumps(body).encode() if body is not None else None,
                              headers=headers, method=method)
            try:
                response = urlopen(request, timeout=15)
            except HTTPError as error:
                response = error
            with response:
                data = response.read()
                assert response.code == expected, (path, response.code, data.decode(errors='replace'))
                return json.loads(data) if data else None

        log = (root / 'core.log').open('wb')
        core = None

        def start():
            process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log,
                                       creationflags=subprocess.CREATE_NO_WINDOW)
            def ready():
                if process.poll() is not None:
                    raise AssertionError((root / 'core.log').read_text(errors='replace'))
                try:
                    return api('GET', '/v1/ready')['ready']
                except OSError:
                    return False
            until(ready, 'readiness')
            return process

        def session(sid):
            return api('GET', f'/v1/sessions/{sid}')['session']

        def open_session(key, **extra):
            result = api('POST', '/v1/sessions', {'request_id': key, 'harness': 'codex', **extra}, expected=202)
            sid = result['session_id']
            until(lambda: session(sid)['state'] == 'idle', f'{sid} started')
            return sid

        def prompt(sid, key, text):
            api('POST', f'/v1/sessions/{sid}/prompts', {'request_id': key, 'text': text}, expected=202)

        try:
            core = start()
            api('GET', '/v1/health', auth=False, expected=401)
            capabilities = api('GET', '/v1/capabilities')
            assert capabilities['platform'] == 'windows'
            assert capabilities['isolation'] == 'same_user'
            for name in ['command_guard', 'computer_use', 'terminals', 'teleport', 'sync']:
                assert capabilities[name] is False, name
            assert capabilities['harnesses'][0]['models'][0]['model'] == 'fixture'
            for option in ['command_guard_enabled', 'strip_ai_co_authors']:
                api('POST', '/v1/sessions', {'request_id': option, option: True}, expected=409)
            for path in ['/v1/sync', '/v1/teleports', '/v1/accounts/codex/import']:
                api('POST', path, {}, expected=501)
                api('POST', path, {}, auth=False, expected=401)

            sid = open_session('native')
            prompt(sid, 'hello', 'hello from Windows — ø')
            until(lambda: session(sid)['receipts']['hello']['state'] == 'completed', 'prompt receipt')
            before = api('GET', f'/v1/sessions/{sid}/events')
            assert before, 'History was not saved'
            native = session(sid)['native_id']
            assert 'hello from Windows' in (home / '.codex/sessions' / f'{native}.jsonl').read_text()

            workspace_sid = open_session('workspace', workspace='project', workspace_name='Windows-project')
            workspace = api('GET', f'/v1/sessions/{workspace_sid}/workspace')
            assert 'Windows-project' in json.dumps(workspace)
            assert (root / 'state/code/Windows-project').is_dir()

            prompt(sid, 'holding', 'hold')
            pid_file = repository / f'{native}.pid'
            until(pid_file.exists, 'background child')
            descendant = ProcessHandle(int(pid_file.read_text()))
            core.kill()
            core.wait(timeout=10)
            descendant.wait()
            core = start()
            until(lambda: session(sid)['state'] in ('idle', 'stopped'), 'session recovered')
            recovered = session(sid)
            assert recovered['native_id'] == native
            assert recovered['receipts']['hello']['state'] == 'completed'
            assert recovered['receipts']['holding']['state'] == 'unknown_after_restart'
            assert api('GET', f'/v1/sessions/{sid}/events'), 'History lost after restart'

            closed = open_session('close-child')
            closed_native = session(closed)['native_id']
            prompt(closed, 'holding-close', 'hold')
            close_pid = repository / f'{closed_native}.pid'
            until(close_pid.exists, 'second background child')
            descendant = ProcessHandle(int(close_pid.read_text()))
            api('POST', f'/v1/sessions/{closed}/close', {'request_id': 'close'}, expected=202)
            until(lambda: session(closed)['state'] == 'closed', 'session closed')
            descendant.wait()
            print('PASS: native API/auth/catalog, prompts, workspace paths with spaces, journal recovery, and descendant cleanup on close/crash')
        finally:
            if core and core.poll() is None:
                core.kill()
                core.wait(timeout=10)
            log.close()


if __name__ == '__main__':
    main()
