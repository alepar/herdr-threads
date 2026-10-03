import os, pathlib, socket, subprocess, tempfile, threading, json, sys, shutil, time
if not os.environ.get('CODEX_BINARY'):
    raise SystemExit('set CODEX_BINARY to the absolute Codex executable to probe')
codex_binary=pathlib.Path(os.environ['CODEX_BINARY']).resolve(strict=True)
if not codex_binary.is_file():
    raise SystemExit('CODEX_BINARY must name a regular executable file')
root=pathlib.Path(tempfile.mkdtemp(prefix='ht-codex160-'))
ch=root/'codex-home'; ch.mkdir(); home=root/'home'; home.mkdir()
profile_home=root/'profile-home'; profile_home.mkdir()
allow=root/'allow.sock'; other=root/'other.sock'
listeners=[]; accepted={'allow':0,'other':0}; stop=threading.Event()
def listen(path,key):
    s=socket.socket(socket.AF_UNIX); s.bind(str(path)); s.listen(8); s.settimeout(.2); listeners.append(s)
    def run():
        while not stop.is_set():
            try:
                c,_=s.accept(); accepted[key]+=1; c.close()
            except socket.timeout: pass
            except OSError: break
    t=threading.Thread(target=run,daemon=True); t.start(); return t
threads=[listen(allow,'allow'),listen(other,'other')]
tcp=socket.socket();tcp.bind(('127.0.0.1',0));tcp.listen(8);tcp.settimeout(.2);listeners.append(tcp);port=tcp.getsockname()[1]
script=root/'test.py'
script.write_text('''import socket,sys,json,os\nout={}\nfor label,family,address in [('allow',socket.AF_UNIX,sys.argv[1]),('other',socket.AF_UNIX,sys.argv[2]),('loopback',socket.AF_INET,('127.0.0.1',int(sys.argv[3]))),('external',socket.AF_INET,('1.1.1.1',53))]:\n s=socket.socket(family);s.settimeout(2)\n try:s.connect(address);out[label]='connected'\n except OSError as e:out[label]='errno:'+str(e.errno)\n finally:s.close()\nprint(json.dumps({'results':out,'proxies':{k:bool(os.getenv(k)) for k in ('HTTP_PROXY','HTTPS_PROXY','ALL_PROXY')}}))\n''')
config='[sandbox_workspace_write]\nnetwork_access = true\n[features.network_proxy]\nenabled = true\nunix_sockets = { '+json.dumps(str(allow))+' = "allow" }\n'
(ch/'config.toml').write_text(config)
(profile_home/'config.toml').write_text('default_permissions = "socket"\n[permissions.socket]\nextends = ":workspace"\n[permissions.socket.network]\nenabled = false\n[permissions.socket.network.unix_sockets]\n'+json.dumps(str(allow))+' = "allow"\n')
base=[str(codex_binary),'sandbox','-c','sandbox_mode="workspace-write"']
env=os.environ.copy();env.update(CODEX_HOME=str(ch),HOME=str(home))
baseline=subprocess.run(['python3',str(script),str(allow),str(other),str(port)],env=env,text=True,capture_output=True,timeout=15)
print('unsandboxed baseline',baseline.returncode,baseline.stdout.strip(),flush=True)
for label,extra in [('config',[]),('flag-no-proxy',['-c','sandbox_workspace_write.network_access=false','--allow-unix-socket',str(allow)]),('empty-no-proxy',['-c','sandbox_workspace_write.network_access=false'])]:
 try:
  started=time.monotonic()
  p=subprocess.run(base+extra+['--','python3',str(script),str(allow),str(other),str(port)],env=env,text=True,capture_output=True,timeout=20)
  print(label,'rc',p.returncode,'elapsed_ms',round((time.monotonic()-started)*1000),'stdout',p.stdout.strip(),'stderr',p.stderr.strip()[:500],flush=True)
 except Exception as e:print(label,type(e).__name__,str(e),flush=True)
profile_env=env.copy(); profile_env['CODEX_HOME']=str(profile_home)
try:
 started=time.monotonic()
 p=subprocess.run([base[0],'sandbox','-P','socket','--','python3',str(script),str(allow),str(other),str(port)],env=profile_env,text=True,capture_output=True,timeout=20)
 print('named-profile rc',p.returncode,'elapsed_ms',round((time.monotonic()-started)*1000),'stdout',p.stdout.strip(),'stderr',p.stderr.strip()[:500],flush=True)
except Exception as e:print('named-profile',type(e).__name__,str(e),flush=True)
print('accepted',accepted,'scratch',root,flush=True)
stop.set()
for s in listeners:s.close()
for t in threads:t.join(timeout=1)
shutil.rmtree(root)
