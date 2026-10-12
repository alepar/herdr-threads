#!/usr/bin/env python3
from pathlib import Path
from collections import defaultdict
import re,os,subprocess,json,hashlib
ROOT=Path('/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign')
WS=ROOT/'.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish'

import argparse
ap=argparse.ArgumentParser(description="Read-only V091 source/selection preparation with released Main1-36 provenance; only new WS files. No Cargo/runtime/Git mutation. Final source sealing requires explicit parent-reviewed clean commit after causal fix.")
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

# Historical b6 Main244/391/657 facts below stay immutable. This is an additive
# current-Main acceptance floor, not a source-native full matrix or feature gate.
CURRENT_MAIN_BASE = '565def869ae50e47e6da7388ea333229050eaf8d'
CURRENT_MAIN_HEAD = '20e7ca73e41a9449376169ab6875c0675da02edc'
CURRENT_MAIN_TREE = '55565e591f37fbdf73d04ac8711d18857661920c'
CURRENT_MAIN_PROOF_SHA256 = 'cbd0eeea9d9cafec5b9e7e99a0d6ec1fc2f1c5586e183e351a78a50c0a63e9c8'
CURRENT_MAIN_NEW_KEYS = (('combined', 'direct_send::json_output_uses_the_versioned_send_envelope_with_the_thread'),
 ('combined', 'lazy_attention::a_sibling_lifecycle_check_in_resets_only_its_own_in_turn_service'),
 ('combined', 'lazy_attention::in_turn_display_of_a_nudge_owes_no_idle_wake'),
 ('combined', 'lazy_attention::in_turn_display_of_an_invitation_owes_no_idle_wake'),
 ('hook_entrypoint', 'lazy_mail_reaches_the_next_codex_user_prompt_hook'),
 ('hook_entrypoint', 'pair_accept_delivers_mixed_opening_mail_once'),
 ('hook_entrypoint', 'pair_rejoin_with_two_bindings_pins_read_positions'),
 ('lib', 'cli::hook_inbox::tests::in_turn_presentation_is_sent_only_to_a_daemon_that_records_it'),
 ('lib', 'scheduler::tests::new_attention_retries_a_refusal_backed_off_seat_at_once_and_keeps_the_backoff'),
 ('lib',
  'store::control::tests::direct_threads::accepting_a_pair_invitation_keeps_the_opening_messages_unread'),
 ('lib', 'store::control::tests::direct_threads::rejoin_never_reshows_a_lazy_row_displayed_out_of_order'),
 ('lib', 'store::wake_tests::a_binding_served_one_reason_is_still_woken_for_another'),
 ('lib', 'store::wake_tests::a_recovery_hint_wake_to_a_served_binding_is_refused'),
 ('lib', 'store::wake_tests::an_in_turn_offer_after_reservation_cancels_that_bindings_wake'),
 ('lib', 'store::wake_tests::in_turn_service_is_decided_for_the_binding_the_composer_reaches'))
CURRENT_MAIN_REQUIRED_KEYS = (('combined', 'contracts::wake_candidate_exposes_pending_obligation_with_zero_projection_bits'),
 ('combined', 'direct_send::json_output_uses_the_versioned_send_envelope_with_the_thread'),
 ('combined', 'golden_surfaces::contract_id_output'),
 ('combined', 'golden_surfaces::help_texts'),
 ('combined', 'golden_surfaces::hook_stdout_per_event'),
 ('combined', 'golden_surfaces::setup_reports'),
 ('combined', 'golden_surfaces::setup_status_reports'),
 ('combined', 'lazy26_prerequisite::fresh_store_installs26_and_reopens'),
 ('combined', 'lazy26_prerequisite::historical25_store_upgrade_preserves_evidence_and_defaults_modes'),
 ('combined', 'lazy_attention::a_sibling_lifecycle_check_in_resets_only_its_own_in_turn_service'),
 ('combined', 'lazy_attention::in_turn_display_of_a_nudge_owes_no_idle_wake'),
 ('combined', 'lazy_attention::in_turn_display_of_an_invitation_owes_no_idle_wake'),
 ('combined', 'lazy_schema::lazy_schema_upgrade25_defaults_existing_messages_to_ordinary'),
 ('combined', 'setup_cli::codex_setup_installs_user_hooks_and_unsetup_restores_bytes'),
 ('combined', 'topology_handoff::schema_tests::registered28_fresh_and_genuine_historical_chains_reopen'),
 ('combined',
  'topology_handoff::schema_tests::registered28_preserves25_and26_history_and_rolls_back_late_ddl_failure'),
 ('hook_entrypoint', 'lazy_mail_reaches_the_next_codex_tool_hook'),
 ('hook_entrypoint', 'lazy_mail_reaches_the_next_codex_user_prompt_hook'),
 ('hook_entrypoint', 'pair_accept_delivers_mixed_opening_mail_once'),
 ('hook_entrypoint', 'pair_rejoin_with_two_bindings_pins_read_positions'),
 ('integration',
  'channel_activation::channel_activation_real_daemon_loads_fresh_21_and_22_and_owns_archival_lane'),
 ('integration', 'session_bindings_smoke::config8_upgrade_from_a_v0_6_0_database'),
 ('lib', 'cli::hook_inbox::tests::cli_inbox_prints_first_contact_once'),
 ('lib', 'cli::hook_inbox::tests::in_turn_presentation_is_sent_only_to_a_daemon_that_records_it'),
 ('lib', 'cli::hook_inbox::tests::inbox_sends_claim_and_advances_after_full_flush'),
 ('lib', 'cli::hook_inbox::tests::subagent_inbox_never_sends_ack_displayed'),
 ('lib', 'cli::hook_inbox::tests::truncated_page_advances_only_what_was_displayed'),
 ('lib', 'harness::adapter_seam::adapter_seam_settings_kinds_name_their_adapter_and_declare_today_s_groups'),
 ('lib', 'harness::codex_tests::declaration_reports_native_transport_and_receipt_unsupported'),
 ('lib', 'harness::codex_tests::every_declared_hook_is_installed_by_setup_and_encodes_context_only'),
 ('lib', 'harness::contract_tests::adapters_local_dispatch_keeps_legacy_contract_id_bytes'),
 ('lib', 'harness::contract_tests::contract_ids_are_pinned'),
 ('lib', 'harness::setup_tests::codex_session_plan_registers_event_per_group'),
 ('lib', 'harness::setup_tests::codex_setup_registers_event_per_group'),
 ('lib', 'protocol::capabilities::tests::bare_daemon_advertises_legacy_capabilities_without_v2_recorder'),
 ('lib', 'protocol::capabilities::tests::capability_constants_are_stable'),
 ('lib', 'protocol::capabilities::tests::every_advertised_capability_has_a_handler'),
 ('lib', 'scheduler::tests::new_attention_retries_a_refusal_backed_off_seat_at_once_and_keeps_the_backoff'),
 ('lib', 'scheduler::wake_text_tests::a_recovery_hint_always_names_inbox'),
 ('lib', 'scheduler::wake_text_tests::a_seat_holding_parked_ids_is_woken_with_the_marker'),
 ('lib', 'scheduler::wake_text_tests::native_wake_types_the_bound_harness_wake_text'),
 ('lib', 'scheduler::wake_text_tests::registry_wake_text_is_the_marker_unless_the_policy_injects'),
 ('lib', 'scheduler::wake_text_tests::the_inbox_marker_follows_the_composer_bindings_parking'),
 ('lib', 'scheduler::wake_text_tests::wake_texts_are_one_line_and_the_minimal_one_is_content_free'),
 ('lib',
  'store::connection::tests::absorption_main24_and25_to26_preserves_diagnostics_and_matches_fresh_catalog'),
 ('lib', 'store::connection::tests::direct_pair_migration_preserves_v28_and_enforces_identity'),
 ('lib', 'store::connection::tests::user_intent_schema22_fresh_and_v21_upgrade'),
 ('lib', 'store::connection::tests::v28_upgrade_migrates_bindings_to_session_keyed_v32'),
 ('lib', 'store::connection::tests::v34_to_v35_moves_attention_frontiers_to_bindings'),
 ('lib',
  'store::control::tests::direct_threads::accepting_a_pair_invitation_keeps_the_opening_messages_unread'),
 ('lib', 'store::control::tests::direct_threads::rejoin_never_reshows_a_lazy_row_displayed_out_of_order'),
 ('lib', 'store::wake_tests::a_binding_served_one_reason_is_still_woken_for_another'),
 ('lib', 'store::wake_tests::a_recovery_hint_wake_to_a_served_binding_is_refused'),
 ('lib', 'store::wake_tests::an_in_turn_offer_after_reservation_cancels_that_bindings_wake'),
 ('lib', 'store::wake_tests::in_turn_service_is_decided_for_the_binding_the_composer_reaches'))
CURRENT_MAIN_MIGRATIONS = {'migrations/0001_initial.sql': {'git_blob_id': 'ffeaade84f5ad44efd496a5619a6ceeb06645d86',
                                 'sha256': '987b8ba240af579c64f6e622f35854f2e6b59cf7d2d28b61e02a487422ce59d2'},
 'migrations/0002_service_substrate.sql': {'git_blob_id': '6831f3a20f80393acaaaa0fb82da10ffd7f1b3f1',
                                           'sha256': '9b91d697bc713397e276e3814545b1934872cc90bd066e22293d5110ac550f1e'},
 'migrations/0003_invitation_cancellations.sql': {'git_blob_id': '481d3ef3cc8e25d0bcd48afdb150f0bdee07a5ad',
                                                  'sha256': '2b58325a2dfb5ae2ca25a01e3897d7e0dd8d62bcd61d5abf3d69250a502f2fac'},
 'migrations/0004_voluntary_membership.sql': {'git_blob_id': 'c88ddbdb4d3edafe61a43835e71a7c36e0369e43',
                                              'sha256': 'ace84ea5090fb9ba579641eab59cf2664846ee011d778df1e52bc3144b82eaa3'},
 'migrations/0005_service_notifications.sql': {'git_blob_id': 'ee8f04c5b4a1c96c1c2168c9fd220f34e580a792',
                                               'sha256': '4483d608cbdc871145114825d54da25d9ffae6b337a1037413226c27a63642fa'},
 'migrations/0006_retirement_health.sql': {'git_blob_id': '8438928a18d1afa2a9371f4232966f85a71d506c',
                                           'sha256': '6137f9393ef15758558c7dec39c5899dfcad9726d5a2cd50a780482c64cd5495'},
 'migrations/0007_attention_digest.sql': {'git_blob_id': 'c4297bd74419ef30333907be40cd4b9ebdf32579',
                                          'sha256': 'de9d2ccae1d86fef0a74c0215276a338268e00efafda3c7b2ef677c7000978dc'},
 'migrations/0008_digest_pending_paths.sql': {'git_blob_id': '22c6850ce559ea78e04a44d75dc200758c37c65c',
                                              'sha256': 'f1d0385323435a429e72478458cb1dbb1f3998dd7db3017677ba53d673740101'},
 'migrations/0009_human_occupant.sql': {'git_blob_id': 'be735bca5e0c5dfb0ea596fd4809b76d04d99f85',
                                        'sha256': '43a85a3642b1df9eecb613c84243a3b4432f45a8ab7ebed8ea5cf5904bf36c79'},
 'migrations/0010_b5_trust_guards.sql': {'git_blob_id': 'ba4bba6d2fa40cf7f8d2fefb638651db0837c564',
                                         'sha256': '1ae7ac8a64e3735e8c416123dccc82d3ecb443dc5bb73d4ddeab942e29d104ee'},
 'migrations/0011_cooperative_only.sql': {'git_blob_id': 'cdc8899d3f04bad6a5dd448d09ea5b0ac5ae2c86',
                                          'sha256': '7bc928a3203e851c3ad7ee3727a6933f95322a137e50b43a4c6fced5d7950011'},
 'migrations/0012_harness_version_evidence.sql': {'git_blob_id': '2760de22cfd67c7f331e6131851cb917479f703d',
                                                  'sha256': '30653883773fc1cbf4264f546fbc5d3254213227663bb8d521e61cd8084674d4'},
 'migrations/0013_thread_summaries.sql': {'git_blob_id': '6a719f36df8e972adbe8a3fd62b1b98e0f451b91',
                                          'sha256': 'ae82fc0dcdb58e35da333f6e3768d2dca1d1fd97981e8eb07f6d47238049c889'},
 'migrations/0014_catch_up_release.sql': {'git_blob_id': 'b742830cb46770ae93cb7fe9b1be555942ca174f',
                                          'sha256': '560c1247f23c045bd7db6f3aef854f5fab0b9d431f26f1e2b3f50b9e5eaff4aa'},
 'migrations/0015_preparation_retention.sql': {'git_blob_id': 'b326ac0c40645ad3bfc9ee97156b7e3035ffa02a',
                                               'sha256': '733197e3e2ec367ca8ab978c9e5fa1f84b5fd7020273f073d2efa491e9d27c14'},
 'migrations/0016_human_receipt_waivers.sql': {'git_blob_id': 'a70c4cef4aa13b1566a24d755d3b6323e8add989',
                                               'sha256': 'e5c9337a7a02c6c06dc0b08cf711298a4b0fdb75183719c4b2b1b0a59fb27b60'},
 'migrations/0017_wake_batches.sql': {'git_blob_id': '3de5632eef923c875c5c331b19f81cb758581fb0',
                                      'sha256': 'f28763acbbc032906e9864c5e52b14fdb40083d2b847f3b871553718c73ba115'},
 'migrations/0018_warning_conditions.sql': {'git_blob_id': '0e0d1b3a19f28dcaa58eff59b2d9923f07125697',
                                            'sha256': '4f9d4ba662067632bb1cd285c16a4a15bef73af3746e57d58781f90bebcc8b4f'},
 'migrations/0019_thread_names.sql': {'git_blob_id': '59df4103564b47fe547bffb0c1995908b6b88296',
                                      'sha256': '3dd24c210d32f35f703b7712c67dfff88563c645b0f2a912f2c5412f3f56a41b'},
 'migrations/0020_recent_activity.sql': {'git_blob_id': 'c0f0568ff6f0d9f69344de749079c1750c5b5390',
                                         'sha256': 'f771782df955e347412474ca40c189855adda0891672bc6df88905d9c50952a4'},
 'migrations/0021_invitation_rejections.sql': {'git_blob_id': 'c76679e0189972cdf3abe975f0be8e1c988398c1',
                                               'sha256': 'a6213407b257ffd92136dac2da78e73a65d8e227ef5b873fe0cf6576258531fb'},
 'migrations/0022_user_message_intent.sql': {'git_blob_id': 'e0ac9bfb33f252ef68bc2fb0c179a2624714bd37',
                                             'sha256': 'cdc20ce86f3c98179cae7eeb507b3d7ccedc384332eab04d1b6cff7f2279d384'},
 'migrations/0023_channel_archival.sql': {'git_blob_id': 'd0bc1aaee3a240aeaa4b2fb6f979e2952249ee74',
                                          'sha256': 'f10b178cebe67c226da3be753ff14bf6ac4facdd0fac65b23880d4047574125b'},
 'migrations/0024_harness_contract_diagnostics.sql': {'git_blob_id': 'cd149cf9eafdf0977d1ec9e965f7ffcae79ccc13',
                                                      'sha256': 'c7d919671587677085f1b7cf2ded8c9bbc8b4bd80b5683f70d4bc9f2a6a963d1'},
 'migrations/0025_warning_notice_delivery.sql': {'git_blob_id': 'f0cdb580c151abcdf42ae15fb091c96b220cded8',
                                                 'sha256': 'be0ff728fb17015517364604d2d2fe55a5a826665fb1f34e23ecf2f854431417'},
 'migrations/0026_lazy_message_delivery.sql': {'git_blob_id': 'd95b416092f47fc73632c3b4788da09a045999b9',
                                               'sha256': 'fc0d7cbef24a613fc5b96bebb06f653ed2af10b84424128d969b601e11b9f088'},
 'migrations/0027_harness_adapters.sql': {'git_blob_id': 'b7995bedb994959b3af912c7b357183569bbfdc2',
                                          'sha256': 'b755fbed6f9c0b61e9e2f1bbe8dcd7c5cd967d06a200fcf6fde383dbc1f25a7a'},
 'migrations/0028_handoff_topology.sql': {'git_blob_id': '01852f36265dae509c1183a50a7d6639a736e76b',
                                          'sha256': 'cadc1fbfd7246deeec836b9236a0e5b69486e630306fe31d2a5c0851bfb19f22'},
 'migrations/0029_direct_threads.sql': {'git_blob_id': '42d0f534cc1f7ef52bd1fcbf4ae21101d9d0ee69',
                                        'sha256': '8eb79b580a05c907e31854efeb3b8fe91dd493dff07230a31278457c8e39b112'},
 'migrations/0030_automatic_handoff.sql': {'git_blob_id': '49ca7cf1e47ce6a50b4f0b07894d267a4327ec51',
                                           'sha256': '3b3a7f837df0f53f5f7506243f6ee3bdef1f350e94708d0666a2aa904f51f644'},
 'migrations/0031_seat_wake_on.sql': {'git_blob_id': '1daf0194717c4d90c88871c25425fbd22b8c6b6a',
                                      'sha256': '32f9c35e3ebc657c6e4227454cd276c3fceebe6043b13a60cba4fad29f3f55f2'},
 'migrations/0032_session_bindings.sql': {'git_blob_id': 'b3f71eb889bc98919c81803adef6dd93d451a170',
                                          'sha256': '8c64d36344aceefb7fbbf82487d24d5a93d33da9d20297ad4547bf48abcd1033'},
 'migrations/0033_archival_scan_scope.sql': {'git_blob_id': 'ce711f97fc57193630f55a3604586332ebbb0b64',
                                             'sha256': '8cab27994ee53b25a5e08fb75b5c8dd18a5d0ac5ea48e06029abc85974f1309a'},
 'migrations/0034_binding_delivery_state.sql': {'git_blob_id': '349491bd789c562ae5750c0f90f24f2486d5701a',
                                                'sha256': '64636bb15a05555a37f418c1041bcc84782aba881e0fa17ca4c7b78d3d7e2d3b'},
 'migrations/0035_binding_attention_frontiers.sql': {'git_blob_id': '55d7cf7894162f2648783507bb1426233011d453',
                                                     'sha256': '628f64be5f80b7813445f79f2b747dcdffd2d6a3b64d463ff5c9a9cbc6dea960'},
 'migrations/0036_in_turn_offer.sql': {'git_blob_id': '5ed1fdc2a09ab4baf318f4822b4783315222c6b5',
                                       'sha256': 'dd17b386f774d238232f2d4d945a85621eb42d6b781a185a2c389704e7662e11'}}
CURRENT_MAIN_PROOF_INPUT = "git:" + CURRENT_MAIN_HEAD + ":required-test-registration-proof"
CURRENT_MAIN_MIGRATIONS_INPUT = "git:" + CURRENT_MAIN_HEAD + ":released-migrations-1-36"
CURRENT_MAIN_MIGRATIONS_SHA256 = '610b2afeaacf08b8190a5415a045d5c65a0bfcffbedea8cbf18a044f097bd02a'

# The native-event contract seam is feature-owned, absent from released Main.
FEATURE_FLOOR_HEAD = '0b04f91ba5fbaf4f5df4819fe8ccda6a704c26a2'
FEATURE_FLOOR_TREE = '29f7d42d742fa09a56bf64f9de1648420e2565e3'
FEATURE_FLOOR_PROOF_SHA256 = 'c4452c2eb8d717644ed585db8727bad87dc2821f79c6c0b8588dc4d4ab8aa252'
FEATURE_FLOOR_KEYS = (('hook_entrypoint',
  'native_event_entrypoints_dispatch_declared_codecs_and_refuse_invalid_pairs_before_effects'),)
FEATURE_FLOOR_INPUT = "git:" + FEATURE_FLOOR_HEAD + ":native-event-contract-proof"


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
# Validate the pinned released source, then materialize a concrete additive proof.
if gt('rev-parse',CURRENT_MAIN_HEAD+'^{tree}')!=CURRENT_MAIN_TREE:raise ValueError('current Main tree changed')
current_main=Tree(CURRENT_MAIN_TREE).enumerate()
current_main_proof=[dict(current_main.one(k)) for k in CURRENT_MAIN_REQUIRED_KEYS]
if dig(json.dumps(current_main_proof,sort_keys=True,separators=(',',':')).encode())!=CURRENT_MAIN_PROOF_SHA256:raise ValueError('current Main registration proof changed')
current_main_meta={'base':CURRENT_MAIN_BASE,'head':CURRENT_MAIN_HEAD,'tree':CURRENT_MAIN_TREE,'source_proof_sha256':CURRENT_MAIN_PROOF_SHA256,'source_proof':current_main_proof,'new_keys':[{'target':t,'name':n} for t,n in CURRENT_MAIN_NEW_KEYS]}
released_migrations={'head':CURRENT_MAIN_HEAD,'tree':CURRENT_MAIN_TREE,'floor':36,'source_files':CURRENT_MAIN_MIGRATIONS}
for snapshot in (current_main,current):
 released_paths={p for p in snapshot.paths if re.fullmatch(r'migrations/00(?:0[1-9]|[12][0-9]|3[0-6])_.*\.sql',p)}
 if released_paths!=set(CURRENT_MAIN_MIGRATIONS):raise ValueError('released Main1-36 migration paths changed: '+snapshot.ref)
for path,expected in CURRENT_MAIN_MIGRATIONS.items():
 for snapshot in (current_main,current):
  if {'sha256':dig(snapshot.raw(path)),'git_blob_id':gt('rev-parse',snapshot.ref+':'+path)}!=expected:raise ValueError('released Main migration changed: '+path)
actual_author=set(current.found)-set(main.found)-set(old.found)
if gt('rev-parse',FEATURE_FLOOR_HEAD+'^{tree}')!=FEATURE_FLOOR_TREE:raise ValueError('feature floor tree changed')
feature_floor=Tree(FEATURE_FLOOR_TREE)
feature_floor.scan('tests/hook_entrypoint.rs',[],'hook_entrypoint',[])
feature_floor_proof=[dict(feature_floor.one(k)) for k in FEATURE_FLOOR_KEYS]
if dig(json.dumps(feature_floor_proof,sort_keys=True,separators=(',',':')).encode())!=FEATURE_FLOOR_PROOF_SHA256:raise ValueError('feature native-event contract source proof changed')
feature_floor_meta={'head':FEATURE_FLOOR_HEAD,'tree':FEATURE_FLOOR_TREE,'source_proof_sha256':FEATURE_FLOOR_PROOF_SHA256,'source_proof':feature_floor_proof}
negative_commit='71bb7ea92b820321b03b9c4206cd2e4a2fb23011'
negative_key=('lib','cli::topology_handoff::tests::live::automatic_live::automatic_child_begin_expired_permit_has_no_effects_and_fresh_permit_succeeds')
negative=Tree(negative_commit).enumerate().one(negative_key)
keys={norm(k) for k in expanded}|{norm(k) for k in original}|{norm(k) for k in main_keys}|actual_author|set(CURRENT_MAIN_REQUIRED_KEYS)|set(FEATURE_FLOOR_KEYS)|{(r['target'],r['canonical']) for r in rename_rows}
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
 if key in CURRENT_MAIN_REQUIRED_KEYS:groups.append('current_Main_v091')
 if key in FEATURE_FLOOR_KEYS:groups.append('feature_native_event_contract')
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
paths={r['source_path'] for r in rows}|{c['host'] for r in rows for c in r['registration_chain']}|{'Cargo.toml','.config/nextest.toml','src/store/schema.rs','src/protocol/capabilities.rs','src/protocol/wire.rs','src/client/local.rs'}|set(CURRENT_MAIN_MIGRATIONS)|{'scripts/wake-e2e.py','scripts/wake_e2e_selftest.py'}
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
# New Main and affected existing rows remain focused; historical mandatory6 is unchanged.
if any(r['selection']!='focused_nextest' for r in rows if (r['target'],r['name']) in set(CURRENT_MAIN_REQUIRED_KEYS)|set(FEATURE_FLOOR_KEYS)):raise ValueError('additive source requirement entered mandatory selection')
inputs={str(historical657_path):historical657_sha256,str(baseline_path):sha(baseline_path),str(proposal):sha(proposal),str(disposition):sha(disposition),CURRENT_MAIN_PROOF_INPUT:CURRENT_MAIN_PROOF_SHA256,CURRENT_MAIN_MIGRATIONS_INPUT:CURRENT_MAIN_MIGRATIONS_SHA256,FEATURE_FLOOR_INPUT:FEATURE_FLOOR_PROOF_SHA256}
seal={'schema':'herdr-threads.v090.selection-seal.v1','phase':phase,'runtime_verdict':None,'source_tree':REF,'source_head':source_head,'selection_fingerprint':fingerprint,'original391_requirement_map':traces,'historical657':historical657_meta,'current_Main_v091':current_main_meta,'feature_native_event_contract':feature_floor_meta,'released_Main_migrations':released_migrations,'historical657_requirement_map':historical657_traces,'reviewed_key_dispositions':rename_rows,'published_Main244_keys':[{'target':t,'name':n} for t,n in sorted(main_keys)],'actual_composed_author_keys':[{'target':t,'name':n} for t,n in sorted(actual_author)],'pending_required':pending,'explicit_source_evidenced_allowlist':proof,'input_sha256':inputs,'counts':{'selected_static':len(rows),'original_requirements':391,'normalized_original_execution_keys':len({norm(k) for k in original}),'focused_static':len(focused),'mandatory_static':6,'runtime_selected':None,'runtime_executed':None},'history_policy':'Preserve391 original requirement rows individually. Runtime execution deduplicates current canonical keys. Both reviewed disposition requirements are separate from391 because the historical names are absent in the frozen391 selection. No fabricated original membership or subtraction.','final_seal_required':'Parent causal fix/import/review, clean committed final source, exact refreshed source registration/fingerprints, pending_required empty and matching source/selection seal. No runtime success implied.'}
inv_path=WS/('resume-v091-'+args.label+'-inventory.json')
seal_path=WS/('resume-v091-'+args.label+'-selection-seal.json')
inventory={'schema':'herdr-threads.v090.inventory.v1','preparation_only':phase=='PREPARATION_ONLY','phase':phase,'execution':None,'runtime_counts':None,'source':{'path':str(SOURCE),'head':source_head,'tree':REF,'identity_kind':identity_kind,'observed_staged_provenance':stamp},'expected_tests':rows,'source_files':files,'counts':{'unique_expected_tests':len(rows),'mandatory_separate':6,'focused_nextest':len(focused),'by_target':dict(Counter(r['target'] for r in rows)),'runtime_selected':None,'runtime_executed':None},'selection_fingerprint':fingerprint,'selection_seal':{'path':str(seal_path),'phase':phase},'mandatory_recovery':{'source_file_sha256':files['tests/cli/lazy_recovery_context.rs']['sha256'],'expected_tests':[r['name'] for r in rows if r['selection']=='mandatory_cargo_separate'],'projection_wait_seconds':30,'projection_poll_milliseconds':20},'parent_runner_requirements':baseline['parent_runner_requirements'],'historical657':historical657_meta,'current_Main_v091':current_main_meta,'feature_native_event_contract':feature_floor_meta,'released_Main_migrations':released_migrations,'baseline391':{'path':str(baseline_path),'sha256':sha(baseline_path),'original_keys_preserved':391},'pending_required':pending,'history_input_sha256':inputs,'source_membership_note':'Explicit frozen source-evidenced keys only. Static author discovery from immutable parents converted to concrete seal rows; the runtime runner never discovers/adopts wildcard extras.'}
for p,data in ((inv_path,inventory),(seal_path,seal)):
 assert not p.exists(),str(p)
 p.write_text(json.dumps(data,indent=2)+'\n')
for p,h in old_hashes.items():assert sha(p)==h,('old artifact modified',p)
print(json.dumps({'inventory':str(inv_path),'seal':str(seal_path),'phase':phase,'counts':inventory['counts'],'pending_required':[r['name'] for r in pending],'actual_author_keys':seal['actual_composed_author_keys'],'dispositions':rename_rows,'selection_fingerprint':fingerprint,'inventory_sha256':sha(inv_path),'seal_sha256':sha(seal_path)},indent=2))
