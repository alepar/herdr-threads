import argparse,hashlib,json,os,pathlib,subprocess,time,uuid
p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--target',required=True);p.add_argument('--name',action='append',required=True);p.add_argument('--evidence',required=True);a=p.parse_args();src=pathlib.Path(a.source);e=pathlib.Path(a.evidence);e.mkdir();env=dict(os.environ);rid=str(uuid.uuid4());origin=pathlib.Path(env['HOME']);env.setdefault('CARGO_HOME',str(origin/'.cargo'));env.setdefault('RUSTUP_HOME',str(origin/'.rustup'))
for k in ['HOME','CLAUDE_CONFIG_DIR','CODEX_HOME','GEMINI_CONFIG_DIR','GEMINI_CLI_HOME']:
 d=e/k;d.mkdir();env[k]=str(d)
env.update(CARGO_BUILD_JOBS='4',CARGO_TARGET_DIR='/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target',HT_LEAK_RUN_ID=rid)
def git(*x):return subprocess.check_output(['git',*x],cwd=src,text=True).strip()
head=git('rev-parse','HEAD');tree=git('rev-parse','HEAD^{tree}');assert not git('status','--porcelain','--untracked-files=no');r={'head':head,'tree':tree,'run_id':rid,'expected':a.name,'commands':[]};expression=' | '.join('test(='+n+')' for n in a.name);target=['--lib'] if a.target=='lib' else ['--test',a.target];binary='herdr-threads' if a.target=='lib' else 'herdr-threads::'+a.target
base=['nice','cargo','nextest'];tail=['--locked','--all-features',*target]
def run(label,argv):
 t=time.monotonic()
 with (e/(label+'.log')).open('wb') as f:c=subprocess.run(argv,cwd=src,env=env,stdout=f,stderr=subprocess.STDOUT).returncode
 r[label]={'exit':c,'seconds':time.monotonic()-t,'argv':argv};print(label,c,flush=True);return c
code=1
try:
 t=time.monotonic()
 with (e/'list.json').open('wb') as f, (e/'list.stderr').open('wb') as err:c=subprocess.run([*base,'list',*tail,'--message-format','json','-E',expression],cwd=src,env=env,stdout=f,stderr=err).returncode
 r['list']={'exit':c,'seconds':time.monotonic()-t};assert c==0
 data=json.loads((e/'list.json').read_text());selected=[]
 for b,suite in data['rust-suites'].items():
  for n,test in suite['testcases'].items():
   if test['filter-match']['status']=='matches':
    assert test['ignored'] is False and test['kind']=='test';selected.append((b,n))
 assert set(selected)=={(binary,n) for n in a.name} and len(selected)==len(a.name),selected
 r['selected']=selected;r['binaries']=[{'path':s['binary-path'],'sha256':hashlib.sha256(pathlib.Path(s['binary-path']).read_bytes()).hexdigest()} for s in data['rust-suites'].values()]
 assert git('rev-parse','HEAD')==head and not git('status','--porcelain','--untracked-files=no')
 code=run('run',[*base,'run',*tail,'--no-fail-fast','-E',expression]);assert git('rev-parse','HEAD')==head and git('rev-parse','HEAD^{tree}')==tree and not git('status','--porcelain','--untracked-files=no')
finally:
 r['leak_exit']=run('leaks',['scripts/check-no-leaked-processes','--run-id',rid]);(e/'execution.json').write_text(json.dumps(r,indent=2)+'\n')
print(json.dumps(r),flush=True);raise SystemExit(code or r['leak_exit'])
