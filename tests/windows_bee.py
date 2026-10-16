"""Cloud Bee mode, authorization, handoff, persistence and exactly-once fixture checks."""
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
from bee_fixture import ChatFixture
from deepseek_fixture import exercise_gemini
from windows_native import until

ROOT = Path(__file__).resolve().parents[1]


def main():
    assert sys.platform == 'win32'
    binary = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'target/debug/cloudroom.exe'
    chat = ChatFixture()
    try:
        with tempfile.TemporaryDirectory(prefix='cloud bee modes ') as folder:
            root = Path(folder)
            home, repo = root / 'home', root / 'repo'
            (home / '.codex').mkdir(parents=True)
            repo.mkdir()
            wrapper = root / 'fixture.cmd'
            wrapper.write_text(f'@echo off\r\nset "CODEX_HOME=%CODEX_HOME:~4%"\r\n"{sys.executable}" -u "{ROOT / "tests/core_fixture.py"}" %*\r\n')
            with socket.socket() as sock:
                sock.bind(('127.0.0.1', 0))
                port = sock.getsockname()[1]
            token = secrets.token_hex(32)
            env = {k: v for k, v in os.environ.items() if not k.startswith('CLOUDROOM_')}
            env.update(CLOUDROOM_TOKEN=token, CLOUDROOM_LISTEN=f'127.0.0.1:{port}', CLOUDROOM_WINDOWS_LOCAL='1',
                       CLOUDROOM_LOCAL_HISTORY='1', CLOUDROOM_UNPROTECTED_TEST_MODE='1', CLOUDROOM_STORE='bee-fixture',
                       CLOUDROOM_STATE_DIR=str(root / 'state'), CLOUDROOM_ACCOUNT_HOME=str(home),
                       CLOUDROOM_REPOSITORY=str(repo), CLOUDROOM_CODEX_BINARY=str(wrapper),
                       CLOUDROOM_CODEX_HOME=str(home / '.codex'), CLOUDROOM_CODEX_MODEL='fixture')

            def api(method, path, body=None, expected=200):
                request = Request(f'http://127.0.0.1:{port}{path}', method=method,
                                  data=json.dumps(body).encode() if body is not None else None,
                                  headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
                try:
                    response = urlopen(request, timeout=20)
                except HTTPError as error:
                    response = error
                with response:
                    data = response.read()
                    assert response.code == expected, (path, response.code, data)
                    return json.loads(data)

            logs = (root / 'core.log').open('wb')
            core = None

            def start():
                process = subprocess.Popen([str(binary)], env=env, stdout=logs, stderr=logs, creationflags=subprocess.CREATE_NO_WINDOW)
                def ready():
                    assert process.poll() is None, (root / 'core.log').read_text(errors='replace')
                    try:
                        return api('GET', '/v1/ready')['ready']
                    except OSError:
                        return False
                until(ready, 'Core readiness')
                return process

            try:
                core = start()
                caps = api('GET', '/v1/bee/capabilities')['harnesses']
                assert caps[0]['modes'] == 'WORK_ONLY' and caps[0]['chat_models'] == []
                assert any(h['id'] == 'gemini' and any(m['model'] == 'gemini-2.5-pro' for m in h['models']) for h in caps)
                for model in ('fixture', 'basic'):
                    api('POST', '/v1/bee/profiles', dict(harness='codex', model=model, provider='local', endpoint=chat.endpoint, api_key='PRIVATE-BEE-KEY'))
                api('POST', '/v1/bee/profiles', dict(harness='local', model='local-fixture', provider='local', endpoint=chat.endpoint, api_key=''))
                api('POST', '/v1/bee/profiles', dict(harness='gemini', model='gemini-2.5-pro', provider='gemini', endpoint='https://generativelanguage.googleapis.com/v1beta/openai/chat/completions', api_key='PRIVATE-BEE-KEY'))
                api('POST', '/v1/bee/profiles', dict(harness='local', model='bad', provider='local', endpoint='http://10.0.0.1/v1/chat/completions', api_key=''), expected=409)
                caps = api('GET', '/v1/bee/capabilities')['harnesses']
                assert next(h for h in caps if h['id'] == 'codex')['modes'] == 'CHAT_AND_WORK'
                assert next(h for h in caps if h['id'] == 'local')['modes'] == 'CHAT_ONLY'
                assert next(h for h in caps if h['id'] == 'gemini')['modes'] == 'CHAT_ONLY'
                assert 'PRIVATE-BEE-KEY' not in json.dumps(caps)

                def create(key, model='fixture', harness='codex'):
                    body = dict(request_id=key, harness=harness, model=model, workspace='bee_' + key, workspace_name='bee-' + key, group='council-fixture')
                    result = api('POST', '/v1/bee/sessions', body)
                    assert api('POST', '/v1/bee/sessions', body) == result
                    return result['session_id']

                lead, peer = create('lead'), create('peer', 'basic')
                local = create('local', 'local-fixture', 'local')
                def state(sid):
                    return api('GET', '/v1/bee/sessions/' + sid)['session']
                def post(sid, command, body, expected=200):
                    return api('POST', '/v1/bee/sessions/' + sid + '/' + command, body, expected)
                def prompt(sid, key, text):
                    return post(sid, 'prompts', dict(request_id=key, text=text))
                def settled(sid, request):
                    return state(sid)['receipts'].get(request, {}).get('state') in ('completed', 'failed', 'unknown_after_restart')

                assert api('GET', '/v1/sessions')['total'] == 0, 'Chat room creation spawned an agent'
                exercise_gemini(api, chat, until)
                chat.calls.clear()
                prompt(lead, 'think', 'Discuss a project')
                until(lambda: settled(lead, 'think'), 'Chat response')
                prompt(lead, 'think', 'Discuss a project')
                assert len(chat.calls) == 1, 'Retry duplicated inference'
                post(lead, 'prompts', dict(request_id='think', text='different'), 409)
                prompt(lead, 'tool', 'CHAT_FIXTURE_TOOL')
                until(lambda: settled(lead, 'tool'), 'rejected tool response')
                assert state(lead)['receipts']['tool']['state'] == 'failed'
                assert api('GET', '/v1/sessions')['total'] == 0 and not (repo / 'denied').exists()

                handoff = dict(objective='Implement approved design', context='Discuss a project ' + token + ' PRIVATE-BEE-KEY', requirements='Preserve design', constraints='Stay in workspace', acceptance='Fixture passes', artifacts='design.txt')
                switch = dict(request_id='work_switch', mode='work', authorize_work=False, handoff=handoff)
                post(lead, 'mode', switch, 409)
                post(local, 'mode', {**switch, 'authorize_work': True}, 409)
                assert api('GET', '/v1/sessions')['total'] == 0
                switch['authorize_work'] = True
                post(lead, 'mode', switch)
                post(lead, 'mode', switch)
                until(lambda: state(lead)['state'] == 'idle', 'authorized Work startup')
                native = state(lead)['execution_session_id']
                assert api('GET', '/v1/sessions')['total'] == 1
                native_state = api('GET', '/v1/sessions/' + native)['session']
                rollout = Path(native_state['native_path'])
                launch = next(json.loads(line) for line in rollout.read_text().splitlines() if json.loads(line).get('fixture') == 'launch')
                assert launch['approvalPolicy'] == 'untrusted' and launch['sandbox'] == 'workspace-write'
                assert token not in json.dumps(state(lead)['handoffs']) and 'PRIVATE-BEE-KEY' not in json.dumps(state(lead)['handoffs'])
                prompt(lead, 'approval', 'BEE_FIXTURE_APPROVAL')
                work_folder = Path(native_state['workspace']['path'])
                until(lambda: (work_folder / 'bee-approval.json').exists(), 'approval interaction denied')
                assert json.loads((work_folder / 'bee-approval.json').read_text())['error']['code'] == -32601
                assert not (work_folder / 'bee-unapproved-action').exists()
                until(lambda: state(lead)['receipts']['approval']['state'] == 'completed', 'denied approval turn finishes')
                prompt(lead, 'execute', 'BEE_FIXTURE_HOLD')
                until(lambda: state(lead)['state'] == 'running', 'Work tool running')
                assert (Path(native_state['workspace']['path']) / (native_state['native_id'] + '.pid')).exists()
                assert 'Implement approved design' in rollout.read_text(), 'handoff was not delivered'
                chat_switch = dict(request_id='chat_switch', mode='chat', handoff={**handoff, 'objective': 'Discuss current progress'})
                before = len(chat.calls)
                post(lead, 'mode', chat_switch)
                assert len(chat.calls) == before, 'Switching mode made an inference call'
                assert state(lead)['mode'] == 'chat' and state(lead)['work_state'] == 'running'
                assert state(peer)['mode'] == 'chat'
                prompt(lead, 'execute', 'BEE_FIXTURE_HOLD')
                assert len(chat.calls) == before, 'Retry after mode switch changed dispatch mode'
                post(lead, 'interrupt', dict(request_id='cancel'))
                until(lambda: state(lead)['work_state'] == 'idle', 'background Work interruption')
                prompt(lead, 'compare_lead', 'Compare designs')
                prompt(peer, 'compare_peer', 'Compare designs')
                until(lambda: settled(lead, 'compare_lead') and settled(peer, 'compare_peer'), 'Council broadcast')
                assert any('User-reviewed handoff' in json.dumps(call) for call in chat.calls)
                api('POST', '/v1/sessions/' + native + '/close', dict(request_id='native_close'), expected=202)
                until(lambda: state(lead)['work_state'] == 'closed', 'Work close')

                prompt(peer, 'uncertain', 'CHAT_FIXTURE_WAIT')
                until(lambda: any('CHAT_FIXTURE_WAIT' in json.dumps(call) for call in chat.calls), 'pending Chat reached provider')
                count = len(chat.calls)
                core.kill(); core.wait(timeout=10)
                core = start()
                assert next(h for h in api('GET', '/v1/bee/capabilities')['harnesses'] if h['id'] == 'gemini')['chat_models'] == ['gemini-2.5-pro', 'gemini-2.5-flash']
                assert state(lead)['mode'] == 'chat' and state(lead)['execution_session_id'] == native
                assert state(peer)['receipts']['uncertain']['state'] == 'unknown_after_restart'
                prompt(peer, 'uncertain', 'CHAT_FIXTURE_WAIT')
                time.sleep(.15)
                assert len(chat.calls) == count, 'Restart replayed an uncertain request'
                assert state(peer)['group'] == state(lead)['group'] == 'council-fixture'
                assert state(peer)['workspace']['path'] != state(lead)['workspace']['path']
                assert state(lead)['receipts']['compare_lead']['usage']['total_tokens'] == 7
                assert 'PRIVATE-BEE-KEY' not in (root / 'core.log').read_text(errors='replace')
                assert token not in (root / 'core.log').read_text(errors='replace')
                print('PASS: tool-free Chat, authorized Work tool, scoped policy, handoffs, background cancellation, per-model modes, capability honesty, durable restart, immutable retries, Council broadcast, credential redaction, usage')
            finally:
                if core is not None and core.poll() is None:
                    core.kill(); core.wait(timeout=10)
                logs.close()
    finally:
        chat.close()


if __name__ == '__main__':
    main()
