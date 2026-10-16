"""Deterministic cross-provider Council protocol. Never calls a real model."""
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class CouncilFixture:
    def __init__(self):
        self.calls = []
        self.release = threading.Event()
        self.hold_tasks = threading.Event()
        self.task_release = threading.Event()
        self.lock = threading.Lock()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                anthropic = self.path == '/v1/messages'
                if anthropic:
                    assert self.headers['anthropic-version'] == '2023-06-01'
                    assert self.headers['x-api-key'] == 'fixture-claude-key'
                    assert body['max_tokens'] == 8192 and not body.get('tools')
                else:
                    assert body['tool_choice'] == 'none' and not body.get('tools')
                    if body['model'] == 'gemini-2.5-pro':
                        assert self.headers['Authorization'] == 'Bearer fixture-gemini-key'
                        assert body['max_tokens'] == 8192 and 'thinking' not in body
                assert not body['stream']
                assert all(m['role'] in ('user', 'assistant') for m in body['messages'])
                with owner.lock:
                    owner.calls.append(body)
                first = body['messages'][0]['content']
                text = body['messages'][-1]['content']
                if first.startswith('COUNCIL_PLAN_V1'):
                    if 'BAD_PLAN' in first:
                        output = json.dumps({'tasks': [dict(title='Cycle', owner=0, depends_on=[0], instructions='Bad')]})
                    else:
                        count = 4 if '"harness":"claude"' in first else 2
                        jobs = [dict(title='Inspect the shared file', owner=0, depends_on=[], instructions='TASK_EDIT' if 'EDIT_FIXTURE' in first else 'TASK_INSPECT'),
                                dict(title='Review the inspection', owner=1 if count > 1 else 0, depends_on=[0], instructions='TASK_REVIEW')]
                        if count == 4:
                            jobs.extend([dict(title='Independent file review', owner=2, depends_on=[], instructions='TASK_INSPECT'),
                                         dict(title='Combine the reviews', owner=3, depends_on=[1, 2], instructions='TASK_REVIEW')])
                        if '"harness":"gemini"' in first:
                            jobs.append(dict(title='Gemini independent review', owner=4, depends_on=[], instructions='TASK_INSPECT'))
                        output = json.dumps({'tasks': jobs})
                elif first.startswith('COUNCIL_TASK_V1'):
                    if owner.hold_tasks.is_set():
                        owner.task_release.wait(30)
                    if 'ACCESS_ATTACK' in first:
                        value = dict(action='write_file', path='note.txt', expected_revision='0'*64, text='DENIED')
                    elif 'PATH_ATTACK' in first and len(body['messages']) == 1:
                        value = dict(action='read_file', path='../outside.txt')
                    elif 'TASK_REVIEW' in first:
                        assert 'Task result:' in first, 'Dependencies were not shared with the assigned model'
                        value = dict(action='done', result='Task result: reviewed dependency output')
                    elif '"discuss"' in first:
                        value = dict(action='done', result='Task result: discussion analysis')
                    elif len(body['messages']) == 1:
                        value = dict(action='read_file', path='note.txt')
                    elif 'TASK_EDIT' in first and len(body['messages']) == 3:
                        operation = json.loads(text.split(': ', 1)[1])
                        value = dict(action='write_file', path='note.txt', expected_revision=operation['revision'], text='Edited by Council\n')
                    else:
                        value = dict(action='done', result='Task result: inspected shared UTF-8 file')
                    output = json.dumps(value)
                elif first.startswith('COUNCIL_SUMMARY_V1'):
                    assert 'Task result:' in first
                    output = 'Council shared result: all assigned microtasks reported completion. No shell tests were run.'
                else:
                    if 'COUNCIL_HOLD' in text:
                        owner.release.wait(30)
                    output = 'Independent reply from ' + body['model'] + ': ' + text
                if anthropic:
                    data = json.dumps(dict(content=[dict(type='text', text=output)], usage=dict(input_tokens=5, output_tokens=9))).encode()
                else:
                    data = json.dumps(dict(choices=[dict(message=dict(role='assistant', content=output))], usage=dict(total_tokens=14))).encode()
                try:
                    self.send_response(200)
                    self.send_header('Content-Type', 'application/json')
                    self.send_header('Content-Length', str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    pass

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}/v1/chat/completions'
        self.anthropic = f'http://127.0.0.1:{self.server.server_port}/v1/messages'
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.release.set()
        self.task_release.set()
        self.server.shutdown()
        self.server.server_close()


def configure_council(api, provider):
    members = []
    for harness, model in [('codex', 'gpt-fixture'), ('grok', 'grok-fixture'), ('claude', 'claude-fixture'), ('qwen', 'qwen-fixture'), ('gemini', 'gemini-2.5-pro')]:
        api('POST', '/v1/bee/profiles', dict(harness=harness, model=model, provider='anthropic' if harness == 'claude' else 'gemini' if harness == 'gemini' else 'local', endpoint=provider.anthropic if harness == 'claude' else provider.endpoint, api_key='fixture-claude-key' if harness == 'claude' else 'fixture-gemini-key' if harness == 'gemini' else ''))
        members.append(dict(harness=harness, model=model))
    return members


def exercise_council(api, repo, provider, until):
    members = configure_council(api, provider)
    (repo / 'note.txt').write_text('Original shared file — 蜂\n', encoding='utf-8')
    (repo.parent / 'outside.txt').write_text('OUTSIDE_COUNCIL_CANARY')
    catalog = api('GET', '/v1/bee/councils/catalog')
    assert catalog['access'] == ['discuss', 'read', 'edit'] and catalog['max_members'] == 5
    assert all(any(m['model'] == member['model'] for m in catalog['models']) for member in members)
    api('GET', '/v1/bee/councils', expected=401, auth=False)

    def create(key, access='discuss', expected=200, **changes):
        return api('POST', '/v1/bee/councils', dict(request_id=key, title='Shared Council', members=members, workspace=str(repo), access=access, authorize_access=access != 'discuss') | changes, expected)

    def submit(identity, key, text, coordinate=True, expected=200):
        return api('POST', '/v1/bee/councils/' + identity + '/messages', dict(request_id=key, text=text, coordinate=coordinate), expected)

    def state(identity):
        return api('GET', '/v1/bee/councils/' + identity)['council']

    def finished(identity):
        return until(lambda: (row if row['state'] in ('completed', 'failed', 'cancelled') else None) if (row := state(identity)['rounds'][-1]) else None, 'Council finished')

    create('denied', 'read', expected=409, authorize_access=False)
    create('missing_model', expected=409, members=[dict(harness='claude', model='never-configured')])
    create('duplicate', expected=409, members=[members[0], members[0]])
    api('POST', '/v1/bee/profiles', dict(harness='gemini', model='gemini-2.5-flash', provider='gemini', endpoint=provider.endpoint, api_key='fixture-gemini-key'))
    create('too_many', expected=409, members=members + [dict(harness='gemini', model='gemini-2.5-flash')])
    first = create('discussion')['council_id']
    assert create('discussion')['council_id'] == first
    create('discussion', 'read', expected=409)
    submit(first, 'broadcast', 'Same message to every provider', coordinate=False)
    round = finished(first)
    assert round['state'] == 'completed' and len(round['replies']) == 5 and not round['tasks']
    assert all('Same message to every provider' in r['text'] for r in round['replies'])
    calls = len(provider.calls)
    submit(first, 'broadcast', 'Same message to every provider', coordinate=False)
    assert len(provider.calls) == calls
    submit(first, 'broadcast', 'Different payload', expected=409)
    submit(first, 'plan', 'Divide this discussion into microtasks')
    round = finished(first)
    assert round['state'] == 'completed' and len(round['tasks']) == 5
    assert {t['owner'] for t in round['tasks']} == {0, 1, 2, 3, 4}
    assert all(t['state'] == 'completed' and t['result'] for t in round['tasks'])
    assert 'shared result' in round['summary']
    for task in round['tasks']:
        for dependency in task['depends_on']:
            assert round['tasks'][dependency]['finished_at'] <= task['started_at']
    submit(first, 'bad_plan', 'BAD_PLAN')
    assert finished(first)['state'] == 'failed' and not state(first)['rounds'][-1]['tasks']
    count = len(provider.calls)
    api('POST', '/v1/bee/councils/' + first + '/messages', dict(request_id='membership', text='Only GPT and Qwen this time', coordinate=False, members=[members[0], members[3]]))
    round = finished(first)
    assert round['state'] == 'completed' and [m['model'] for m in round['members']] == ['gpt-fixture', 'qwen-fixture']
    assert {c['model'] for c in provider.calls[count:]} == {'gpt-fixture', 'qwen-fixture'}
    assert len(state(first)['rounds'][0]['members']) == 5, 'Membership changes rewrote historical model identity'
    api('POST', '/v1/bee/councils/' + first + '/messages', dict(request_id='membership', text='Only GPT and Qwen this time', coordinate=False, members=members), expected=409)
    page = api('GET', '/v1/bee/councils/' + first)
    assert page['rounds_total'] == 4 and page['rounds_offset'] == 1 and len(page['council']['rounds']) == 3
    older = api('GET', '/v1/bee/councils/' + first + '?before=1')
    assert older['rounds_offset'] == 0 and older['council']['rounds'][0]['request_id'] == 'broadcast'
    assert older['latest']['request_id'] == 'membership', 'Loading older history hid active/latest progress'
    read = create('read_shared', 'read')['council_id']
    submit(read, 'read', 'Read the shared folder')
    round = finished(read)
    assert round['state'] == 'completed' and any(t['operations'] for t in round['tasks'])
    assert (repo / 'note.txt').read_text(encoding='utf-8').startswith('Original')
    submit(read, 'access_attack', 'ACCESS_ATTACK')
    assert finished(read)['state'] == 'failed'
    assert (repo / 'note.txt').read_text(encoding='utf-8').startswith('Original')
    submit(read, 'outside', 'PATH_ATTACK')
    round = finished(read)
    assert any(o['state'] == 'rejected_or_uncertain' for t in round['tasks'] for o in t['operations'])
    assert 'OUTSIDE_COUNCIL_CANARY' not in json.dumps(provider.calls)
    edit = create('edit_shared', 'edit')['council_id']
    submit(edit, 'edit', 'EDIT_FIXTURE')
    round = finished(edit)
    assert round['state'] == 'failed', 'Chat-only Council participant obtained write privileges'
    assert (repo / 'note.txt').read_text(encoding='utf-8').startswith('Original')
    assert not api('GET', '/v1/bee/files/' + edit + '/history?path=note.txt')['saves']
    calls = len(provider.calls)
    submit(edit, 'edit', 'EDIT_FIXTURE')
    assert len(provider.calls) == calls
    provider.hold_tasks.set()
    held = create('cancel_shared', 'read')['council_id']
    submit(held, 'held', 'Show real task progress')
    until(lambda: any(t['state'] == 'running' for t in state(held)['rounds'][-1]['tasks']), 'visible running microtasks')
    assert submit(held, 'held', 'Show real task progress')['state'] == 'executing'
    submit(held, 'busy', 'A second message', expected=409)
    api('POST', '/v1/bee/councils/' + held + '/cancel', {})
    assert finished(held)['state'] == 'cancelled'
    provider.task_release.set()
    provider.hold_tasks.clear()
    # Real Anthropic protocol also works in ordinary tool-free Chat.
    chat = api('POST', '/v1/bee/sessions', dict(request_id='claude_chat_fixture', harness='claude', model='claude-fixture'))['session_id']
    api('POST', '/v1/bee/sessions/' + chat + '/prompts', dict(request_id='claude_text', text='Claude text only'))
    until(lambda: api('GET', '/v1/bee/sessions/' + chat)['session']['receipts']['claude_text']['state'] == 'completed', 'Claude text transport')
    gemini_lead = create('gemini_coordinator', members=[members[4], *members[:4]])['council_id']
    submit(gemini_lead, 'gemini_lead_plan', 'Coordinate this discussion with Gemini')
    round = finished(gemini_lead)
    assert round['state'] == 'completed' and len(round['tasks']) == 5 and 'shared result' in round['summary']
    return first, create('restart_shared', 'read')['council_id']
