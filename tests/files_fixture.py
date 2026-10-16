"""Workspace preview boundaries, using files and junctions in disposable roots."""
import subprocess
import hashlib
import json
import os
from urllib.parse import urlencode


def exercise_files(api, conversation, repo):
    base = '/v1/bee/files/' + conversation
    assert api('GET', '/v1/bee/capabilities')['workspace_file_preview'] is True
    api('GET', base + '/list', expected=401, auth=False)
    (repo / 'notes ø').mkdir()
    (repo / 'notes ø/note.md').write_text('# Native preview\n\nUnicode ø — 蜂\n', encoding='utf-8-sig')
    (repo / 'raw.bin').write_bytes(b'\x00\xff')
    (repo / 'huge.txt').write_bytes(b'x' * (256 * 1024 + 1))
    (repo / 'source.py').write_text('def hello():\n    print("Windows")\n', encoding='utf-8')
    listing = api('GET', base + '/list')
    assert listing['read_only'] and any(e['name'] == 'notes ø' and e['directory'] for e in listing['entries'])
    row = api('GET', base + '/preview?' + urlencode({'path': 'notes ø/note.md'}))
    assert row['markdown'] and row['text'].startswith('# Native preview') and '蜂' in row['text']
    assert api('GET', base + '/preview?path=source.py')['language'] == 'python'
    for path in ('../outside.txt', 'C:\\Windows\\win.ini', '//host/share/file', 'raw.bin', 'huge.txt', 'source.py:stream', 'CON', 'CONIN$', 'LPT1', 'notes ø/../source.py'):
        api('GET', base + '/preview?' + urlencode({'path': path}), expected=409)
    api('GET', '/v1/bee/files/no-such-session/list', expected=404)
    outside = repo.parent / 'outside-preview'
    outside.mkdir()
    (outside / 'private.txt').write_text('OUTSIDE_FILE_MUST_STAY_PRIVATE')
    junction = repo / 'outside-link'
    result = subprocess.run(['cmd.exe', '/d', '/c', 'mklink', '/J', str(junction), str(outside)], capture_output=True, text=True)
    assert result.returncode == 0, (result.stdout, result.stderr)
    try:
        api('GET', base + '/preview?path=outside-link/private.txt', expected=409)
        api('GET', base + '/list?path=outside-link', expected=409)
    finally:
        junction.rmdir()  # Removes the junction itself, never the target directory.
    assert (outside / 'private.txt').read_text() == 'OUTSIDE_FILE_MUST_STAY_PRIVATE'
    assert api('GET', '/v1/bee/capabilities')['workspace_file_editing'] is True
    path = 'notes ø/note.md'
    original = (repo / path).read_bytes()
    body = dict(request_id='file_save_1', path=path, expected_revision=row['revision'], text='# Edited\r\n\r\nUnicode 蜂\r\n', authorize_write=True)
    api('POST', base + '/save', body, expected=401, auth=False)
    api('POST', base + '/save', {**body, 'authorize_write': False}, expected=409)
    api('POST', base + '/save', {**body, 'unexpected': True}, expected=422)
    saved = api('POST', base + '/save', body)
    expected = b'\xef\xbb\xbf' + body['text'].encode()
    assert (repo / path).read_bytes() == expected
    assert saved['revision'] == hashlib.sha256(expected).hexdigest()
    assert api('POST', base + '/save', body) == saved
    api('POST', base + '/save', {**body, 'text': 'different'}, expected=409)
    api('POST', base + '/save', {**body, 'request_id': 'stale'}, expected=409)
    history = api('GET', base + '/history?' + urlencode({'path': path}))['saves']
    assert len(history) == 1 and history[0]['before'].encode() == original and history[0]['after'].encode() == expected
    assert api('GET', base + '/saves/file_save_1')['save']['state'] == 'saved'
    # A receipt retry cannot overwrite a later external edit.
    (repo / path).write_bytes(b'External change\n')
    assert api('POST', base + '/save', body) == saved
    assert (repo / path).read_bytes() == b'External change\n'
    for bad in ('../outside.txt', 'outside-link/private.txt', 'raw.bin', 'huge.txt', 'source.py:stream', 'missing.txt'):
        api('POST', base + '/save', {**body, 'request_id': 'bad_path', 'path': bad}, expected=409)
    api('POST', base + '/save', {**body, 'request_id': 'nul', 'text': 'x\0y'}, expected=409)
    api('POST', base + '/save', {**body, 'request_id': 'large', 'text': 'x' * (256 * 1024 + 1)}, expected=409)
    edge = repo / 'edge.txt'
    edge.write_bytes(b'')
    edge_row = api('GET', base + '/preview?path=edge.txt')
    edge_body = {**body, 'request_id': 'max_utf8', 'path': 'edge.txt', 'expected_revision': edge_row['revision'], 'text': 'ø' * (128 * 1024)}
    api('POST', base + '/save', edge_body)
    assert edge.stat().st_size == 256 * 1024, 'The JSON body limit prevented a supported UTF-8 edit'
    edge.write_bytes(b'\xef\xbb\xbf')
    edge_row = api('GET', base + '/preview?path=edge.txt')
    api('POST', base + '/save', {**edge_body, 'request_id': 'bom_limit', 'expected_revision': edge_row['revision']}, expected=409)
    assert edge.read_bytes() == b'\xef\xbb\xbf'
    assert api('GET', base + '/preview?' + urlencode({'path': path}))['newline'] == 'lf'
    hard = repo / 'hard.txt'
    os.link(outside / 'private.txt', hard)
    hard_row = api('GET', base + '/preview?path=hard.txt')
    api('POST', base + '/save', {**body, 'request_id': 'hard_link', 'path': 'hard.txt', 'expected_revision': hard_row['revision']}, expected=409)
    assert (outside / 'private.txt').read_text() == 'OUTSIDE_FILE_MUST_STAY_PRIVATE'
    hard.unlink()
    # Windows sharing semantics: a live reader prevents an exclusive save.
    with (repo / 'source.py').open('rb'):
        loaded = api('GET', base + '/preview?path=source.py')
        api('POST', base + '/save', {**body, 'request_id': 'busy', 'path': 'source.py', 'expected_revision': loaded['revision']}, expected=409)
    # Successful edits retain only the newest ten complete backups per file.
    for i in range(12):
        loaded = api('GET', base + '/preview?path=source.py')
        api('POST', base + '/save', {**body, 'request_id': f'retention_{i:02d}', 'path': 'source.py', 'expected_revision': loaded['revision'], 'text': f'print({i})\n'})
    history = api('GET', base + '/history?path=source.py')['saves']
    assert len(history) == 12 and sum(r['before'] is not None for r in history) == 10
    assert api('GET', base + '/saves/retention_00')['save']['state'] == 'saved'
    return body, saved


def prepare_file_recovery(state, repo):
    """Persist the exact journal states possible across a crash, in disposable state."""
    folder = state / 'extensions/file-edits'
    partial = folder / 'save_file_save_1.json'
    record = json.loads(partial.read_text(encoding='utf-8'))
    record['state'] = 'prepared'
    partial.write_text(json.dumps(record), encoding='utf-8')
    (repo / record['path']).write_bytes(b'PARTIAL_WRITE')
    complete = folder / 'save_retention_11.json'
    record = json.loads(complete.read_text(encoding='utf-8'))
    record['state'] = 'prepared'
    complete.write_text(json.dumps(record), encoding='utf-8')


def verify_file_recovery(api, conversation, repo, body, saved):
    base = '/v1/bee/files/' + conversation
    receipt = api('GET', base + '/saves/file_save_1')['save']
    assert receipt['state'] == 'unknown_after_restart' and receipt['before'] is not None
    assert (repo / body['path']).read_bytes() == b'PARTIAL_WRITE', 'Restart replayed a file write'
    api('POST', base + '/save', body, expected=409)
    # Recovery requires an explicit new save with the currently loaded disk revision.
    disk = api('GET', base + '/preview?' + urlencode({'path': body['path']}))
    restored = receipt['before'].removeprefix('\ufeff')
    api('POST', base + '/save', {**body, 'request_id': 'recovery_reviewed', 'expected_revision': disk['revision'], 'text': restored, 'bom': receipt['before'].startswith('\ufeff')})
    assert (repo / body['path']).read_bytes() == receipt['before'].encode('utf-8'), 'Recovery did not restore the original BOM'
    complete = api('GET', base + '/saves/retention_11')['save']
    retry = dict(request_id=complete['request'], path=complete['path'], expected_revision=complete['before_hash'], text=complete['after'], authorize_write=True)
    assert complete['state'] == 'unknown_after_restart'
    assert api('POST', base + '/save', retry)['state'] == 'saved', 'Completed write could not reconcile its receipt'
