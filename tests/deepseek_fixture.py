"""Gemini Chat contract and endpoint routing, without real inference."""
import json


def exercise_gemini(api, provider, until):
    endpoint = 'https://generativelanguage.googleapis.com/v1beta/openai/chat/completions'
    key = 'fixture-gemini-key'
    for model in ('gemini-2.5-pro', 'gemini-2.5-flash'):
        profile = dict(provider='gemini', harness='gemini', model=model, endpoint=endpoint, api_key=key)
        api('POST', '/v1/bee/profiles', profile)
        for changed in [dict(api_key=''), dict(endpoint=endpoint.replace('https:', 'http:')),
                        dict(endpoint=endpoint.replace('.com/', '.com.evil.invalid/')),
                        dict(endpoint=endpoint + '?key=leak'), dict(harness='codex')]:
            api('POST', '/v1/bee/profiles', profile | changed, expected=409)
        # Only explicit unprotected fixture mode permits the numeric loopback endpoint.
        api('POST', '/v1/bee/profiles', profile | dict(endpoint=provider.endpoint))
        sid = api('POST', '/v1/bee/sessions', dict(request_id='gemini_' + model.replace('.', '_').replace('-', '_'), harness='gemini', model=model))['session_id']

        def state():
            return api('GET', '/v1/bee/sessions/' + sid)['session']

        def send(request, text):
            return api('POST', '/v1/bee/sessions/' + sid + '/prompts', dict(request_id=request, text=text))

        def settled(request):
            return until(lambda: (row if row['state'] in ('completed', 'failed') else None)
                         if (row := state()['receipts'].get(request)) else None, 'Gemini response')

        send('hello', 'Gemini text response')
        assert settled('hello')['state'] == 'completed'
        calls = len(provider.calls)
        send('hello', 'Gemini text response')
        assert len(provider.calls) == calls, 'Gemini retry duplicated inference'
        for request, text in [('tool', 'CHAT_FIXTURE_TOOL'), ('limited', 'GEMINI_LIMIT'), ('denied', 'GEMINI_UNAVAILABLE')]:
            calls = len(provider.calls)
            send(request, text)
            receipt = settled(request)
            assert receipt['state'] == 'failed' and len(provider.calls) == calls + 1, 'Provider failure caused fallback or retry'
            if request == 'limited':
                assert 'rate limit' in json.dumps(receipt)
        handoff = {field: 'Review only' for field in ('objective', 'context', 'requirements', 'constraints', 'acceptance', 'artifacts')}
        api('POST', '/v1/bee/sessions/' + sid + '/mode', dict(request_id='work', mode='work', authorize_work=True, handoff=handoff), expected=409)
        assert state()['execution_session_id'] is None
        assert key not in json.dumps(state())
    capabilities = api('GET', '/v1/bee/capabilities')
    harness = next(h for h in capabilities['harnesses'] if h['id'] == 'gemini')
    assert harness['modes'] == 'CHAT_ONLY' and not harness['work']
    assert 'fixture-gemini-key' not in json.dumps(capabilities)
    print('PASS: Gemini profiles, exact endpoint/key routing, bounded tool-free transport, immutable retry, limits without fallback, denied native Work')
