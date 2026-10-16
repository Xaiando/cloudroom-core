"""Library acceptance contracts; invoked only inside the disposable extension fixture."""
from windows_native import until


def exercise_library(api, plugin, provider, conversation):
    base = '/v1/bee/library'
    api('GET', base, expected=401, auth=False)

    def content(kind, **changes):
        return dict(kind=kind, title='Windows — 蜂', body='# Notes\nUnicode ø', workspace=None,
                    tags=['windows'], status='todo', memory_kind='fact', use_in_context=False) | changes

    def create(key, row, expected=200, authorize=False):
        return api('POST', base, dict(request_id=key, content=row, authorize_context=authorize), expected)

    def update(key, identity, revision, row, expected=200, authorize=False):
        return api('POST', base + '/' + identity + '/update', dict(request_id=key, expected_revision=revision,
                   content=row, authorize_context=authorize), expected)

    create('not_installed', content('docs'), expected=409)
    for name in ('simple-notes', 'memory', 'tasks'):
        plugin(name, 'install')
    create('bad_scope', content('memory', workspace='never-created'), expected=409)
    assert not any(w['id'] == 'never-created' for w in api('GET', base)['workspaces'])
    create('bad_tag', content('docs', tags=['bad\nline']), expected=409)
    create('too_big', content('docs', body='x' * 49153), expected=409)
    create('bad_kind', content('no-plugin'), expected=409)
    create('doc_context', content('docs', use_in_context=True), expected=409, authorize=True)
    doc = create('doc', content('docs'))['id']
    assert create('doc', content('docs'))['id'] == doc
    create('doc', content('docs', body='changed'), expected=409)
    first = update('doc_edit', doc, 1, content('docs', body='second revision'))
    assert first['revision'] == 2
    assert update('doc_edit', doc, 1, content('docs', body='second revision')) == first
    update('doc_stale', doc, 1, content('docs'), expected=409)
    update('kind_changed', doc, 2, content('tasks'), expected=409)
    row = api('GET', base + '/' + doc)['entry']
    assert row['history'][0]['content']['body'] == '# Notes\nUnicode ø'
    for number in range(7):
        update('revision_' + str(number), doc, 2 + number, content('docs', body=str(number)))
    assert len(api('GET', base + '/' + doc)['entry']['history']) == 5
    task = create('task', content('tasks', status='in_progress'))['id']
    update('task_done', task, 1, content('tasks', status='done'))
    assert api('GET', base + '/' + task)['entry']['content']['status'] == 'done'
    plugin('tasks', 'disable')
    update('task_disabled', task, 2, content('tasks'), expected=409)
    assert api('GET', base + '/' + task)['entry']['content']['status'] == 'done'
    plugin('tasks', 'enable')

    create('memory_auth', content('memory', use_in_context=True), expected=409)
    memory = create('memory', content('memory', body='SAVED_MEMORY_FIRST', use_in_context=True), authorize=True)['id']
    create('default_memory', content('memory', body='DEFAULT_MEMORY', workspace='legacy', use_in_context=True), authorize=True)
    other = api('POST', '/v1/bee/sessions', dict(request_id='other_workspace', harness='codex', model='fixture',
                workspace='memory_other', workspace_name='Other-project'))['session_id']
    create('project_memory', content('memory', body='OTHER_PROJECT_MEMORY', workspace='memory_other', use_in_context=True), authorize=True)
    path = '/v1/bee/sessions/' + conversation
    calls = len(provider.calls)
    provider.release.clear()
    prompt = dict(request_id='memory_snapshot', text='CHAT_FIXTURE_WAIT')
    api('POST', path + '/prompts', prompt)
    until(lambda: len(provider.calls) == calls + 1, 'memory context snapshotted')
    api('GET', '/v1/bee/threads', expected=401, auth=False)
    organization = dict(request_id='organize', expected_revision=0, title='Windows — 蜂', collection='Native work', pinned=True, archived=True)
    api('POST', '/v1/bee/threads/unknown/organize', organization, expected=404)
    api('POST', '/v1/bee/threads/' + conversation + '/organize', organization)
    api('POST', '/v1/bee/threads/' + conversation + '/organize', organization)
    assert api('GET', path)['session']['state'] == 'running', 'Archiving interrupted running Chat'
    api('POST', '/v1/bee/threads/' + conversation + '/organize', {**organization, 'request_id': 'stale_org'}, expected=409)
    api('POST', '/v1/bee/threads/' + conversation + '/organize', {**organization, 'request_id': 'restore_org', 'expected_revision': 1, 'archived': False})
    assert api('GET', '/v1/bee/threads')['threads'][conversation]['title'] == 'Windows — 蜂'
    sent = provider.calls[-1]['messages'][-1]['content']
    assert 'SAVED_MEMORY_FIRST' in sent and 'DEFAULT_MEMORY' in sent and 'OTHER_PROJECT_MEMORY' not in sent
    update('memory_edit', memory, 1, content('memory', body='SAVED_MEMORY_SECOND', use_in_context=True), authorize=True)
    provider.release.set()
    until(lambda: api('GET', path)['session']['receipts']['memory_snapshot']['state'] == 'completed', 'memory Chat finished')
    api('POST', path + '/prompts', prompt)
    assert len(provider.calls) == calls + 1 and 'SAVED_MEMORY_FIRST' in provider.calls[-1]['messages'][-1]['content']
    other_path = '/v1/bee/sessions/' + other
    api('POST', other_path + '/prompts', dict(request_id='other_memory', text='Another project'))
    until(lambda: api('GET', other_path)['session']['receipts']['other_memory']['state'] == 'completed', 'project Chat finished')
    sent = provider.calls[-1]['messages'][-1]['content']
    assert 'SAVED_MEMORY_SECOND' in sent and 'OTHER_PROJECT_MEMORY' in sent and 'DEFAULT_MEMORY' not in sent
    plugin('memory', 'disable')
    api('POST', path + '/prompts', dict(request_id='memory_disabled', text='No saved memory'))
    until(lambda: api('GET', path)['session']['receipts']['memory_disabled']['state'] == 'completed', 'disabled memory Chat finished')
    assert 'SAVED_MEMORY_SECOND' not in provider.calls[-1]['messages'][-1]['content']
    plugin('memory', 'enable')
    for action, revision in [('archive', 9), ('restore', 10)]:
        api('POST', base + '/' + doc + '/' + action, dict(request_id='doc_' + action, expected_revision=revision))
        assert api('GET', base + '/' + doc)['entry']['archived'] == (action == 'archive')
    plugin('simple-notes', 'uninstall')
    assert api('GET', base + '/' + doc)['entry']['history'], 'uninstall erased library data'
    plugin('simple-notes', 'install')
    return doc, task, memory
