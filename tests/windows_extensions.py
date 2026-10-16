"""Native Automations and Plugins contracts, using disposable scripts and providers only."""
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
from windows_native import until, ProcessHandle
from library_fixture import exercise_library
from files_fixture import exercise_files, prepare_file_recovery, verify_file_recovery
from council_fixture import CouncilFixture, exercise_council
from traffic_fixture import exercise_traffic, verify_traffic_restart

ROOT = Path(__file__).resolve().parents[1]


def main():
    assert sys.platform == 'win32'
    binary = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'target/debug/cloudroom.exe'
    provider = ChatFixture()
    council_provider = CouncilFixture()
    try:
        with tempfile.TemporaryDirectory(prefix='cloud bee extensions ') as folder:
            root = Path(folder)
            home, repo = root / 'home', root / 'workspace'
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
                       CLOUDROOM_LOCAL_HISTORY='1', CLOUDROOM_UNPROTECTED_TEST_MODE='1', CLOUDROOM_STORE='extensions-test',
                       CLOUDROOM_STATE_DIR=str(root / 'state'), CLOUDROOM_ACCOUNT_HOME=str(home),
                       CLOUDROOM_REPOSITORY=str(repo), CLOUDROOM_CODEX_BINARY=str(wrapper),
                       CLOUDROOM_CODEX_HOME=str(home / '.codex'), CLOUDROOM_CODEX_MODEL='fixture')
            log = (root / 'core.log').open('wb')
            core = None

            def api(method, path, body=None, expected=200, auth=True):
                headers = {'Content-Type': 'application/json'}
                if auth:
                    headers['Authorization'] = 'Bearer ' + token
                request = Request(f'http://127.0.0.1:{port}{path}', method=method,
                                  data=json.dumps(body).encode() if body is not None else None, headers=headers)
                try:
                    response = urlopen(request, timeout=25)
                except HTTPError as error:
                    response = error
                with response:
                    raw = response.read()
                    assert response.code == expected, (path, response.code, raw)
                    if expected == 422:
                        return {'error': raw.decode('utf-8')}
                    return json.loads(raw)

            def start():
                process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log, creationflags=subprocess.CREATE_NO_WINDOW)
                def ready():
                    assert process.poll() is None, (root / 'core.log').read_text(errors='replace')
                    try:
                        return api('GET', '/v1/ready')['ready']
                    except OSError:
                        return False
                until(ready, 'Core ready')
                return process

            serial = 0
            def plugin(name, action, settings=None, expected=200):
                nonlocal serial
                serial += 1
                body = {'request_id': f'plugin_{serial}'}
                if settings is not None:
                    body['settings'] = settings
                return api('POST', f'/v1/bee/plugins/{name}/{action}', body, expected)

            def definition(source, interpreter='python', schedule=None, enabled=False, timeout=10000):
                return dict(name='Fixture automation', enabled=enabled, schedule=schedule or dict(type='interval', every_ms=60000),
                            execution=dict(type='script', interpreter=interpreter, source=source, timeout_ms=timeout), workspace=None, workspace_name=None)

            def create(key, spec, expected=200, authorize=True):
                return api('POST', '/v1/bee/automations', dict(request_id=key, automation=spec, authorize_execution=authorize), expected)

            def detail(aid):
                return api('GET', '/v1/bee/automations/' + aid)

            def action(aid, name, key, expected=200, **extra):
                return api('POST', f'/v1/bee/automations/{aid}/{name}', dict(request_id=key, **extra), expected)

            def finish(aid, run, timeout=30):
                return until(lambda: next((r for r in detail(aid)['runs'] if r['id'] == run and r['state'] != 'running'), None), 'automation finished', timeout=timeout)

            try:
                core = start()
                api('GET', '/v1/bee/automations', expected=401, auth=False)
                assert len(api('GET', '/v1/bee/plugins')['plugins']) == 6
                plugin('unknown', 'install', expected=409)
                plugin('keep-awake', 'install')
                until(lambda: next(p for p in api('GET', '/v1/bee/plugins')['plugins'] if p['id'] == 'keep-awake')['runtime_status'] == 'active', 'Windows power request active')
                plugin('keep-awake', 'disable')
                until(lambda: next(p for p in api('GET', '/v1/bee/plugins')['plugins'] if p['id'] == 'keep-awake')['runtime_status'] == 'disabled', 'power request released')
                plugin('keep-awake', 'uninstall')
                plugin('custom-instructions', 'settings', {'instructions': 'PLUGIN_FIRST'})
                plugin('custom-instructions', 'settings', {'instructions': 'x' * 4097}, expected=409)
                api('POST', '/v1/bee/profiles', dict(harness='codex', model='fixture', provider='local', endpoint=provider.endpoint, api_key=''))
                conv = api('POST', '/v1/bee/sessions', dict(request_id='extension_chat', harness='codex', model='fixture'))['session_id']
                path = '/v1/bee/sessions/' + conv
                first = dict(request_id='first', text='CHAT_FIXTURE_WAIT')
                api('POST', path + '/prompts', first)
                until(lambda: len(provider.calls) == 1, 'Chat started')
                plugin('custom-instructions', 'settings', {'instructions': 'PLUGIN_SECOND'})
                provider.release.set()
                until(lambda: api('GET', path)['session']['receipts']['first']['state'] == 'completed', 'Chat finished')
                api('POST', path + '/prompts', first)
                assert len(provider.calls) == 1 and 'PLUGIN_FIRST' in provider.calls[0]['messages'][-1]['content']
                api('POST', path + '/prompts', dict(request_id='second', text='Another message'))
                until(lambda: len(provider.calls) == 2, 'second Chat started')
                assert 'PLUGIN_SECOND' in provider.calls[-1]['messages'][-1]['content']
                until(lambda: api('GET', path)['session']['receipts']['second']['state'] == 'completed', 'second Chat finished')
                plugin('custom-instructions', 'disable')
                api('POST', path + '/prompts', dict(request_id='third', text='No extra instructions'))
                until(lambda: len(provider.calls) == 3, 'third Chat started')
                assert 'PLUGIN_SECOND' not in provider.calls[-1]['messages'][-1]['content']
                until(lambda: api('GET', path)['session']['receipts']['third']['state'] == 'completed', 'third Chat finished')
                library_ids = exercise_library(api, plugin, provider, conv)
                file_body, file_saved = exercise_files(api, conv, repo)
                saved_council, restart_council = exercise_council(api, repo, council_provider, until)
                hive_saved = exercise_traffic(api, root, until)

                spec = definition("import os; from pathlib import Path; print('automation hello — ø'); print('token_present', 'CLOUDROOM_TOKEN' in os.environ); Path('count.txt').write_text('once')")
                create('denied', spec, expected=409, authorize=False)
                aid = create('script', spec)['id']
                assert create('script', spec)['id'] == aid
                create('script', {**spec, 'name': 'Different'}, expected=409)
                run = action(aid, 'run', 'manual')['run_id']
                assert action(aid, 'run', 'manual')['run_id'] == run
                saved = finish(aid, run)
                assert saved['state'] == 'completed' and saved['exit_code'] == 0
                assert 'automation hello — ø' in saved['output'] and 'token_present False' in saved['output']
                assert len(detail(aid)['runs']) == 1

                for interpreter, source in [('powershell', "Write-Output 'native PowerShell — ø'; exit 7"), ('node', "console.log('native Node');")]:
                    # Hosted Windows can take over 10 seconds to cold-start PowerShell 5.1.
                    # The dedicated one-second timeout contract below remains unchanged.
                    other = create(interpreter, definition(source, interpreter, timeout=60000))['id']
                    row = finish(other, action(other, 'run', 'test')['run_id'], timeout=75)
                    assert row['exit_code'] == (7 if interpreter == 'powershell' else 0), (interpreter, row)
                    assert row['state'] == ('failed' if interpreter == 'powershell' else 'completed')
                    if interpreter == 'powershell':
                        assert 'native PowerShell — ø' in row['output']
                timed = create('timeout', definition('import time; time.sleep(20)', timeout=1000))['id']
                assert finish(timed, action(timed, 'run', 'timed')['run_id'])['state'] == 'timed_out'
                big = create('bounded', definition("print('x'*200000)"))['id']
                bounded = finish(big, action(big, 'run', 'big')['run_id'])
                assert len(bounded['output']) < 66000 and 'truncated' in bounded['output']

                child = "from pathlib import Path; import time\nfor i in range(300):\n Path('ticks').write_text(str(i)); time.sleep(.1)\n"
                source = f"import subprocess,sys,time\nfrom pathlib import Path\np=subprocess.Popen([sys.executable,'-c',{child!r}])\nPath('child.pid').write_text(str(p.pid))\ntime.sleep(30)\n"
                held = create('cancel', definition(source, timeout=60000))['id']
                rid = action(held, 'run', 'held')['run_id']
                until(lambda: (repo / 'child.pid').exists(), 'owned descendant started')
                handle = ProcessHandle(int((repo / 'child.pid').read_text()))
                action(held, 'run', 'overlap', expected=409)
                plugin('automations', 'uninstall', expected=409)
                action(held, 'cancel', 'stop', run_id=rid)
                assert finish(held, rid)['state'] == 'cancelled'
                handle.wait()

                once = create('once', definition("print('scheduled')", schedule=dict(type='once', at_ms=int(time.time()*1000)+2000), enabled=True))['id']
                until(lambda: any(r['state'] == 'completed' for r in detail(once)['runs']), 'one-shot schedule')
                assert detail(once)['automation']['definition']['enabled'] is False
                recurring = create('interval', definition("print('recurring')", schedule=dict(type='interval', every_ms=1000), enabled=True))['id']
                until(lambda: len(detail(recurring)['runs']) >= 2, 'recurring schedule')
                revision = detail(recurring)['automation']['revision']
                action(recurring, 'pause', 'pause', expected_revision=revision)
                count = len(detail(recurring)['runs']); time.sleep(1.5)
                assert len(detail(recurring)['runs']) == count
                action(recurring, 'enable', 'stale', expected=409, expected_revision=revision, authorize_execution=True)
                plugin('automations', 'disable')
                action(aid, 'run', 'disabled', expected=409)
                plugin('automations', 'enable')

                agent_spec = {**definition(''), 'execution': dict(type='agent', conversation_id=conv, prompt='Automated Work prompt')}
                create('chat_agent_denied', agent_spec, expected=409)
                handoff = dict(objective='Fixture work', context='', requirements='', constraints='', acceptance='', artifacts='')
                api('POST', path + '/mode', dict(request_id='work', mode='work', authorize_work=True, handoff=handoff))
                until(lambda: api('GET', path)['session']['state'] == 'idle', 'Work ready')
                agent_id = create('agent', agent_spec)['id']
                api('POST', path + '/mode', dict(request_id='chat', mode='chat', handoff=handoff))
                calls = len(provider.calls)
                agent_run = finish(agent_id, action(agent_id, 'run', 'agent_run')['run_id'])
                assert agent_run['state'] == 'completed' and agent_run['native_session']
                assert 'SAVED_MEMORY_SECOND' in agent_run['instructions'] and 'OTHER_PROJECT_MEMORY' not in agent_run['instructions']
                assert len(provider.calls) == calls, 'Agent automation silently dispatched to Chat'

                # Updating a definition retains history and requires the current revision.
                current = detail(aid)['automation']
                updated = definition("print('updated')")
                action(aid, 'update', 'edit', automation=updated, expected_revision=current['revision'], authorize_execution=True)
                assert len(detail(aid)['runs']) == 1
                assert 'updated' in finish(aid, action(aid, 'run', 'updated')['run_id'])['output']
                plugin('custom-instructions', 'settings', {'instructions': 'SURVIVES_RESTART'})
                plugin('custom-instructions', 'enable')
                (repo / 'child.pid').unlink()
                unknown = action(held, 'run', 'crash')['run_id']
                until(lambda: (repo / 'child.pid').exists(), 'crash descendant started')
                descendant = ProcessHandle(int((repo / 'child.pid').read_text()))
                prepare_file_recovery(root / 'state', repo)
                api('POST', '/v1/bee/councils/' + restart_council + '/messages', dict(request_id='restart_hold', text='COUNCIL_HOLD', coordinate=True))
                until(lambda: any(call['messages'][-1]['content'] == 'COUNCIL_HOLD' for call in council_provider.calls), 'Council provider request active at restart')
                council_calls = len(council_provider.calls)
                core.kill(); core.wait(timeout=10); descendant.wait()
                core = start()
                verify_traffic_restart(api, hive_saved, until)
                restored = api('GET', '/v1/bee/councils/' + restart_council)['council']['rounds'][-1]
                assert restored['state'] == 'unknown_after_restart'
                assert api('POST', '/v1/bee/councils/' + restart_council + '/messages', dict(request_id='restart_hold', text='COUNCIL_HOLD', coordinate=True))['state'] == 'unknown_after_restart'
                assert len(council_provider.calls) == council_calls, 'Council restart replayed a provider request'
                assert api('GET', '/v1/bee/councils/' + saved_council)['rounds_total'] == 4
                council_provider.release.set()
                verify_file_recovery(api, conv, repo, file_body, file_saved)
                for identity in library_ids:
                    assert api('GET', '/v1/bee/library/' + identity)['entry']['id'] == identity
                assert api('GET', '/v1/bee/threads')['threads'][conv]['title'] == 'Windows — 蜂'
                assert next(r for r in detail(held)['runs'] if r['id'] == unknown)['state'] == 'unknown_after_restart'
                assert action(held, 'run', 'crash')['run_id'] == unknown
                assert len(detail(held)['runs']) == 2, 'Restart/retry replayed an uncertain run'
                instructions = next(p for p in api('GET', '/v1/bee/plugins')['plugins'] if p['id'] == 'custom-instructions')
                assert instructions['settings']['instructions'] == 'SURVIVES_RESTART' and instructions['enabled']
                revision = detail(aid)['automation']['revision']
                action(aid, 'delete', 'delete', expected_revision=revision)
                assert not any(a['id'] == aid for a in api('GET', '/v1/bee/automations')['automations'])
                assert len(detail(aid)['runs']) == 2, 'Delete removed run history'
                assert token not in (root / 'core.log').read_text(errors='replace')
                print('PASS: native plugin lifecycle/settings/power requests; instruction and scoped memory snapshots; Docs/Memory/Tasks revisions and retention; workspace previews/editing, revision conflicts, BOM/Unicode/body limits, busy handles, hard links, backup retention and restart recovery without replay; thread organization without interruption; scripts/agent automation; schedules/history; timeout/output bounds; descendant cancellation; authorization; five-provider coordinated Council; HTC real parallel Jobs/worktrees, conflict queue, reviewed integration, stale baseline rejection, Chat write denial and restart recovery')
            finally:
                if core is not None and core.poll() is None:
                    core.kill(); core.wait(timeout=10)
                log.close()
    finally:
        provider.close()
        council_provider.close()


if __name__ == '__main__':
    main()
