## Task 13: Automatic Ensure creation policy and durable handoff composition

filesTouched: ["src/protocol/commands.rs", "src/protocol/capabilities.rs", "src/protocol/handoff.rs", "src/protocol/handoff/automatic.rs", "src/protocol/handoff/automatic_bootstrap.rs", "src/cli/commands.rs", "src/cli/mod.rs", "src/cli/input.rs", "src/cli/direct_send.rs", "src/cli/handoff.rs", "src/cli/handoff_delivery.rs", "src/cli/topology_handoff.rs", "src/cli/topology_handoff/automatic.rs", "src/cli/topology_runtime.rs", "src/cli/topology_recover/automatic.rs", "src/cli/journal.rs", "src/cli/retry.rs", "src/store/control.rs", "src/store/handoff.rs", "src/store/handoff/automatic.rs", "src/store/topology_handoff/automatic.rs", "src/store/topology_handoff/attachment.rs", "src/store/topology_handoff/runtime.rs", "src/archival_legacy.rs", "src/store/archival.rs", "src/service/dispatch.rs", "src/daemon/control.rs", "tests/cli/commands.rs", "tests/cli/direct_send.rs", "tests/cli/handoff.rs", "tests/cli/handoff_delivery.rs", "tests/cli/topology_handoff.rs", "tests/cli/journal.rs", "tests/handoff_topology_authority.rs", "tests/cli/launch.rs", "tests/harness/launch.rs", "tests/store/control.rs", "tests/store/archival_legacy.rs", "tests/protocol/capabilities.rs", "README.md", "docs/agent-usage.md", "integrations/skill/SKILL.md", "TRUST-POLICY.md", "src/protocol/wire.rs", "src/daemon/transport.rs", "src/client/local.rs", "tests/daemon/transport.rs"]

bead: ht-0lo.8.2

Dependencies: ht-0lo.8.1

### Proposed full bead body (coordinator persistence required)

```json
{
  "id": "ht-0lo.8.2",
  "title": "Automatic Ensure creation policy and durable handoff composition",
  "description": "Own optional frozen Ensure creation policy Lazy/RequireAck+duration absent=historical timed, modern directsend Lazy newpair and automatichandoff lazy/timed initial creation, body acquisition/freezing, independentACKbody/message deadline, capability-before-effects, canonical automatic validators/adapters/public routes/journal/retry/loader/archival/readiness and user docs. Consume Task12 actual additive schema/capacity map; do not edit released history or invent standalone Invite. Preserve existing joined/pending/rejected/left pair semantics exactly and actual outcome presentation. Normative refinement in upstream-composition-refinement.md supersedes prior flag refusal.",
  "acceptance_criteria": "Actual automatic existing-seat/existing-pane/launched-pane/new-tab and explicit routes support nonemptyUTF8<=65536 positional/file/stdin acquired once, maximum ordinary/escaped/NUL/whitespace exact bodies and source mutation/deletion retries. Initial Ensure Lazy/RequireAck/default/explicit durations correct; absent historical exact requests/digests unchanged; duplicate policy mismatch retains original episode/deadline/origin, AlreadyJoined creates no obligation, rejected/left never reinvite. Message remains addressed ordinaryACK independently,7second message/11second invite, creation/send-based clocks, registration/launch never acceptance/ACK. Capability discovery precedes journal/allocation/Ensure/topology/native effects incl ordinary directsend and pending retries; completed historical inspection unchanged with Task1 conservative unsupported limits. Exact frozen children/attachment/published proof, lostEnsure\u2192reject/leave, lostSend, childcomplete/outerincomplete, possible-start/unknowncreate preserve no duplicate/restage/relaunch. Actual all body-bearing loader/recovery/terminal/archival paths honor Task12 proven ceilings and before-effect refusal. Advisory missing/untrusted/wrongprofile hooks with actualusedfingerprint and structural/grammar/trust guards retained. Focused public/canonical tests, fmt/clippy; independent interface+publicroute review includes clean-auto-merge paths, not just conflicts. New Ensure creation-policy capability direct_threads.ensure_creation_policy_v1 is independently required for modern policy, including Lazy directsend beforeeffects; advertise only actual canonical activation. Oldv0.7 automatic/direct peers includingLazy prove zeroeffects; Task13 owns semanticcapability, Task12 activation-ready schema ifneeded without speculativeDDL.",
  "dependencies": [
    "ht-0lo.8.1"
  ],
  "parent": "ht-0lo.8",
  "status": "proposed; coordinator creates bead"
}
```

### Acceptance and settled producer contract

Actual automatic existing-seat/existing-pane/launched-pane/new-tab and explicit routes support nonemptyUTF8<=65536 positional/file/stdin acquired once, maximum ordinary/escaped/NUL/whitespace exact bodies and source mutation/deletion retries. Initial Ensure Lazy/RequireAck/default/explicit durations correct; absent historical exact requests/digests unchanged; duplicate policy mismatch retains original episode/deadline/origin, AlreadyJoined creates no obligation, rejected/left never reinvite. Message remains addressed ordinaryACK independently,7second message/11second invite, creation/send-based clocks, registration/launch never acceptance/ACK. Capability discovery precedes journal/allocation/Ensure/topology/native effects incl ordinary directsend and pending retries; completed historical inspection unchanged with Task1 conservative unsupported limits. Exact frozen children/attachment/published proof, lostEnsure→reject/leave, lostSend, childcomplete/outerincomplete, possible-start/unknowncreate preserve no duplicate/restage/relaunch. Actual all body-bearing loader/recovery/terminal/archival paths honor Task12 proven ceilings and before-effect refusal. Advisory missing/untrusted/wrongprofile hooks with actualusedfingerprint and structural/grammar/trust guards retained. Focused public/canonical tests, fmt/clippy; independent interface+publicroute review includes clean-auto-merge paths, not just conflicts.

## Authorized upstream composition refinement — supersedes automatic invitation-flag refusal

Parent fact-driven ruling: Ensure actually creates the initial invitation. Automatic handoff supports --require-invite-ack and --invite-deadline for INITIAL pair creation, independently from the always-addressed ACK-required body and frozen message deadline. Add optional frozen Ensure creation policy Lazy/RequireAck plus duration; ABSENT means historical timed behavior and preserves old bytes/digests/keys. Modern direct send explicitly requests Lazy for a NEW pair invitation. Modern automatic handoff defaults that initial invitation to Lazy and can request timed acceptance. Registration/launch is never acceptance or ACK.

Preserve canonical pair lifecycle exactly: existing joined/pending episodes are reused without policy/deadline/origin changes; rejected/left pairs refuse without reinvite. A later caller requesting different creation policy cannot override/reset an existing episode. Present the ACTUAL retained policy/outcome honestly; AlreadyJoined creates no new invitation/obligation. Do not manufacture a separate Invite child: automatic remains Ensure→Send with exact canonical attachment/publication/completion proof. Existing explicit paths retain their Invite children. Test initial Lazy/RequireAck/default-duration/explicit-duration, historical absent exact bytes, duplicate policy mismatch retaining original, AlreadyJoined, rejected/left refusal and independent message/invitation deadlines (7/11 seconds).

All automatic and explicit handoff routes acquire exactly one nonempty valid UTF-8 source <=65536 bytes once, freeze exact body/policy/durations before effects, preserve NUL/whitespace/newlines and retry without rereading. Require send-time/invitation/automatic/direct capabilities as applicable BEFORE journal/allocation/Ensure/topology/native effects. Hook/setup/trust/profile readiness is advisory; structural/caller/seat/native grammar/actually-used owned fullconfig fingerprint checks and truthful native trust refusal remain mandatory. No auto trust/accept/ACK/adoption.

Released main478ec4a9 migrations0029_direct_threads and0030_automatic_handoff remain exact. Our unlanded29..35 shift31..37; add migration38 for automatic body-bearing original/recovery capacities proven with actual serializers, never rewrite released30 SQL. Preserve historical retained rows/requests/digests, genuine main30→combined upgrade/rollback/reopen and all dependent FK relationships. Count nested body copies across child/attachment/recovery/terminal/archival and enforce exact limits before effects; no clipping/unconditional globalwire increase.

Owned composition epic ht-0lo.8 has serial leaves .8.1/.8.2/.8.3, plan ordinals12/13/14. Work remains in separate isolated candidate workspace, distinct task branches/provenance reusing one owned warm validation slot serially. No provisional or unresolved composition merges into root integration until the whole reviewed A/B/C contract passes. Root closure waits. Parent owns final PR roast and subsequent expensive/defaultfeature exact-tip readiness. Prior11 ordinals and three edge-audit budget remain unchanged.

### Ensure creation-policy capability refinement

New optional Ensure creation-policy fields require their OWN honest additive capability, provisionally direct_threads.ensure_creation_policy_v1; invitation.confirmation_v1 and old automatic/direct capability do not establish support in released v0.7.0 Ensure handlers. Task13 owns constant, canonical activation/advertisement and every modern caller preflight. Advertise only when canonical Ensure policy creation is actually active. Modern Lazy directsend remains exempt from messages.send_time_receipts_v1 but MUST require this new Ensure-policy capability BEFORE journal/pair/allocation/native effects; modern automatic policy likewise requires it before any topology/durable/native effects, in addition to applicable automatic/invitation/receipt capabilities. Explicit historical absent fields preserve historical behavior/bytes and completed inspection; unsupported modern semantics never silently fall back. Test real old-v0.7 capability peers for automatic and direct routes, INCLUDING Lazy directsend, and assert no new journal/pair/topology/native effects. Task12 supplies the actual schema/canonical activation-ready cutpoint if needed with additive38; do not invent extra DDL merely to gate a capability. This is parent-authorized autonomous bounded composition within ht-0lo.8; no new confirmation request is required.

### Global constraints (verbatim project instructions)

##### Agent guidelines for herdr-threads

###### Development rules

- Per-change check: `nice cargo clippy --locked --all-targets --all-features -- -D warnings` (clippy already type-checks every target; a separate `cargo check` is redundant). Before a merge, also `nice scripts/check-default-features` (a default-feature `cargo check --all-targets` that fails on any warning; CI runs the full default-feature clippy job as well); format: `cargo fmt`.
- Tests: run the tests relevant to your change (`nice cargo test --locked --all-features <filter>`); the full suite is `nice cargo nextest run --locked --all-targets --all-features` (cargo-nextest: one process per test, every binary in one pool; test groups in `.config/nextest.toml` keep the budget- and latency-sensitive chains one at a time; `nice scripts/full-suite-gate N` runs it N times in a row with leak checks, and `nice scripts/flake-hunt` hunts flaky tests). A test that must not overlap others of its kind gets a nextest test group, not only an in-binary mutex (a mutex does not span processes).
- Never restart, stop or kill the shared Herdr server, and never close Herdr workspaces/panes you did not create. Tests that need Herdr up/down use an isolated named Herdr test session (see `scripts/lib/isolated-herdr.sh`).
- Never write to the real user config: `~/.claude`, `~/.codex`, `~/.aisw`. Tests use isolated HOME / CLAUDE_CONFIG_DIR / CODEX_HOME under a temp dir.
- Never run `git push` or `git stash` (the stash stack is shared across worktrees). Commit on your own branch.
- A finished task stops its processes: every daemon, private Herdr server and helper process a test or script starts is stopped before the task reports done. Spawn test children with `herdr_threads::test_support::spawn` (`spawn_owned` / `command` / `tag`); never a bare `Command::spawn` in `tests/`.
- After a full-suite run (and after each run of the 10-run gate or the integration sweep), run `scripts/check-no-leaked-processes`. Export `HT_LEAK_RUN_ID=<uuid>` before the suite and pass `--run-id "$HT_LEAK_RUN_ID"` to it, so another worktree's live run is never flagged. A non-zero exit lists each leaked pid; stop them (or rerun with `--kill`) and fix the test that leaked them.
- Test targets are listed explicitly (`autotests = false`). A new suite that keeps no process-wide state goes into `tests/combined.rs` as a `#[path]` module (one binary, one link); a suite that redirects stdio, installs a panic hook, runs an in-process daemon, arms failpoints or changes the umask gets its own `[[test]]` entry. A target that uses `herdr_threads::test_support` (including `test_support::spawn`) needs `required-features = ["test-support"]`; otherwise `scripts/check-default-features` and the default-feature clippy job fail.
- The daemon's owner watch exists only in `test-support` builds: always test with `--all-features`.

###### Trust policy

[TRUST-POLICY.md](TRUST-POLICY.md) is normative for seat continuity, caller attribution, receipt provenance
and operator repair. Read it before changing anything in `src/identity/`, `src/store/seats.rs`,
`src/store/receipts.rs`, `src/store/control.rs`, `src/protocol/authority.rs`, `src/cli/me.rs`,
`src/cli/hook.rs`, launch or wake.

- Do not add adversarial caller verification; the model is cooperative and same-user.
- Record claims honestly: a new way to attribute an action needs a provenance value defined in the policy.
- Never merge seats, never move a seat or end a binding on heuristic evidence.
- Decide in the daemon against the canonical view (A2); client-local files are hints, never authority.
- A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit.

###### Speed budgets

- The full test suite must stay under **5 minutes** wall-clock (`cargo nextest run --locked --all-targets --all-features`). Treat a regression past it as a bug: fix slow or serialized tests rather than raising the budget.
- An incremental build must stay under **1 minute** (edit one source file, then `cargo build` / `cargo test --no-run`), and so must the per-change lint (`cargo clippy --all-targets --all-features -- -D warnings`).
- Until the flakiness side quest (ht-zo4) lands, do not run the full suite routinely; run focused tests for what you changed.



### Composition execution boundary

Serial distinct task branches reuse one owned warm validation slot in isolated .8 candidate workspace with exact branch/source/test provenance. No simultaneous writer or Cargo lane. Source frozen during builds; preflight actual schema/caller/membership/binding/permit/serializer/cutpoint before first meaningfulRED. Wake completionbool is restoration, Text versusJSON explicit, nonzero selection mandatory. New target only for actual need with explicit registration/test-support feature and scoped coordinator admission. Never alter runstate/ledger from task worktrees. Parent owns final PR roast followed by exact-tip defaultfeature/expensive readiness; this task does not certify later tasks or authorize rootintegration before whole A/B/C review.


### Measured automatic response seam

Read task-13-preflight.md and task-13-wire-preflight.md. Exact additional source/test paths are admitted in mapping. Before wire production, consume A actual serializer measurements and report concrete exceptional limit/validation path to coordinator; preserve unrelated request/reply bounds. Parent-authorized narrow adjustment requires actual typed evidence, not assumed body copy count.


### Latest AGY composition cutpoint and ownership supersession

Consume upstream-composition-refinement.md latest mainab2291c5 ruling. Released29..31 exact; redesign32..38 and automaticcapacity39, genuine oldmain31 upgrade. Previous provisional numbers are historical only. Task15 .8.4 depends on Task12 .8.1 and solely owns shared cli/harness launch-readiness source plus AGY neutralbridge/pull-ledger semantics; Task13 .8.2 remains afterA with no B-Dedge. Task14 .8.3 depends BOTH B andD, including actual Lazyinvitation/Unchanged/whole_read/departedorigin/passiveclear proof. Parent wire-preflight notes remain intact; numeric scopedresponse cap awaits actualfixtures. Latest AGENTS requires check-harness-neutral, truthful Outside-adapter trailer on EVERYnewcommit, no real ~/.gemini. Prepared briefs do notauthorizeexecution or provisionalrootpromotion.

### Supported aggregate-envelope refinement

Parent-settled aggregate envelope: escaped65536-byte body supported under ordinary bounded metadata, not simultaneous independent metadata maxima. Full-max combinations refuse truthfully before durable/native effects; general requests remain1MiB. Exact supported original must fit every later Complete/Recover request, reply, local/recovery serializer including near-aggregate boundary and future bounded fields, or initial preflight refuses. Never accept an origin that later becomes unrecoverable. Preserve exact retained bytes/digests; positive maxbody ordinarymetadata and fullmax negative controls required. Exceptional AutomaticBootstrap response only, final numeric limit derived from full supported-envelope fixtures.

### Final A typed evidence and required public admission proof

Consume task-12-capacity-inventory.json at reviewed A fd7fb6c9. Corrected coherent nested fixtures supersede stale lowerbounds: near original524284, attachment524590, Complete1308800, Recover1073320; ordinary body plus max argv original460250/Complete1708024/Recover945252. SQL original admission is insufficient. BEFORE journal/pair/native effects, public admission must account together for original, child/attachment and every supported later Complete/Recover request/reply/local/recovery/report shape and future bounded fields. Derive an aggregate supported envelope; preserve general requests1MiB, no unconditional global enlargement. Ordinary escaped65536-byte body under ordinary bounded metadata is a required positive. Simultaneous independent metadata maxima may truthfully refuse. Add actual public near-boundary supported positive and oversized negative controls and exact-key retry/recovery proving accepted originals remain recoverable. Do not treat typed storage fixtures with deliberately bypassed current body validator as public accepted-shape evidence.

Superseding evidence notice: fd7 capacity report argv omitted canonical composed startup prompt; ALL Complete figures above remain lowerbounds until Task12 round1 genuine retained-argv correction. Do not derive final admission/response caps from them. Actual canonical Complete relation (store/topology_handoff/automatic.rs349–369) must hold, in addition to generic retained.validate and nested identities. Original/attachment storage overruns remain genuine; exact supported future shapes await corrected inventory.

### Authoritative canonical-composition evidence after A round1

Final A e196ae11 composes real retained startup prompt/template and asserts canonical retained_argv_matches; small-body full Complete.validate and real store Complete pass. Corrected Complete ordinary135879/escaped791239, valid maxargv1706303/2361663, near1309639. Near original524284/attachment524590/Recover1073320 remains a refusal case. See final task-12-capacity-inventory.json; ALL earlier bareargv Complete figures are superseded lowerbounds. B must derive its accepted aggregate envelope from actual generated retained prompt/template, attachment, bounded report and future fields rather than hardcode fixture numbers. Required public near-admission-boundary success plus above-boundary zero effects, and exact retry/recovery. No generic request-cap increase; ordinary escaped64KiB under ordinary metadata remains required.

### Scoped optional Ensure constructor admission

Admit tests/store/direct_threads.rs and tests/service/cooperative.rs solely for existing EnsureDirectThread struct literal adaptation to the optional field; preserve historical None bytes and behavioral assertions. No unrelated test relaxation.

### Parent-authorized bounded modern automatic completion summary

The supported envelope is the public producer-generated envelope, not every arbitrary externally authored 1MiB diagnostic report. Modern automatic canonical completion retains a DISTINCT explicitly versioned bounded completion summary. Keep the full generated launch report locally unchanged, with its exact existing bytes, digest and history. The canonical summary contains truthful required outcome/pane/seat/harness/EXACT composed argv, a fixed format marker, and optionally the full-report digest plus an explicit diagnostics-retained-locally/omitted marker. Omit optional unbounded diagnostics entirely; never clip identity/history or pretend summary bytes equal the full local report. Bound every summary field from actual canonical ID/argv constraints, and preflight reserves the exact worst supported summary, attachment, request/reply and local/recovery shapes including generated startup prompt/template. General requests remain1MiB.

Historical canonical reports remain untouched. Modern retries and completed presentation distinguish canonical summary from full local report honestly; preserve no-relaunch and canonical Completed read-only semantics, and recover/present the full local report where available. Missing-local completed presentation uses actual retained canonical representation without inventing diagnostics. Inspect and update TRUST-POLICY wording that assumes the full report is canonically retained. Fresh interface/public-route review and actual tests cover oversized post-launch diagnostics, exact full local retention, bounded versioned summary/digest, missing-local completed historical presentation, exact retries/recovery and unsupported old-peer zero effects. This parent-authorized autonomous composition refinement resolves otherwise unbounded provider/runtime diagnostic fields without a generic request-cap expansion.
