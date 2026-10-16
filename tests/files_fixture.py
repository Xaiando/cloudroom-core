"""Workspace preview boundaries, using files and junctions in disposable roots."""
import subprocess
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
