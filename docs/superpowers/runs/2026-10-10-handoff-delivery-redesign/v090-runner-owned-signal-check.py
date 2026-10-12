import ast, hashlib, importlib.util, json, os, pathlib, selectors, signal, subprocess, sys, time, uuid
root=pathlib.Path(__file__).parent
runner=root/'resume-v090-run.py'
evidence=pathlib.Path(sys.argv[1]);evidence.mkdir()
spec=importlib.util.spec_from_file_location('reviewed_runner',runner);module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
tree=ast.parse(runner.read_text());main=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='main');function=next(n for n in main.body if isinstance(n,ast.FunctionDef) and n.name=='command')
result={'runner_sha256':hashlib.sha256(runner.read_bytes()).hexdigest(),'scope':'AST-extracted actual command; real private Python children and OS signals; no Cargo or project runtime','cases':[]}
real_popen=subprocess.Popen
for sig in (signal.SIGTERM,signal.SIGINT):
    cancellation=module.Cancellation();record={'commands':[]};env=dict(os.environ);env['HT_LEAK_RUN_ID']=str(uuid.uuid4());owned=[]
    class Proxy:
        PIPE=subprocess.PIPE;STDOUT=subprocess.STDOUT
        def Popen(self,*args,**kwargs):
            child=real_popen(*args,**kwargs);owned.append(child)
            os.kill(os.getpid(),sig)
            return child
    ns=dict(vars(module));ns.update(evidence=evidence,record=record,record_path=evidence/('record-'+str(sig)+'.json'),source=root,env=env,cancellation=cancellation,subprocess=Proxy())
    exec(compile(ast.Module(body=[function],type_ignores=[]),str(runner),'exec'),ns)
    previous={s:signal.signal(s,cancellation.handle) for s in (signal.SIGTERM,signal.SIGINT)}
    caught=False
    try:
        ns['command']('actual-signal-'+str(sig),[sys.executable,'-c','import time; time.sleep(60)'])
    except module.TerminationRequested as exc:
        caught=exc.signum==sig
    finally:
        for s,h in previous.items():signal.signal(s,h)
        # Emergency cleanup of these exact private children if assertions expose a regression.
        for child in owned:
            if child.poll() is None:module.terminate_owned_group(child)
    assert caught and len(owned)==1
    child=owned[0];entry=record['commands'][0]
    try:os.killpg(child.pid,0);gone=False
    except ProcessLookupError:gone=True
    assert entry.get('owned_process_group')==child.pid and gone and child.poll() is not None,(sig,entry,gone)
    result['cases'].append({'signal':signal.Signals(sig).name,'pid':child.pid,'exit':child.returncode,'command_owned_cleanup_recorded':True,'owned_group_absent':gone,'run_id':env['HT_LEAK_RUN_ID']})
(evidence/'execution.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result),flush=True)
