"""Claude subscription text fixture. No real accounts or inference."""
import json
import os
from pathlib import Path
import sys

args = sys.argv[1:]
assert os.environ.get('CLAUDE_CODE_SAFE_MODE') == '1'
assert not any(k.startswith('CLOUDROOM_') or k.endswith('API_KEY') for k in os.environ)
if '--version' in args:
    print('2.1.301 (Claude Code)')
    raise SystemExit
if 'auth' in args:
    signed_in = not (Path(os.environ['USERPROFILE']) / '.claude/no-login').exists()
    print(json.dumps({'loggedIn': signed_in, 'authMethod': 'claude.ai' if signed_in else 'none'}))
    raise SystemExit(0 if signed_in else 1)
assert args[args.index('--tools') + 1] == ''
assert args[args.index('--setting-sources') + 1] == ''
assert '--strict-mcp-config' in args and '--no-session-persistence' in args
assert json.loads(args[args.index('--settings') + 1])['disableAllHooks'] is True
prompt = json.loads(sys.stdin.read())
text = prompt['messages'][-1]['content']
print(json.dumps({'type': 'system', 'subtype': 'ui_invalidate'}), flush=True)
print(json.dumps({'type': 'system', 'subtype': 'init', 'tools': []}), flush=True)
if 'CLI_TOOLS' in text:
    print(json.dumps({'type': 'assistant', 'message': {'content': [{'type': 'tool_use'}]}}))
    raise SystemExit
print(json.dumps({'type': 'rate_limit_event', 'rate_limit_info': {}}))
print(json.dumps({'type': 'assistant', 'message': {'content': [{'type': 'text', 'text': 'Claude fixture reply'}]}}))
print(json.dumps({'type': 'result', 'subtype': 'success', 'is_error': False, 'result': 'Claude fixture reply', 'usage': {'output_tokens': 3}}))
