"""Hive coordination with real disposable Git worktrees and Windows-owned native fixtures."""
import json
import subprocess
import time
from pathlib import Path
from windows_native import ProcessHandle


def git(repo, *args):
    return subprocess.check_output(['git', '-c', 'core.hooksPath=NUL', '-C', str(repo), *args], stderr=subprocess.STDOUT, creationflags=subprocess.CREATE_NO_WINDOW).decode('utf-8').strip()


def prepare_project(root, name='hive-project'):
    assert root.name.startswith(('cloud bee extensions ', 'cloudroom desktop test '))
    repo = root / name
    repo.mkdir()
    git(repo, 'init', '-q')
    git(repo, 'config', 'user.name', 'Disposable fixture')
    git(repo, 'config', 'user.email', 'fixture@example.invalid')
    git(repo, 'config', 'core.autocrlf', 'false')
    (repo / 'note.txt').write_bytes('Shared baseline — 蜂\n'.encode('utf-8'))
    (repo / 'other.txt').write_bytes(b'Independent baseline\n')
    git(repo, 'add', '.')
    git(repo, 'commit', '-qm', 'Disposable baseline')
    return repo


def exercise_traffic(api, root, until):
    repo = prepare_project(root)
    members = [dict(harness='codex', model='gpt-fixture'), dict(harness='qwen', model='qwen-fixture')]
    serial = 0

    def council(tag, access='edit'):
        nonlocal serial
        serial += 1
        cid = api('POST', '/v1/bee/councils', dict(request_id=f'htc_council_{serial}', title='Hive fixture', members=members, workspace=str(repo), access=access, authorize_access=access != 'discuss'))['council_id']
        api('POST', f'/v1/bee/councils/{cid}/messages', dict(request_id='plan', text=tag, coordinate=True))
        until(lambda: api('GET', f'/v1/bee/councils/{cid}')['latest']['state'] == 'completed', 'HTC research plan')
        return cid

    def start(cid, key, path='note.txt', expected=200, **extra):
        body = dict(request_id=key, council_id=cid, round_id='plan', microtask=0, harness='codex', model='fixture', path=path, authorize_work=True) | extra
        return api('POST', '/v1/bee/traffic/tasks', body, expected)

    def detail(identity):
        return api('GET', '/v1/bee/traffic/tasks/' + identity)

    def settled(identity, states=('verification', 'waiting', 'failed')):
        return until(lambda: d if (d := detail(identity))['task']['state'] in states else None, 'isolated Work settled', timeout=40)

    def act(identity, action, expected=200, key=None, **extra):
        nonlocal serial
        serial += 1
        d = detail(identity)
        body = dict(request_id=key or f'htc_action_{serial}', expected_revision=d['task']['revision'], authorize=True,
                    proposed_revision=(d['proposed'] or {}).get('revision', '')) | extra
        return api('POST', '/v1/bee/traffic/tasks/' + identity + '/' + action, body, expected)

    api('GET', '/v1/bee/traffic', expected=401, auth=False)
    assert any(m['model'] == 'fixture' for m in api('GET', '/v1/bee/traffic/catalog')['models'])
    read = council('HTC_FIXTURE_APPLY', 'read')
    start(read, 'chat_denied', expected=409)
    gate = root / 'release native workers'
    cid = council('HTC_FIXTURE_APPLY HTC_FIXTURE_GATE=' + str(gate))
    start(cid, 'provider_denied', expected=409, harness='claude')
    start(cid, 'model_denied', expected=409, model='gpt-fixture')
    start(cid, 'traversal_denied', path='../outside.txt', expected=409)
    start(cid, 'untracked_denied', path='missing.txt', expected=409)
    one = start(cid, 'same_one')['id']
    assert start(cid, 'same_one')['id'] == one
    start(cid, 'same_one', path='other.txt', expected=409)
    two = start(cid, 'same_two')['id']
    # Real private execution overlaps; two instances of the same model have separate Jobs/worktrees.
    try:
        until(lambda: all(detail(i)['task']['state'] == 'executing' for i in (one, two)), 'parallel native workers')
    except AssertionError:
        raise AssertionError([detail(i) for i in (one, two)])
    gate.touch()
    assert all(detail(i)['task']['traffic'] == 'red' for i in (one, two))
    da, db = settled(one), settled(two)
    assert {da['task']['state'], db['task']['state']} == {'verification', 'waiting'}, (da, db)
    first, second = (one, two) if da['task']['state'] == 'verification' else (two, one)
    first_data = detail(first)
    assert first_data['task']['quiescent'] and first_data['task']['traffic'] == 'blue'
    assert (repo / 'note.txt').read_text(encoding='utf-8').startswith('Shared baseline')
    assert Path(first_data['task']['worktree']).joinpath('note.txt').read_bytes() == b'Native isolated proposal\n'
    assert first_data['task']['worktree'] != detail(second)['task']['worktree']
    assert detail(second)['task']['queue_position'] == 1
    # Manual Core editing is also fenced while integration ownership is held.
    api('POST', f'/v1/bee/files/{cid}/save', dict(request_id='manual_htc_denied', path='note.txt', expected_revision=first_data['base']['revision'], text='BYPASS', authorize_write=True), 409)
    act(first, 'approve', expected=409, authorize=False)
    act(first, 'approve', expected=409, proposed_revision='0'*64)
    before = first_data['task']['revision']
    api('POST', f'/v1/bee/traffic/tasks/{first}/approve', dict(request_id='stale_action', expected_revision=before-1, authorize=True, proposed_revision=first_data['proposed']['revision']), 409)
    approval = dict(request_id='same_approval', expected_revision=before, authorize=True, proposed_revision=first_data['proposed']['revision'])
    accepted = api('POST', f'/v1/bee/traffic/tasks/{first}/approve', approval)
    assert api('POST', f'/v1/bee/traffic/tasks/{first}/approve', approval) == accepted
    assert detail(first)['task']['state'] == 'integrated'
    assert (repo / 'note.txt').read_bytes() == b'Native isolated proposal\n'
    settled(second, ('verification',))
    act(second, 'approve', expected=409)  # Its baseline is stale; it cannot overwrite the accepted file.
    assert (repo / 'note.txt').read_bytes() == b'Native isolated proposal\n'
    act(second, 'reject')
    assert detail(second)['task']['traffic'] == 'green'
    history = api('GET', f'/v1/bee/files/{cid}/history?path=note.txt')['saves']
    assert len(history) == 1 and history[0]['before'].startswith('Shared baseline')
    (repo / 'note.txt').write_text('Reset disposable baseline\n', encoding='utf-8')
    git(repo, 'add', '.')
    git(repo, 'commit', '-qm', 'Next disposable baseline')
    # Independent files can await review at the same time, without a conflicting queue.
    independent = council('HTC_FIXTURE_APPLY')
    a = start(independent, 'independent_a')['id']
    b = start(independent, 'independent_b', path='other.txt')['id']
    assert settled(a)['task']['state'] == 'verification'
    assert detail(a)['task']['queue_position'] is None
    act(a, 'reject')
    assert settled(b)['task']['state'] == 'verification'
    act(b, 'reject')
    # A failure inside the guarded save must fence integration immediately, even before TTL expiry.
    guarded = start(independent, 'guarded_save', path='other.txt')['id']
    checked = settled(guarded)
    assert checked['task']['state'] == 'verification'
    old_generation = checked['task']['lease']['generation']
    alias = root / 'hardlink-save-guard.txt'
    import os
    os.link(repo / 'other.txt', alias)
    try:
        act(guarded, 'approve', expected=409)
        recovery = detail(guarded)
        assert recovery['task']['state'] == 'recovery' and recovery['task']['traffic'] == 'blue'
        assert recovery['task']['lease']['state'] == 'recovery' and recovery['task']['lease']['generation'] > old_generation
        assert alias.read_bytes() == b'Independent baseline\n'
        current = api('GET', f'/v1/bee/files/{independent}/preview?path=other.txt')
        act(guarded, 'recover', current_revision=current['revision'])
        assert detail(guarded)['task']['traffic'] == 'green'
    finally:
        alias.unlink()
    invalid = council('HTC_FIXTURE_INVALID')
    bad = start(invalid, 'invalid_proposal', path='other.txt')['id']
    assert settled(bad)['task']['state'] == 'failed'
    act(bad, 'approve', expected=409)
    assert (repo / 'other.txt').read_bytes() == b'Independent baseline\n'
    for attempt in (2, 3):
        act(bad, 'retry')
        assert settled(bad)['task']['attempt'] == attempt
        assert detail(bad)['task']['state'] == 'failed'
    act(bad, 'retry', expected=409)
    # Safe cancellation kills the descendant and preserves the private modification.
    held = council('HTC_FIXTURE_APPLY BEE_FIXTURE_HOLD')
    stopped = start(held, 'cancel_owned', path='other.txt')['id']
    until(lambda: list(Path(detail(stopped)['task']['worktree']).glob('*.pid')), 'owned descendant running')
    worktree = Path(detail(stopped)['task']['worktree'])
    descendant = ProcessHandle(int(next(worktree.glob('*.pid')).read_text()))
    act(stopped, 'pause')
    settled(stopped, ('paused',))
    descendant.wait()
    assert (worktree / 'other.txt').read_bytes() == b'Native isolated proposal\n'
    assert (repo / 'other.txt').read_bytes() == b'Independent baseline\n'
    assert detail(stopped)['task']['traffic'] == 'green'
    act(stopped, 'cancel')
    assert detail(stopped)['task']['state'] == 'cancelled'
    old_session = detail(stopped)['task']['session_id']
    act(stopped, 'retry')
    until(lambda: len(list(worktree.glob('*.pid'))) == 2, 'explicit retry started a fresh owned worker')
    retry_pid = max(worktree.glob('*.pid'), key=lambda p: p.stat().st_mtime_ns)
    retried_child = ProcessHandle(int(retry_pid.read_text()))
    assert detail(stopped)['task']['worktree'] == str(worktree) and detail(stopped)['task']['session_id'] != old_session
    act(stopped, 'cancel')
    settled(stopped, ('cancelled',))
    retried_child.wait()
    # Deliberate restart fixture retains ownership and never replays the owned native session.
    restart = start(held, 'restart_owned', path='other.txt')['id']
    until(lambda: list(Path(detail(restart)['task']['worktree']).glob('*.pid')), 'restart owned descendant')
    worktree = Path(detail(restart)['task']['worktree'])
    descendant = ProcessHandle(int(next(worktree.glob('*.pid')).read_text()))
    return dict(task=restart, worktree=str(worktree), descendant=descendant, council=held, root=str(repo))


def verify_traffic_restart(api, saved, until):
    saved['descendant'].wait()
    path = '/v1/bee/traffic/tasks/' + saved['task']
    restored = api('GET', path)
    assert restored['task']['state'] == 'recovery' and restored['task']['traffic'] == 'blue', restored
    worktree = Path(saved['worktree'])
    audits = list(worktree.glob('*.requests'))
    assert len(audits) == 1 and len(audits[0].read_text().splitlines()) == 1
    time.sleep(1)
    assert len(audits[0].read_text().splitlines()) == 1, 'Restart replayed a native Work request'
    api('POST', path + '/approve', dict(request_id='stale_restart', expected_revision=restored['task']['revision'], authorize=True), 409)
    current = api('GET', f'/v1/bee/files/{saved["council"]}/preview?path=other.txt')
    api('POST', path + '/recover', dict(request_id='reviewed_recovery', expected_revision=restored['task']['revision'], current_revision='0'*64, authorize=True), 409)
    api('POST', path + '/recover', dict(request_id='reviewed_recovery', expected_revision=restored['task']['revision'], current_revision=current['revision'], authorize=True))
    assert api('GET', path)['task']['state'] == 'rejected'
    assert api('GET', path)['task']['traffic'] == 'green'
    assert (worktree / 'other.txt').read_bytes() == b'Native isolated proposal\n'
    assert Path(saved['root']).joinpath('other.txt').read_bytes() == b'Independent baseline\n'
