"""Disposable native CLI, including a descendant to exercise cancellation ownership."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import time
from urllib.request import Request, urlopen

args = sys.argv[1:]
if 'exec' not in args and '--prompt-file' not in args:
    runpy.run_path(str(Path(__file__).with_name('core_fixture.py')), run_name='__main__')
    raise SystemExit
assert not any(k.startswith('CLOUDROOM_') or k in ('OPENAI_API_KEY', 'XAI_API_KEY') for k in os.environ)
assert Path.cwd().name == 'workspace' and not list(Path.cwd().iterdir())

def option(name):
    return args[args.index(name) + 1]

model = option('--model')
if 'exec' in args:
    assert all(x in args for x in ('--ignore-user-config', '--ephemeral', '--skip-git-repo-check', '--sandbox', 'read-only', '--json'))
    settings = [args[i + 1] for i, a in enumerate(args) if a == '-c']
    assert 'features.shell_tool=false' in settings and 'features.hooks=false' in settings
    catalog = json.loads(Path(json.loads(next(s.split('=', 1)[1] for s in settings if s.startswith('model_catalog_json=')))).read_text())
    assert catalog['models'][0]['shell_type'] == 'disabled' and catalog['models'][0]['apply_patch_tool_type'] is None
    base = next((json.loads(s.split('=', 1)[1]) for s in settings if s.startswith('model_providers.cloud_bee_policy.base_url=')), None)
    if base:
        assert not list(Path(os.environ['CODEX_HOME']).iterdir())
        # The installed Codex CLI omits tools entirely when its registry is empty.
        payload = {'model': model}
        if model == 'unsafe': payload['tools'] = [{'type': 'function'}]
        request = Request(base + '/responses', data=json.dumps(payload).encode(), headers={'Content-Type': 'application/json'})
        try:
            with urlopen(request) as response:
                assert b'CLOUD_BEE_POLICY_OK' in response.read()
        except Exception:
            raise SystemExit(1)
        answer = 'CLOUD_BEE_POLICY_OK'
    else:
        data = json.loads(sys.stdin.read())
        answer = None
else:
    assert option('--tools') == '' and option('--permission-mode') == 'dontAsk'
    assert all(x in args for x in ('--no-subagents', '--disable-web-search', '--verbatim'))
    data = json.loads(Path(option('--prompt-file')).read_text())
    answer = None
if answer is None:
    first = data['messages'][0]['content']
    text = data['messages'][-1]['content']
    if 'CLI_HOLD' in text:
        child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])
        (Path(os.environ['USERPROFILE']) / 'descendant.pid').write_text(str(child.pid))
        time.sleep(120)
    if 'CLI_TOOLS' in text:
        print(json.dumps({'type': 'item.completed', 'item': {'type': 'command_execution', 'command': 'denied'}}), flush=True)
        raise SystemExit(0)
    if first.startswith('COUNCIL_PLAN_V1'):
        answer = json.dumps({'tasks': [dict(title='Inspect', owner=0, depends_on=[], instructions='Inspect'), dict(title='Review', owner=1, depends_on=[0], instructions='Review')]})
    elif first.startswith('COUNCIL_TASK_V1'):
        answer = json.dumps(dict(action='done', result='Task result: CLI analysis'))
    elif first.startswith('COUNCIL_SUMMARY_V1'):
        answer = 'Council CLI summary'
    else:
        answer = 'Independent CLI reply: ' + text
if 'exec' in args:
    print(json.dumps({'type': 'item.completed', 'item': {'type': 'agent_message', 'text': answer}}))
    print(json.dumps({'type': 'turn.completed'}))
else:
    print(json.dumps({'text': answer, 'stopReason': 'end_turn', 'usage': {'total_tokens': 2}}))
