import argparse, hashlib, json, os, pathlib, subprocess, time
p=argparse.ArgumentParser();p.add_argument('--source',required=True);p.add_argument('--tree',required=True);p.add_argument('--evidence',required=True);a=p.parse_args();src=pathlib.Path(a.source);out=pathlib.Path(a.evidence);out.mkdir();env=dict(os.environ);origin=pathlib.Path(env['HOME']);env.setdefault('CARGO_HOME',str(origin/'.cargo'));env.setdefault('RUSTUP_HOME',str(origin/'.rustup'))
for key in ['HOME','CLAUDE_CONFIG_DIR','CODEX_HOME','GEMINI_CONFIG_DIR','GEMINI_CLI_HOME']:
 d=out/key;d.mkdir();env[key]=str(d)
env.update(CARGO_BUILD_JOBS='4',CARGO_TARGET_DIR='/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target')
def stamp():
 for cmd in [['git','diff','--quiet'],['git','diff','--cached','--quiet',a.tree]]:assert subprocess.run(cmd,cwd=src).returncode==0
 return hashlib.sha256(subprocess.check_output(['git','ls-files','-s'],cwd=src)).hexdigest()
frozen=stamp();r={'staged_tree':a.tree,'index_sha256':frozen,'commands':[]};code=0
try:
 for stage,argv in [('fmt',['cargo','fmt','--','--check']),('neutral',['scripts/check-harness-neutral']),('clippy',['nice','cargo','clippy','--locked','--all-targets','--all-features','--','-D','warnings']),('default',['nice','scripts/check-default-features'])]:
  assert stamp()==frozen;start=time.monotonic();log=out/(stage+'.log')
  with log.open('wb') as f:code=subprocess.run(argv,cwd=src,env=env,stdout=f,stderr=subprocess.STDOUT).returncode
  item={'stage':stage,'argv':argv,'exit':code,'seconds':time.monotonic()-start,'log':str(log),'sha256':hashlib.sha256(log.read_bytes()).hexdigest()};r['commands'].append(item);print(json.dumps(item),flush=True);assert stamp()==frozen
  if code:break
finally:
 r['exit']=code;(out/'execution.json').write_text(json.dumps(r,indent=2)+'\n')
raise SystemExit(code)
