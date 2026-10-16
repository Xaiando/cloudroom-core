"""Projects and independent threads, using a disposable local Core and fixture CLI."""
import json, os, secrets, socket, subprocess, sys, tempfile
from pathlib import Path
from urllib.request import Request, urlopen
from urllib.error import HTTPError
from windows_native import until

ROOT = Path(__file__).resolve().parents[1]

def main():
    with tempfile.TemporaryDirectory(prefix='cloud bee projects ') as folder:
        root = Path(folder)
        for name in ('repo', 'project one', 'project two', 'home/.codex'):
            (root / name).mkdir(parents=True)
        wrapper = root / 'fixture.cmd'
        wrapper.write_text(f'@echo off\r\n"{sys.executable}" -u "{ROOT / "tests/core_fixture.py"}" %*\r\n')
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0)); port = s.getsockname()[1]
        token = secrets.token_hex(32)
        env = {k:v for k,v in os.environ.items() if not k.startswith('CLOUDROOM_')}
        env.update(CLOUDROOM_TOKEN=token,CLOUDROOM_LISTEN=f'127.0.0.1:{port}',CLOUDROOM_WINDOWS_LOCAL='1',CLOUDROOM_LOCAL_HISTORY='1',CLOUDROOM_UNPROTECTED_TEST_MODE='1',CLOUDROOM_STORE='projects-test',CLOUDROOM_STATE_DIR=str(root/'state'),CLOUDROOM_ACCOUNT_HOME=str(root/'home'),CLOUDROOM_REPOSITORY=str(root/'repo'),CLOUDROOM_CODEX_BINARY=str(wrapper),CLOUDROOM_CODEX_HOME=str(root/'home/.codex'),CLOUDROOM_CODEX_MODEL='fixture')
        def api(method, path, body=None, expected=200):
            req=Request(f'http://127.0.0.1:{port}'+path,method=method,data=json.dumps(body).encode() if body is not None else None,headers={'Authorization':'Bearer '+token,'Content-Type':'application/json'})
            try: response=urlopen(req,timeout=10)
            except HTTPError as error: response=error
            with response:
                raw=response.read(); assert response.code==expected,(path,response.code,raw)
                return json.loads(raw)
        log=(root/'core.log').open('wb')
        core=None
        def start():
            process=subprocess.Popen([str(Path(sys.argv[1]).resolve())],env=env,stdout=log,stderr=log,creationflags=subprocess.CREATE_NO_WINDOW)
            def ready():
                assert process.poll() is None, (root/'core.log').read_text(errors='replace')
                try: return api('GET','/v1/ready')['ready']
                except OSError: return False
            until(ready,'Core ready'); return process
        try:
            core=start()
            route='/v1/bee/projects/test_project'
            write=dict(request_id='project_create',expected_revision=0,name='Research 蜂',description='Independent chats',color='blue',folder=str(root/'project one'),archived=False)
            result=api('POST',route,write); assert api('POST',route,write)==result
            api('POST',route,{**write,'name':'changed'},409)
            project=api('GET','/v1/bee/projects')['projects']['test_project']
            ids=[]
            for i in range(2):
                sid=api('POST','/v1/bee/sessions',dict(request_id=f'project_chat_{i}',harness='codex',model='fixture',workspace=project['workspace_id'],group=f'group_{i}'))['session_id']
                ids.append(sid)
                assignment=dict(request_id=f'assign_{i}',project_id='test_project',expected_project=None)
                assert api('POST','/v1/bee/projects/threads/'+sid,assignment)==api('POST','/v1/bee/projects/threads/'+sid,assignment)
            assert ids[0]!=ids[1]
            state=lambda sid:api('GET','/v1/bee/sessions/'+sid)['session']
            assert Path(state(ids[0])['workspace']['path']).samefile(root/'project one')
            handoff=dict(objective='Inspect fixture project',context='',requirements='',constraints='',acceptance='',artifacts='')
            api('POST','/v1/bee/sessions/'+ids[0]+'/mode',dict(request_id='work',mode='work',authorize_work=False,handoff=handoff),409)
            api('POST','/v1/bee/sessions/'+ids[0]+'/mode',dict(request_id='work',mode='work',authorize_work=True,handoff=handoff))
            until(lambda:state(ids[0])['state']=='idle','Work ready in selected project')
            native=state(ids[0])['execution_session_id']
            assert Path(api('GET','/v1/sessions/'+native)['session']['workspace']['path']).samefile(root/'project one')
            api('POST','/v1/bee/sessions/'+ids[0]+'/close',dict(request_id='close'))
            api('POST',route,{**write,'request_id':'stale','name':'stale'},409)
            update={**write,'request_id':'project_edit','expected_revision':1,'folder':str(root/'project two'),'name':'Renamed','archived':True}
            api('POST',route,update)
            assert Path(state(ids[1])['workspace']['path']).samefile(root/'project one'), 'Project edit moved an existing thread'
            api('POST','/v1/bee/projects/threads/'+ids[1],dict(request_id='wrong_move',project_id=None,expected_project=None),409)
            api('POST','/v1/bee/projects/threads/missing',dict(request_id='missing',project_id='test_project',expected_project=None),404)
            move=dict(request_id='move',project_id=None,expected_project='test_project')
            api('POST','/v1/bee/projects/threads/'+ids[1],move)
            before=api('GET','/v1/bee/projects')
            core.terminate();core.wait(timeout=15);core=start()
            assert api('GET','/v1/bee/projects')==before
            assert api('POST','/v1/bee/projects/threads/'+ids[1],move)['project_id'] is None
            assert before['projects']['test_project']['archived'] and len(before['threads'])==1
            api('POST',route,{**update,'request_id':'invalid','expected_revision':2,'folder':str(root/'missing')},409)
            print('PASS: project CRUD/revisions/retries, Unicode, independent thread membership, external local workspace, explicit Work authorization, preserved old workspaces, moves, archive and restart without inference')
        finally:
            if core and core.poll() is None: core.terminate();core.wait(timeout=15)
            log.close()

if __name__=='__main__':main()
