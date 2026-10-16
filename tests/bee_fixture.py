"""Text-only HTTP provider used by native and WPF tests; no inference or credentials."""
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class ChatFixture:
    def __init__(self):
        self.calls = []
        self.release = threading.Event()
        owner = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_args):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
                assert body['tool_choice'] == 'none' and not body.get('tools'), body
                assert all(message['role'] in ('user', 'assistant') for message in body['messages'])
                owner.calls.append(body)
                if body['model'] in ('deepseek-flash', 'deepseek-v4.1-flash-free'):
                    assert body['thinking'] == {'type': 'disabled'} and body['max_tokens'] == 8192
                    key = 'fixture-opencode-key' if body['model'].endswith('-free') else 'fixture-deepseek-key'
                    assert self.headers['Authorization'] == 'Bearer ' + key
                text = body['messages'][-1]['content']
                if text.startswith('DEEPSEEK_LIMIT') or text.startswith('DEEPSEEK_UNAVAILABLE'):
                    self.send_response(429 if text.startswith('DEEPSEEK_LIMIT') else 403)
                    self.send_header('Content-Length', '0')
                    self.end_headers()
                    return
                if 'CHAT_FIXTURE_WAIT' in text:
                    owner.release.wait(30)
                message = {'content': 'bee hello: ' + text, 'role': 'assistant'}
                if 'CHAT_FIXTURE_TOOL' in text:
                    message['tool_calls'] = [{'id': 'denied', 'type': 'function', 'function': {'name': 'exec_command', 'arguments': '{"command":"touch denied"}'}}]
                data = json.dumps({'choices': [{'message': message}], 'usage': {'total_tokens': 7}}).encode()
                try:
                    self.send_response(200)
                    self.send_header('Content-Length', str(len(data)))
                    self.end_headers()
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.endpoint = f'http://127.0.0.1:{self.server.server_port}/v1/chat/completions'
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.release.set()
        self.server.shutdown()
        self.server.server_close()
