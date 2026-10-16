"""CLI-backed Council without API profiles. No real accounts or inference."""
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import sys
import tempfile
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from windows_native import until, ProcessHandle

ROOT = Path(__file__).resolve().parents[1]

def main():
    binary = Path(sys.argv[1]).resolve()
    with tempfile.TemporaryDirectory(prefix='cloud bee council cli ') as folder:
        root = Path(folder)
        home, repo = root / 'home', root / 'repo'
        repo.mkdir()
        for name in ('.codex', '.grok'):
            (home / name).mkdir(parents=True)
            (home / name / 'auth.json').write_text('{}')
        (home / '.codex/models_cache.json').write_text(json.dumps({'identity': 'NEVER_EXPORT_THIS', 'models': [dict(slug=m, visibility='list') for m in ('fixture', 'unsafe')]}))
        wrapper = root / 'fixture.cmd'
        wrapper.write_text(f'@echo off\r\n"{sys.executable}" -u "{ROOT / "tests/council_cli_fixture.py"}" %*\r\n')
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        token = secrets.token_hex(32)
        env = {k: v for k, v in os.environ.items() if not k.startswith('CLOUDROOM_')}
        env.update(CLOUDROOM_TOKEN=token, CLOUDROOM_LISTEN=f'127.0.0.1:{port}', CLOUDROOM_WINDOWS_LOCAL='1', CLOUDROOM_LOCAL_HISTORY='1', CLOUDROOM_UNPROTECTED_TEST_MODE='1', CLOUDROOM_STORE='cli-test', CLOUDROOM_STATE_DIR=str(root / 'state'), CLOUDROOM_ACCOUNT_HOME=str(home), CLOUDROOM_REPOSITORY=str(repo), CLOUDROOM_CODEX_BINARY=str(wrapper), CLOUDROOM_CODEX_HOME=str(home / '.codex'), CLOUDROOM_CODEX_MODEL='fixture', CLOUDROOM_GROK_BINARY=str(wrapper), CLOUDROOM_GROK_HOME=str(home / '.grok'), OPENAI_API_KEY='must-not-inherit', XAI_API_KEY='must-not-inherit')
        def api(method, path, body=None, expected=200):
            req = Request(f'http://127.0.0.1:{port}{path}', method=method, data=json.dumps(body).encode() if body is not None else None, headers={'Content-Type':'application/json', 'Authorization':'Bearer ' + token})
            try:
                response = urlopen(req, timeout=10)
            except HTTPError as e:
                response = e
            with response:
                raw = response.read()
                assert response.code == expected, (path, response.code, raw)
                return json.loads(raw)
        with (root / 'log').open('wb') as log:
            core = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                def ready():
                    assert core.poll() is None, (root / 'log').read_text(errors='replace')
                    try: return api('GET', '/v1/ready')['ready']
                    except OSError: return False
                until(ready, 'Core ready')
                catalog = api('GET', '/v1/bee/councils/catalog')
                assert len(catalog['models']) == 6 and 'NEVER_EXPORT_THIS' not in json.dumps(catalog), catalog
                members = [dict(harness='codex', model='fixture', provider='codex_cli'), dict(harness='grok', model='grok-4.7', provider='grok_cli')]
                def create(key, selected=members, expected=200):
                    return api('POST', '/v1/bee/councils', dict(request_id=key,title='CLI Council',members=selected,workspace=None,access='discuss',authorize_access=False), expected)
                create('no_fallback', [dict(harness='codex', model='fixture')], 409)
                identity = create('native')['council_id']
                route = '/v1/bee/councils/' + identity
                def round_done(key):
                    def done():
                        rounds = api('GET', route)['council']['rounds']
                        r = next((r for r in rounds if r['request_id'] == key), None)
                        return r if r and r['state'] not in ('broadcasting','planning','executing','synthesizing','cancelling') else None
                    return until(done, 'Council round ' + key)
                api('POST', route + '/messages',dict(request_id='hello',text='hello 蜂',coordinate=True))
                r = round_done('hello')
                assert r['state'] == 'completed' and len(r['tasks']) == 2 and all(t['state'] == 'completed' for t in r['tasks']), r
                assert r['members'] == members
                api('POST', route + '/messages',dict(request_id='tool',text='CLI_TOOLS',coordinate=False,members=members[:1]))
                assert round_done('tool')['replies'][0]['state'] == 'failed'
                unsafe = create('unsafe', [dict(harness='codex',model='unsafe',provider='codex_cli')])['council_id']
                api('POST', '/v1/bee/councils/' + unsafe + '/messages',dict(request_id='reject',text='hello',coordinate=False))
                until(lambda: api('GET', '/v1/bee/councils/' + unsafe)['council']['rounds'][-1]['replies'][0]['state'] == 'failed', 'unsafe CLI policy rejected')
                api('POST', route + '/messages',dict(request_id='hold',text='CLI_HOLD',coordinate=False,members=members[:1]))
                until(lambda: (home / 'descendant.pid').exists(), 'CLI descendant started')
                handle = ProcessHandle(int((home / 'descendant.pid').read_text()))
                api('POST', route + '/cancel', {})
                round_done('hold')
                handle.wait()
                (home / '.grok/hooks.json').write_text('{}')
                assert all(m['provider'] != 'grok_cli' for m in api('GET', '/v1/bee/councils/catalog')['models'])
                create('blocked_hooks', members[1:], 409)
                assert not list(repo.iterdir()), 'CLI accessed the real workspace'
                core.terminate()
                core.wait(timeout=15)
            finally:
                if core.poll() is None: core.kill(); core.wait(timeout=10)
        print('CLI Council passed: no API profiles, explicit backend, broadcast, microtasks, policy rejection, cancellation and descendant cleanup, customization guard')

if __name__ == '__main__': main()
