# agy harness adapter on a harness-neutral core

## Goal

A Google Antigravity CLI (`agy`) session in a Herdr pane is a full herdr-threads seat, on a par with Claude Code. It enrolls on its own, gets its context, receives message bodies directly between model calls, is woken when idle, and supports `launch`/`handoff`.

agy gets there as just another `HarnessAdapter`, on a core that names no harness. Claude's v0.5.0 mod delivery is re-fitted onto the same interface, a check enforces the rule, and the final report measures every change made outside the adapter.

Interactive root design (Mode A) with the user, 2026-10-09. Inputs:
- Research: `~/Documents/Agy_Integration_Research_20261008/report.md` and `work/*/findings.md`.
- Toolcall/skills probe evidence: `/private/tmp/claude-501/-Users-alepar-AleCode-herdr-threads/e411a093-57e8-4332-840c-ec519a3f8779/scratchpad/probe-toolcall/runs/`.
- Decision log: [design-decisions.md](design-decisions.md).
- Normative: [TRUST-POLICY.md](../../../../TRUST-POLICY.md).
- Prior designs: [harness adapters](../2026-10-03-harness-adapters/2026-10-03-harness-adapters-design.md), [Claude mod inbound delivery](../2026-10-09-claude-mod-inbound-delivery/2026-10-09-claude-mod-inbound-delivery-design.md) (v0.5.0), [versionless admission](../../specs/2026-10-05-harness-versionless-design.md).

## Requirements

- R1. A bare `agy` started in a Herdr pane enrolls a seat on its first prompt, through globally installed hooks (no launch required), like Claude and Codex.
- R2. A subagent's conversation never ends, moves or replaces the parent's binding. A `/clear` rebinds. The difference is decided in the daemon from on-disk witnesses the hook reports, failing toward "child".
- R3. Resume (`agy -c` / `--conversation`) keeps the seat through C1 cooperative continuity.
- R4. Pending message bodies (ordinary and lazy) reach the agy model directly between model calls, framed as untrusted peer data, without the agent running `herdr-threads inbox`. Receipts settle only after the session's own transcript shows the delivery.
- R5. An idle agy with pending nudge-class attention is woken through Herdr into an empty composer, and only then.
- R6. `herdr-threads setup agy` / `unsetup agy` install and remove, reversibly and with recorded ownership:
  - an owned plugin directory (hooks and skill);
  - one owned `permissions.allow` entry.

  `setup-status` and `doctor` report every degraded condition listed in §Doctor.
- R7. `launch --kind agy` and `handoff … --kind agy` start agy under the guarded launch, with validated arguments.
- R8. Core code (cli, service, daemon, store, host, protocol, app) names no harness. Every harness-specific behaviour sits behind `HarnessAdapter` or its policy traits in `src/harness/<id>/`. `scripts/check-harness-neutral` enforces this in the per-change checks and in CI, with an allowlist whose entries each carry a reason.
- R9. Claude's v0.5.0 mod delivery runs through the new `DeliveryPolicy` with byte-identical wire and behaviour.
- R10. AGENTS.md documents the harness-neutral rule and the core delivery/wake contract (§Delivery contract). The current code is audited against the contract, and divergences are recorded and fixed.
- R11. A seat can opt in at runtime to being woken by any message, lazy included (`wake_on: all`). The herdr-threads SKILL explains how to enable it and when.
- R12. Testing, in three tiers:
  - fixture tests in CI (no binary);
  - an opt-in model-free native gate against the genuine agy binary and a scripted fake Gemini server;
  - a budgeted live smoke through a relay to OpenRouter, run locally only, never in CI.
- R13. The final report states how much code outside `src/harness/agy/` the agy adapter needed:
  - trait extensions;
  - reworked implementations;
  - callers;
  - allowlist delta.

## Background: the agy surface (1.3.2/1.3.3, verified by probe)

- **Hooks.** Global `~/.gemini/config/hooks.json`, and plugin `hooks.json` under `~/.gemini/config/plugins/<name>/`.
  - Every top-level key is a namespace, and one malformed key silently drops the whole file.
  - A hook's cwd is the folder that contains its `hooks.json`.
  - Events:
    - `PreToolUse` and `PostToolUse` (grouped matcher/hooks);
    - `PreInvocation`, `PostInvocation` and `Stop` (flat);
    - an undocumented `SessionStart` that works.
  - The payload has no event-name field, so the event is passed in argv.
  - Common fields: `conversationId`, `workspacePaths`, `transcriptPath`, `artifactDirectoryPath`, `modelName`.
  - `PreInvocation` adds `invocationNum` (reset each user turn) and `initialNumSteps`. `Stop` adds `terminationReason`, `error` and `fullyIdle`.
- **Injection.** `PreInvocation` and `SessionStart` accept `injectSteps`:
  - `ephemeralMessage`: one request only.
  - `userMessage`: a permanent `USER_REQUEST` turn.
  - Undocumented `{"systemMessage":{"systemMessage":"…"}}`: permanent. agy wraps it as `<SYSTEM_MESSAGE> not actually sent by the user … injected by a PreInvocation hook`. It is not shown in the TUI.
  - `toolCall`: broken on 1.3.2 and 1.3.3 (internal_only and deprecated; aborts the turn with `unknown injected step type: <nil>`).
  - A well-formed unsupported step aborts the turn. Malformed output is silently ignored together with any valid sibling steps.
- **Failure behaviour.** `PreInvocation` fails open. `PreToolUse` fails closed: `{}` denies, empty output allows.
- **Lifecycle.**
  - `SessionStart` fires at the first prompt of a new conversation and after `/clear` (alias `/new`), which also creates a new id.
  - `-c` / `--conversation <id>` keeps the id and fires no `SessionStart`.
  - No exit hook.
- **Stores.** `brain/<id>/`, `conversations/<id>.db`, `presence/<id>.lock`, `cache/last_conversations.json`.
- **Subagents.**
  - Hooks fire in children under their own `conversationId`, with no parent field.
  - The parent's `brain/<parent>/.system_generated/subagents/<child>.json` is written about 120 ms before the child's `SessionStart`.
  - A presence lock is created for top-level conversations (`/clear` included, about 2.4 s before its `SessionStart`) and never for a child.
- **Permissions.**
  - A hook allow does not approve a command.
  - Only `~/.gemini/antigravity-cli/settings.json` `permissions.allow` is honoured.
  - On 1.3.2, `command(herdr-threads)` matches whole leading words and refuses `herdr-threadsX` and chained commands.
  - agy rewrites this file itself; accepting the trust prompt dropped an unknown key.
- **Environment.**
  - Hooks inherit agy's environment (`HERDR_PANE_ID` included) plus `ANTIGRAVITY_CONVERSATION_ID`.
  - The model's `run_command` shell also gets `ANTIGRAVITY_AGENT=1`.
- **Skills.** `skills/<name>/SKILL.md` (frontmatter `name`/`description`) in plugins, `~/.gemini/config/skills/` and `.agents/`. Each is listed by name and path in the system prompt, and the model reads the body with `view_file`.
- **Herdr.** Herdr 0.9.1 detects agy by process name (`agy`, `antigravity`, `antigravity-cli`), parses its screen states, and has its own `"herdr"` integration hook that reports `conversationId`. No process hint is needed. The Herdr agent kind is `agy`.

## Delivery contract (core; documented in AGENTS.md)

The goal is token efficiency. Sessions that are done and idle stay idle, spinning sessions get efficient delivery, and nudges are rare, because excessive nudges disorient some models. Every delivery policy must keep these invariants:

1. Messages are lazy by default.
2. Lazy messages never wake an idle session (unless the seat opted into `wake_on: all`, R11).
3. Turn hooks and channels deliver every pending item, lazy or not, directly into the model's context, avoiding extra `herdr-threads` calls as far as the harness allows.
4. A nudge wakes a session only when it is idle. A session that is generating, or running a tool, gets no nudge; it is served by in-turn delivery.
5. Messages that require an ACK are always nudge-class.
6. Who performs an idle wake is the only per-harness variation:
   - `Native`: Herdr types the wake marker into an empty composer;
   - `Channel`: the harness wakes itself, as Claude's mod submits a turn.

The contract audit (R10) checks the current code against each rule before the delivery refactor. Each divergence is fixed in this run or recorded with a reason.

## Architecture

### 1. Harness-neutral core (R8, R10)

**AGENTS.md** gains a "Harness-neutral core" section:
- harness-specific code lives only in `src/harness/<id>/` behind `HarnessAdapter` and its policy traits;
- core code looks the adapter up through the registry by the id from the binding or claim;
- new harness behaviour means a new trait method or policy with a neutral default, never a branch in core.

It also gains the §Delivery contract above.

**`scripts/check-harness-neutral`** (shell, run by the per-change checks and in CI) fails on any of these outside `src/harness/<id>/`, `tests/`, `#[cfg(test)]` modules and the allowlist:
- a harness id string literal;
- a `Harness::<Name>` path;
- a harness-named type or field.

The allowlist `scripts/harness-neutral-allowlist.txt` holds one entry per line, `path:pattern — reason`. Only things that must name a harness forever are allowed:
- frozen wire shapes: v1 `HarnessHealth{codex,claude}`, v1 `HarnessEvidence`/`HookParseFailure`, legacy contract ids, and the `not_claude` refusal code the shipped mod speaks;
- historical migrations;
- the `builtins()` registration list.

The check prints its count, and the report uses the before/after counts.

**The sweep is total.** Every production site the inventory found moves behind the adapter or goes on the allowlist with a frozen-wire or history reason:

| Site (today) | Becomes |
|---|---|
| `cli/mod.rs` `AGENT_ENV_MARKERS`, `cli/output.rs` `HARNESS_MARKERS` | `AdapterMetadata.agent_env_markers` (Claude, Codex, Hermes, agy `ANTIGRAVITY_AGENT`) |
| `cli/mod.rs` `validate_actor_harness` (Human vs Claude/Codex) | `Harness::Agent(_)`; fixes the Hermes gap |
| `store/queries.rs:6310` `harness IN ('claude','codex')` | Registry-derived list; fixes a latent Hermes bug |
| `cli/setup.rs` `config_roots` snapshot (codex/claude only) | Each adapter resolves its own root (`setup_environment_inputs` / `resolve_setup_scope`) |
| `cli/setup.rs` help, `cli/launch.rs` help and text, `cli/handoff.rs` help, `cli/me.rs` marker text | Adapter-supplied help fragments, assembled from the registry |
| `cli/setup/foreground.rs` | Adapter method `foreground_advice` |
| `daemon/harness_states.rs:586` notes | Adapter-provided note |
| `cli/doctor.rs` per-harness text sections and fields | `doctor_projection`; v1 JSON fields allowlisted as frozen |
| `harness/contract.rs` normalize / `contract-id`, `harness/attribution.rs`, `daemon/harness_evidence.rs`, `cli/hook_evidence.rs` | Adapter methods where they serve current behaviour; allowlisted where v1-only |
| `cli/hook.rs` `InstalledHarness`, `report_parse_failure` | Registry-driven; v1 API allowlisted |
| `harness/setup.rs` `SettingsKind::{ClaudeUser,CodexUser}` | Settings kinds supplied by adapters (agy adds its own `permissions.allow` member kind) |
| `harness/registry.rs`/`context.rs` `CODEX`/`CLAUDE` consts, `OccupantHarness` | Kept only inside the registry (allowlisted) or moved to adapter modules |
| `service/dispatch.rs`, `cli/watch.rs`, `protocol/watch.rs`, `service/mod_channels.rs` | §2 |
| `app.rs` named inputs, `trigger_unseen_versions` | Registry iteration |
| `cli/topology_*`, `protocol/handoff.rs`, `ports.rs`, `store/receipts.rs`, `store/control.rs`, `daemon/health.rs` remaining references | Registry/adapter, or allowlisted frozen wire |

Tests that pin the exact registry list are updated to the new list once. Fixture-only `for h in ["claude","codex"]` loops stay.

### 2. Delivery as an adapter policy (R4, R9, R11)

```rust
fn delivery_policy(&self) -> Option<&dyn DeliveryPolicy> { None } // None: hooks + Native wake (Codex, Hermes)

pub trait DeliveryPolicy: Send + Sync {
    fn mechanism(&self) -> DeliveryMechanism;   // PushChannel | HookPull
    fn provenance(&self) -> &'static str;       // cooperative_mod_delivery | cooperative_hook_delivery
    fn idle_wake(&self) -> IdleWake;            // Native | Channel
}
```

- **Neutral `src/delivery/` module.** It moves out of `watch` unchanged:
  - v0.5.0 D4 framing (fixed header, `[herdr-threads] <kind> <id> in <thread> from <sender>[ markers]:` lines, two-space-indented bodies, no leading `/`);
  - the 8 KiB per-message truncation with the `body`/`ack` hint;
  - pages of at most 32 items or 64 KiB;
  - id de-duplication;
  - settlement of ordinary receipts (as `AckDisplayed`) and lazy rows (display-complete).

  It adds an *offered* ledger keyed by `(seat, binding generation, id)` for pull confirmation.
- **`PushChannel`.** Today's `ModChannels` registry, `WatchRequest`/`WatchFrame` and `watch`/`watch ack` with every Claude check replaced:
  - "harness is Claude" becomes "the bound harness's adapter declares `PushChannel`";
  - `watch --harness <id>` is validated through the registry;
  - the internal refusal becomes `NoPushChannel`, and the wire keeps the `not_claude` code (allowlisted) so the shipped mod's exit-code table is unchanged.

  The Claude adapter declares `PushChannel`, `cooperative_mod_delivery` and `Channel`. Mod assets and their setup stay in the Claude adapter.
- **`HookPull`.** Two requests on the ordinary one-shot connection:
  - `DeliveryPull{claim, seen_attention_version}`: returns `Unchanged`, or one framed page, and records its ids as offered to the claim's binding generation.
  - `DeliveryConfirm{claim, ids}`: settles only ids offered to that same generation, under the policy's provenance. Offered-but-unconfirmed ids stay pending and are offered again. A rotated generation (lifecycle check-in) discards its offered set.

  Both are decided against A2: open `cooperative_top_level` binding, matching native session, resolved and not held.
- **Routing.**
  - For a seat whose binding's harness declares a delivery policy, the hook check-in omits digest and ready commands while delivery is active: live channel for push, always for pull.
  - Wake follows the contract. A nudge-class pending item on an idle seat wakes it through `idle_wake()`: `Native` uses the existing wake dispatcher; `Channel` is skipped by the dispatcher while the channel is live, and falls back to `Native` when it isn't.
- **`wake_on` (R11).** A per-seat runtime preference, `wake_on: nudge | all`, set by the seat's top-level agent with a new `herdr-threads wake-on <nudge|all>` command (cooperative top-level action). It is stored on the seat and read by the core wake decision; `all` makes lazy items wake-eligible. The herdr-threads SKILL gains a short section:
  - what it does;
  - how to enable and disable it;
  - when it is appropriate: coordinators and moderators that must react to every message but don't loop constantly. It is not for ordinary workers, because it costs tokens.

### 3. The agy adapter (`src/harness/agy/`)

**Metadata.**
- id `agy`, display label "Antigravity CLI", executable `agy`, host kinds `["agy"]`;
- env marker `ANTIGRAVITY_AGENT`;
- hook admission `RegisteredContract`, contract domain `Native`.

**Hooks (owned plugin).** Setup writes `~/.gemini/config/plugins/herdr-threads/` as owned assets, with a manifest, baseline and generation in the existing setup-manifest machinery; `agy plugin install` is not used. The plugin contains:
- `plugin.json` (name `herdr-threads`, version = binary version);
- `hooks.json` registering `SessionStart`, `PreInvocation` and `Stop`, each invoking the absolute owned `herdr-threads` executable with an `internal hook agy <event>` argv. No `PreToolUse` and no `PostToolUse`.
- `skills/herdr-threads/SKILL.md`: the existing skill text with agy-specific tool guidance (`run_command`) as an adapter `OutputPolicy` fragment.

No `rules/` file. Unsetup removes only owned files. Status reports installed versus enabled from `config/config.json` (`plugins.herdr-threads.enabled`); a disabled plugin is reported, never re-enabled.

**Permission entry.** Setup owns one `command(herdr-threads)` member of the `permissions.allow` array in `~/.gemini/antigravity-cli/settings.json`, through the shared JSON engine's owned-array-member support (the Codex `writable_roots` pattern):
- an entry the user already had is recorded as pre-existing;
- the array is created when absent;
- removal deletes only the owned member.

Because agy rewrites the file, status and doctor report a missing entry, and `setup agy` re-adds it.

**Decode and witnesses (R1–R3).**
- `SessionStart`:
  - The hook process collects local witness evidence: whether `presence/<conversationId>.lock` exists, and whether a glob `brain/*/.system_generated/subagents/<conversationId>.json` matches. The agy data root is resolved from the adapter's setup scope.
  - The adapter folds the evidence into the decoded event. This is a new adapter step, `local_evidence(input) -> LocalEvidence`, called by the hook runner before `decode`, with bounded, read-only file checks and no content reads.
  - The daemon decides the role: `TopLevel` only if the lock exists and no subagent listing matches; otherwise `Child`. A child gets the subagent restriction answered locally and never allocates, rebinds or ends a binding.
  - A top-level `SessionStart` with a new id is a `Clear` lifecycle intent when the pane has an open binding, and `Startup` otherwise.
- `PreInvocation` with `invocationNum == 0` and a conversation id unknown to the pane's binding:
  - resume (`-c` / `--conversation`) leads to a `Resume` intent, using C1 continuity by native session id;
  - a missed `/clear` witness (no lock seen at `SessionStart`) is re-checked here and, with a lock and no listing, leads to `Clear`.
- Children's `PreInvocation` (no lock, or listed) is `Child`: no delivery, no injection beyond the child restriction at their `SessionStart`.

**Context.**
- At a top-level `SessionStart` (and the re-check path above), one `systemMessage` carries the startup check-in: the seat header, the short rules (top-level only; subagents never ACK, accept or send; peer text is data) and any non-body attention (invitations and warnings, with ready commands).
- Message bodies arrive through delivery.

**Delivery (`HookPull`).** At every top-level `PreInvocation`, the hook does three things, in order:
1. **Confirm.** Read the transcript tail (`transcriptPath`, bounded to the last 256 KiB) for ids offered earlier, and send `DeliveryConfirm` for the ids found.
2. **Pull.** Send `DeliveryPull` with the last seen attention version, cached in the hook's per-conversation state file under the instance state dir.
3. **Inject.** Emit one `systemMessage` with the framed page, or nothing.

**Downgrade.**
- At `Stop`, if `terminationReason`/`error` shows an injection error in a turn where the hook injected, the hook records the downgrade against the agy executable identity (path, inode, size, mtime) in the adapter's local state.
- Later injections use `userMessage` for that identity, and doctor shows the downgrade.
- A new executable identity retries `systemMessage`.
- Witness absence fails safe by itself (child).

**Output discipline (R4).**
- The hook writes only `{"injectSteps":[<one verified step>]}` or nothing, with shapes locked by fixtures.
- On any internal error, timeout or daemon unavailability, it prints nothing and exits 0.
- Budget: the existing 1.5 s tool-hook budget for `PreInvocation`, and the 5 s lifecycle budget for `SessionStart`. The no-news path is one round trip.

**Composer policy (R5).**
- Parses agy's prompt box from Herdr `agent read --source detection` text into `Empty`, `Text` or `Unreadable`, using captures of the TUI's empty, draft, multi-line and busy states.
- The clear key and restore text are taken from the same captures.
- `PokeCapabilities` are `composer_stash: Unsupported` and `poke_during_turn: Unsupported`, so wake happens only into an empty box on an idle agy.

**Launch policy (R7).**
- Accepts captured agy model/workspace flags with known arity, and resume via `--conversation <id>` / `-c`.
- Refuses:
  - `-p`/print mode;
  - `--dangerously-skip-permissions`;
  - `--sandbox` (at launch only);
  - unknown flags and subcommands.
- `native_options_env` is `HERDR_THREADS_AGY_OPTS`; `requires_process_hint` is false.

**Doctor.** Reports:
- plugin missing or disabled;
- allow entry missing;
- a `systemMessage` → `userMessage` downgrade in effect;
- `SessionStart` seen without a witness;
- the `--sandbox` socket limit (informational; a bare `agy --sandbox` is not refused).

**Version handling.** Admission follows the versionless policy: no version probe as a prerequisite, `contract_declared`. There is no version ladder for injection; the Stop-hook downgrade covers breaking releases.

### 4. TRUST-POLICY changes (same commits as the code)

- **Startup enrollment.** Extend "Global hooks enroll supported top-level Herdr sessions" to agy. The agy top-level role is decided from the on-disk presence-lock and subagent-listing witnesses the hook reports, failing toward child. This is the honest name for the evidence: a native runtime artifact, decided in the daemon. It is not payload-declared.
- **New provenance `cooperative_hook_delivery`** (receipt action observation; lazy completion claim): the pane's top-level session's own hook injected the framed item, and a later hook call of the same binding generation found the item's id in the session's transcript. It is a cooperative delivery claim, not proof the model read it. A truncated item never carries it.
- **`cooperative_mod_channel` / `cooperative_mod_delivery`.** Reword "harness `claude`" to "a harness whose adapter declares a push delivery channel". Semantics are unchanged.
- **`wake_on`.** The seat preference is a cooperative top-level setting. It changes only wake eligibility; never receipts or authority.
- **Accepted limits:**
  - a child conversation that agy *does* give a presence lock would read as top-level, but the subagent listing still makes it a child;
  - a transcript the hook cannot read leaves items pending, never settled.

### 5. Testing (R12)

1. **Fixtures (CI).**
   - Recorded agy 1.3.2 hook payloads for every event, including the child `SessionStart` vs `/clear` vs resume trio.
   - On-disk layouts (presence locks, subagent listings) and transcript tails.
   - TUI composer captures.
   - Tests cover:
     - decode, local evidence and the daemon role decision;
     - encode output shapes, byte-locked;
     - pull/confirm/settle, the Stop downgrade, setup/status/unsetup (plugin and owned allow entry, pre-existing and rewritten-by-agy cases), the composer parser and launch argv;
     - the check script's own tests;
     - Claude mod regression (the existing v0.5.0 tests unchanged).
2. **Native gate (opt-in, model-free).** When `HT_AGY_BIN` is set, it runs the genuine agy binary under an isolated HOME and loopback sandbox against `scripts/agy-fake-gemini/` (Python, grown from the probe harness: a scripted Gemini-native server). It covers:
   - enroll, `/clear`, a subagent, resume;
   - delivery with transcript-confirmed settlement;
   - the downgrade path (forced by a scripted unsupported step);
   - the permission entry, and unsetup.

   It runs in the TUI through a private tmux and an isolated named Herdr test session for wake. It is not part of the default suite and does not count against the 5-minute budget.
3. **Live smoke (local only, never CI).** `scripts/agy-relay/` is a thin Gemini↔OpenAI relay that replaces LiteLLM plus the shim:
   - user role for function responses;
   - unique tool ids;
   - joined system parts;
   - streaming.

   It forwards to OpenRouter `google/gemini-2.5-flash`. The relay enforces a cap of 40 requests and $0.50 per run, and reads the key from `~/.hermes/.env` locally. The smoke proves a real model reads a delivered message and acts on it with `herdr-threads` (ACK/reply), plus one idle wake in an isolated Herdr session. The orchestrator runs it once at the final SHA. The record goes to `docs/evidence/agy-live-smoke/`.

### 6. Adapter-model accounting (R13)

Each bead's implementer records, in its commit message trailer `Outside-adapter: <none | list>`, every change outside `src/harness/agy/`. The integration sweep and the run report aggregate:
- trait extensions (`delivery_policy`, `local_evidence`, help/doctor/foreground fragments, env markers);
- reworked implementations (Claude mod re-fit, sweep moves);
- callers touched;
- `check-harness-neutral` counts before and after.

Target: once the neutral core lands, the agy adapter needs zero edits outside `src/harness/agy/` beyond its `builtins()` row, its TRUST-POLICY text and its docs rows. Anything else is a finding.

## Decomposition guidance (top split)

1. **Neutral rule seam:** AGENTS.md sections, `check-harness-neutral` and its allowlist (initially listing every current site as `to-migrate`), and the delivery-contract audit. Goes first.
2. **Core sweep:** §1's table, split by area (cli setup/help/markers; doctor; store/daemon; contract/attribution/evidence; app/host/topology). Each area empties its `to-migrate` entries. Runs in parallel.
3. **Delivery policy:** the `src/delivery/` extraction, `DeliveryPolicy`, Claude `PushChannel` re-fit, `HookPull` requests, `wake_on` and the SKILL section.
4. **agy adapter:** metadata, plugin setup, owned allow entry, skill, decode and witnesses, context, composer, launch, doctor, TRUST-POLICY startup text. Needs only 1; runs alongside 2 and 3.
5. **agy `HookPull` delivery:** confirm/pull/inject, the Stop downgrade, provenance. Needs 3 and 4.
6. **Test infrastructure:** the fake Gemini server and native gate, the relay and the live-smoke script. Runs alongside; the gate's flows need 4 and 5.

The final live smoke is gated on the fix loop's exit (super-auto §Gates).

## Refinements from the nested designs (2026-10-09)

- **Role decision placement (agy-adapter subspec D4).** The harness-neutral witness rule `RoleWitness::decide`, which fails toward child, is applied in the hook runner over facts that the agy adapter maps from files (`local_evidence`). The resulting role travels as the ordinary cooperative `claim.role`, exactly as payload-declared roles do for Claude and Codex. The daemon decides every enrollment, rotation and reattachment against A2, and there is no wire change. TRUST-POLICY states this honestly. It refines "the daemon decides" in R2 and §3.
- **Hook argv** is the existing registered form `hook agy --event <SessionStart|PreInvocation|Stop>`, not `internal hook agy <event>`.
- **`agy -c` is `--continue`** (the most recent conversation), not an alias of `--conversation`. TUI `Stop` reports `terminationReason: "NO_TOOL_CALL"`.
- **Second neutral seam, `hook_step` (R13 finding).** The delivery leaf has no core files, so the runner gains a neutral `HookStep` seam for top-level observer events. It provides a claim hint, a one-shot daemon caller and the adapter's private state dir.
- **Bootstrap plan V2 (R13 finding, core-sweep subspec D16).** The frozen V1 handoff bootstrap admits only Claude and Codex, so `handoff --kind agy` needs an adapter-neutral V2 (bead ht-poh.8).
- **`contract::classify` needs a payload discriminator (R13 note).** agy has none, so the adapter injects a synthetic `$event` key and core is not edited.

## Non-goals and follow-on

**Out of scope:**
- Hermes global enrollment (bead ht-4ne, separate tab).
- Live tests for the other harnesses (bead ht-1hn).
- Delivery policies for Codex and Hermes (they keep hooks and wake).
- Linux native validation.
- agy with a real Google login.
- `--sandbox` socket support.
- agy's `agentapi send-message` as a wake channel.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
