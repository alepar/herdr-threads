#!/usr/bin/env python3
"""No-model native codex exec policy probe. Only private mock API and owned sockets/files."""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, threading, time, pty, select, signal, fcntl, termios, struct
if not os.environ.get('CODEX_BINARY') or not os.environ.get('PROBE_OUTSIDE_BASE'):
 raise SystemExit('set CODEX_BINARY to an absolute Codex executable and PROBE_OUTSIDE_BASE to a writable non-temporary directory outside the probe workspace')
CODEX=pathlib.Path(os.environ['CODEX_BINARY']).resolve(strict=True)
if not CODEX.is_file():raise SystemExit('CODEX_BINARY must name a regular executable file')
OUTSIDE_BASE=pathlib.Path(os.environ['PROBE_OUTSIDE_BASE']).resolve(strict=True)
if not OUTSIDE_BASE.is_dir() or any(OUTSIDE_BASE.is_relative_to(pathlib.Path(p).resolve()) for p in (tempfile.gettempdir(),'/tmp')):
 raise SystemExit('PROBE_OUTSIDE_BASE must be an existing directory outside temporary writable roots')
HERE=pathlib.Path(__file__).resolve().parent
ROOT=pathlib.Path(tempfile.mkdtemp(prefix='ht-native-codex-'))
WORKDIR=ROOT/'project';WORKDIR.mkdir()
OUTSIDE=pathlib.Path(tempfile.mkdtemp(prefix='ht-native-outside-',dir=OUTSIDE_BASE))
for name in ('intents','contexts','sibling'):(OUTSIDE/name).mkdir(exist_ok=True)
ALLOW=ROOT/'allow.sock';OTHER=ROOT/'other.sock'
stop=threading.Event(); listeners=[]; threads=[]
def unix_listener(path):
 s=socket.socket(socket.AF_UNIX);s.bind(str(path));s.listen(8);s.settimeout(.2);listeners.append(s)
 def loop():
  while not stop.is_set():
   try:c,_=s.accept();c.close()
   except socket.timeout:pass
   except OSError:break
 t=threading.Thread(target=loop,daemon=True);t.start();threads.append(t)
for path in (ALLOW,OTHER):unix_listener(path)
tcp=socket.socket();tcp.bind(('127.0.0.1',0));tcp.listen(8);tcp.settimeout(.2);listeners.append(tcp);local_port=tcp.getsockname()[1]
service=socket.socket();service.bind(('127.0.0.1',0));api_port=service.getsockname()[1];service.close()
server=None
try:
 script=ROOT/'tool.py'
 script.write_text('''import json,socket,sys,pathlib\nout={}\nfor label,family,address in [('allow',socket.AF_UNIX,sys.argv[1]),('other',socket.AF_UNIX,sys.argv[2]),('loopback',socket.AF_INET,('127.0.0.1',int(sys.argv[3]))),('external',socket.AF_INET,('1.1.1.1',53))]:\n s=socket.socket(family);s.settimeout(2)\n try:s.connect(address);out[label]='connected'\n except OSError as e:out[label]='errno:'+str(e.errno)\n finally:s.close()\nfor label,path in [('intent',sys.argv[4]),('context',sys.argv[5]),('sibling',sys.argv[6])]:\n try:pathlib.Path(path).write_text('probe');out[label]='wrote'\n except OSError as e:out[label]='errno:'+str(e.errno)\nprint('NATIVE_POLICY_RESULT',json.dumps(out))\n''')
 home=ROOT/'home';home.mkdir();(home/'config.toml').write_text('')
 log=ROOT/'mock.jsonl'
 server=subprocess.Popen(['python3',str(HERE/'mock_probe_responses.py'),str(api_port),str(log)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE,start_new_session=True)
 for _ in range(100):
  try:
   c=socket.create_connection(('127.0.0.1',api_port),.1);c.close();break
  except OSError:time.sleep(.02)
 else:raise RuntimeError('mock provider did not start')
 def toml(s):return json.dumps(s)
 command='python3 '+str(script)+' '+str(ALLOW)+' '+str(OTHER)+' '+str(local_port)+' '+str(OUTSIDE/'intents'/'probe.txt')+' '+str(OUTSIDE/'contexts'/'probe.txt')+' '+str(OUTSIDE/'sibling'/'probe.txt')
 args=[str(CODEX),'exec','--ephemeral','--skip-git-repo-check','-C',str(WORKDIR),'-c','model_provider="mock"','-c',f'model_providers.mock={{name="mock",base_url="http://127.0.0.1:{api_port}/v1",wire_api="responses"}}','-c','model="canary"','-c','approval_policy="never"','-c','sandbox_mode="workspace-write"','-c','sandbox_workspace_write.network_access=true','-c','features.network_proxy.enabled=true','-c','features.network_proxy.unix_sockets={'+toml(str(ALLOW))+'="allow"}','-c','sandbox_workspace_write.writable_roots=['+','.join(toml(str(OUTSIDE/name)) for name in ('intents','contexts'))+']','PROBE '+command]
 env=os.environ.copy();env.update(CODEX_HOME=str(home),HOME=str(ROOT/'home'),OPENAI_API_KEY='local-mock-key')
 started=time.monotonic();p=subprocess.run(args,env=env,text=True,capture_output=True,timeout=30)
 print('native exec rc',p.returncode,'elapsed_ms',round((time.monotonic()-started)*1000))
 print('stdout',p.stdout[-1000:]);print('stderr',p.stderr[-2400:]);print('mock log',log.read_text()[-1000:] if log.exists() else '<absent>')
 # Repeat the identical native policy through interactive Codex in a PTY.
 tui_args=[str(CODEX),'--no-daemon','-C',str(WORKDIR),'-c','model_provider="mock"','-c',f'model_providers.mock={{name="mock",base_url="http://127.0.0.1:{api_port}/v1",wire_api="responses"}}','-c','model="canary"','-c','approval_policy="never"','-c','sandbox_mode="workspace-write"','-c','sandbox_workspace_write.network_access=true','-c','features.network_proxy.enabled=true','-c','features.network_proxy.unix_sockets={'+toml(str(ALLOW))+'="allow"}','-c','sandbox_workspace_write.writable_roots=['+','.join(toml(str(OUTSIDE/name)) for name in ('intents','contexts'))+']','-c','projects.'+toml(str(WORKDIR))+'.trust_level="trusted"','PROBE '+command]
 master,slave=pty.openpty()
 fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',40,120,0,0))
 tui=subprocess.Popen(tui_args,env=env,stdin=slave,stdout=slave,stderr=slave,start_new_session=True)
 os.close(slave);output=b'';started=time.monotonic()
 try:
  while time.monotonic()-started<12:
   ready,_,_=select.select([master],[],[],.2)
   if ready:
    try:chunk=os.read(master,65536)
    except OSError:break
    if not chunk:break
    output+=chunk
    if b'\x1b[6n' in chunk:os.write(master,b'\x1b[1;1R')
    if b'NATIVE_POLICY_RESULT' in output and log.exists() and log.read_text().count('"req"') >= 4:break
   if tui.poll() is not None:break
  print('native TUI elapsed_ms',round((time.monotonic()-started)*1000),'result_seen',b'NATIVE_POLICY_RESULT' in output)
  print('tui output',output.decode('utf-8','replace')[-2500:])
  for line in log.read_text().splitlines() if log.exists() else []:
   record=json.loads(line)
   if record.get('tool_outputs'):
    print('tool output for request',record['req'],record['tool_outputs'])
 finally:
  if tui.poll() is None:
   os.killpg(tui.pid,signal.SIGTERM)
   try:tui.wait(timeout=3)
   except subprocess.TimeoutExpired:os.killpg(tui.pid,signal.SIGKILL);tui.wait()
  os.close(master)
finally:
 stop.set()
 for s in listeners:s.close()
 for t in threads:t.join(timeout=.5)
 if server:
  server.terminate()
  try:server.wait(timeout=3)
  except subprocess.TimeoutExpired:server.kill();server.wait()
 shutil.rmtree(ROOT,ignore_errors=True)
 shutil.rmtree(OUTSIDE,ignore_errors=True)
