## Task 15: AGY delivery setup and launch semantic composition

filesTouched: ["src/harness/adapter.rs", "src/harness/setup.rs", "src/harness/launch.rs", "src/harness/agy/launch.rs", "src/harness/agy/setup.rs", "src/harness/agy/plugin.rs", "src/harness/agy/delivery.rs", "src/harness/agy/mod.rs", "src/harness/claude/setup.rs", "src/harness/codex/setup.rs", "src/cli/hook.rs", "src/cli/hook_evidence.rs", "src/cli/setup.rs", "src/cli/setup/foreground.rs", "src/cli/launch.rs", "src/delivery/assemble.rs", "src/delivery/ledger.rs", "src/delivery/page.rs", "src/delivery/routing.rs", "src/delivery/settle.rs", "src/service/delivery_pull.rs", "src/store/attention.rs", "src/store/queries.rs", "src/store/seats.rs", "src/store/wake.rs", "tests/harness/adapter_seam.rs", "tests/harness/agy_launch.rs", "tests/harness/agy_setup.rs", "tests/harness/agy_delivery.rs", "tests/harness/agy_wake.rs", "tests/harness/setup.rs", "tests/harness/launch.rs", "tests/cli/hook_neutral.rs", "tests/cli/launch.rs", "tests/hook_entrypoint/agy.rs", "tests/service/delivery_pull.rs", "tests/delivery/extraction.rs", "tests/delivery/ledger.rs", "tests/store/attention.rs", "tests/store/mod_seat_view.rs", "tests/scheduler/hook_pull_routing.rs", "tests/integration/agy_sweep.rs", "tests/handoff_agy.rs", "README.md", "docs/agent-usage.md", "docs/dev/outside-adapter-report.md", "integrations/skill/SKILL.md", "TRUST-POLICY.md"]

bead: ht-0lo.8.4

Dependencies: ht-0lo.8.1

### Proposed full bead body (coordinator persistence required)

```json
{
  "id": "ht-0lo.8.4",
  "title": "AGY delivery setup and launch semantic composition",
  "description": "Own bounded AGY neutral hook-event/setup/launch and new offered-ledger/pull warning semantics after actual reviewed Task12 ab2291c5 composition. Consume stable A registry/adapter/delivery interfaces; do not depend on B policy work. Sole shared launch-readiness owner (src/cli/launch.rs,src/harness/launch.rs), preserving B generic automatic producer interfaces. Exact stableevent owned bridges, AGY rewrite refusal, diagnostic hook/plugin/profile readiness versus structural/currentownedfingerprint/trust guards; independent canonicalorigin warning visibility through pull/ledger and passiveclear. Preserve NULL/service attribution and finite due-driver, no new authority/evidence. Read normative AGY refinement and actual AGENTS; scope extras require concrete admission.",
  "acceptance_criteria": "Actual neutral hook-event command/dispatch/setup bridges preserve exact canonical3field routing and pinnedfallback, stable per-event commands on equivalent reinstall, exact manifest/group ownership, foreign/drifted/legacy groups and interrupted explicit migration. No harness branches in neutralcore or versiongating. Actual AGY launch tests distinguish advisory missing/stale/disabled/unknownplugin/profile/hook state from structural/argv/native rewrite refusals and actuallyusedconfigurationfingerprint recheck; unsupportedownedargv remainsrefused, no fabricatedfingerprint/trustapproval/nativeaccept/ACK. Real service DeliveryPull path exercises hook_pull_view\u2192mod_seat_view/fingerprint\u2192attention_version\u2192ledgerUnchanged/whole_read/reoffer/window\u2192pagedsource/assembler\u2192settlement/confirm for EACH canonical original sender/inviter incl departed and settlednoticefrontier; no unrelatedpending masks earlygates, no helper-onlyproof. Shared A1/A2/B1 canonicalphysical/publishedsparse warning staysoneevent, partialACK retainsstillopencondition, finalsettlement clears passively with no warningonlywake, exactassociations/provenance unchanged. Dynamic addedorigin after previouslyoffered event becomesvisible without fakeevent; NULL attribution/serviceauthors remainhonest, no inferredseat. Lazyinvitation discoverablewithoutattention; RequireAck/historicalNone timed; complete/truncated offer/confirm and ACKboundary truthful, boundedpage/ledger/reoffer retained, lazy idlewake onlywake_on:all. Due-driver finite SQL highwaters,total100/shared50repair/expandedH, combinedatomicreset/NULL semantics unchanged; delivery/native keys nevergate dueprogress. Focused existing AGY/setup/launch/delivery/integration tests, fmt/alltargetallfeatureclippy and scripts/check-harness-neutral pass; independent fresh semantic review audits complete upstreamcleanmerge/interface and actualearlygate chain. Everynewcommit Outside-adapter trailer truthful; isolatedHOME no ~/.gemini orotherrealconfigs; taggedownedchildren stopped, scopedleakcheck aftersweep. Parent finalroast/exacttipdefaultfeature later; no readiness claim frompriorrelease evidence. Lazy invitation suppresses only confirmation/nativewake, neverinturnpage/context/bookkeeping; actualAssembler+turnhook/servicepull positiveLazyvisible and ordinarynativewakeabsent tested, whole(item) remainsdisplaycompleteness. Properinformationalclassification preservesR9/R17 and explicitwake_onpolicy.",
  "dependencies": [
    "ht-0lo.8.1"
  ],
  "parent": "ht-0lo.8",
  "status": "proposed; coordinator persists bead"
}
```

### Acceptance and actual sibling interface preflight

Actual neutral hook-event command/dispatch/setup bridges preserve exact canonical3field routing and pinnedfallback, stable per-event commands on equivalent reinstall, exact manifest/group ownership, foreign/drifted/legacy groups and interrupted explicit migration. No harness branches in neutralcore or versiongating. Actual AGY launch tests distinguish advisory missing/stale/disabled/unknownplugin/profile/hook state from structural/argv/native rewrite refusals and actuallyusedconfigurationfingerprint recheck; unsupportedownedargv remainsrefused, no fabricatedfingerprint/trustapproval/nativeaccept/ACK. Real service DeliveryPull path exercises hook_pull_view→mod_seat_view/fingerprint→attention_version→ledgerUnchanged/whole_read/reoffer/window→pagedsource/assembler→settlement/confirm for EACH canonical original sender/inviter incl departed and settlednoticefrontier; no unrelatedpending masks earlygates, no helper-onlyproof. Shared A1/A2/B1 canonicalphysical/publishedsparse warning staysoneevent, partialACK retainsstillopencondition, finalsettlement clears passively with no warningonlywake, exactassociations/provenance unchanged. Dynamic addedorigin after previouslyoffered event becomesvisible without fakeevent; NULL attribution/serviceauthors remainhonest, no inferredseat. Lazyinvitation discoverablewithoutattention; RequireAck/historicalNone timed; complete/truncated offer/confirm and ACKboundary truthful, boundedpage/ledger/reoffer retained, lazy idlewake onlywake_on:all. Due-driver finite SQL highwaters,total100/shared50repair/expandedH, combinedatomicreset/NULL semantics unchanged; delivery/native keys nevergate dueprogress. Focused existing AGY/setup/launch/delivery/integration tests, fmt/alltargetallfeatureclippy and scripts/check-harness-neutral pass; independent fresh semantic review audits complete upstreamcleanmerge/interface and actualearlygate chain. Everynewcommit Outside-adapter trailer truthful; isolatedHOME no ~/.gemini orotherrealconfigs; taggedownedchildren stopped, scopedleakcheck aftersweep. Parent finalroast/exacttipdefaultfeature later; no readiness claim frompriorrelease evidence.

## Parent-authorized AGY composition refinement — latest provisional migration sequence

Mainab2291c5 v0.8.0 supersedes the earlier upstream478ec4a9 composition target. Released migrations0029_direct_threads.sql,0030_automatic_handoff.sql and0031_seat_wake_on.sql remain byte-for-byte exact. Our unlanded redesign29..35 shift to32..38, and automatic capacity migration becomes0039_automatic_handoff_body_bounds.sql. Prior31..37/capacity38 proposals are planning history only. Execute genuine populated released main31→combined latest upgrades/rollback/reopen; never relabel initialized latest or rewrite historicalSQL. A imports actualab2291c5 only after its frozen build drains; an owned-branch478 mechanical checkpoint is permitted ancestry, never ungated root promotion.

Additional bounded Task15 ht-0lo.8.4 owns AGY neutral hook-event/setup/launch and delivery-ledger/pull semantic composition after Task12. Task13 remains dependent only on Task12. Task14 consumes BOTH Task13 and Task15 actual reviewed artifacts; no invented B↔D dependency. Work stays in isolated .8 candidate branches/provenance; one owned warm validation slot has one writer/Cargo command at a time. Whole A/B/C/D contract must clear before root integration/closure. Original11 ordinals and prior edge-audit budget unchanged.

Preserve parent-authorized optional frozen Ensure creationpolicy Lazy/RequireAck+duration absenthistoricaltimed, modern directsendLazy initialinvite and automatichandoff independentACKbody/invite deadlines; no superseded automaticflag refusal. Task13 also owns the approved structural response-only transport seam: request-known AutomaticBootstrap plus matching result family can use a measured narrow response allowance; default request/service/watch/unrelatedreply limits remain1MiB. Numeric ceiling requires actual serializer evidence, no clipping/dedup/history rewrite.

AGY hooks/setup/launch compose via neutral registry/adapter traits, not harness-name branches in neutral core. Stable per-event owned hook command bridges remain exact-route, equivalent reinstall unchanged, foreign/drifted groups survive and explicit migration recoverable. AGY rejects rewritten/ownednativeargv: preserve structural/native grammar/host and usertrust refusal, while hook/setup/plugin/profile readiness is diagnostic-only for an otherwise safe launch. Recheck actually-used owned fullconfig fingerprint only when used; absence never fabricates one. No auto trust/accept/ACK/adoption or version admission gates.

New in-turn delivery must expose independent still-open warnings for EACH canonical original receipt sender/inviter, including departed origins, after informational frontier settlement and offered-ledger/Unchanged/whole-read suppression. Preserve one coalesced condition/event, exact associations, partial/final lifetime, passive clear, canonical nullable attribution and service-author handling; never synthesize sender from NULL/currentmembership. Lazy invitation is discoverable but not timed/native attention, RequireAck/historicalNone remaintimed. Preserve finite due-driver highwaters/budget/fairness/expandedH combinedreset; delivery visibility or native completion cannot gate discovery. Message ACK remains receipt only.

Read ACTUAL upstream AGENTS.md. Every NEW commit includes Outside-adapter: <none | comma-separated repo-relative paths/globs> accounting for all changed paths outside src/harness/agy/; diff remains authoritative. Run scripts/check-harness-neutral in addition to fmt/clippy, no new permissive allowlist entries without actual frozen-wire/registration rationale. No real ~/.claude/~/.codex/~/.aisw/~/.gemini writes. Use isolatedHOME/config, taggedchildren/private sessions and scoped leak cleanup. Lazy messages never idlewake unless wake_on:all opted-in; ACKmessages nudgeclass, generating/tool-running servedinturn notnudged; wakeprompt contentfree. Never gate on exact harnessversion, degrade only observedbroken capability. Parent owns final roast then exact-tip expensive/defaultfeature readiness.

### Global constraints (actual upstream ab2291c5 AGENTS.md)

# Agent guidelines for herdr-threads

## Development rules

- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings` and `scripts/check-harness-neutral` (harness names stay out of the neutral core; allowlist `scripts/harness-neutral-allowlist.txt`; clippy already type-checks every target; a separate `cargo check` is redundant). Before a merge, also `nice scripts/check-default-features` (a default-feature `cargo check --all-targets` that fails on any warning; CI runs the full default-feature clippy job as well); format: `cargo fmt`.
- Commit trailer `Outside-adapter: <none | list>` (R13 accounting for the agy adapter epic): every commit names, in one trailer line placed with the other trailers, each changed path outside `src/harness/agy/` as a comma-separated list of repo-relative paths or globs, or `none`. The diff stays authoritative; the trailer only annotates it.
- Tests: run the tests relevant to your change (`nice cargo test --locked --all-features <filter>`); the full suite is `nice cargo nextest run --locked --all-targets --all-features` (cargo-nextest: one process per test, every binary in one pool; test groups in `.config/nextest.toml` keep the budget- and latency-sensitive chains one at a time; `nice scripts/full-suite-gate N` runs it N times in a row with leak checks, and `nice scripts/flake-hunt` hunts flaky tests). A test that must not overlap others of its kind gets a nextest test group, not only an in-binary mutex (a mutex does not span processes).
- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`, `~/.gemini`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- After a full-suite run (and after each run of the 10-run gate or the integration sweep), run `scripts/check-no-leaked-processes`. Export `HT_LEAK_RUN_ID=<uuid>` before the suite and pass `--run-id "$HT_LEAK_RUN_ID"` to it, so another worktree's live run is never flagged. A non-zero exit lists each leaked pid; stop them (or rerun with `--kill`) and fix the test that leaked them.
- Test targets are listed explicitly (`autotests = false`). A new suite that keeps no process-wide state goes into `tests/combined.rs` as a `#[path]` module (one binary, one link); a suite that redirects stdio, installs a panic hook, runs an in-process daemon, arms failpoints or changes the umask gets its own `[[test]]` entry. A target that uses `herdr_threads::test_support` (including `test_support::spawn`) needs `required-features = ["test-support"]`; otherwise `scripts/check-default-features` and the default-feature clippy job fail.
- The daemon's owner watch exists only in `test-support` builds: always test with `--all-features`.

## Trust policy

[TRUST-POLICY.md](TRUST-POLICY.md) is normative for seat continuity, caller attribution, receipt provenance
and operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
`src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
`src/cli/hook.rs`, launch or wake.

- Do not add adversarial caller verification; the model is cooperative and same-user.
- Record claims honestly: a new way to attribute an action needs a provenance value defined in the policy.
- Never merge seats, never move a seat or end a binding on heuristic evidence.
- Decide in the daemon against the canonical view (A2); client-local files are hints, never authority.
- A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit.

## Harness-neutral core

- Harness-specific code lives only in `src/harness/<id>/` behind `HarnessAdapter` and its policy traits. Core code looks the adapter up through the registry by the id from the binding or claim.
- New harness behaviour means a new trait method or policy with a neutral default, never a branch in core.
- Core is `src/cli`, `src/client`, `src/service`, `src/daemon`, `src/store`, `src/host`, `src/protocol`, `src/app` and the neutral `src/harness/*.rs` files. Adapter-owned paths are `src/harness/<id>.rs`, `src/harness/<id>/**` and `src/harness/<id>_*.rs` for the ids in `builtins()`.
- `scripts/check-harness-neutral` enforces this, with `scripts/harness-neutral-allowlist.txt` as the exception list. Permanent entries are only frozen wire shapes, historical migrations and registration. See "Development rules" for how to run the checks.

## Harness and Herdr versions

Herdr and the harnesses (Claude Code, Codex, Hermes, agy) release too often to track, and pinning them would force updates on plugin users. So:

- Assume backward compatibility: what was designed and captured on an older version is assumed to work on newer ones until breakage is detected at runtime.
- On detected breakage, degrade only the broken capability, keyed on what was observed (for example agy's `systemMessage` → `userMessage` downgrade, keyed on the executable identity); everything else keeps working.
- Never gate admission, setup, launch or a feature on an exact version, never ship an exact-version recipe as a gate, and never make a version probe an operational prerequisite.
- Record version facts only as historical evidence and diagnostics.

Worked examples: `docs/superpowers/specs/2026-10-05-harness-versionless-design.md` and the agy design (`docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter-design.md`, "Version handling" and "Downgrade").

## Delivery contract

The goal is token efficiency. Sessions that are done and idle stay idle, spinning sessions get efficient delivery, and nudges are rare, because excessive nudges disorient some models. Every delivery policy must keep these invariants:

1. Messages are lazy by default.
2. Lazy messages never wake an idle session (unless the seat opted into `wake_on: all`).
3. Turn hooks and channels deliver every pending item, lazy or not, directly into the model's context, avoiding extra `herdr-threads` calls as far as the harness allows.
4. A nudge wakes a session only when it is idle. A session that is generating, or running a tool, gets no nudge; it is served by in-turn delivery.
5. Messages that require an ACK are always nudge-class.
6. Who performs an idle wake is the only per-harness variation:
   - `Native`: Herdr types the wake marker into an empty composer;
   - `Channel`: the harness wakes itself, as Claude's mod submits a turn.

Every new delivery policy (`DeliveryPolicy`) and every change to wake, poke or check-in delivery is checked against these rules; the 2026-10 audit is in `docs/superpowers/runs/2026-10-09-agy-harness-adapter/delivery-contract-audit.md`.

## Token-efficient communication

Applies to anyone designing or changing comms protocols or any model-visible text (deliveries, hook context, wake prompts, check-ins, agent-facing CLI output): every byte a model reads costs every recipient context and attention.

- (a) Static explanations live once per session in the skill, never repeated per message or per turn.
- (b) Per-message framing is the minimum structure needed to parse and attribute (header line + body), no boilerplate.
- (c) Wake prompts carry no content; the turn-start hook or channel delivers it.
- (d) Prefer ids and short markers over prose.
- (e) Peer messages are cooperative input from local agents (collaborate by default), not adversarial; keep only the invariant that they never override the user's instructions or permissions.
- (f) A new model-visible string states its expected size in the PR/commit and is checked against these rules.

Worked example: this run's R17 (compact cooperative framing) and R18 (content-free wake prompts), `docs/superpowers/runs/2026-10-09-agy-harness-adapter/`.

What we do today:

- Lazy delivery by default; lazy messages never wake an idle session (Delivery contract rules 1-2).
- Nudges only for idle sessions; a generating or tool-running session is served in-turn, never nudged (rule 4).
- Bodies are delivered in-turn instead of `herdr-threads inbox` round trips (rule 3): `integrations/claude/mod/hooks/register.js` (tool.call context), `src/harness/agy/delivery.rs` (PreInvocation systemMessage).
- Wake prompts are content-free (`herdr-threads: wake up`; R18): `src/notification/policy.rs` (`WAKE_UP`), `DeliveryPolicy::wake_text` in `src/harness/adapter.rs`, the mod's idle path in `register.js`.
- Per-message framing is one header line (`[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:`) plus an indented body, no static explanation per delivery (R17): `src/delivery/frame.rs`, `register.js` `frame()`.
- The skill explains framing, markers and the trust stance once per session; the full skill loads on demand, not in every check-in: `integrations/skill/SKILL.md` (Trust model), `src/harness/agy/plugin.rs` (agy plugin `skills/`).
- Long bodies are truncated at 8 KiB with a one-line `body <id>` continuation; pages are bounded (32 items / 64 KiB): `src/protocol/watch.rs` (`WATCH_BODY_LIMIT_BYTES`), `src/delivery/page.rs`, `src/service/delivery_pull.rs` (`PAGE_MAX_BLOCKS`, `PAGE_MAX_TEXT_BYTES`).
- Re-offers are bounded (60 s spacing, 3 per generation) and an `Unchanged` attention-version check makes the no-news hook path one cheap round trip with no injection: `src/delivery/ledger.rs` (`REOFFER_AFTER_MS`, `MAX_OFFERS_PER_GENERATION`), `src/protocol/delivery.rs` (`DeliveryPullReply::Unchanged`).
- Hook output on quiet turns is empty: `src/cli/hook.rs`, `src/harness/agy/delivery.rs`.
- Inbox ACKs on display, so reading needs no separate ACK call: `integrations/skill/SKILL.md` ("What an ACK means").
- Agent-facing CLI output is compact: one row per item, ids not prose, `next:` continuation lines instead of dumps: `src/protocol/output_compact.rs`.

## Speed budgets

- The full test suite must stay under **5 minutes** wall-clock (`cargo nextest run --locked --all-targets --all-features`). Treat a regression past it as a bug: fix slow or serialized tests rather than raising the budget.
- An incremental build must stay under **1 minute** (edit one source file, then `cargo build` / `cargo test --no-run`), and so must the per-change lint (`cargo clippy --all-targets --all-features -- -D warnings`).
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.

### Actual fixture and review preflight

src/service/delivery_pull.rs::hook_pull_view validates top-level instance/livecontinuity/hold/cooperativebinding/HookPull/native_session before delivery_pull reads mod fingerprint, attention_version and offered-ledger Unchanged/whole_read. src/delivery/assemble.rs::Assembler::push currently classifies every invitation Attention; consume actual merged optionalconfirmation and preserve Lazy display completeness. Trace each upstream exclusion using actual service pull, not manuallyassembled pending sets. src/harness/agy/launch.rs::LaunchPolicy prepare_launch currently vetoes missing/stale/disabled/unknownplugin and requiresConfiguredHook; compose_argv rejects ownedargs. Preserve rewrite refusal while making readiness advisory via neutral traits. src/harness/agy/plugin.rs::{install,remove,status} owns adapterlocal plugin bridges; preserve exact-ownedevent migration across neutral cli::hook/setup contracts. tests/service/delivery_pull.rs, tests/harness/{agy_delivery,agy_launch,agy_setup,adapter_seam}.rs, tests/integration/agy_sweep.rs and tests/handoff_agy.rs are existing concrete fixture hosts; no newtarget needed bydefault. Confirm actual target registration/callerbinding/generation/completeACKpublication prerequisites and nonzero selectedtests before firstRED. Wakecompletebool is restoration, not genericsuccess. Copy actual Ainterfaces afterreview, no B-specifictypesassumed. B remains owner Ensurepolicy/automaticpayload/capability/wire; Downs underlying neutral launch/readiness+pullvisibility. C waitsboth. Serial warmvalidation only, never source/Cargo concurrently.


### Lazy invitation positive delivery acceptance clarification

Lazy invitation confirmation/nativewake suppression MUST NOT suppress in-turn pages, turn-hook/channel context or bookkeeping/discoverability (R9/R17). Preserve full display/completeness: whole(item) is not actionability. Give Lazy invitation proper informational classification while carrying it through actual delivery::Assembler and public turn-hook/service DeliveryPull output. Add positive actual Lazy invitation page/context/bookkeeping assertions AND nativewake/confirmationtimer absence under ordinary wake_on policy, with explicit wake_on policy behavior retained. RequireAck and historical None retain their timed attention. Preserve independent departed canonical sender/inviter warning visibility through ledgerUnchanged/whole_read/noticefrontier and final passiveclear; never remove rows just to suppress attention.

### Actual fixture preflight

Read task-15-preflight.md before first regression build. Canonical pending_invitation fixture is historicaltimed/manualwake, AGY sweep bootstrap assumes timedinvitationwake; neither proves modernLazy behavior unchanged. Real DomainService pull/registeredhook and actualcanonicalorigin/wake paths required.
