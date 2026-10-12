#!/usr/bin/env python3
from pathlib import Path
from collections import defaultdict
import re,os,subprocess,json,hashlib
ROOT=Path('/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign')
WS=ROOT/'.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish'

import argparse
ap=argparse.ArgumentParser(description="Read-only V090 source/selection preparation; only new WS files. No Cargo/runtime/Git mutation. Final source sealing requires explicit parent-reviewed clean commit after causal fix.")
ap.add_argument("--source",type=Path,required=True)
ap.add_argument("--tree",required=True)
ap.add_argument("--label",required=True)
ap.add_argument("--reviewed-final-commit",help="Parent only after causal fix/import/review; creates source seal, never runtime PASS")
args=ap.parse_args()
if not re.fullmatch(r"[A-Za-z0-9_-]+",args.label):ap.error("simple filename-safe label required")

SOURCE=args.source.resolve()
REF=args.tree
TARGET_ROOTS={'lib':'src/lib.rs','combined':'tests/combined.rs','hook_entrypoint':'tests/hook_entrypoint.rs','integration':'tests/integration.rs','service':'tests/service.rs'}
def git(*args):return subprocess.check_output(['git',*args],cwd=SOURCE)
def gt(*args):return git(*args).decode().strip()
def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
def dig(b):return hashlib.sha256(b).hexdigest()
def lex(s):
 out=[];i=0
 while i<len(s):
  if s[i].isspace():i+=1;continue
  if s.startswith('//',i):
   j=s.find('\n',i);i=len(s) if j<0 else j;continue
  if s.startswith('/*',i):
   n=1;i+=2
   while n and i<len(s):
    if s.startswith('/*',i):n+=1;i+=2
    elif s.startswith('*/',i):n-=1;i+=2
    else:i+=1
   continue
  r=re.compile(r'(?:br|r)(#*)"').match(s,i)
  if r:
   end='"'+r.group(1);j=s.find(end,r.end());assert j>=0
   j+=len(end);out.append((s[i:j],i));i=j;continue
  r=re.compile(r'b?"(?:\\.|[^"\\])*"|b?\'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\'\\])\'|[A-Za-z_][A-Za-z_0-9]*|[0-9]+',re.S).match(s,i)
  if r:out.append((r.group(),i));i=r.end();continue
  out.append((s[i],i));i+=1
 return out
class Tree:
 def __init__(self,ref):
  self.ref=ref;self.paths=set(gt('ls-tree','-r','--name-only',ref).splitlines());self.cache={};self.found=defaultdict(list)
 def raw(self,path):
  if path not in self.cache:self.cache[path]=git('show',self.ref+':'+path)
  return self.cache[path]
 def scan(self,path,prefix,target,chain):
  assert path in self.paths,('missingmodule',path)
  raw=self.raw(path);s=raw.decode();ts=lex(s);v=[x[0] for x in ts];pair={};st=[]
  for z,t in enumerate(v):
   if t in ('{','[','('):st.append(z)
   elif t in ('}',']',')'):
    assert st and v[st[-1]]=={'}':'{',']':'[',')':'('}[t],('tokens',path,z)
    pair[st.pop()]=z
  assert not st,('unclosed',path)
  def scope(lo,hi,prefix,chain,inline):
   i=lo;attrs=[]
   while i<hi:
    if v[i]=='#' and i+1<hi and v[i+1]=='[':
     j=pair[i+1];attrs.append(s[ts[i][1]:ts[j][1]+1]);i=j+1;continue
    if v[i] in ('pub','unsafe','async') or (v[i]=='(' and i>lo and v[i-1]=='pub'):
     i=pair[i]+1 if v[i]=='(' else i+1;continue
    if v[i]=='mod' and i+2<hi:
     name=v[i+1];term=v[i+2];line=s.count('\n',0,ts[i][1])+1
     link={'host':path,'line':line,'module':name,'kind':'inline' if term=='{' else 'external','attributes':list(attrs)}
     if term=='{':
      j=pair[i+2];scope(i+3,j,prefix+[name],chain+[link],inline+[name]);i=j+1
     elif term==';':
      explicit=next((m.group(1) for a in attrs if (m:=re.search(r'path\s*=\s*"([^"]+)"',a))),None)
      if explicit:
       child=os.path.normpath(str(Path(path).parent.joinpath(*inline,explicit)))
       if child not in self.paths:child=os.path.normpath(str(Path(path).parent/explicit))
      else:
       base=Path(path).parent if Path(path).name in ('lib.rs','main.rs','mod.rs') else Path(path).with_suffix('')
       base=base.joinpath(*inline);child=str(base/(name+'.rs'))
       if child not in self.paths:child=str(base/name/'mod.rs')
      self.scan(child,prefix+[name],target,chain+[link]);i+=3
     else:i+=1
     attrs=[];continue
    if v[i]=='fn' and i+1<hi:
     name=v[i+1];j=i+2
     while j<hi and v[j] not in ('{',';'):j+=1
     end=pair.get(j,j)
     if any(re.match(r'#\[\s*(?:tokio\s*::\s*)?test(?:\s*\(|\s*\])',a) for a in attrs):
      key=(target,'::'.join(prefix+[name]))
      self.found[key].append({'target':target,'name':key[1],'source_path':path,'source_line':s.count('\n',0,ts[i][1])+1,'source_file_sha256':dig(raw),'source_function_sha256':dig(s[ts[i][1]:ts[end][1]+1].encode()),'source_tree':self.ref,'source_registered':True,'test_attributes':list(attrs),'registration_chain':chain})
     i=end+1;attrs=[];continue
    if v[i]=='{':i=pair[i]+1;attrs=[];continue
    if v[i]==';':attrs=[]
    i+=1
  scope(0,len(v),prefix,chain,[])
 def enumerate(self):
  for t,p in TARGET_ROOTS.items():self.scan(p,[],t,[])
  return self
 def one(self,key):
  matches=self.found.get(key,[])
  if len(matches)!=1:raise ValueError(f'unresolved/duplicate registered key {key}: {len(matches)}')
  return matches[0]

from collections import Counter
baseline_path=WS/'resume-composed-matrix-preparation-f330bdab.json'
baseline=json.loads(baseline_path.read_text())
original={(r['target'],r['name']):r for r in baseline['expected_tests']}
assert len(original)==391
historical657_path=WS/'resume-v090-9a573c3f-final-inventory.json'
historical657_sha256='c433673ebf52ce5d64b4352f8b9f6d512dcad60edb3a66d5034a7b22f699f3a8'
historical657_bytes=historical657_path.read_bytes()
if dig(historical657_bytes)!=historical657_sha256:raise ValueError('immutable expanded657 inventory changed')
historical657=json.loads(historical657_bytes)
expanded={(r['target'],r['name']):r for r in historical657['expected_tests']}
if len(expanded)!=657 or len(historical657['expected_tests'])!=657:raise ValueError('expanded657 keys changed')
historical657_meta={'path':str(historical657_path),'sha256':historical657_sha256,'original_keys_preserved':657}
oldfiles=[p for p in WS.iterdir() if p.name.startswith('resume-composed-') and p.is_file()]
old_hashes={str(p):sha(p) for p in oldfiles}
proposal=WS/'resume-main-v090-test-selection-proposal.md'
disposition=WS/'resume-main-v090-test-key-dispositions.md'
main_keys=set()
q=chr(96)
pattern=re.compile(r'\| [+=] \| '+q+r'([a-z_]+)'+q+r' \| '+q+r'([^'+q+r']+)'+q+r' \|')
for line in proposal.read_text().splitlines():
 m=pattern.match(line)
 if m:main_keys.add((m.group(1),m.group(2)))
assert len(main_keys)==244
rename_rows=[
 {'target':'lib','historical':'store::control::tests::registration_rejects_exhausted_binding_generation_without_anchor','canonical':'store::control::tests::registration_ignores_an_exhausted_binding_history_generation_and_rolls_back_without_anchor'},
 {'target':'combined','historical':'invitation_content::released_v39_invitation_epoch_alter_records_nested_work_and_preserves_state','canonical':'invitation_content::composed_v43_invitation_epoch_alter_records_nested_work_and_preserves_state'},
]
def norm(key):
 for r in rename_rows:
  if key==(r['target'],r['historical']):return r['target'],r['canonical']
 return key
current=Tree(REF).enumerate()
main=Tree('b6dd5dd122772599a02e2cc7f76b20e5492f0af3').enumerate()
old=Tree('f330bdab3abee8535213228e5465b085b65be750').enumerate()
for key in main_keys:main.one(key)
actual_author=set(current.found)-set(main.found)-set(old.found)
negative_commit='71bb7ea92b820321b03b9c4206cd2e4a2fb23011'
negative_key=('lib','cli::topology_handoff::tests::live::automatic_live::automatic_child_begin_expired_permit_has_no_effects_and_fresh_permit_succeeds')
negative=Tree(negative_commit).enumerate().one(negative_key)
keys={norm(k) for k in expanded}|{norm(k) for k in original}|{norm(k) for k in main_keys}|actual_author|{(r['target'],r['canonical']) for r in rename_rows}
restored=('lib','store::effective::tests::unpublished_staging_is_invisible_and_first_later_anchor_is_immutable')
if restored in current.found:keys.add(restored)
pending=[]
if negative_key in current.found:keys.add(negative_key)
else:pending.append({'target':negative_key[0],'name':negative_key[1],'status':'required_not_imported_not_selected','evidence_commit':negative_commit,'source_evidence':negative,'blocks_final_seal':True})
mandatory={norm((r['target'],r['name'])) for r in original.values() if r['selection']=='mandatory_cargo_separate'}
assert len(mandatory)==6
identity={t:('herdr-threads::'+t,t,'test') for t in TARGET_ROOTS if t!='lib'}
identity['lib']=('herdr-threads','herdr_threads','lib')
rows=[]
for key in sorted(keys):
 r=dict(current.one(key));bid,bname,kind=identity[key[0]]
 assert not any(re.search(r'#\[\s*ignore',a) for a in r['test_attributes'])
 groups=[]
 if key in {norm(k) for k in expanded}:groups.append('historical657')
 if key in {norm(k) for k in original}:groups.append('original391')
 if key in {norm(k) for k in main_keys}:groups.append('published_Main244')
 if key in actual_author:groups.append('actual_composed_author')
 if key in {(v['target'],v['canonical']) for v in rename_rows}:groups.append('reviewed_key_disposition')
 if key==restored:groups.append('restored_Main_availability_control')
 if key==negative_key:groups.append('negative_AutoPeer_control')
 r.update(binary_id=bid,binary_name=bname,target_kind=kind,selection='mandatory_cargo_separate' if key in mandatory else 'focused_nextest',coverage_groups=groups,execution=None)
 rows.append(r)
assert len(rows)==len({(r['binary_id'],r['name']) for r in rows})>=391
traces=[]
for historical,previous in sorted(original.items()):
 canonical=norm(historical);current.one(canonical)
 traces.append({'historical_target':historical[0],'historical_name':historical[1],'canonical_target':canonical[0],'canonical_name':canonical[1],'original_source_path':previous['source_path'],'original_source_file_sha256':previous['source_file_sha256'],'original_source_function_sha256':previous['source_function_sha256'],'baseline_path':str(baseline_path),'baseline_sha256':sha(baseline_path)})
historical657_fields=('source_path','source_file_sha256','source_function_sha256','source_tree','registration_chain','selection')
historical657_traces=[]
for historical,previous in sorted(expanded.items()):
 canonical=norm(historical)
 if ('mandatory_cargo_separate' if canonical in mandatory else 'focused_nextest')!=previous['selection']:raise ValueError(f'historical657 selection class changed: {historical}')
 historical657_traces.append({'historical_target':historical[0],'historical_name':historical[1],'canonical_target':canonical[0],'canonical_name':canonical[1],'historical_source':{k:previous[k] for k in historical657_fields}})
for r in rename_rows:r.update(historical_was_in_original391=(r['target'],r['historical']) in original,canonical_selected=True,reviewed_disposition_path=str(disposition),reviewed_disposition_sha256=sha(disposition))
proof_fields=('target','binary_id','binary_name','target_kind','name','source_path','source_line','source_file_sha256','source_function_sha256','source_tree','selection','registration_chain','test_attributes')
proof=[{k:r[k] for k in proof_fields} for r in sorted(rows,key=lambda r:(r['binary_id'],r['name']))]
fingerprint=dig(json.dumps(proof,sort_keys=True,separators=(',',':')).encode())
paths={r['source_path'] for r in rows}|{c['host'] for r in rows for c in r['registration_chain']}|{'Cargo.toml','.config/nextest.toml','src/store/schema.rs','src/protocol/capabilities.rs','src/protocol/wire.rs','src/client/local.rs'}
files={p:{'sha256':dig(current.raw(p)),'git_blob_id':gt('rev-parse',REF+':'+p)} for p in sorted(paths)}
head=gt('rev-parse','HEAD')
stamp={'head':head,'HEAD_tree':gt('rev-parse','HEAD^{tree}'),'index_stage_sha256':dig(git('ls-files','--stage')),'unmerged':gt('ls-files','-u'),'tracked_status':gt('status','--porcelain','--untracked-files=no')}
# Read-only comparisons verify the parent's staged snapshot without write-tree.

if gt('cat-file','-t',REF)!='tree':raise ValueError("--tree must be immutable tree")
phase="PREPARATION_ONLY";identity_kind="immutable_source_snapshot_not_execution_sealed"
source_head=head if gt('rev-parse','HEAD^{tree}')==REF else None
if args.reviewed_final_commit:
 final_commit=gt('rev-parse',args.reviewed_final_commit)
 if pending:raise ValueError("required71bb negative source is not imported; final source seal blocked")
 if head!=final_commit or gt('rev-parse',final_commit+'^{tree}')!=REF:raise ValueError("final source commit/tree not current exact HEAD")
 if gt('status','--porcelain','--untracked-files=no') or gt('ls-files','-u'):raise ValueError("final source/index must be clean")
 merge=subprocess.run(['git','rev-parse','--verify','MERGE_HEAD'],cwd=SOURCE,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
 if merge.returncode==0:raise ValueError("final source still merging")
 phase="sealed_parent_reviewed_final_source";identity_kind="committed_frozen";source_head=final_commit
stamp['source_seal_policy']='Source metadata only: no runtime verdict. Parent final review/import/causal-fix attestation required for final commit mode.'

focused=[r for r in rows if r['selection']=='focused_nextest']
inputs={str(historical657_path):historical657_sha256,str(baseline_path):sha(baseline_path),str(proposal):sha(proposal),str(disposition):sha(disposition)}
seal={'schema':'herdr-threads.v090.selection-seal.v1','phase':phase,'runtime_verdict':None,'source_tree':REF,'source_head':source_head,'selection_fingerprint':fingerprint,'original391_requirement_map':traces,'historical657':historical657_meta,'historical657_requirement_map':historical657_traces,'reviewed_key_dispositions':rename_rows,'published_Main244_keys':[{'target':t,'name':n} for t,n in sorted(main_keys)],'actual_composed_author_keys':[{'target':t,'name':n} for t,n in sorted(actual_author)],'pending_required':pending,'explicit_source_evidenced_allowlist':proof,'input_sha256':inputs,'counts':{'selected_static':len(rows),'original_requirements':391,'normalized_original_execution_keys':len({norm(k) for k in original}),'focused_static':len(focused),'mandatory_static':6,'runtime_selected':None,'runtime_executed':None},'history_policy':'Preserve391 original requirement rows individually. Runtime execution deduplicates current canonical keys. Both reviewed disposition requirements are separate from391 because the historical names are absent in the frozen391 selection. No fabricated original membership or subtraction.','final_seal_required':'Parent causal fix/import/review, clean committed final source, exact refreshed source registration/fingerprints, pending_required empty and matching source/selection seal. No runtime success implied.'}
inv_path=WS/('resume-v090-'+args.label+'-inventory.json')
seal_path=WS/('resume-v090-'+args.label+'-selection-seal.json')
inventory={'schema':'herdr-threads.v090.inventory.v1','preparation_only':phase=='PREPARATION_ONLY','phase':phase,'execution':None,'runtime_counts':None,'source':{'path':str(SOURCE),'head':source_head,'tree':REF,'identity_kind':identity_kind,'observed_staged_provenance':stamp},'expected_tests':rows,'source_files':files,'counts':{'unique_expected_tests':len(rows),'mandatory_separate':6,'focused_nextest':len(focused),'by_target':dict(Counter(r['target'] for r in rows)),'runtime_selected':None,'runtime_executed':None},'selection_fingerprint':fingerprint,'selection_seal':{'path':str(seal_path),'phase':phase},'mandatory_recovery':{'source_file_sha256':files['tests/cli/lazy_recovery_context.rs']['sha256'],'expected_tests':[r['name'] for r in rows if r['selection']=='mandatory_cargo_separate'],'projection_wait_seconds':30,'projection_poll_milliseconds':20},'parent_runner_requirements':baseline['parent_runner_requirements'],'historical657':historical657_meta,'baseline391':{'path':str(baseline_path),'sha256':sha(baseline_path),'original_keys_preserved':391},'pending_required':pending,'history_input_sha256':inputs,'source_membership_note':'Explicit frozen source-evidenced keys only. Static author discovery from immutable parents converted to concrete seal rows; the runtime runner never discovers/adopts wildcard extras.'}
for p,data in ((inv_path,inventory),(seal_path,seal)):
 assert not p.exists(),str(p)
 p.write_text(json.dumps(data,indent=2)+'\n')
for p,h in old_hashes.items():assert sha(p)==h,('old artifact modified',p)
print(json.dumps({'inventory':str(inv_path),'seal':str(seal_path),'phase':phase,'counts':inventory['counts'],'pending_required':[r['name'] for r in pending],'actual_author_keys':seal['actual_composed_author_keys'],'dispositions':rename_rows,'selection_fingerprint':fingerprint,'inventory_sha256':sha(inv_path),'seal_sha256':sha(seal_path)},indent=2))
