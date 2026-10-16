"""DeepSeek Chat contract and promotional model routing, without real inference."""
import json


def exercise_deepseek(api, provider, until):
    for route, model, endpoint, key in [
        ('opencode', 'deepseek-v4.1-flash-free', 'https://opencode.ai/inference/openai/v1/chat/completions', 'fixture-opencode-key'),
        ('deepseek', 'deepseek-flash', 'https://api.deepseek.com/chat/completions', 'fixture-deepseek-key'),
    ]:
        profile = dict(provider=route, harness='deepseek', model=model, endpoint=endpoint, api_key=key)
        api('POST', '/v1/bee/profiles', profile)
        for changed in [dict(api_key=''), dict(endpoint=endpoint.replace('https:', 'http:')),
                        dict(endpoint=endpoint.replace('.com/', '.com.evil.invalid/').replace('.ai/', '.ai.evil.invalid/')),
                        dict(endpoint=endpoint + '?key=leak'), dict(harness='codex')]:
            api('POST', '/v1/bee/profiles', profile | changed, expected=409)
        if route == 'opencode':
            api('POST', '/v1/bee/profiles', profile | dict(model='deepseek-v4.1-flash'), expected=409)
        # Only explicit unprotected fixture mode permits the numeric loopback endpoint.
        api('POST', '/v1/bee/profiles', profile | dict(endpoint=provider.endpoint))
        sid = api('POST', '/v1/bee/sessions', dict(request_id='deepseek_' + route, harness='deepseek', model=model))['session_id']

        def state():
            return api('GET', '/v1/bee/sessions/' + sid)['session']

        def send(request, text):
            return api('POST', '/v1/bee/sessions/' + sid + '/prompts', dict(request_id=request, text=text))

        def settled(request):
            return until(lambda: (row if row['state'] in ('completed', 'failed') else None)
                         if (row := state()['receipts'].get(request)) else None, 'DeepSeek response')

        send('hello', 'DeepSeek text response')
        assert settled('hello')['state'] == 'completed'
        calls = len(provider.calls)
        send('hello', 'DeepSeek text response')
        assert len(provider.calls) == calls, 'DeepSeek retry duplicated inference'
        for request, text in [('tool', 'CHAT_FIXTURE_TOOL'), ('limited', 'DEEPSEEK_LIMIT'), ('denied', 'DEEPSEEK_UNAVAILABLE')]:
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
    harness = next(h for h in capabilities['harnesses'] if h['id'] == 'deepseek')
    assert harness['modes'] == 'CHAT_ONLY' and not harness['work']
    assert 'fixture-opencode-key' not in json.dumps(capabilities)
    print('PASS: DeepSeek direct/free profiles, exact endpoint/key routing, bounded tool-free transport, immutable retry, limits without paid fallback, denied native Work')
