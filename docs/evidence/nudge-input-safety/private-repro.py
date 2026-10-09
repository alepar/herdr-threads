import sys, os, json, tempfile, time, uuid
from pathlib import Path
sys.path.insert(0, '/Users/alepar/AleCode/herdr-threads/.worktrees/nudge-input-safety/tests/native/recovery')
from private_host import PrivateHerdr, CommandLog
root=Path(tempfile.mkdtemp(prefix='htnud-',dir='/private/tmp'))
log=CommandLog(root/'commands.jsonl')
host=PrivateHerdr(root,log,'nudge-input-safety')
host.herdr='/Users/alepar/AleCode/herdr-threads/target/coordinator/herdr-093/bin/herdr'
run_id=str(uuid.uuid4()); os.environ['HT_LEAK_RUN_ID']=run_id
print(json.dumps({'root':str(root),'run_id':run_id}),flush=True)
(root/'nudge-input-safety-cfg').mkdir()
(root/'nudge-input-safety-cfg/herdr.toml').write_text("default_shell = '/bin/sh'\n")
bin=root/'bin'; bin.mkdir()
received=root/'received.jsonl'
standin=bin/'claude'
standin.write_text("#!/bin/sh\nprintf '\033]0;✳ Dummy\007\033[2J\033[H────────────────────\n❯ \n────────────────────\n'\nwhile IFS= read -r line; do\n printf '%s\n' \"$line\" >> "+str(received)+"\n printf '\033]0;✳ Dummy\007\033[2J\033[H• observed:%s\n────────────────────\n❯ \n────────────────────\n' \"$line\"\ndone\n")
standin.chmod(0o700)
results={}
try:
 host.start(); results['binary']=host.check_binary()
 ws=host.api('workspace.create',{'label':'nudge-input-safety','cwd':str(root),'focus':False,'env':{'PATH':str(bin)+':/usr/bin:/bin'}})
 pane=ws['root_pane']['pane_id']
 host.api('agent.start',{'name':'nudge-private','kind':'claude','pane_id':pane,'args':[],'timeout_ms':10000})
 deadline=time.monotonic()+15
 while time.monotonic()<deadline:
  agent=host.api('agent.get',{'target':pane})['agent']
  if agent.get('agent_status') in ('idle','done') and agent.get('interactive_ready'): break
  time.sleep(.1)
 else: raise AssertionError('stand-in not ready')
 marker='herdr-threads: attention pending; run herdr-threads inbox'
 host.api('pane.send_text',{'pane_id':pane,'text':'DUMMY-prefix split my inp'})
 results['draft_agent']=host.api('agent.get',{'target':pane})
 results['draft_screen']=host.api('agent.read',{'target':pane,'source':'detection'})
 results['prompt']=host.api('agent.prompt',{'target':pane,'text':marker})
 host.api('pane.send_text',{'pane_id':pane,'text':'ut DUMMY-suffix'})
 host.api('pane.send_keys',{'pane_id':pane,'keys':['enter']})
 deadline=time.monotonic()+5
 while time.monotonic()<deadline:
  lines=received.read_text().splitlines() if received.exists() else []
  if len(lines)>=2: break
  time.sleep(.05)
 results['received']=lines
 assert lines==['DUMMY-prefix split my inp'+marker,'ut DUMMY-suffix'], lines
 results['verdict']='REPRODUCED: prefix merged and submitted; suffix became separate submission'
 print(json.dumps(results,ensure_ascii=False,indent=2),flush=True)
finally:
 host.stop(); log.close()
 (root/'results.json').write_text(json.dumps(results,ensure_ascii=False,indent=2)+'\n')
 (root/'run-id').write_text(run_id+'\n')
 print('private host stopped',flush=True)
