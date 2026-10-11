## Goals

### ht-poh (root)
A Google Antigravity CLI (`agy`) session in a Herdr pane is a full herdr-threads seat, on a par with Claude Code. It enrolls on its own, gets its context, receives message bodies directly between model calls, is woken when idle, and supports `launch`/`handoff`.

agy gets there as just another `HarnessAdapter`, on a core that names no harness. Claude's v0.5.0 mod delivery is re-fitted onto the same interface, a check enforces the rule, and the final report measures every change made outside the adapter.

Interactive root design (Mode A) with the user, 2026-10-09. Inputs:
- Research: `~/Documents/Agy_Integration_Research_20261008/report.md` and `work/*/findings.md`.
- Toolcall/skills probe evidence: `/private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/e411a093-57e8-4332-840c-ec519a3f8779/scratchpad/probe-toolcall/runs/`.
- Decision log: [design-decisions.md](design-decisions.md).
- Normative: [TRUST-POLICY.md](../../../../TRUST-POLICY.md).
- Prior designs: [harness adapters](../2026-10-03-harness-adapters/2026-10-03-harness-adapters-design.md), [Claude mod inbound delivery](../2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md) (v0.5.0), [versionless admission](../../specs/2026-10-05-harness-versionless-design.md).

### ht-poh.2 — Core sweep: move every production harness-specific site behind the adapter (empty all to-migrate allowlist entries)
After this subtree, the core (cli, client, service, daemon, store, host, protocol, app and the neutral `src/harness/*.rs` files) names no harness except in permanent allowlist entries that give a frozen-wire, history or registration reason. `scripts/check-harness-neutral` reports zero `to-migrate` entries. Claude, Codex and Hermes behave as before: wire, JSON, hook output and setup/doctor reports stay byte-identical, and the existing tests guard them. The first leaf lands every new adapter hook with an inert, neutral default. That way the agy adapter (ht-poh.4) can be written entirely inside `src/harness/agy/` plus its `builtins()` row.
summary: Spec §1 table (R8).

### ht-poh.3 — Delivery policy: src/delivery extraction, DeliveryPolicy trait, Claude PushChannel re-fit, HookPull requests, wake_on + SKILL section
Delivery is an adapter policy instead of a Claude special case:
- `src/delivery/` holds the v0.5.0 delivery mechanics, without naming any harness.
- Claude's mod runs through `DeliveryPolicy` (`PushChannel`). Its wire and behaviour stay byte-identical, and the existing v0.5.0 tests pass unmodified.
- A `HookPull` harness (agy, ht-poh.5) can pull framed pages with `DeliveryPull` and settle them with `DeliveryConfirm`, decided against A2.
- A seat can opt in at runtime to waking on every message with `herdr-threads wake-on all`, which the herdr-threads SKILL explains.
summary: Spec §2 (R4 core side, R9, R11).

### ht-poh.4 — agy adapter (minus delivery): metadata, owned plugin + skill, owned allow entry, local evidence + decode/witness, context, composer, launch, doctor
A bare `agy` started in a Herdr pane enrolls a seat on its first prompt and gets its startup check-in as a hook-injected `systemMessage`. A subagent never rotates or replaces the pane's binding, a `/clear` rebinds, and `agy -c` / `--conversation` resumes through C1. `herdr-threads setup|setup-status|unsetup agy` manage an owned plugin directory (hooks and skill) plus one owned `permissions.allow` member. `launch --kind agy` runs with validated arguments, an idle agy is woken only into an empty composer, and `doctor` reports every agy degradation.

All agy code lives in `src/harness/agy/`. Outside that directory there are only:
- the `builtins()` row;
- two neutral runner seams: `local_evidence`, and a `hook_step` that ht-poh.5's delivery consumes;
- TRUST-POLICY text and docs.
summary: Spec §3 except the HookPull delivery part (R1, R2, R3, R5, R6, R7).

### ht-poh.6 — Test infrastructure: agy fake Gemini server + native gate, Gemini↔OpenAI relay + live smoke script
A developer with a genuine agy binary (`HT_AGY_BIN`) runs `scripts/agy-native-gate`, a model-free gate. It drives the real agy TUI in an isolated named Herdr session against a scripted fake Gemini server, and proves every agy flow from ht-poh.4 and ht-poh.5 end to end:
- enroll, subagent, `/clear`, resume;
- delivery with transcript-confirmed settlement, and the downgrade;
- the permission entry, setup and unsetup, launch;
- idle wake.

The orchestrator runs `scripts/agy-live-smoke --live` once at the final SHA. It goes through a budget-capped relay to OpenRouter `google/gemini-2.5-flash` and writes a record to `docs/evidence/agy-live-smoke/`. That record shows a real model being woken, receiving a message through a hook-injected `systemMessage`, and ACKing and replying with `herdr-threads`. Neither runner ever runs in CI.
summary: Spec §5 (R12).

## Task tree

- ht-poh · agy harness adapter on a harness-neutral core (epic) · Root epic for super-auto run 2026-10-09-agy-harness-adapter. · deps: none
  - ht-poh.1 · Neutral rule seam: check-harness-neutral script + allowlist (format, to-migrate entries, D14 path rule), CI and per-change wiring · Spec §1 (R8) — seam: scripts/check-harness-neutral (shell) failing on harness id literals / Harness::<Name> / harness-named types or fields outside src/harness/ · deps: none
      owns: check-harness-neutral interface (allowlist format, to-migrate entries). Accounting (R13): every commit carries an 'Outside-adapter: <none | list>' trailer
  - ht-poh.2 · Core sweep: move every production harness-specific site behind the adapter (empty all to-migrate allowlist entries) (epic) · Spec §1 table (R8). · deps: none
      consumes: allowlist to-migrate entries and check script.
      owns: new trait hooks agent_env_markers, help fragments, foreground_advice, harness note, adapter-supplied settings kinds. Files: src/cli/{mod,output,setup,laun
    - ht-poh.2.1 · Adapter trait hooks seam · Lands every D1 hook with a neutral default, plus the `agent_env_markers` metadata field (all 48 literals in src and tests). · deps: ht-poh.1
        owns: `HarnessAdapter` hooks (`text`, `default_config_root`, `settings_kind`, `foreground_advice`, `setup_status_extras`, `setup_report_lines`, `doctor_text`, `
        consumes: `check-harness-neutral` and the allowlist format and `to-migrate` entries (ht-poh.1); the adapter-owned path rule of D14 (ht-poh.1).
    - ht-poh.2.2 · CLI help, env markers, launch/handoff surface · D2, D3 and the L2 rows of D15. · deps: ht-poh.2.1, ht-poh.2.7
        consumes: L1 `AgentEnvMarker`/`Registry::{env_markers,person_guard_markers}`, `AdapterText` (`setup_*`, `launch_notes`, `config_root_hint`).
    - ht-poh.2.3 · Setup engine and setup CLI de-harnessed · D4, D5, D6 (callers), D8 and the L3 rows of D15. · deps: ht-poh.2.1, ht-poh.2.7
        consumes: L1 `default_config_root`, `settings_kind`/`SettingsKindSpec`, `foreground_advice`, `setup_status_extras`, `setup_report_lines`, `AdapterText.trust_rem
        owns: the stable legacy helper signatures above, which L4 consumes unchanged.
    - ht-poh.2.4 · Doctor via adapter projections and text · D7. · deps: ht-poh.2.1, ht-poh.2.7
        consumes: L1 `doctor_text`, `DoctorTextForm`, `doctor_summary_claims`, the forwarder modules.
    - ht-poh.2.5 · Daemon, store, app and health neutral · D11, D12 and the L5 rows of D15. · deps: ht-poh.2.1, ht-poh.2.7
        consumes: L1 `AdapterText.version_history_note`; existing `legacy_contract_id()`.
    - ht-poh.2.6 · Hook runner, contract, attribution and evidence neutral · D9, D10 and the L6 rows of D15. · deps: ht-poh.2.1, ht-poh.2.7
        consumes: L1 `version_syntax`/`VersionSyntax`, `transcript_version`/`TranscriptVersionSpec`.
    - ht-poh.2.7 · Golden snapshots of per-harness surfaces before the sweep and re-fit · Byte-identity guard for R8 and R9. · deps: none
        owns: golden snapshot fixtures for per-harness surfaces.
  - ht-poh.3 · Delivery policy: src/delivery extraction, DeliveryPolicy trait, Claude PushChannel re-fit, HookPull requests, wake_on + SKILL section (epic) · Spec §2 (R4 core side, R9, R11). · deps: none
      owns: DeliveryPolicy trait, HookPull wire (DeliveryPull/DeliveryConfirm), delivery module API, wake_on.
      consumes: neutral rule and allowlist. Files: src/delivery/*, src/service/{dispatch,mod_channels,workers}.rs, src/protocol/watch.rs, src/protocol/(new delivery).
    - ht-poh.3.1 · Delivery seam: `DeliveryPolicy` trait, registry lookup, `DeliveryPull`/`DeliveryConfirm` wire, inert handlers · **Seam for ht-poh.5.** Add `DeliveryMechanism`, `IdleWake`, the `DeliveryPolicy` trait and `HarnessAdapter::delivery_policy()` (default `None`) (DP1). · deps: ht-poh.1
        owns: the `DeliveryPolicy` trait and registry lookup; the HookPull wire types (`DeliveryPull`, `DeliveryPullReply`, `DeliveryPage`, `DeliveryConfirm`, report ty
        consumes: the ht-poh.1 check script (must pass).
    - ht-poh.3.2 · Extract `src/delivery/` from `watch` and port the D4 frame to Rust · Move-only extraction (DP2). · deps: ht-poh.7
        owns: the src/delivery API (`Assembler`, `DeliveryItem`, `PageSource`, `drain_pages`, `DrainEvent`, `frame`, `FrameItem`, `HEADER`, page constants).
    - ht-poh.3.3 · Claude `PushChannel` re-fit: every Claude check becomes a policy lookup · DP3 and the DP7 idle-wake part. · deps: ht-poh.1, ht-poh.2.7, ht-poh.3.1, ht-poh.7
        owns: `ack_policy_delivered` (store settlement entry point), `delivery::settle`, `wake_suppressed`.
        consumes: the `DeliveryPolicy` trait and registry lookup from L1; the ht-poh.1 allowlist (frozen `not_claude`).
    - ht-poh.3.4 · HookPull handlers: `DeliveryPull`/`DeliveryConfirm`, offered ledger, pull-seat routing · Replace L1's inert arms with DP5/DP6: `src/delivery/ledger.rs` (pure bounded ledger) and `src/service/delivery_pull.rs` (Mutex owner, store-backed `PageSource`, · deps: ht-poh.3.1, ht-poh.3.2, ht-poh.3.3, ht-poh.7
        owns: HookPull request semantics (refusals, page, ledger, confirm outcomes), the `cooperative_hook_delivery` provenance.
        consumes: the L1 wire types; the L2 src/delivery API (`Assembler`, `drain_pages`, `frame`); the L3 `ack_policy_delivered` and `delivery::settle`.
    - ht-poh.3.5 · `wake_on` preference: migration, store, wire and `herdr-threads wake-on` · DP8 storage, wire, store and CLI. · deps: ht-poh.1
        owns: the `wake_on` storage and accessor (`seats::wake_on(db, seat)`), the `wake-on` command, the `last_lazy_ordinal` column.
    - ht-poh.3.6 · Core wake decision honours `wake_on: all`, plus SKILL section · DP8 core wake. · deps: ht-poh.3.5, ht-poh.7
        consumes: L5's `seats.wake_on` accessor and `wake_work.last_lazy_ordinal`.
    - ht-poh.3.7 · Push-channel promotion of `wake_on: all` lazy rows · DP8 promotion. · deps: ht-poh.3.2, ht-poh.3.5
        consumes: L5's `wake_on` accessor; L2's `delivery::page`.
  - ht-poh.4 · agy adapter (minus delivery): metadata, owned plugin + skill, owned allow entry, local evidence + decode/witness, context, composer, launch, doctor (epic) · Spec §3 except the HookPull delivery part (R1, R2, R3, R5, R6, R7). · deps: ht-poh.1
      owns: agy adapter module, local_evidence step, agy hook argv contract.
      consumes: neutral rule (no new core harness names). Files: src/harness/agy/*, src/harness/mod.rs, src/harness/registry.rs (one row), src/harness/adapter.rs (loc
    - ht-poh.4.1 · Neutral hook seams: `local_evidence` + witness rule, `hook_step` · Lands D4 and D6, inert by default. · deps: ht-poh.1, ht-poh.2.1, ht-poh.2.6
        owns: the neutral witness rule and both seams.
        consumes: ht-poh.2.1 (adapter.rs, registry.rs and hook.rs as left by the seam); ht-poh.1 check.
    - ht-poh.4.2 · agy module skeleton, metadata, registration, contract, decode/encode, hook argv · D1 skeleton: `mod.rs` with every trait method forwarding to stub submodules (setup/status/unsetup refuse or report not installed; `launch_policy` returns a stub · deps: ht-poh.1, ht-poh.2.1
        owns: the agy module, the hook argv contract (`hook agy --event SessionStart|PreInvocation|Stop`), the conversation-state location, `InjectionStep` encodings.
        consumes: ht-poh.2.1 (`agent_env_markers`, `AdapterText`, `default_config_root`).
    - ht-poh.4.3 · Witness evidence, lifecycle reading (startup, /clear, resume via C1), enrollment with startup check-in context, TRUST-POLICY startup text · `witness.rs`: D5 `local_evidence` (presence lock, the listing over bound parent and bounded glob of 65,536 entries plus deadline, K hint, lifecycle reading) and · deps: ht-poh.4.1, ht-poh.4.2
        owns: agy witness mapping; TRUST startup text and the "child with presence lock still a child via listing" limit.
        consumes: L1 (`local_evidence` trait, `refine`, runner call); L2a (decode, `AgyRoots`, `conversation_state`).
    - ht-poh.4.4 · Owned plugin dir, skill, allow entry: setup/status/unsetup · `plugin.rs` + `setup.rs` per D7: render plugin.json, hooks.json and SKILL.md (`SKILL_MD` + `AGY_SKILL_GUIDANCE`); owned-asset manifest, conflicts, unsetup resid · deps: ht-poh.2.3, ht-poh.4.2
        owns: the plugin layout (consumed by ht-poh.6) and the `plugin::status()` API (consumed by L5, L6).
        consumes: L2a (`paths::hook_command`, `AgyRoots`, stubs to fill); ht-poh.2.1 (`SettingsKindSpec` allow-member-only engine).
    - ht-poh.4.5 · Composer policy from model-free TUI captures · Capture `agent read --source detection` for agy states (empty, draft, multi-line, busy, permission prompt). · deps: ht-poh.4.2
        owns: agy `ComposerPolicy` and composer fixtures.
        consumes: L2a (`composer_policy` forwarder and stub file).
    - ht-poh.4.6 · Launch policy and `HERDR_THREADS_AGY_OPTS` · `AgyLaunch` per D8: flag table from the `agy --help` 1.3.2 fixture (`tests/fixtures/agy/help/agy-1.3.2.txt`), refusals, `-c`/`--conversation` conflict, `native_ · deps: ht-poh.2.2, ht-poh.4.2, ht-poh.4.4
        owns: agy `LaunchPolicy`.
        consumes: L2a (launch stub); L3 (`plugin::status()`, `ConfiguredHook`).
    - ht-poh.4.7 · Doctor projection and text · `doctor.rs` per D10: projection from L3 status, witness-health scan (bounded, read-only), sandbox note, limitations, safe/manual repairs, `doctor_text` Debug an · deps: ht-poh.2.3, ht-poh.2.4, ht-poh.4.3, ht-poh.4.4
        owns: agy doctor projection and text.
        consumes: L3 (`plugin::status()`, status projection); L2b (witness helpers: `AgyRoots` data paths, listing check fn).
    - ht-poh.4.8 · Docs, README rows, handoff check · README harness table row and registry sentence (`README.md:114-128`); `integrations/agy/README.md` (operator guide: setup, plugin enable, allow entry, rewrites, · deps: ht-poh.4.4, ht-poh.4.6, ht-poh.8
        owns: agy docs rows.
        consumes: L3 (setup behaviour to document); L5 (`LaunchPolicy`, `prepare_startup_input`); ht-poh.8 (bootstrap plan V2).
  - ht-poh.5 · agy HookPull delivery: transcript confirm, pull, systemMessage inject, Stop downgrade · Spec §3 Delivery/Downgrade/Output discipline (R4). · deps: ht-poh.3.1, ht-poh.4.1, ht-poh.4.2, ht-poh.4.3
      owns: agy delivery behaviour.
      consumes: HookPull wire and DeliveryPolicy trait, agy adapter module. Files: src/harness/agy/delivery.rs, src/harness/agy/*, tests/fixtures/agy/.
      consumes: offered-ledger confirm semantics (DeliveryConfirm outcomes, re-offer bounds) owned by ht-poh.3.4; the D4 frame text via DeliveryPage owned by ht-poh.3
  - ht-poh.6 · Test infrastructure: agy fake Gemini server + native gate, Gemini↔OpenAI relay + live smoke script (epic) · Spec §5 (R12). · deps: none
      owns: fake-server scenario format, relay.
      consumes: agy hook argv contract (for flows), not needed to build the server/relay themselves. Files: scripts/agy-fake-gemini/*, scripts/agy-relay/*, scripts/ag
    - ht-poh.6.1 · Fake Gemini server, scenario format `agy-fake-gemini/1`, CI line for offline agy-script tests · Grow `harness-fake_server.py` into `server.py` plus `scenario.py` per D2: routes, reply kinds, `delay_ms` and chunked SSE, match routing, `{var.*}`/`{frame.*}`  · deps: none
        owns: scenario format `agy-fake-gemini/1`, the `requests.jsonl` schema, and the `system_messages`/`user_requests`/`frames` helpers.
        consumes: none.
    - ht-poh.6.2 · Isolated agy world, native gate runner, `boot` flow, `fetch-agy` · Implement `agyenv.py` per D4/D5/D8: root via `ih_root_new`, seed settings, sandbox profile, binary staging (`cp -c`) and wrapper, `restage_agy`, isolated Herdr  · deps: ht-poh.6.1
        owns: the `World`/`AgyPane`/`Flow`/`ModelBackend` API and the observation primitives.
        consumes: L1 server CLI and scenario format.
    - ht-poh.6.3 · Gemini↔OpenAI relay with enforced budget, plus the OpenAI fake upstream · `translate.py` (D9: joined system parts, `functionResponse` to `tool` with id pairing, preserved or aliased Gemini ids, fresh unique emitted ids, thought and si · deps: none
        owns: the relay CLI and run-dir files (`port`, `local-token`, `budget.json`, `relay.jsonl`) and the fake upstream script format.
        consumes: none.
    - ht-poh.6.4 · Gate flows: setup, permission, allow-rewrite, unsetup, launch · Flow modules and scenarios for the D7 rows `setup`, `permission`, `allow-rewrite`, `unsetup` and `launch`, with flow-local DB/CLI queries. · deps: ht-poh.4.4, ht-poh.4.6, ht-poh.4.7, ht-poh.4.8, ht-poh.6.1, ht-poh.6.2, ht-poh.8
        consumes: the L2 World/Flow API; L1 scenario format; ht-poh.4.4 (owned plugin, allow member, `setup-status` JSON shape); ht-poh.4.6 (launch policy and refusal t
    - ht-poh.6.5 · Gate flows: enroll, subagent, `/clear`, resume · Flow modules and scenarios for the D7 rows `enroll`, `subagent` (the `invoke_subagent` shape from research B, with match-routed child and parent steps), `clear` · deps: ht-poh.4.2, ht-poh.4.3, ht-poh.4.4, ht-poh.6.1, ht-poh.6.2
        consumes: the L2 API; L1 matching and templates; ht-poh.4.2 (hook argv, decode/encode), ht-poh.4.3 (witnesses, lifecycle reading, enrollment), ht-poh.4.4 (insta
    - ht-poh.6.6 · Gate flows: delivery with transcript confirm, and the forced downgrade · `flows/delivery.py` (lazy, ordinary, ACK-required and truncated items; a held inv0 to prove nothing settles before confirm; ACK via `run_command` and the allow  · deps: ht-poh.3.4, ht-poh.4.7, ht-poh.5, ht-poh.6.1, ht-poh.6.2
        owns: `step_mutator.py`.
        consumes: the L2 API (including `restage_agy`); L1 hold/bind/templates; ht-poh.5 (pull/confirm/inject, Stop downgrade keyed on executable identity, doctor downg
    - ht-poh.6.7 · Gate flow: idle wake into the composer · `flows/wake.py`, D7 row `wake` (a)–(e): wake on an ACK-required message into an empty composer with in-request delivery; no wake for lazy-only, busy (`delay_ms` · deps: ht-poh.3.4, ht-poh.3.6, ht-poh.4.5, ht-poh.5, ht-poh.6.1, ht-poh.6.2
        consumes: the L2 API (`draft`, `read_detection`); L1 `delay_ms`; ht-poh.4.5 (agy ComposerPolicy), ht-poh.5 (woken-turn delivery), ht-poh.3.4 (pull routing), ht-
    - ht-poh.6.8 · Live smoke runner (`--dry-run` / `--live`) and evidence README · `RelayBackend` (in its own module), `live_smoke.py` and `scripts/agy-live-smoke` per D11: clean-SHA rule, CI guard, a flow of enroll, then wake, `systemMessage` · deps: ht-poh.3.4, ht-poh.3.6, ht-poh.4.3, ht-poh.4.4, ht-poh.4.5, ht-poh.5, ht-poh.6.2, ht-poh.6.3
        owns: the evidence record format.
        consumes: the L2 World and `ModelBackend` API; the L3 relay CLI, run-dir files and `fake_upstream`; ht-poh.4.3/4.4 (enroll, install), ht-poh.5 (delivery), ht-po
    - ht-poh.6.9 · Final-SHA evidence: full native gate + live smoke, commit records · R12 evidence at the final code SHA. · deps: ht-poh.10, ht-poh.6.4, ht-poh.6.5, ht-poh.6.6, ht-poh.6.7, ht-poh.6.8
  - ht-poh.7 · AGENTS.md harness-neutral rule + delivery contract, and delivery-contract audit with inline fixes · Spec §1 and §Delivery contract (R8 text, R10). · deps: none
      owns: AGENTS.md contract text, audit report. Accounting (R13): every commit carries an 'Outside-adapter: <none | list>' trailer naming each change outside src/h
  - ht-poh.8 · Bootstrap plan V2: adapter-neutral retained composition for handoff · Finding from ht-poh.2 D16 (R7, R13): the frozen V1 bootstrap grammar (harness/launch.rs compose_bootstrap_v1_argv ~:266-281; cli/handoff.rs bootstrap_v1::retain · deps: none
      owns: bootstrap plan V2 format. Outside-adapter: core (records the out-of-adapter cost for R13). Files: src/harness/launch.rs, src/cli/handoff.rs, src/store/top
  - ht-poh.9 · Outside-adapter accounting tool: scripts/outside-adapter-report · R13. · deps: none
      owns: outside-adapter report format.
  - ht-poh.10 · Gate: fix loop exited · Released by super-auto when the phase-5 code fix loop exits (final code SHA). · deps: none
