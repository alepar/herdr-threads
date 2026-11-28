# Task tree (ht-z41e) — snapshot for design roast round 3 (code redesign cycle 1 amend roast)

## ht-z41e — Per-seat config and Codex /side queue wake routing [open]

Root epic of super-auto run 2026-11-26-per-seat-config-side-wake. Spec: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md

## ht-z41e.1 — Codex native wake defers on a /side chat composer [closed]

Spec §Section 3. Remove `Ask a follow-up question` from CODEX_PLACEHOLDERS in src/harness/codex/composer.rs so a side-chat composer reads as not empty and native wake refuses/retries instead of typing into it.
files: src/harness/codex/composer.rs
Acceptance:
- [ ] CODEX_PLACEHOLDERS no longer contains the side placeholder; the doc comment says why (side chat is not the wake target)
- [ ] the existing `side` capture test now asserts the side composer is not Empty (ComposerRead::Text or the refusal the main-path gate expects), `side-draft` unchanged
- [ ] main-conversation placeholder still reads Empty
- [ ] clippy and scripts/check-harness-neutral pass
- [ ] documented (doc comment + skill text): with programmatic_wake off (the default), a wake owed while a /side chat is displayed defers on the ladder until the side chat closes; the sender's overdue line shows the refused wake (wake=) as today
- [ ] delivery-contract check recorded in the commit message (rules 2 and 4 hold; composer admission only narrows); AGENTS.md 'What we do today' entry updated where it names Codex placeholders
- [ ] test (fake clock): a side-composer pre-send refusal keeps retrying past the normal ladder length and the wake is typed once the composer reads Empty again; if the ladder is bounded, the bound is stated as an accepted limit in the skill and AGENTS.md instead
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.2 — Seat config store, registry and daemon commands (seat.config_v1) [closed]

Spec §Section 1 Storage, Registry, Commands (protocol), Error handling.
owns: SetSeatConfig carries the caller tier (agent | human/operator) the daemon checks against the key's registry tier; seat config registry (neutral core, key/description/type/default/tier/backing), seat_config table + node migration, GetSeatConfig/SetSeatConfig protocol commands and capability seat.config_v1, store read fn for a seat's effective value (used by queue selection), operator-tier provenance on writes.
Daemon decides writes against A2: agent tier only on the caller's own seat through its open top-level binding (as set_nudge_threshold, cooperative payload hash); human tier as operator decision operator:local-user:<uid> on any local seat. nudge_threshold key reads/writes the existing seats.nudge_threshold/wake_on columns (no data move). Final retirement deletes the seat's seat_config rows; reversible retirement keeps them. Unknown key/bad value refused with allowed values.
files: migrations/node/*, src/store/seat_config.rs (new), src/store/seats/threshold.rs, src/protocol/{commands,results,capabilities,seat_config}.rs, src/service/dispatch.rs, src/store/schema.rs, scripts/wire-compat-baseline.txt
Acceptance:
- [ ] registry has exactly nudge_threshold (agent, -1..9, default 0) and programmatic_wake (human, on/off, default off)
- [ ] store tests: default vs explicit, invalid key/value refused, retirement cleanup, nudge_threshold backed by the seats column
- [ ] service tests: agent-tier write on own seat via top-level binding succeeds; agent write of a human-tier key refused; agent write to another seat refused; subagent write refused; operator write to any seat recorded with operator provenance
- [ ] new wire fields #[serde(default)], capability const added; scripts/check-wire-compat and check-harness-neutral pass
- [ ] TRUST-POLICY.md updated in the same commit: seat-config write rules (agent tier = own seat's top-level binding, cooperative_top_level; human tier = operator decision operator:local-user:<uid>) and the accepted limit that the human tier is enforced by the harness permission ask-rule only
- [ ] reads: GetSeatConfig succeeds for a subagent caller and for a seat other than the caller's (service test)
- [ ] nudge_threshold writes through SetSeatConfig and SetNudgeThreshold share one service path (same wake reconsideration, same WakeStatus/no-wake result)
- [ ] scope: config is node-local; a remote seat target (name@B) is refused with a stated one-line error; named and terminal seats are supported (tests)
- [ ] programmatic_wake's registry description states it enables idle wake through a harness's programmatic transport (today the Codex queue, Linux/macOS) including the root session while a side chat is open; setting it on a seat whose harness has none is accepted and inert
- [ ] closed registry (human decision S1): unknown keys refused with the key list; a doc comment and test show a new key is one registry entry with no schema or wire change
- [ ] agent-tier authority equals today's SetNudgeThreshold check; tests per binding class (hook-registered, plain CLI, managed launch, named, terminal); any write from a subagent, and an agent-tier write from a HERDR_THREADS_SEAT_READONLY binding, refused (stated rule + tests)
- [ ] UnsetSeatConfig { seat?, key, tier } (same tier rules): table-backed key deletes its row; column-backed key writes the default and upserts its audit row (tests)
- [ ] a value outside the key's type is refused naming the valid set; no synonyms accepted (true/enable → refused with 'on|off')
- [ ] reversible retirement and revival keep rows, final retirement drops them (test); TRUST-POLICY states a human-tier key survives reversible retirement
- [ ] TRUST-POLICY residual limits: human tier only through immediate argv[1] `human` (existing rule); ask-rules match only the installed spellings (absolute paths / env prefixes may bypass); per-harness status of the human ask-rule (from ht-z41e.3) recorded
- [ ] column-backed keys per spec §Section 1 'Column-backed keys': value read from the column only; get shows (default) by equality; unset writes the default (and wake_on); every write upserts an audit-only seat_config row (value NULL, writer fields); strict -1..9 on SetSeatConfig while the aliases keep the client clamp; operator path may set another seat's nudge_threshold (A5 exception in TRUST-POLICY) — tests for each
- [ ] every column-backed value change (agent/human set, both unsets, aliases) calls wake::note_new_attention for the target seat in the same transaction (test: seat at T=0 with a pending p3, human sets nudge_threshold 3, the seat becomes owed)
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.3 — ht config seat CLI: list, get, set, and ht human config seat set [closed]

Spec §Section 1 Commands (CLI), B3-B7.
consumes: GetSeatConfig/SetSeatConfig commands and the registry (from ht-z41e.2).
owns: CLI surface `config seat list|get|set` and `human config seat set [--seat S]`, their text/machine output, nudge-threshold/wake-on as aliases over the nudge_threshold key, skill text explaining the commands once.
list is static (no daemon): one line per key `<key> <values> default=<d> tier=<agent|human> — <description>` (< 160 bytes/key) and --json. get: effective value, `(default)` when unset, any seat via --seat, subagents may read. set (agent path): own seat only; human-tier key refused with one line naming `ht human config seat set KEY VALUE` (< 100 bytes). human path: unified seat selector for --seat. Older daemon without seat.config_v1 → M13-style missing capability.
files: src/cli/config_seat.rs (new), src/cli/commands.rs, src/cli/nudge_threshold.rs, src/cli/wake_on.rs, src/cli/help.rs, integrations/skill/SKILL.md, AGENTS.md (token-efficient section entry)
blocked-by ht-z41e.2: consumes seat config protocol commands and registry
Acceptance:
- [ ] CLI tests for each command and both tiers, including refusal texts and stated byte sizes
- [ ] nudge-threshold N and wake-on behave and print exactly as before (existing tests unchanged and green)
- [ ] skill and help mention list/get/set once; AGENTS.md 'What we do today' gains one entry with the model-visible sizes
- [ ] an agent shown only `ht config seat list` output can name the command to set programmatic_wake (help text states the ht human spelling)
owns: the CLI capability gate on seat.config_v1 (site_capability pattern, M13-style one-line refusal with stated size)
- [ ] get/list accept --seat for any local seat and run from a subagent context (CLI tests)
- [ ] the agent-path refusal of a human-tier key prints the exact `ht human config seat set KEY VALUE` spelling (size stated)
- [ ] `config seat set nudge_threshold N` prints M12 on a no-wake seat exactly as nudge-threshold does
- [ ] against a daemon without seat.config_v1 the CLI prints the one-line capability refusal (test)
- [ ] tests that the installed Claude and Codex permission rules ask on both `ht human config seat set …` and `herdr-threads human config seat set …` (extend setup only if a spelling is not covered)
- [ ] `ht config seat unset KEY` and `ht human config seat unset [--seat S] KEY` (tests)
- [ ] list output shows key, values, default, tier and description for every key (test)
- [ ] a seatless caller (plain shell, no pane/name/terminal) can run `ht human config seat set --seat S …` and `config seat get --seat S`; omitting --seat there gives a stated one-line error (tests)
- [ ] human tier only when `human` is argv[1]: tests that `herdr-threads --pane X human config seat set …` and `--human config seat set …` do not get human tier
- [ ] per built-in harness (Claude, Codex, Hermes, agy, opencode, Pi): test or table of whether setup's permission policy asks on the human path; harnesses with none listed for ht-z41e.2's TRUST-POLICY line
- [ ] every shipped skill copy (integrations/skill/SKILL.md, agy plugin skills/, any opencode/Hermes/Pi copies) carries the new text, or a test keeps them in sync
- [ ] a terse README/config-docs entry for the config seat command family
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.4 — Queue selection reads per-seat programmatic_wake; node setting becomes obsolete [closed]

Spec §Section 1 Removing the node setting, B1, B10 (no migration).
consumes: store read fn for a seat's effective programmatic_wake (from ht-z41e.2).
owns: removal of the process-wide programmatic_wake flag from ServiceConfig/QueueWakeRuntime; queue_wake_selection reads the seat's value inside its selection transaction (fail closed to native on read error); node settings still parse the key with no effect; doctor prints one obsolete line pointing at `ht human config seat set programmatic_wake on` when the file sets it.
files: src/daemon/settings.rs, src/service/config.rs, src/app.rs, src/notification/queue.rs, src/store/wake_intents/selection.rs, src/cli/doctor.rs, TRUST-POLICY.md (if it mentions the node switch), CHANGELOG.md
blocked-by ht-z41e.2: consumes seat config store read fn
Acceptance:
- [ ] a seat with programmatic_wake on gets queue selection; another seat on the same daemon with it off stays native (fake-peer test)
- [ ] toggling the seat value takes effect on the next attempt without daemon restart
- [ ] node settings containing programmatic_wake true still load and have no effect; doctor shows the obsolete line
- [ ] existing queue wake tests updated from node flag to seat flag and green
- [ ] a seat-config read failure inside queue_wake_selection selects native, never queue (test)
- [ ] TRUST-POLICY's queue-wake authorization text (incl. the stale-authority limit) restated for per-seat authority in the same commit; a test that an intent armed while the seat is on, after the seat flips off, takes the stale-authority native fallback
- [ ] every doc naming node programmatic_wake (README, config docs, AGENTS.md delivery section) points at `ht human config seat set programmatic_wake on`; CHANGELOG states that node-level queue wake no longer takes effect (behavior change for users who enabled it)
- [ ] any CLI path that writes programmatic_wake into node settings refuses it and names the seat command (if none exists, the commit says so)
- [ ] AGENTS.md 'What we do today' entry for per-seat queue enablement
- [ ] a programmatic_wake write runs the same wake reconsideration as a nudge_threshold write and lifts the AwaitingReceipt/unreceived suppression for that seat (test: unreceived native → flip on → queue attempt follows)
- [ ] doctor lists the seats whose programmatic_wake is on (diagnosability after the node flag goes)
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.5 — Release tooling enables queue wake per seat [closed]

Spec §Section 1 Removing the node setting (release tooling paragraph).
consumes: `ht human config seat set --seat S programmatic_wake on` CLI (from ht-z41e.3).
owns: scripts/release-qualify selects queue proof only with explicit --queue-proof (node-settings probe removed); scripts/wake-e2e.py under --queue-proof stops writing programmatic_wake into owned node settings and sets it on each owned Codex recipient seat through the human CLI in its owned instance before the first send; selftests updated; AGENTS.md 'Pre-release wake e2e' queue paragraph and RELEASE.md follow.
files: scripts/release-qualify, scripts/wake-e2e.py, scripts/wake_e2e_selftest.py, scripts/wake_e2e_queue.py, AGENTS.md, RELEASE.md
blocked-by ht-z41e.3: consumes ht human config seat set CLI
Acceptance:
- [ ] python3 scripts/wake-e2e.py --self-test passes, including new tests that --queue-proof enables per seat and never writes programmatic_wake to node settings
- [ ] release-qualify selftests (if any) and its --help reflect explicit-only selection
- [ ] never writes real user config; operator node settings untouched
consumes: per-seat queue selection (from ht-z41e.4)
blocked-by ht-z41e.4: consumes per-seat queue selection
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.6 — Spike: Codex /side root vs side identity, concurrency, arrival and UI (S1-S4) [closed]

Spec §Section 2 Spikes. Live, isolated: owned test instance, scratch CODEX_HOME with a private mode-0600 copy of ~/.aisw/profiles/codex/codex-2/auth.json (user-approved 2026-11-26), isolated Herdr session (scripts/lib/isolated-herdr.sh), own daemon; never writes real config, never touches panes it did not create, stops every process it starts.
owns: evidence doc docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/spike-side-identity.md answering:
S1 what distinguishes a root session's hook-registered binding from a /side session's (hook payload fields, session/rollout storage, persistence), stable across resume;
S2 whether a queued root turn starts while a side turn is busy, and while the side view is displayed idle;
S3 the woken root turn's UserPromptSubmit carries the root session id; side hooks do not fire for it;
S4 side view stays displayed, side composer draft survives, root turn completes in background.
Throwaway probe scripts live under the run dir evidence/; not product code.
Fallback when live access is refused or Codex cannot run: record the refusal and answer S1-S4 from docs/superpowers/runs/2026-11-14-codex-queue-wake/evidence and Codex source reading, marking each answer observed vs inferred.
files: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/spike-side-identity.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/**
Acceptance:
- [ ] each of S1-S4 has an answer marked observed (with evidence path) or inferred (with reason)
- [ ] S1 names a concrete, testable root marker the Codex adapter can read, or states none exists
- [ ] S2 states whether the all-bindings-idle admission rule must stay
- [ ] check-no-leaked-processes clean after the spike
- [ ] the doc ends with a one-line routing verdict for ht-z41e.7: 'marker: <name>' and 'busy-sibling rule: keep|drop', or 'no marker' (which makes ht-z41e.7 stop with a blocker)
- [ ] runs with a scratch CODEX_HOME holding a mode-0600 copy of ~/.aisw/profiles/codex/codex-2/auth.json, owned Herdr workspace closed at the end, no real config written
- [ ] verdict line also carries 'ui: safe|blocker' from S4, and whether a lone root binding (no /side) carries the S1 marker
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.7 — Queue wake targets the process's root session with /side bindings present [closed]

Spec §Section 2 Selection, B8.
consumes: S1 root marker and S2 admission answer (from ht-z41e.6); per-seat queue selection path (from ht-z41e.4).
owns: neutral adapter policy method naming a process's root session among a seat's connected top-level bindings (default None, no root known), its Codex implementation from the S1 marker, queue_wake_selection rule: all connected top-level bindings in one process (same liveness pid/start/boot) and exactly one named root → target root by exact native session UUID; side sessions never targets; across processes or no root → native. Admission requires no binding in the process Busy unless S2 shows it safe. Arrival (R5): only the root binding's contact settles the queued wake; a side binding's contact never does.
files: src/harness/adapter.rs (or neutral policy file), src/harness/codex/** (root marker), src/store/wake_intents/selection.rs, src/notification/queue.rs, src/store/wake_intents/*.rs (arrival), TRUST-POLICY.md if arrival/fallback wording changes
blocked-by ht-z41e.6: consumes S1 root marker and S2 admission answer
blocked-by ht-z41e.4: consumes per-seat queue selection path in queue_wake_selection
Acceptance:
- [ ] fake-peer tests: root + one or two side bindings in one process → queue targets root UUID; two processes → native; adapter returns None → native; a side binding Busy → refused (or allowed, per S2); side contact does not settle the root wake, root contact does
- [ ] default adapter method returns None for every non-Codex harness; scripts/check-harness-neutral passes
- [ ] existing single-binding queue tests unchanged and green
owns: the observable queue-intent record carrying the target root session UUID and the seat's side session UUIDs (named field in the existing intent/diagnostic output the wake e2e driver reads), consumed by ht-z41e.8
If ht-z41e.6 reports 'no marker', this bead implements nothing beyond the neutral default and reports a blocker (scope decision for the human) instead of a None-only Codex implementation.
- [ ] test: a native fallback while a side composer is displayed is a pre-send refusal retried on the ladder, never typed (crosses the ht-z41e.1 behaviour)
- [ ] native wake receipt settlement: apply S3's result, or state in TRUST-POLICY/commit that native keeps any-binding contact as receipt evidence
- [ ] TRUST-POLICY updated where queue-wake arrival/target rules change; delivery-contract check in the commit; AGENTS.md 'What we do today' entry
consumes: side composer reads non-Empty (from ht-z41e.1); S4 ui verdict (from ht-z41e.6)
blocked-by ht-z41e.1: consumes side composer non-Empty read
On 'no marker' or 'ui: blocker' the blocker goes to the human with the S1/S4 evidence; ht-z41e.8 and ht-z41e.10 are re-scoped by that decision.
- [ ] adapter-level test that a lone Codex top-level binding is named root from the S1 marker as observed (or the single-binding path does not require a root name)
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.8 — Wake e2e opt-in case side-queue/group [closed]

Spec §Section 2 Qualification.
consumes: per-seat queue enablement in the driver (from ht-z41e.5).
owns: new opt-in Codex case side-queue/group under --queue-proof: the recipient uses a /side chat (one side turn so it registers its binding), side left displayed, then a p0 is sent; pass requires an Accepted queue intent naming the root session UUID, the item delivered in the root turn, and no wake text typed into the side chat (pane capture). Offline selftests for the case's judging logic; AGENTS.md opt-in list gains the case.
files: scripts/wake-e2e.py, scripts/wake_e2e_queue.py, scripts/wake_e2e_queue_selftest.py, scripts/wake_e2e_selftest.py, AGENTS.md
blocked-by ht-z41e.5: consumes per-seat queue enablement in the wake e2e driver
Acceptance:
- [ ] python3 scripts/wake-e2e.py --self-test passes with new judging tests (accepted-root pass; side-typed fail; native-only fail under required proof)
- [ ] the case is opt-in, not in the default matrix, and refused without --queue-proof
consumes: the queue-intent record's target root UUID field (from ht-z41e.7)
blocked-by ht-z41e.7: consumes queue-intent target root UUID record format
Also owns an opt-in case side-native-defer/group (queue off): with a /side chat displayed, a p0 types nothing into it; after the side chat closes the root wakes and gets the item. Reviews side-nudge/group and side-draft/group against ht-z41e.1 (a displayed side chat now defers) and updates their expectations.
- [ ] selftests for side-native-defer judging; side-nudge/side-draft expectations consistent with ht-z41e.1
blocked-by ht-z41e.1: consumes side placeholder defer behaviour (case expectations)
owns (amended): side-native-defer/group — runs without --queue-proof (refused under it): display a /side chat, send p0, assert nothing typed into it, close the side chat, assert the root wakes and gets the item. Per-case queue mode: side-queue (proof required), side-native-defer (queue off); side-nudge/side-draft and the co-plain recipient get expected results under both modes (including the S2 busy-sibling keep outcome). AGENTS.md lists both new cases.
- [ ] side-queue pass also asserts the side chat's pane shows no unexpected change
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.9 — Gate: fix loop exited [open]

Gate bead (super-auto): closed by the orchestrator when phase 5's code fix loop exits. Holds final-SHA live evidence.

## ht-z41e.10 — Live qualification: queue wake per seat and side-queue at the final SHA [open]

Final-SHA evidence, gated so later fixes do not stale it. Live, owned instance, codex-2 auth copy (user-approved 2026-11-26), from a Herdr pane in the run worktree at the final SHA:
nice scripts/build.sh; python3 scripts/wake-e2e.py --self-test; python3 scripts/wake-e2e.py --harness codex --queue-proof --codex-auth-home ~/.aisw/profiles/codex/codex-2 (default Codex matrix with per-seat enablement); then --cases side-queue/group,side-nudge/group,side-draft/group with --queue-proof.
Evidence dirs kept under docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/; results summarized in qualification.md. A FAIL is a confirmed defect (filed through the fix path), not waived here.
files: docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/qualification.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/qualification/**
blocked-by ht-z41e.9: consumes fix loop exited (gate)
blocked-by ht-z41e.7: consumes root-session queue routing
blocked-by ht-z41e.8: consumes side-queue/group e2e case
blocked-by ht-z41e.1: consumes side placeholder fix (side-nudge/side-draft expectations)
Acceptance:
- [ ] Codex --queue-proof matrix exit 0 with per-seat enablement (needs: ht-z41e.5)
- [ ] side-queue/group passes: Accepted intent on the root UUID, item delivered in the root turn, nothing typed into the side chat (needs: ht-z41e.8)
- [ ] side-nudge/group and side-draft/group pass
- [ ] check-no-leaked-processes clean; no real config written
blocked-by ht-z41e.5: consumes per-seat queue enablement in the driver
- [ ] side-native-defer/group passes (queue off) (needs: ht-z41e.8)
- [ ] invocations named per case: --queue-proof for the matrix and side-queue; no --queue-proof for side-native-defer, and for side-nudge/side-draft per ht-z41e.8's per-case mode
- [ ] the default Codex matrix (no --queue-proof) exits 0 at the final SHA (native composer path regression check for ht-z41e.1)
blocked-by ht-z41e.11: consumes integration sweep
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.11 — Integration sweep: per-seat config and /side queue routing [closed]

Root integration sweep. Verify end to end: (1) a person runs `ht config seat list`, then `ht human config seat set --seat S programmatic_wake on`, and S's next owed idle wake selects the queue while another seat stays native; (2) an agent sets its own nudge_threshold through `ht config seat set` and the wake frontier honours it; (3) with a /side binding present, the queue targets the root session (fake peers), native defers on a side composer. Add integration tests no per-seam bead covers; sweep for unwired values (registry key not read anywhere, capability not advertised, doctor line missing, docs naming the node key). Fix small gaps inline, file blockers for big ones.
files: tests/** (new integration tests), any small inline fixes
blocked-by ht-z41e.1: consumes all leaves (integration sweep)
blocked-by ht-z41e.2: consumes all leaves (integration sweep)
blocked-by ht-z41e.3: consumes all leaves (integration sweep)
blocked-by ht-z41e.4: consumes all leaves (integration sweep)
blocked-by ht-z41e.5: consumes all leaves (integration sweep)
blocked-by ht-z41e.6: consumes all leaves (integration sweep)
blocked-by ht-z41e.7: consumes all leaves (integration sweep)
blocked-by ht-z41e.8: consumes all leaves (integration sweep)
Acceptance:
- [ ] the three flows above have passing integration tests (needs: ht-z41e.4)
- [ ] no doc, script or help text still tells anyone to set node programmatic_wake
- [ ] clippy, check-harness-neutral, check-wire-compat, check-default-features pass
Environment: the host has no C toolchain; run every cargo/nextest/clippy/scripts/check-* command as `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb <cmd>` (nice 19, -j4, ht-build2 toolbox). Seed a new task worktree's build first: `scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target` (copy-on-write). Run focused tests only (`cargo test --locked --all-features <filter>` or nextest with a filter); never the full suite. Never touch the shared Herdr server or real user config (AGENTS.md).

## ht-z41e.21 — Codex root marker from transcript_path nullness on every hook event [open]

Owns the Codex adapter's per-event classification (null `transcript_path` on SessionStart/UserPromptSubmit/PreToolUse/PostToolUse/Stop reported as neutral auxiliary-session evidence on the existing hook evidence path), the binding store's sticky persistence (`concurrent_session=1` plus a new neutral `auxiliary_session` column via a node migration; never cleared by a later non-null event; `source: fork` still sets only `concurrent_session`), `RootCandidate.auxiliary_session`, and Codex `wake_process_root` (exactly one non-concurrent candidate and every other candidate auxiliary, else None). Consumes `process_root` in src/store/wake_intents/selection.rs (lone concurrent binding never a target, unchanged). Tests enroll through real Codex hook payloads, never raw SQL: root + fork-SessionStart side, root + side first seen by UserPromptSubmit and by PreToolUse (root targeted; side alone falls back to native; a later non-null event keeps the mark); a non-/side fork (fork, non-null path) beside a connected original selects native. Rewords TRUST-POLICY "Queue wake target with /side sessions present" (marker, real fork → rootless/native, accepted limit: a side with no hook event yet is invisible and the root is targeted) and the AGENTS.md queue-wake bullet in the same commit; existing raw-SQL tests are moved to real enrollment where they assert selection. Harness-neutral and wire-compat checks apply (new wire field `#[serde(default)]`). Run cargo through `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb` (no host toolchain). | src/harness/codex.rs, src/harness/adapter.rs, src/harness/registry.rs, src/store/bindings.rs, migrations/node/, src/store/wake_intents/selection.rs, tests/store/bindings.rs, tests/store/cooperative_checkin.rs, tests/store/wake_queue_cost.rs, tests/notification/queue_runtime.rs, TRUST-POLICY.md, AGENTS.md | covers: [Must-fix] untested: Codex non-/side fork with original binding connected — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
Acceptance:
- [ ] owns: the neutral evidence field on check-in/claim (#[serde(default)], scripts/check-wire-compat passes), the bindings.auxiliary_session node migration (default 0, never cleared, scoped to the binding row so a new binding starts at 0), RootCandidate.auxiliary_session, the Codex wake_process_root predicate (supersedes ht-z41e.7's Codex predicate), the TRUST-POLICY 'Queue wake target with /side sessions present' rewording incl. the not-yet-enrolled-side limit and that approval-bypass harness modes (Codex --approve-for-me / sandbox bypass) skip the human ask, and the AGENTS.md entry — all in one commit
- [ ] tests through real Codex hook payloads (never raw SQL): fork-SessionStart side, side first seen by UserPromptSubmit, side first seen by PreToolUse, later non-null event keeps the mark, non-/side fork beside a connected original → native, lone side → native, root + unenrolled side → root, plain-CLI root+side bindings → native (documented: /side routing needs hook-registered Codex)
- [ ] scripts/check-harness-neutral passes; delivery-contract check in the commit message

## ht-z41e.22 — Queued-wake receipt counts only the root binding's contact (R5) [open]

Owns `DeliveryPullState` contact bookkeeping (per (seat, binding) contact time beside the seat-scoped one; `record_contact` records both) and the scheduler's closed-loop `AwaitingReceipt`: a queued attempt records its intent target binding and its `silent` check reads that binding's contact; native attempts keep the seat-scoped check. Consumes the intent target from wake_intents. Adds tests: side-only contact leaves a queued root attempt `unreceived`; root contact settles it; native settles on any binding. Run cargo through `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb` (no host toolchain). | src/scheduler/mod.rs, src/service/delivery_pull.rs, src/service/dispatch.rs, tests/scheduler/wake_receipt.rs | covers: [Should-fix] src/scheduler/mod.rs:1012 — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
Acceptance:
- [ ] owns the mapping from the intent's root session UUID to binding contact (any binding with that native session UUID counts, so a root re-registration between reservation and contact still settles); test for re-registration
- [ ] the unreceived suppression lift for a queued attempt uses the same root-scoped contact; test that side-only contact neither settles nor re-arms a retype loop
- [ ] receipt scope follows the transport actually used: a queue-selected attempt that fell back to native (stale-authority NoEffect, flag flipped off before arm) is seat-scoped; test combined with the ht-z41e.26 flip-off scenario
- [ ] AGENTS.md closed-loop entry and TRUST-POLICY A4 wake text updated; delivery-contract check in the commit

## ht-z41e.23 — Store test: attention witness carries programmatic_wake [open]

Owns one store-level test over the production `load_candidate` (src/store/wake.rs) asserting `attention_witness.programmatic_wake()` is true with the seat key on and false off, so removing `.with_programmatic_wake(...)` fails a test. No production change. Run cargo through `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb` (no host toolchain). | src/store/wake.rs, tests/store/ (new case in an existing wake suite) | covers: [Should-fix] src/store/wake.rs:272 — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.

## ht-z41e.24 — Refuse operator seat-config write on a finally retired seat [open]

Owns `operator_write` in src/store/seat_config.rs: refuse (one-line error, existing refusal style) when the seat's retirement is final; a reversibly retired seat still accepts. Consumes retirement state from src/store/control/retirement.rs. Test: write to a finally retired seat is refused and leaves no `seat_config` row (doctor does not list it); reversible retirement still writes. Run cargo through `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb` (no host toolchain). | src/store/seat_config.rs, tests/store/ (seat config suite) | covers: [Nit] src/store/seat_config.rs:489; src/store/seat_config.rs:462 — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
Acceptance:
- [ ] operator set, operator unset and column-backed (nudge_threshold) operator writes are all refused on a finally retired seat and leave no seat_config row (tests); the refusal line's byte size stated in the commit (AGENTS.md rule f)

## ht-z41e.25 — side-native-defer proves a refused attempt before closing the side chat [open]

Owns `side_defer_case` and `side_defer_passed` in scripts/wake-e2e.py: before `close_side()` poll the daemon `wake_state()` for a refused native attempt completed after `sent_ms` (or an advanced `retry_step`) and fail setup when none appears; pass input gains the refused-attempt fact. Extends the driver self-test for the no-refusal case. Python only; `python3 scripts/wake-e2e.py --self-test` (no agents). | scripts/wake-e2e.py | covers: [Nit] scripts/wake-e2e.py:3044 — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
Acceptance:
- [ ] confirm the existing wake_state() diagnostic exposes a refused native attempt with its completion time (or retry_step); if it does not, the driver uses an existing field and the commit says which
- [ ] python3 scripts/wake-e2e.py --self-test passes

## ht-z41e.26 — Queue-runtime test: programmatic_wake flipped off between selection and arm [open]

Owns one test through the queue runtime (not store arm only): select a queue wake with the seat key on, flip it off before arm, assert the stale-authority NoEffect refusal and the same attempt settling as a native wake. Consumes `arm` in src/store/wake_intents.rs and `QueueWakeRuntime`. Run cargo through `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb` (no host toolchain). | tests/notification/queue_runtime.rs, src/notification/queue.rs | covers: [Must-fix] untested: stale-authority fallback after flag flip only store-level — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.

## ht-z41e.27 — Gate: full-suite measurement on the post-fix head (phase-6 sweep) [open]

After the code items above land, run `nice cargo nextest run --locked --all-targets --all-features` once on the branch head through `$HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb`, with `HT_LEAK_RUN_ID` exported and `scripts/check-no-leaked-processes --run-id` after; record wall-clock against the 5-minute budget and the SHA in the run evidence. Observe the durak CPU caps (-j 4, --test-threads 4, quiet window). | docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/ | covers: [Must-fix] untested: no full-suite measurement of this branch — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
Gate (super-auto): closed by the orchestrator when the phase-6 sweep passes at the tip; never dispatched.
blocked-by ht-z41e.21: consumes cycle-1 code fixes
blocked-by ht-z41e.22: consumes cycle-1 code fixes
blocked-by ht-z41e.23: consumes cycle-1 code fixes
blocked-by ht-z41e.24: consumes cycle-1 code fixes
blocked-by ht-z41e.25: consumes cycle-1 code fixes
blocked-by ht-z41e.26: consumes cycle-1 code fixes

## ht-z41e.28 — Gate: unbounded whole-epic review with complete provenance [open]

After the code items above land, one review pass over the full `4e4f628a4...<head>` diff, including every docs path and every TRUST-POLICY hunk (queue wake target with /side sessions, stale authority, seat config), with review-inputs provenance complete: first repair the malformed collection record at run.md:31 that broke review-inputs selection (read-only for the reviewer; the caller fixes run state), then re-collect. Records any findings for the loop. | .superpowers/sdd/ht-z41e-plan/, TRUST-POLICY.md, docs/ | covers: [Must-fix] untested: whole-epic review incomplete (bounded hunks only), [Must-fix] untested: review-inputs provenance gap, [Must-fix] untested: changed docs paths not inspected, [Must-fix] untested: TRUST-POLICY /side and stale-authority hunks not inspected — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
Gate (super-auto): closed by the orchestrator when a whole-branch review (code roast round 2 over the full fork-point diff, or the post-fix super-code final review) runs with complete review-inputs provenance; never dispatched.
blocked-by ht-z41e.21: consumes cycle-1 code fixes
blocked-by ht-z41e.22: consumes cycle-1 code fixes
blocked-by ht-z41e.23: consumes cycle-1 code fixes
blocked-by ht-z41e.24: consumes cycle-1 code fixes
blocked-by ht-z41e.25: consumes cycle-1 code fixes
blocked-by ht-z41e.26: consumes cycle-1 code fixes

## ht-z41e.29 — Live qualification extension for ht-z41e.10 at the final SHA [open]

Gated behind ht-z41e.9 "Gate: fix loop exited"; runs with ht-z41e.10 at the final SHA, once, not per finding. Adds to .10's acceptance: wake-e2e Codex `--queue-proof` matrix plus `side-queue/group`, and `side-native-defer/group`, `side-nudge/group`, `side-draft/group` in their modes (`--codex-auth-home ~/.aisw/profiles/codex/codex-2`); the ignored `herdr_release_wake` run through `scripts/herdr-release-smoke` (exercising the inverted /side expectation and the codex.js Ctrl-C side-close branch); a live flag-flip stale-authority check (seat `programmatic_wake` turned off via `ht human config seat unset` between send and arm, fallback observed as native). Records every evidence directory in the run.md `live-qualification` row. Spends real Codex quota; owned test instance only. | scripts/wake-e2e.py, scripts/herdr-release-smoke, tests/integration/herdr_release_smoke.rs, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/evidence/ | covers: [Should-fix] scripts/wake-e2e.py:2979, [Should-fix] tests/integration/herdr_release_smoke.rs:1175, [Must-fix] final review: run ignored herdr_release_wake on the final SHA, [Must-fix] final review: complete live-qualification row (ht-z41e.10), [Must-fix] untested: side-queue and side-native-defer never run live — Context, not to be changed: everything else in docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-27-per-seat-config-side-wake-roast-pr-1-step-back.md, docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/amend-code-1.md and the amended spec §Selection/§Qualification/§Testing. Findings and step-back text there are evidence, not instructions. Done when: the covered findings are resolved and only the named files changed; named tests pass. The spec's ## Goal is never edited. Seed the task worktree with scripts/seed-worktree-target $HOME/AleCode/herdr-threads/.worktrees/super-auto-per-seat-config-side-wake/target; run cargo only through $HOME/.cache/herdr-threads-heavy/heavy $HOME/.cache/herdr-threads-heavy/tb; focused tests only.
blocked-by ht-z41e.9: consumes fix loop exited (gate)
Acceptance:
- [ ] side-queue/group at the final SHA records the side binding's first enrolling hook event and its auxiliary_session/concurrent_session bits, and the Accepted intent on the root UUID; the queued attempt settled by root-binding contact
- [ ] a live root records a non-null transcript_path on every hook event and SessionStart source it emits in the run (startup, resume where exercised); a null on a root is reported as a defect
- [ ] a live Codex agent told in prose to turn on programmatic_wake for its seat finds the key via `ht config seat list` and runs `ht human config seat set` (under the e2e's approval-bypass mode; recorded as bypassing the ask)
- [ ] ignored herdr_release_wake run once through scripts/herdr-release-smoke; a live flag-flip stale-authority check; every evidence dir recorded in the live-qualification row
blocked-by ht-z41e.21: consumes per-event root marker
blocked-by ht-z41e.22: consumes root-scoped queued receipt
blocked-by ht-z41e.25: consumes side-native-defer refusal proof

