# Parent-only successor verification plan

This is verification planning for the explicitly authorized Main successor, not a code review, new audit, roast round, or automatic fix loop. This author ran only source reads, Git reads and read-only Python source/manifest comparisons; no Cargo, runtime tests, Herdr, seat mutation or delegation. Only this plan is written.

## Fixed scope and evidence boundaries

The three tasks change Claude mod JavaScript/TypeScript, the wake driver and its fake selftests, and usage/changelog prose. They intend no Rust, schema, scheduling, source-registration or delivery-envelope changes. Run task-specific causal RED/GREEN controls already declared by their authors, strict actual Claude plugin validation and the entire actual module suite, and the entire exact fake wake-driver suite. Do not invent new task scopes or repeat review rounds. The final parent acceptance is the existing source-sealed selection of 827 cases: 821 focused plus six mandatory recovery cases, using the immutable old V091 helpers.

The historical 6abdf38f acceptance and dc15706e consumer result remain historical. The former seals tree `fb3f09bcc0349b05351ff308333388cc0cdd0d63`; it does not seal the successor. The latter's 124 PASS and `final-6abdf38f-current-consumer-byteproof.json` depended on identical consumer blobs. Changing `hooks/register.js` or module tests invalidates that reuse. A new actual plugin run must record its actual count, including newly added tests; do not call it 124 PASS in advance.

No authenticated/native release matrix, installation, real credentials/profile, real config, production version probe or exact-version gate is in this plan. `scripts/test-claude-mod` can exit zero after a version-based skip, so use the direct plugin commands below: unsupported or unavailable commands are missing evidence, never PASS. `python3 scripts/wake-e2e.py --self-test` returns before preflight, layout, harness launch or credential provisioning and uses `wake_e2e_selftest.py` with fake credentials and fake services. Running the driver without `--self-test` is outside this plan.

## Source-grounded constraints

Observed clean HEAD before concurrent task work: `d5a8ee66`. `git diff 7ac8c4bf6cfd88509df1336f354e8a230f65cc36 HEAD -- scripts/wake-e2e.py scripts/wake_e2e_selftest.py` was empty. Thus the preserved Main7ac human producer and marker stripping are the starting driver bytes, not an older cooperative producer.

The current CLI JSON envelope comes from `src/protocol/output.rs:249`: `{"version":7,"result":...}` (`PROTOCOL_VERSION` is in `src/protocol/wire.rs:37`). `CommandResult` uses the `kind`/`data` tagged shape. Direct send keeps `result.kind=message_sent`, `result.data=<message ID>`, and adds `result.thread` and `result.selection`; see `src/cli/direct_send.rs:454` and `tests/cli/direct_send.rs:1890`. The driver correctly reads the message ID at `result.data`, not at the top level. The independent `combined::direct_send::json_output_uses_the_versioned_send_envelope_with_the_thread` case is already in the final selection.

Main's `Run.ht` strips `HERDR_PANE_ID`, all `CALLER_SESSION_ENV` entries, and an empty `CLAUDE_CODE_PLUGIN_DIRS`, then explicitly restores the owned producer pane. Person actions place `human` immediately after the executable and before global options. `Recipient.coord` always passes `human=True`; layout uses guarded `human me init`. Preserve all of that. The marker list includes Claude, Codex, Hermes and agy markers and is checked against `src/harness` by the fake selftest. Human priority changes catch-up holding, not the explicit Lazy invitation/body wake policy.

The exact canonical oracle grounding is unchanged: `src/cli/direct_send.rs:253` supplies `EnsureCreationPolicy::Lazy` for direct sends even with a Human claim; `src/delivery/invitation.rs:108` does not wake that unrequired Lazy invitation; `src/store/lazy_delivery.rs:904` publishes Lazy body wake work only for `wake_on=all`. `src/cli/commands.rs:1873` clears explicit invitation confirmation on Human group invite, retaining the generic group's established timed behavior. Consequently direct Lazy invitation/opening body is passive under nudge and may wake under all; generic Human group invite keeps its existing expectation. Busy delivery must remain no-nudge during or after the turn. Fake tests demonstrate the driver's oracle and call shapes, not actual authenticated uptake.

The consumer framed append envelope is grounded in `src/service/delivery_pull.rs:37`: at most 32 blocks and 64 KiB framed UTF-8 text. Task 1's tests must exercise stalled/refused append, one preserved inflight object plus newest pending invitation context, partial-to-whole preference at the same version, distinct-ID bounded batches, late old-run failures and sibling live context. A body-only byte sum, UTF-16 string length, truncation, or fabricated acceptance/ACK is not the declared check. Capture the exact author-provided test names and RED/GREEN commands in the task evidence; the entire module suite below is the final common check.

The old preparer scans these exact registration roots (including inline modules and external `#[path]` registration chains): `src/lib.rs`, `tests/combined.rs`, `tests/hook_entrypoint.rs`, `tests/integration.rs`, `tests/service.rs`. Binary identities are respectively `herdr-threads` / `herdr_threads` / `lib`, and `herdr-threads::<target>` / `<target>` / `test`. `tests/combined.rs:46` registers `cli/lazy_recovery_context.rs`; line127 registers `cli/direct_send.rs`. Current helper literals contain 55 current-Main required keys, 15 additive Main keys, and one separate feature-owned native-event proof; historical prose saying 54 is not the current source count. No harness version is a selection condition.

## Parent setup and private environment

Wait for all three authors and their single light task reviews to finish; serially integrate their authorized branches, complete required compile-only merge checks, and commit the final reports before freezing acceptance. Do not run Cargo while another task/parent Cargo lane is active. Freeze the final reviewed committed source for the whole final plugin/selftest/827 epoch. If any tracked bytes change later, invalidate the corresponding evidence and source seal; do not reuse a clean earlier HEAD merely because Rust is unchanged.

All commands below are parent-only, from this worktree. These variables are task-specific; private HOME/config values are assigned only to child environments.

```sh
TASK_ROOT=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign
TASK_WS="$TASK_ROOT/.superpowers/sdd/2026-10-12-handoff-main-review-successor"
TASK_OLDWS="$TASK_ROOT/.superpowers/sdd/2026-10-11-handoff-delivery-redesign-finish"
TASK_TARGET="$TASK_ROOT/.worktrees/super-auto-handoff-delivery-redesign--task-ht-0lo.3.2/target"
cd "$TASK_ROOT"
git status --porcelain --untracked-files=no
git ls-files -u
git rev-parse --verify MERGE_HEAD
```

The first two must be empty; the last must fail because no merge is in progress. Record final HEAD/tree/index hash. Preserve the sole warmed target above (it and its built CLI existed during source planning); never substitute another target. The helper refuses a target differing from its historical baseline. This is an exact selected matrix, not an `--all-targets` full-suite claim.

Run the required merge gates with `CARGO_BUILD_JOBS=4`, that absolute target, preserved toolchain homes and private child configs. Exact gate argv:

```text
cargo fmt --all -- --check
scripts/check-harness-neutral
nice cargo clippy --locked --all-targets --all-features -- -D warnings
nice scripts/check-default-features
```

Record exit/duration/output for each. If formatting must be applied, do so before the final reviewed commit and source seal. Clippy already type-checks all targets; a separate `cargo check` is redundant. These are required compile-only gates, not new tests or evidence of release uptake.

## Actual plugin and exact fake driver commands

Use one fresh absolute evidence directory for these child stages, a fresh UUID tag, private configs and retained logs. The following parent-only inline wrapper supplies exact argv, source/bookend and byte evidence, owned process-group cleanup on SIGINT/SIGTERM, and a scoped scan even on failure. It invokes no authenticated harness. It deliberately runs plugin test and selftests even if validation fails, retaining independent evidence. Never substitute `scripts/test-claude-mod` or omit the self-test flag.

```sh
python3 - "$TASK_ROOT" "$TASK_WS" <<'PY'
import hashlib,json,os,signal,subprocess,sys,time,uuid
from pathlib import Path
root=Path(sys.argv[1]).resolve(); ws=Path(sys.argv[2]).resolve()
rid=str(uuid.uuid4()); evidence=ws/('nonrust-evidence-'+rid); evidence.mkdir()
env=dict(os.environ); env['HT_LEAK_RUN_ID']=rid; env['PYTHONDONTWRITEBYTECODE']='1'
env['TMPDIR']=str(evidence/'tmp'); Path(env['TMPDIR']).mkdir()
for key,leaf in {'HOME':'home','CLAUDE_CONFIG_DIR':'claude','CODEX_HOME':'codex',
                 'GEMINI_CONFIG_DIR':'gemini','GEMINI_CLI_HOME':'gemini-home',
                 'AISW_CONFIG_DIR':'aisw'}.items():
    env[key]=str(evidence/'isolated-config'/leaf); Path(env[key]).mkdir(parents=True)
def git(*argv): return subprocess.check_output(['git',*argv],cwd=root)
scope=['integrations/claude/mod','scripts/test-claude-mod','scripts/wake-e2e.py','scripts/wake_e2e_selftest.py']
def stamp():
    files=git('ls-files','-z','--',*scope).decode().split('\0')
    return {'head':git('rev-parse','HEAD').decode().strip(),
            'tree':git('rev-parse','HEAD^{tree}').decode().strip(),
            'index_sha256':hashlib.sha256(git('ls-files','--stage')).hexdigest(),
            'tracked_status':git('status','--porcelain','--untracked-files=no').decode(),
            'unmerged':git('ls-files','-u').decode(),
            'files':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in files if p}}
record={'run_id':rid,'source_before':stamp(),'commands':[],'child_environment':
        {k:env[k] for k in ['HOME','CLAUDE_CONFIG_DIR','CODEX_HOME','GEMINI_CONFIG_DIR',
                            'GEMINI_CLI_HOME','AISW_CONFIG_DIR','TMPDIR','HT_LEAK_RUN_ID']}}
assert not record['source_before']['tracked_status'] and not record['source_before']['unmerged']
def interrupted(signum,frame): raise KeyboardInterrupt('signal '+str(signum))
for sig in [signal.SIGINT,signal.SIGTERM]: signal.signal(sig,interrupted)
def run(label,argv):
    path=evidence/(label+'.log'); entry={'stage':label,'argv':argv,'cwd':str(root)}
    record['commands'].append(entry); started=time.monotonic(); child=None
    try:
        with path.open('wb') as output:
            child=subprocess.Popen(argv,cwd=root,env=env,stdout=output,stderr=subprocess.STDOUT,start_new_session=True)
            entry['owned_process_group']=child.pid; entry['exit']=child.wait()
    finally:
        if child is not None:
            try: os.killpg(child.pid,signal.SIGTERM)
            except ProcessLookupError: pass
            try: child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try: os.killpg(child.pid,signal.SIGKILL)
                except ProcessLookupError: pass
                child.wait()
        entry['seconds']=time.monotonic()-started
        if path.exists():
            entry['log_sha256']=hashlib.sha256(path.read_bytes()).hexdigest(); entry['log_bytes']=path.stat().st_size
    return entry['exit']
failed=0
try:
    for label,argv in [
        ('plugin-validate',['nice','claude','plugin','validate','--strict',str(root/'integrations/claude/mod')]),
        ('plugin-all-tests',['nice','claude','plugin','test',str(root/'integrations/claude/mod')]),
        ('wake-driver-fake-selftests',['python3','scripts/wake-e2e.py','--self-test'])]:
        failed |= int(run(label,argv)!=0)
except BaseException as error:
    failed=1; record['interruption_or_error']=type(error).__name__+': '+str(error)
finally:
    try: failed |= int(run('scoped-leaks',['scripts/check-no-leaked-processes','--run-id',rid])!=0)
    except BaseException as error: failed=1; record['cleanup_error']=type(error).__name__+': '+str(error)
    record['source_after']=stamp(); failed |= int(record['source_after']!=record['source_before'])
    record['exit']=failed; (evidence/'execution.json').write_text(json.dumps(record,indent=2)+'\n')
print(json.dumps({'exit':failed,'evidence':str(evidence),'run_id':rid}))
sys.exit(failed)
PY
```

Inspect all logs, not just exit zero. Plugin validation must actually execute strict validation successfully; module logs must account for every test and contain no failures/skips. Driver log must include every current fake unittest plus its final `OK` and actual `Ran N tests` count. Record task controls and test names separately: the suite's existing person-producer/marker/guard controls remain present, and new direct Lazy invitation nudge/all/group/busy controls must execute. Do not fix `N` before the task is complete. Preserve source/file manifests plus actual command exits/durations/log hashes. A wrapper failure or incomplete/unknown output is missing evidence.

If a scoped scan reports owned survivors, use only its exact UUID to stop them, then rerun the read-only scan:

```sh
scripts/check-no-leaked-processes --run-id EXACT_RECORDED_UUID --kill
scripts/check-no-leaked-processes --run-id EXACT_RECORDED_UUID
```

Replace the placeholder with that run's recorded UUID. Do not run an unscoped `--kill`, kill a shared server, close unowned panes, or classify another run's live processes. If process visibility is denied by the sandbox, the parent requests approval for the exact scanner command with its UUID; denied/unknown process visibility is not clean evidence. No `ht` call is needed for these fake or plugin stages.

## Refresh the immutable original selection and seal final quiet source

Planning independently compared every `CURRENT_MAIN_MIGRATIONS` entry with observed HEAD: 36 paths, zero SHA/blob differences. Main7ac's extra driver files do not invalidate the prefix check: the preparer compares the exact released 1–36 path set and each blob/SHA in both pinned Main20e7 and the requested candidate tree; later prospective migration files are allowed. It adds both `scripts/wake-e2e.py` and `scripts/wake_e2e_selftest.py` to `source_files`, so their new bytes are refreshed in a new seal. Their observed Main7ac SHA256s already differ from the old final6ab inventory; passing that inventory to the runner would fail honestly before execution.

Treat these existing helper bytes as immutable for this plan:

| Helper | SHA256 at planning |
|---|---|
| OLDWS/resume-v091-prepare.py | `db7479281f80f59fa23ea53c1ce019b49bf6c9b1c9f3ae092071308d3752bb11` |
| OLDWS/resume-v091-run.py | `97f1dae368a7fe2f879855670682c058c57691e4b25580e92164af005e4581bd` |

The earlier helper review's hashes are historical and differ from these current files; do not misquote them as today's identity. Neither helper is modified by this plan. Verify the two recorded hashes before the final commands and after the run. The immutable f330391 and expanded657 inventories and original requirements/dispositions remain untouched.

Known assumptions that can block a final seal: exact current reviewed HEAD must match the requested immutable tree; tracked index/source must be clean, without MERGE_HEAD; required imported71bb expired-permit control must be present (it was in last accepted source); all old/current keys must still uniquely resolve, be registered and nonignored; released Main1–36 blobs must match; historical input hashes must match; and each output label must be fresh. Helpers require Python 3.11+ (`tomllib`, `Path.is_relative_to`), the sole existing target and built CLI. No intrinsic blocker from the stated non-Rust successor was found. If a condition fails, report the concrete condition under the bounded successor; do not edit helpers or start a new fix loop.

The preparer has a hardcoded ROOT/OLDWS and writes only **new** files into OLDWS, even when execution evidence goes to WS. Run it in place rather than copy or patch it. Fresh additive artifacts there do not overwrite the historical epoch. Use a unique simple label and preserve the printed paths. Parent source-seal mode is a final reviewed-source attestation, not a runtime verdict.

```sh
TASK_FINAL_COMMIT=$(git rev-parse HEAD)
TASK_FINAL_TREE=$(git rev-parse 'HEAD^{tree}')
TASK_LABEL="main-successor-$(python3 -c 'import uuid; print(uuid.uuid4().hex)')"
python3 "$TASK_OLDWS/resume-v091-prepare.py" \
  --source "$TASK_ROOT" --tree "$TASK_FINAL_TREE" --label "$TASK_LABEL" \
  --reviewed-final-commit "$TASK_FINAL_COMMIT" \
  > "$TASK_WS/$TASK_LABEL-prepare.log"
TASK_INVENTORY="$TASK_OLDWS/resume-v091-$TASK_LABEL-inventory.json"
TASK_SEAL="$TASK_OLDWS/resume-v091-$TASK_LABEL-selection-seal.json"
```

Before any Cargo, inspect new static counts and compare exact selected identities/classes with the accepted epoch. This read-only comparison must print only `827 821 6`; it does not approve runtime:

```sh
python3 - "$TASK_INVENTORY" "$TASK_SEAL" "$TASK_OLDWS/resume-v091-final-6abdf38f-inventory.json" <<'PY'
import json,sys
from pathlib import Path
new,seal,old=[json.loads(Path(p).read_text()) for p in sys.argv[1:]]
key=lambda r:(r['binary_id'],r['name'],r['selection'])
assert {key(r) for r in new['expected_tests']}=={key(r) for r in old['expected_tests']}
assert new['counts']['unique_expected_tests']==827
assert new['counts']['focused_nextest']==821 and new['counts']['mandatory_separate']==6
assert new['phase']==seal['phase']=='sealed_parent_reviewed_final_source'
assert not new['pending_required'] and not seal['pending_required']
assert new['source']['head']==seal['source_head'] and new['source']['tree']==seal['source_tree']
print(827,821,6)
PY
```

Then run the original runner once with a fresh absolute evidence directory, in a quiet parent epoch with no authors/reviewers changing source or other Cargo lane using its target:

```sh
TASK_MATRIX_EVIDENCE="$TASK_WS/$TASK_LABEL-827-evidence"
python3 "$TASK_OLDWS/resume-v091-run.py" \
  --source "$TASK_ROOT" --inventory "$TASK_INVENTORY" \
  --selection-seal "$TASK_SEAL" --evidence-dir "$TASK_MATRIX_EVIDENCE" --run \
  > "$TASK_WS/$TASK_LABEL-run.log" 2>&1
```

Do not first do a redundant enumerate-only Cargo pass: `--run` itself performs exact enumeration before execution. The runner creates its own fresh `HT_LEAK_RUN_ID`, private HOME/Claude/Codex/Gemini dirs, preserves CARGO_HOME/RUSTUP_HOME, uses build jobs4 and the sole warmed target, and records every child argv. It captures watch activation at the evidence path; catchable interruption terminates only its owned process group and attempts scoped scans. Its logs are bounded and overflowing output is explicitly failed, never incomplete PASS.

Exact runtime stage argv supplied by the runner:

```text
nice cargo nextest list --locked --all-features --lib --test combined --test hook_entrypoint --test integration --test service --message-format json -E <827 exact predicates>
nice cargo nextest run --locked --all-features --lib --test combined --test hook_entrypoint --test integration --test service --status-level pass --final-status-level none --no-fail-fast -E <821 exact predicates>
nice cargo test --locked --all-features --test combined <one recovery name below> -- --exact
```

The final command is six **separate** calls, in this order:

1. `lazy_recovery_context::lazy_recovery_agent_mixed_submission_pins_ambient_journal`
2. `lazy_recovery_context::lazy_recovery_agent_mixed_submission_pins_journal`
3. `lazy_recovery_context::lazy_recovery_human_submission_pins_ambient_journal`
4. `lazy_recovery_context::lazy_recovery_human_submission_pins_journal`
5. `lazy_recovery_context::lazy_recovery_prior_durable_ref_on_second_record_failure_pins_ambient_journal`
6. `lazy_recovery_context::lazy_recovery_prior_durable_ref_on_second_record_failure_pins_journal`

Keep recovery source's 30s projection wait/20ms poll, nextest groups and both automatic-envelope ALLSLOTS positives untouched. A substring filter, bulk recovery run, test skip or reduced count does not meet acceptance.

## Required final evidence and remaining limits

For the 827 run, require `execution.json.status=passed`, exit0, exact selection matched827, complete 821 executed/PASS focused identities with no missing/extra/duplicate statuses, complete six executed/ok recovery identities with no ignored/error rows, and all focused/mandatory/final scoped scans exit0. Require stable committed HEAD/tree/index, inventory/seal/helper/source hashes, and executable/test-binary hashes across all bookends. Retain `nextest-list.json`, `run-selection.json`, `selection-expressions.json`, focused/mandatory result JSON, six exact recovery logs, all command and leak logs, durations, UUID, and watch capture provenance if produced. Do not infer execution from the aggregate nextest total or process exit alone.

Retain new plugin strict-validation/all-module logs and actual counts, full fake-driver logs and actual unittest count, their source manifests and scoped cleanup, task causal RED/GREEN evidence, required merge gate logs, and the final source/selection artifacts alongside the original unchanged historical evidence. Old Rust selected identities can be preserved in the refreshed 827 selection, but old consumer PASS cannot be substituted for the changed JavaScript. The Rust runner fingerprints driver scripts yet does not execute their selftests or seal/execute Claude mod files, so all three evidence components must refer to the same final frozen source epoch.

Known byte-proof limits: manifests only cover tracked files in their stated scopes; clean tracked status does not prove no untracked local artifact exists. Git blob equality proves source equality, not current harness uptake, authentication, plugin loading in a real session, or successful native wake. Fake selftests and the selected827 matrix do not constitute release qualification or a full suite. Current pending controls' new exact test names/counts are only known after the authors finish; record actual runner output then. If final verification fails, preserve the failed evidence and surface the bounded successor gap; this plan authorizes no extra audit, round, production change or automatic retry/fix loop.
