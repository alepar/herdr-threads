import collections,hashlib,json,os,pathlib,selectors,shutil,signal,sqlite3,subprocess,time,uuid
root=pathlib.Path.cwd();out=root/'target/queue-qualification';receiver=json.loads((out/'sqlite-effects-receiver.json').read_text());source=pathlib.Path(receiver['sqlite_home']);scratch=out/'sqlite-effects-private';run_id=os.environ['HT_LEAK_RUN_ID'];session=os.environ.get('CODEX_THREAD_ID') or os.environ['CODEX_SESSION_ID'];exe=pathlib.Path(receiver['executable']);st=exe.stat();assert (st.st_dev,st.st_ino)==(receiver['executable_device'],receiver['executable_inode'])
assert not scratch.exists();assert shutil.disk_usage(out).free>150*1024**3
scratch.mkdir(mode=0o700);dbdir=scratch/'databases';dbdir.mkdir(mode=0o700)
record={'experiment':'populated independent read-only backups; network and real home masked','atomic_cross_database_snapshot':False,'model_or_load_resume_requests':0,'auth_or_config_copied':False,'run_id':run_id,'executable_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'cycles':[]}
def ident(name):return '"'+name.replace('"','""')+'"'
def snap():
 result={}
 for p in sorted(dbdir.glob('*.sqlite')):
  db=sqlite3.connect(p.as_uri()+'?mode=ro',uri=True)
  try:
   schema=db.execute("SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name").fetchall();tables={}
   for (name,) in db.execute("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name"):
    digests=[]
    for row in db.execute('SELECT * FROM '+ident(name)):
     # Values are hashed in memory only; no row values or private names retained.
     payload=json.dumps(row,default=lambda x:{'blob':x.hex()},ensure_ascii=True,separators=(',',':')).encode();digests.append(hashlib.sha256(payload).digest())
    tables[name]={'rows':len(digests),'sha256':hashlib.sha256(b''.join(sorted(digests))).hexdigest()}
   result[p.name]={'schema_sha256':hashlib.sha256(json.dumps(schema).encode()).hexdigest(),'tables':tables}
  finally:db.close()
 return result
children=set()
def opened(child):
 # Host process table discovery only; no argv/env values are retained.
 todo=[child.pid];seen=set();names=set()
 while todo:
  pid=todo.pop()
  if pid in seen:continue
  seen.add(pid)
  try:
   for f in pathlib.Path(f'/proc/{pid}/fd').iterdir():
    try:
     name=f.readlink().name
     if name.endswith('.sqlite') or name.endswith('.sqlite-wal') or name.endswith('.sqlite-shm'):names.add(name)
    except OSError:pass
   todo.extend(map(int,pathlib.Path(f'/proc/{pid}/task/{pid}/children').read_text().split()))
  except OSError:pass
 return sorted(names)
try:
 for p in sorted(source.glob('*.sqlite')):
  before=time.monotonic();src=sqlite3.connect(p.as_uri()+'?mode=ro',uri=True);dst=sqlite3.connect(dbdir/p.name)
  try:
   def progress(*_):
    if time.monotonic()-before>30:raise RuntimeError('backup budget exhausted')
   src.backup(dst,pages=512,progress=progress)
  finally:dst.close();src.close()
 record['before']=snap()
 # Copied receiver ID only addresses its private metadata; correlations are generated.
 for cycle in range(3):
  home=scratch/f'home-{cycle}';home.mkdir(mode=0o700)
  env={'PATH':'/usr/bin:/bin','HOME':f'/tmp/ht-effect/home-{cycle}','CODEX_HOME':f'/tmp/ht-effect/home-{cycle}','CODEX_SQLITE_HOME':'/tmp/ht-effect/databases','CODEX_EXEC_SERVER_URL':'','XDG_CONFIG_HOME':f'/tmp/ht-effect/home-{cycle}/config','XDG_STATE_HOME':f'/tmp/ht-effect/home-{cycle}/state','XDG_CACHE_HOME':f'/tmp/ht-effect/home-{cycle}/cache','HT_LEAK_RUN_ID':run_id,'HT_TEST_OWNER':str(os.getpid())}
  argv=['/usr/bin/bwrap','--die-with-parent','--unshare-net','--unshare-pid','--ro-bind','/','/','--tmpfs',str(pathlib.Path.home()),'--tmpfs','/run/user','--tmpfs','/tmp','--bind',str(scratch),'/tmp/ht-effect','--ro-bind',str(exe),'/tmp/helper','--proc','/proc','--dev','/dev','--clearenv']
  for k,v in env.items():argv+=['--setenv',k,v]
  argv+=['--chdir',f'/tmp/ht-effect/home-{cycle}','/tmp/helper','-c','sqlite_home="/tmp/ht-effect/databases"','app-server','--listen','stdio://']
  child=subprocess.Popen(argv,env={'PATH':'/usr/bin:/bin','HT_LEAK_RUN_ID':run_id,'HT_TEST_OWNER':str(os.getpid())},stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,start_new_session=True);children.add(child);started=time.monotonic();buf=b'';seq=0;opened_names=set();calls=[]
  for pipe in [child.stdout,child.stdin]:os.set_blocking(pipe.fileno(),False)
  def call(method,params):
   global seq,buf
   seq+=1;deadline=min(started+1.75,time.monotonic()+1);payload=json.dumps({'jsonrpc':'2.0','id':seq,'method':method,'params':params}).encode()+b'\n';os.write(child.stdin.fileno(),payload)
   while time.monotonic()<deadline:
    while b'\n' in buf:
     line,buf=buf.split(b'\n',1);reply=json.loads(line)
     if reply.get('id')==seq:
      calls.append({'method':method,'ok':'result' in reply});opened_names.update(opened(child))
      if 'error' in reply:raise RuntimeError('protocol method rejected: '+method)
      return reply['result']
    try:
     chunk=os.read(child.stdout.fileno(),65536)
     if not chunk:raise RuntimeError('protocol EOF')
     buf+=chunk
     if len(buf)>262144:raise RuntimeError('protocol bound')
    except BlockingIOError:time.sleep(.002)
   raise RuntimeError('protocol deadline: '+method)
  try:
   call('initialize',{'clientInfo':{'name':'herdr_threads_queue','version':'1'},'capabilities':{'experimentalApi':True}})
   os.write(child.stdin.fileno(),b'{"jsonrpc":"2.0","method":"initialized"}\n')
   call('thread/queue/list',{'threadId':session,'limit':1,'cursor':None})
   corr=str(uuid.uuid4());added=call('thread/queue/add',{'threadId':session,'clientUserMessageId':corr,'input':[{'type':'text','text':'herdr-threads: wake up'}]})
   qid=added['queuedSubmission']['id'];assert added['queuedSubmission']['clientUserMessageId']==corr
   listed=call('thread/queue/list',{'threadId':session,'limit':32,'cursor':None});assert any(row['id']==qid for row in listed['data'])
   assert call('thread/queue/delete',{'threadId':session,'queuedSubmissionId':qid})['deleted']
  finally:
   try:os.killpg(child.pid,signal.SIGKILL)
   except ProcessLookupError:pass
   child.wait(timeout=2);children.remove(child);child.stdin.close();child.stdout.close()
   record['cycles'].append({'calls':calls,'opened_database_filenames':sorted(opened_names),'seconds':round(time.monotonic()-started,3)})
  record[f'after_cycle_{cycle+1}']=snap()
 record['complete']=True
except BaseException as error:
 record['complete']=False;record['error_type']=type(error).__name__;record['error_stage']=str(error) if isinstance(error,RuntimeError) else 'private experiment error (details suppressed)'
finally:
 for child in children:
  try:os.killpg(child.pid,signal.SIGKILL)
  except ProcessLookupError:pass
  child.wait(timeout=2)
 shutil.rmtree(scratch);record['private_copy_removed']=not scratch.exists();(out/'sqlite-effects-receiver.json').unlink(missing_ok=True)
 record['summary_changes']={}
 for key,after in list(record.items()):
  if not key.startswith('after_cycle_'):continue
  changes={}
  for name,db in after.items():
   before=record['before'].get(name,{})
   tables={table:{'before':before.get('tables',{}).get(table),'after':value} for table,value in db['tables'].items() if before.get('tables',{}).get(table)!=value}
   if tables or before.get('schema_sha256')!=db['schema_sha256']:changes[name]={'schema_changed':before.get('schema_sha256')!=db['schema_sha256'],'tables':tables}
  record['summary_changes'][key]=changes
 (out/'sqlite-effects-aggregate.json').write_text(json.dumps(record,indent=2)+'\n')
 print(json.dumps({'complete':record['complete'],'error_type':record.get('error_type'),'error_stage':record.get('error_stage'),'cycles':len(record['cycles']),'changed_databases':{k:list(v) for k,v in record['summary_changes'].items()},'private_copy_removed':record['private_copy_removed']}))
raise SystemExit(0 if record['complete'] else 1)
