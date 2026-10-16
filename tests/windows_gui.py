"""Exercise the actual WPF window against a disposable Codex fixture, with no inference."""
import json
import os
from pathlib import Path
import secrets
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from bee_fixture import ChatFixture

ROOT = Path(__file__).resolve().parents[1]


class DesktopDirectory(tempfile.TemporaryDirectory):
    def cleanup(self):
        target = Path(self.name).resolve()
        assert target.parent == Path(tempfile.gettempdir()).resolve() and target.name.startswith('cloudroom desktop test '), 'Unexpected disposable cleanup target'
        deadline = time.monotonic() + 5
        while True:
            try:
                return super().cleanup()
            except PermissionError:
                # Core exit can precede final release of its owned Windows child handles.
                if time.monotonic() >= deadline:
                    raise
                time.sleep(.05)


def main():
    assert sys.platform == 'win32', 'Native WPF requires Windows'
    gui = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'integrations/windows/publish/Cloudroom.Desktop.exe'
    core = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else ROOT / 'target/debug/cloudroom.exe'
    artifacts = Path(sys.argv[3]).resolve() if len(sys.argv) > 3 else ROOT / 'private/gui-qa'
    artifacts.mkdir(parents=True, exist_ok=True)
    with DesktopDirectory(prefix='cloudroom desktop test ') as folder:
        root = Path(folder)
        home = root / 'home'
        repository = root / 'repository'
        (home / '.codex').mkdir(parents=True)
        repository.mkdir()
        wrapper = root / 'fixture.cmd'
        wrapper.write_text(f'@echo off\r\nset "CODEX_HOME=%CODEX_HOME:~4%"\r\n"{sys.executable}" -u "{ROOT / "tests/core_fixture.py"}" %*\r\n', encoding='utf-8')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        config = dict(CLOUDROOM_TOKEN=secrets.token_hex(32), CLOUDROOM_LISTEN=f'127.0.0.1:{port}',
                      CLOUDROOM_WINDOWS_LOCAL='1', CLOUDROOM_LOCAL_HISTORY='1', CLOUDROOM_UNPROTECTED_TEST_MODE='1', CLOUDROOM_STORE='gui-fixture',
                      CLOUDROOM_STATE_DIR=str(root / 'state'), CLOUDROOM_ACCOUNT_HOME=str(home),
                      CLOUDROOM_REPOSITORY=str(repository), CLOUDROOM_CODEX_BINARY=str(wrapper),
                      CLOUDROOM_CODEX_HOME=str(home / '.codex'), CLOUDROOM_CODEX_MODEL='fixture')
        settings = root / 'core.json'
        settings.write_text(json.dumps(config), encoding='utf-8')
        env = {k: v for k, v in os.environ.items() if not k.startswith('CLOUDROOM_')}
        env.update(config)
        with (root / 'core.log').open('wb') as log:
            server = subprocess.Popen([str(core)], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                for _ in range(200):
                    if server.poll() is not None:
                        raise AssertionError((root / 'core.log').read_text(errors='replace'))
                    try:
                        request = Request(f'http://127.0.0.1:{port}/v1/ready', headers={'Authorization': 'Bearer ' + config['CLOUDROOM_TOKEN']})
                        with urlopen(request, timeout=2) as response:
                            if json.load(response)['ready']:
                                break
                    except OSError:
                        pass
                    time.sleep(.05)
                else:
                    raise AssertionError('Core did not start')
                chat = ChatFixture()
                for model in ('fixture', 'basic'):
                    request = Request(f'http://127.0.0.1:{port}/v1/bee/profiles', method='POST',
                                      headers={'Authorization': 'Bearer ' + config['CLOUDROOM_TOKEN'], 'Content-Type': 'application/json'},
                                      data=json.dumps(dict(harness='codex', model=model, provider='local', endpoint=chat.endpoint, api_key='')).encode())
                    with urlopen(request, timeout=10) as response:
                        assert response.code == 200
                prompt_requests = {}
                request_trace = []
                dropped_library_read = []
                dropped_library_write = []

                class Proxy(BaseHTTPRequestHandler):
                    protocol_version = 'HTTP/1.1'
                    def handle(self):
                        try:
                            super().handle()
                        except ConnectionError:
                            pass
                    def log_message(self, *_args):
                        pass

                    def do_GET(self):
                        self.forward()

                    def do_POST(self):
                        self.forward()

                    def forward(self):
                        body = self.rfile.read(int(self.headers.get('Content-Length', 0)))
                        request = Request(f'http://127.0.0.1:{port}{self.path}',
                                          data=body if self.command == 'POST' else None,
                                          method=self.command,
                                          headers={'Authorization': self.headers.get('Authorization', ''), 'Content-Type': 'application/json'})
                        try:
                            response = urlopen(request, timeout=20)
                        except HTTPError as error:
                            response = error
                        with response:
                            data, status = response.read(), response.code
                        if self.command == 'GET' and self.path.startswith('/v1/bee/sessions/') and '/events' not in self.path and status == 200:
                            state = json.loads(data)
                            workspace = state.get('session', {}).get('workspace', {}).get('path')
                            if workspace:
                                folder = Path(workspace)
                                folder.mkdir(parents=True, exist_ok=True)
                                (folder / 'preview.md').write_text('# Native preview\n\n**Markdown** with Unicode ø — 蜂.\n\n- [x] Keep our GUI\n- [ ] More parity\n\n| Feature | State |\n| --- | --- |\n| File previews | Native |\n\n```python\ndef hello():\n    print("Cloud Bee")\n```\n\n[Source](source.py#L2)\n', encoding='utf-8')
                                (folder / 'source.py').write_text('def hello():\n    print("Cloud Bee")\n', encoding='utf-8')
                        request_trace.append((self.command, self.path, status))
                        if self.path == '/v1/bee/library' and self.command == 'POST' and not dropped_library_write:
                            dropped_library_write.append(json.loads(body))
                            self.close_connection = True
                            self.connection.shutdown(socket.SHUT_RDWR)
                            self.connection.close()
                            return
                        if self.path == '/v1/bee/library' and self.command == 'GET' and not dropped_library_read:
                            dropped_library_read.append(True)
                            self.close_connection = True
                            self.connection.shutdown(socket.SHUT_RDWR)
                            self.connection.close()
                            return
                        if self.path.endswith('/prompts') and self.command == 'POST':
                            sent = prompt_requests.setdefault(self.path, [])
                            sent.append(json.loads(body))
                            if len(prompt_requests) == 1 and len(sent) == 1:
                                # Core accepted the prompt, but the GUI never receives its receipt.
                                self.close_connection = True
                                self.connection.shutdown(socket.SHUT_RDWR)
                                self.connection.close()
                                return
                        try:
                            self.send_response(status)
                            self.send_header('Content-Type', 'application/json')
                            self.send_header('Content-Length', str(len(data)))
                            self.end_headers()
                            self.wfile.write(data)
                        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                            pass

                proxy = ThreadingHTTPServer(('127.0.0.1', 0), Proxy)
                threading.Thread(target=proxy.serve_forever, daemon=True).start()
                settings.write_text(json.dumps({**config, 'CLOUDROOM_LISTEN': f'127.0.0.1:{proxy.server_port}'}), encoding='utf-8')
                output = root / 'qa'
                try:
                    result = subprocess.run([str(gui), '--config', str(settings), '--smoke-test', str(output)], timeout=120)
                finally:
                    proxy.shutdown()
                    proxy.server_close()
                    chat.close()
                if output.is_dir():
                    for artifact in output.iterdir():
                        shutil.copy2(artifact, artifacts / artifact.name)
                if result.returncode != 0:
                    log.flush()
                    shutil.copy2(root / 'core.log', artifacts / 'core-fixture-failure.log')
                    (artifacts / 'request-fixture-failure.json').write_text(json.dumps(prompt_requests))
                    (artifacts / 'trace-fixture-failure.json').write_text(json.dumps(request_trace))
                assert result.returncode == 0, (output / 'failure.txt').read_text() if (output / 'failure.txt').exists() else f'GUI exited with {result.returncode}'
                report = json.loads((output / 'result.json').read_text())
                assert report['passed'] is True
                extensions = json.loads((output / 'extensions-result.json').read_text())
                assert extensions['passed'] is True
                library = json.loads((output / 'library-result.json').read_text())
                assert library['passed'] is True
                organization = json.loads((output / 'organization-result.json').read_text())
                assert organization['passed'] is True
                presentation = json.loads((output / 'presentation-result.json').read_text())
                assert presentation['passed'] is True
                assert dropped_library_read, 'The library did not exercise transport recovery for read requests'
                assert dropped_library_write, 'The library did not exercise deliberate recovery of an unconfirmed save'
                assert report['retried_lost_acceptance'] is True
                first, *others = [sent for path, sent in prompt_requests.items() if not path.startswith('/v1/bee/')]
                assert len(first) == 2 and first[0] == first[1], 'Retry changed the request ID or payload'
                assert report['multi_model'] is True
                assert len(others) == 2 and all(len(sent) == 1 for sent in others), 'Each model must receive the prompt exactly once'
                assert others[0][0]['request_id'] != others[1][0]['request_id'] and others[0][0]['text'] == others[1][0]['text']
                print('PASS: actual WPF window; ' + report['checks'] + '; ' + extensions['checks'] + '; ' + library['checks'] + '; ' + organization['checks'] + '; ' + presentation['checks'])
                print('Rendered UI:', artifacts)
            finally:
                if server.poll() is None:
                    server.kill()
                    server.wait(timeout=10)


if __name__ == '__main__':
    main()
