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
(no spec)
summary: Spec §1 table (R8).

### ht-poh.3 — Delivery policy: src/delivery extraction, DeliveryPolicy trait, Claude PushChannel re-fit, HookPull requests, wake_on + SKILL section
(no spec)
summary: Spec §2 (R4 core side, R9, R11).

### ht-poh.4 — agy adapter (minus delivery): metadata, owned plugin + skill, owned allow entry, local evidence + decode/witness, context, composer, launch, doctor
After this subtree, the core (cli, client, service, daemon, store, host, protocol, app and the neutral `src/harness/*.rs` files) names no harness except in permanent allowlist entries that give a frozen-wire, history or registration reason. `scripts/check-harness-neutral` reports zero `to-migrate` entries. Claude, Codex and Hermes behave as before: wire, JSON, hook output and setup/doctor reports stay byte-identical, and the existing tests guard them. The first leaf lands every new adapter hook with an inert, neutral default. That way the agy adapter (ht-poh.4) can be written entirely inside `src/harness/agy/` plus its `builtins()` row.
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
  - ht-poh.1 · Neutral rule seam: AGENTS.md harness-neutral rule + delivery contract, check-harness-neutral + allowlist, contract audit · Spec §1 (R8) — seam: scripts/check-harness-neutral (shell) failing on harness id literals / Harness::<Name> / harness-named types or fields outside src/harness/ · deps: none
  - ht-poh.2 · Core sweep: move every production harness-specific site behind the adapter (empty all to-migrate allowlist entries) (epic) · Spec §1 table (R8). · deps: none
    - ht-poh.2.1 · Adapter trait hooks seam · Lands every D1 hook with a neutral default, plus the `agent_env_markers` metadata field (all 48 literals in src and tests). · deps: ht-poh.1
        owns: `HarnessAdapter` hooks (`text`, `default_config_root`, `settings_kind`, `foreground_advice`, `setup_status_extras`, `setup_report_lines`, `doctor_text`, `
        consumes: `check-harness-neutral` and the allowlist format and `to-migrate` entries (ht-poh.1); the adapter-owned path rule of D14 (ht-poh.1).
    - ht-poh.2.2 · CLI help, env markers, launch/handoff surface · D2, D3 and the L2 rows of D15. · deps: ht-poh.2.1
        consumes: L1 `AgentEnvMarker`/`Registry::{env_markers,person_guard_markers}`, `AdapterText` (`setup_*`, `launch_notes`, `config_root_hint`).
    - ht-poh.2.3 · Setup engine and setup CLI de-harnessed · D4, D5, D6 (callers), D8 and the L3 rows of D15. · deps: ht-poh.2.1
        consumes: L1 `default_config_root`, `settings_kind`/`SettingsKindSpec`, `foreground_advice`, `setup_status_extras`, `setup_report_lines`, `AdapterText.trust_rem
        owns: the stable legacy helper signatures above, which L4 consumes unchanged.
    - ht-poh.2.4 · Doctor via adapter projections and text · D7. · deps: ht-poh.2.1
        consumes: L1 `doctor_text`, `DoctorTextForm`, `doctor_summary_claims`, the forwarder modules.
    - ht-poh.2.5 · Daemon, store, app and health neutral · D11, D12 and the L5 rows of D15. · deps: ht-poh.2.1
        consumes: L1 `AdapterText.version_history_note`; existing `legacy_contract_id()`.
    - ht-poh.2.6 · Hook runner, contract, attribution and evidence neutral · D9, D10 and the L6 rows of D15. · deps: ht-poh.2.1
        consumes: L1 `version_syntax`/`VersionSyntax`, `transcript_version`/`TranscriptVersionSpec`.
  - ht-poh.3 · Delivery policy: src/delivery extraction, DeliveryPolicy trait, Claude PushChannel re-fit, HookPull requests, wake_on + SKILL section (epic) · Spec §2 (R4 core side, R9, R11). · deps: none
    - ht-poh.3.1 · Delivery seam: `DeliveryPolicy` trait, registry lookup, `DeliveryPull`/`DeliveryConfirm` wire, inert handlers · **Seam for ht-poh.5.** Add `DeliveryMechanism`, `IdleWake`, the `DeliveryPolicy` trait and `HarnessAdapter::delivery_policy()` (default `None`) (DP1). · deps: ht-poh.1
    - ht-poh.3.2 · Extract `src/delivery/` from `watch` and port the D4 frame to Rust · Move-only extraction (DP2). · deps: ht-poh.7
    - ht-poh.3.3 · Claude `PushChannel` re-fit: every Claude check becomes a policy lookup · DP3 and the DP7 idle-wake part. · deps: ht-poh.1, ht-poh.3.1, ht-poh.7
    - ht-poh.3.4 · HookPull handlers: `DeliveryPull`/`DeliveryConfirm`, offered ledger, pull-seat routing · Replace L1's inert arms with DP5/DP6: `src/delivery/ledger.rs` (pure bounded ledger) and `src/service/delivery_pull.rs` (Mutex owner, store-backed `PageSource`, · deps: ht-poh.3.1, ht-poh.3.2, ht-poh.3.3, ht-poh.7
    - ht-poh.3.5 · `wake_on` preference: migration, store, wire and `herdr-threads wake-on` · DP8 storage, wire, store and CLI. · deps: ht-poh.1
    - ht-poh.3.6 · Core wake decision honours `wake_on: all`, plus SKILL section · DP8 core wake. · deps: ht-poh.3.5, ht-poh.7
    - ht-poh.3.7 · Push-channel promotion of `wake_on: all` lazy rows · DP8 promotion. · deps: ht-poh.3.2, ht-poh.3.5
  - ht-poh.4 · agy adapter (minus delivery): metadata, owned plugin + skill, owned allow entry, local evidence + decode/witness, context, composer, launch, doctor (epic) · Spec §3 except the HookPull delivery part (R1, R2, R3, R5, R6, R7). · deps: ht-poh.1
    - ht-poh.4.1 · Neutral hook seams: `local_evidence` + witness rule, `hook_step` · Lands D4 and D6, inert by default. · deps: ht-poh.2.1
    - ht-poh.4.2 · agy module skeleton, metadata, registration, contract, decode/encode, hook argv · D1 skeleton: `mod.rs` with every trait method forwarding to stub submodules (setup/status/unsetup refuse or report not installed; `launch_policy` returns a stub · deps: ht-poh.2.1
    - ht-poh.4.3 · Witness evidence, lifecycle reading, enrollment, TRUST-POLICY startup text · `witness.rs`: D5 `local_evidence` (presence lock, the listing over bound parent and bounded glob of 65,536 entries plus deadline, K hint, lifecycle reading) and · deps: ht-poh.4.1
    - ht-poh.4.4 · Owned plugin dir, skill, allow entry: setup/status/unsetup · `plugin.rs` + `setup.rs` per D7: render plugin.json, hooks.json and SKILL.md (`SKILL_MD` + `AGY_SKILL_GUIDANCE`); owned-asset manifest, conflicts, unsetup resid · deps: ht-poh.4.2
    - ht-poh.4.5 · Composer policy from model-free TUI captures · Capture `agent read --source detection` for agy states (empty, draft, multi-line, busy, permission prompt). · deps: ht-poh.4.2
    - ht-poh.4.6 · Launch policy and `HERDR_THREADS_AGY_OPTS` · `AgyLaunch` per D8: flag table from the `agy --help` 1.3.2 fixture (`tests/fixtures/agy/help/agy-1.3.2.txt`), refusals, `-c`/`--conversation` conflict, `native_ · deps: ht-poh.4.2
    - ht-poh.4.7 · Doctor projection and text · `doctor.rs` per D10: projection from L3 status, witness-health scan (bounded, read-only), sandbox note, limitations, safe/manual repairs, `doctor_text` Debug an · deps: ht-poh.4.4
    - ht-poh.4.8 · Docs, README rows, handoff check · README harness table row and registry sentence (`README.md:114-128`); `integrations/agy/README.md` (operator guide: setup, plugin enable, allow entry, rewrites, · deps: ht-poh.4.4
  - ht-poh.5 · agy HookPull delivery: transcript confirm, pull, systemMessage inject, Stop downgrade · Spec §3 Delivery/Downgrade/Output discipline (R4). · deps: ht-poh.3.1, ht-poh.4.1, ht-poh.4.2, ht-poh.4.3
  - ht-poh.6 · Test infrastructure: agy fake Gemini server + native gate, Gemini↔OpenAI relay + live smoke script (epic) · Spec §5 (R12). · deps: none
    - ht-poh.6.1 · Fake Gemini server, scenario format `agy-fake-gemini/1`, CI line for offline agy-script tests · Grow `harness-fake_server.py` into `server.py` plus `scenario.py` per D2: routes, reply kinds, `delay_ms` and chunked SSE, match routing, `{var.*}`/`{frame.*}`  · deps: none
    - ht-poh.6.2 · Isolated agy world, native gate runner, `boot` flow, `fetch-agy` · Implement `agyenv.py` per D4/D5/D8: root via `ih_root_new`, seed settings, sandbox profile, binary staging (`cp -c`) and wrapper, `restage_agy`, isolated Herdr  · deps: ht-poh.6.1
    - ht-poh.6.3 · Gemini↔OpenAI relay with enforced budget, plus the OpenAI fake upstream · `translate.py` (D9: joined system parts, `functionResponse` to `tool` with id pairing, preserved or aliased Gemini ids, fresh unique emitted ids, thought and si · deps: none
    - ht-poh.6.4 · Gate flows: setup, permission, allow-rewrite, unsetup, launch · Flow modules and scenarios for the D7 rows `setup`, `permission`, `allow-rewrite`, `unsetup` and `launch`, with flow-local DB/CLI queries. · deps: ht-poh.4.4, ht-poh.4.6, ht-poh.4.7, ht-poh.6.1, ht-poh.6.2
    - ht-poh.6.5 · Gate flows: enroll, subagent, `/clear`, resume · Flow modules and scenarios for the D7 rows `enroll`, `subagent` (the `invoke_subagent` shape from research B, with match-routed child and parent steps), `clear` · deps: ht-poh.4.2, ht-poh.4.3, ht-poh.4.4, ht-poh.6.1, ht-poh.6.2
    - ht-poh.6.6 · Gate flows: delivery with transcript confirm, and the forced downgrade · `flows/delivery.py` (lazy, ordinary, ACK-required and truncated items; a held inv0 to prove nothing settles before confirm; ACK via `run_command` and the allow  · deps: ht-poh.3.4, ht-poh.4.7, ht-poh.5, ht-poh.6.1, ht-poh.6.2
    - ht-poh.6.7 · Gate flow: idle wake into the composer · `flows/wake.py`, D7 row `wake` (a)–(e): wake on an ACK-required message into an empty composer with in-request delivery; no wake for lazy-only, busy (`delay_ms` · deps: ht-poh.3.4, ht-poh.3.6, ht-poh.4.5, ht-poh.5, ht-poh.6.1, ht-poh.6.2
    - ht-poh.6.8 · Live smoke runner (`--dry-run` / `--live`) and evidence README · `RelayBackend` (in its own module), `live_smoke.py` and `scripts/agy-live-smoke` per D11: clean-SHA rule, CI guard, a flow of enroll, then wake, `systemMessage` · deps: ht-poh.4.3, ht-poh.4.4, ht-poh.4.5, ht-poh.5, ht-poh.6.2, ht-poh.6.3
  - ht-poh.7 · AGENTS.md harness-neutral rule + delivery contract, and delivery-contract audit with inline fixes · Spec §1 and §Delivery contract (R8 text, R10). · deps: none
  - ht-poh.8 · Bootstrap plan V2: adapter-neutral retained composition for handoff · Finding from ht-poh.2 D16 (R7, R13): the frozen V1 bootstrap grammar (harness/launch.rs compose_bootstrap_v1_argv ~:266-281; cli/handoff.rs bootstrap_v1::retain · deps: none
