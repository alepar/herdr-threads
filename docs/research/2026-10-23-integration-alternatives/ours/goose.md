# herdr-threads x Goose (Block / AAIF)

How herdr-threads itself handles message injection and waking for Goose. Goose is **not supported** today and is
on the "hoped for" list. So this report has two halves. The first describes what our code actually does when a
Goose agent sits in a Herdr pane; the answer is mostly "nothing, and here is the code path that refuses". The
second half covers what a Goose adapter would need from our traits and which Goose surfaces fit.

Sources and tags:
- herdr-threads code at `main` = `2de3da1c4` (paths are repo-relative, `file:line`).
- Goose facts come from the primer `docs/research/2026-10-23-integration-alternatives/primers/goose.md`
  (cited as "primer §N"). It cites two Goose trees: `goose@1.36` (installed, tag `v1.36.0`) and `goose@main`
  (`c3bfc5f`, 1.56.0), <https://github.com/block/goose>. I re-read the key Goose claims in the clones
  (`tom.rs`, `ops_stop_hook.rs`, `developer/shell.rs`, the `hooks/` grep) and they hold.
- Herdr facts come from the installed `herdr 0.9.5` help output (`--help` only, which touches no server state)
  and from repo docs.
- **VERIFIED** means read in source or docs, or observed in help output, at the cited place. **INFERRED** means my
  reading of verified pieces. **unknown** means not determined.

## Support status

**Not supported. No code path recognizes Goose.** VERIFIED:

- `builtins()` registers exactly Claude, Codex, Hermes and agy, plus a `test-support`-only synthetic fourth
  adapter (`src/harness/registry.rs:1482-1498`).
- There is no `src/harness/goose*` and no `integrations/goose/` (`ls src/harness integrations`).
- `grep -ri goose src integrations` finds nothing outside this research directory, and `bd search goose` finds no
  beads.
- `herdr-threads hook goose` is a usage error, because the hook entrypoint resolves the id through the registry
  and rejects unknown ids (`src/cli/hook.rs:217-224`).
- `setup`, `launch` and `handoff --kind goose` fail the same registry lookup (`Registry::agent`,
  `src/harness/registry.rs:1444-1454`, returns `UnknownAgent`).
- **Herdr itself does not know Goose either.** `herdr agent start --help` (0.9.5) lists the supported kinds
  `pi, claude, codex, gemini, cursor, devin, agy, cline, omp, mastracode, opencode, copilot, kimi, kiro, droid,
  amp, grok, hermes, kilo, qodercli, qwen, letta, maki, muse`, with no `goose`. `strings $(command -v herdr) |
  grep -ci goose` gives `0`. VERIFIED (observed).
  - This matters more than our own gap. Every wake and composer path we have keys on Herdr's `agent` kind for the
    pane (see the wake section), and Herdr reports none for a Goose pane. INFERRED from argv0-based detection,
    which `docs/superpowers/runs/2026-10-09-hermes-global-enrollment/bare-hermes-detection-evidence.md:28-33`
    documents for Hermes, together with the missing kind.

What a Goose user can do today: tell the Goose model to run `herdr-threads ...` from its shell tool. That call is
attributed as a **person**, not an agent (see "Identity" under Known limitations).

## Injection mechanism

### What happens today: no automatic injection

| Path we use for other harnesses | Code | Goose today |
|---|---|---|
| Push channel (Claude mod: `watch` stream into a channel, `frame()` text) | `integrations/claude/mod/hooks/register.js:10,58`; `src/harness/claude/delivery.rs:8-18` | Absent. Goose has no channel-like surface: MCP notifications reach the UI only, and only during a tool call of that server (primer §6). VERIFIED absent in our code; INFERRED impossible natively. |
| Hook pull (agy `PreInvocation` `systemMessage` with a `DeliveryPull`/`DeliveryConfirm` ledger) | `src/harness/agy/delivery.rs:1-13,69-82` | Absent. Goose hooks have **no context-bearing output field** in either version (primer §3.4; I re-checked `goose@main crates/goose/src/hooks`, which has no `additionalContext`). |
| Hook digest / ready commands at lifecycle and tool boundaries (Codex, Claude `hookSpecificOutput.additionalContext`) | `src/cli/hook.rs:1-21`; `src/delivery/routing.rs:21-30` | Absent. `hook goose` is a usage error (`src/cli/hook.rs:217-224`). |
| Text `inbox` run by the model (ACK on display) | `integrations/skill/SKILL.md:34,64,70`; `src/protocol/output_compact.rs:1-15` | **The only path.** It needs the model to call it, and the call is attributed as a human (Known limitations). |

So for Goose:

- **Payload:** none is pushed. What the model sees is whatever `herdr-threads inbox` prints when it chooses to run
  it: compact rows, peer text after the first `": "`, bodies indented two spaces (`src/protocol/output_compact.rs:7-15`).
- **Timing:** only when the model calls the CLI. It is never at turn start, never after a tool call, and never
  without a model decision.
- **Bounds:** the ordinary inbox page bounds and `next:` continuation (`integrations/skill/SKILL.md:79`). The
  framed hook/channel page bounds (32 blocks, 64 KiB, `src/service/delivery_pull.rs:37,40`) and the 8 KiB
  watch body cut (`src/protocol/watch.rs:63`) do not apply, because no framed page is ever built for Goose.
- **Framing:** the per-message frame `[herdr-threads] new messages:` (`src/delivery/frame.rs:10`) is not used.
  The inbox rows are the framing.
- **Dedup and ACK:** text `inbox` ACKs only fully displayed pending receipts, after write and flush
  (`integrations/skill/SKILL.md:34`). Under Goose, though, the caller is the pane's person seat if one exists,
  so any ACK is recorded as `operator_human` provenance (`TRUST-POLICY.md:307`), not agent receipt. INFERRED
  from `src/cli/caller.rs:74-80`.
- **Skill:** the ~48.7 KB `integrations/skill/SKILL.md` is installed only into the supported harnesses. Goose
  never learns the protocol unless a human pastes it. Goose *does* load `skills/*/SKILL.md` from plugins
  (primer §4), so this is a packaging gap, not a capability gap. VERIFIED (no goose packaging); capability per
  primer.

## Wake mechanism

### Our wake pipeline, traced for a Goose pane

1. **Decision (harness-neutral, works for any seat).** `wake_frontier` admits priorities `0..=T`, or all of them
   when `T < 0` (`src/store/lazy_delivery/wake.rs:42-50,68-82`). `AttentionSnapshot::select` adds invitations,
   receipts and warnings (`src/notification/policy.rs:71-90`). Goose is never reached in practice, though,
   because a Goose pane has no agent binding: no hook enrolls one, and the CLI calls run as Human.
2. **Target and identity.** A cooperative wake needs a bound harness on the reservation. Without one it refuses
   before sending: `"no bound harness ... a pre-send refusal"` (`src/notification/dispatch.rs:325-333`).
3. **Final recheck before typing** (`src/host/native.rs:1784-1928`, `cooperative_wake_ready` at `:1682-1735`).
   It requires all of the following:
   - Herdr `agent get` reports an `agent` kind that the registry maps (`registry.by_host_kind`, `:1693-1699`)
     and that matches the bound harness (`:1700-1719`).
   - `agent_status` is in `WAKE_READY_STATUSES = ["idle", "done"]` (`:53,1725-1734`).
   - The composer read through the bound harness's `ComposerPolicy` is `Empty` (`:1876-1878`).
   - A focused pane's composer was seen empty for `FOCUSED_EMPTY_MILLIS = 60_000` (`:71,1744-1766,1880-1884`).
   - Only then does it run `herdr agent prompt <pane> <text>` (`:1896-1900`).
4. **Composer.** `read_composer_in` returns `Unreadable` for any harness with no `composer_policy`
   (`src/harness/composer.rs:58-73`). `observed_ui` maps `Unreadable` to `Unknown` (`:163-175`), and every wake
   and poke path skips `Unknown`. The status-only fallback also gives `Unknown` when Herdr reports no agent kind
   (`src/host/native.rs:2039-2048,2354-2366`).

**Result for Goose today: no wake is possible.** Each of steps 2, 3 and 4 would refuse independently. VERIFIED
per gate; INFERRED that all of them fire for a Goose pane, because Herdr reports no kind and no Goose adapter
exists.

- **Who types what:** nobody. If a Goose adapter existed with no delivery policy, the native wake would type
  `MARKER` = `herdr-threads: attention pending; run herdr-threads inbox` (57 bytes). That is the default
  `wake_text` for a harness whose turn start injects nothing (`src/harness/adapter.rs:466-471`,
  `src/notification/policy.rs:14`, `src/harness/registry.rs:1419-1432`). The content-free `herdr-threads: wake up`
  (22 bytes, `policy.rs:17`) is reserved for policies whose turn start delivers (R18), and Goose has no such hook
  output.
- **Idle and busy detection:** none for Goose. Herdr would have to classify the pane, and it does not know Goose.
  Herdr 0.9.5 does have a self-report API:
  `herdr pane report-agent --source ID --agent LABEL --state idle|working|blocked|unknown PANE_ID` (observed
  `--help`). A Goose `UserPromptSubmit` → `working` / `Stop` → `idle` hook pair could drive it (primer §3.2,
  §10).
  - **unknown:** whether Herdr accepts an unlisted label such as `goose`, and whether a report sets `pane.agent`.
    Our host reads `pane.agent` / `agent get .agent`. The Hermes probe found that `report-agent-session` set only
    `agent_session` and left `pane.agent` absent and `agent_status` `unknown`
    (`bare-hermes-detection-evidence.md:20-26,43-49`). That probe was on `report-agent-session`, not
    `report-agent`.
- **Draft safety:** for Goose it rests entirely on a future `ComposerPolicy`. The rustyline prompt is `"> "` with a
  dim `Enter to send · Ctrl+J newline` hint when empty (primer §9).
  - Our stash/restore poke would need `clear_key`, and Ctrl+C in Goose **kills the whole buffer** and, a second
    time, exits (primer §9). So a Goose `ComposerPolicy` must never declare `composer_stash`. `poke_eligibility`
    then skips typed input (`src/notification/policy.rs:390-393`). INFERRED.
- **Approval and elicitation prompts:** our `ApprovalOrQuestion` exclusion depends on Herdr reporting `blocked`
  (`src/harness/composer.rs:165`, `src/host/native.rs:2354-2362`). With no Herdr detection, a Goose tool-approval
  select would read as `Unknown` at best, or as idle-shaped text at worst. INFERRED hazard (primer §9: injected
  keys answer those selects).
- **Latency (hypothetical adapter):** the same as Codex/agy Native once the gates pass. There is the batching
  window (`src/notification/policy.rs:22-28`), a retry ladder of 30 s / 60 s / 120 s / 300 s on failures
  (`:49,264-294`), and a 60 s focused-empty hold (`src/host/native.rs:71`).
- **Failure modes (hypothetical):**
  - Bytes typed while Goose is busy sit in the tty and become the next prompt (primer §9). Our "never type into
    an active turn" rule depends on a reliable busy signal that Goose cannot give through Herdr today.
  - `GOOSE_PROMPT_EDITOR` replaces the composer with `$EDITOR` (primer §9), so a composer reader would see the
    editor, not the prompt.

## Setup footprint

Today: **none**. `setup goose` is an unknown harness. Nothing is written to `~/.agents/plugins`,
`~/.config/goose` or the Goose config. VERIFIED (registry).

What a Goose adapter would install (INFERRED, from primer §3-§5):
- An Open Plugins-format plugin at `~/.agents/plugins/herdr-threads/` (or project
  `.agents/plugins/herdr-threads/`) containing:
  - `hooks/hooks.json`, with `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `Stop` and `SessionEnd` as
    `sh -c` commands calling `herdr-threads hook goose --event ...`;
  - `skills/herdr-threads/SKILL.md`, the existing skill.
- Optionally the `tom` MOIM variables (`GOOSE_MOIM_MESSAGE_FILE`). In 1.36 these are read from the process env
  only, so a managed launch must set them. On main they can also live in the Goose config.
- Hooks load once per agent construction (primer §3.1, INFERRED), so an already-running session needs a restart,
  as with our other hook harnesses.

## Token cost per message and per quiet turn

| | Today | Hypothetical adapter: MOIM file + hooks |
|---|---|---|
| Quiet turn | 0 bytes (nothing installed) | 0 bytes when the MOIM file is empty or absent (`tom` contributes nothing). Hooks print nothing to the model (no context field). |
| Per message | The model must spend one shell-tool call on `inbox`: the call, plus rows, plus at most one `next:` line. There is no wake, so a message waits until the model happens to look. | The page text (≤ 64 KiB, our bound; Goose `tom` caps at 64 KiB per source, `goose@1.36 .../platform_extensions/tom.rs:15`). **1.36:** the block is ephemeral and re-sent with *every provider call* until we clear the file (primer §5), so k tool batches cost k × page. **main:** persisted once per change and reused within the turn. |
| Wake | n/a | `MARKER`, 57 bytes, plus the round trip it triggers: one `inbox` call. `WAKE_UP` only if MOIM delivery is trusted at turn start. |
| Skill | 0 (not installed) | ~48.7 KB once per session, loaded on demand like the other harnesses (`integrations/skill/SKILL.md`, `wc -c`). |

## Known limitations

1. **A Goose agent is attributed as a human.** INFERRED from verified pieces:
   - `caller::derive` maps `EnvRefusal::NoHarness` to `CallerOrigin::Human` when no registered env marker is set
     (`src/cli/caller.rs:74-80`; `src/cli/output.rs:165-177`). The markers are `CLAUDECODE`,
     `CODEX_THREAD_ID`/`CODEX_SANDBOX*` and `ANTIGRAVITY_AGENT` (`src/harness/claude.rs:1007`,
     `src/harness/codex.rs:1459-1467`, `src/harness/agy/mod.rs:51`).
   - Goose 1.36's shell tool sets only `PATH` (`goose@1.36 crates/goose/src/agents/platform_extensions/developer/shell.rs:623,651`).
     Main sets `AGENT_SESSION_ID` (`goose@main .../developer/shell.rs:616-622`), which is not one of our markers.
   - The A4 person guard therefore finds no env marker. Its second test, Herdr reporting a supported agent kind
     (`src/cli/mod.rs:361-372`, `src/protocol/authority.rs:127-131`), also fails for Goose.
   - Consequence: a Goose model can run `herdr-threads human me init` and act as a person (`operator_human`,
     `TRUST-POLICY.md:307`). It would ACK, send and accept under human provenance.
   - Per AGENTS.md this is cooperative and same-user, so it is not a security bug. It is still an honesty bug in
     attribution that applies to **every unsupported harness**, and TRUST-POLICY does not name it as an accepted
     limit (`grep -n -i "unsupported harness" TRUST-POLICY.md` gives no hit).
2. **No wake is possible, even in principle, until Herdr classifies the pane.** Our wake gates need
   `agent get .agent` in the registry's `host_kinds` and `agent_status` ∈ {idle, done} (`src/host/native.rs:1682-1735`).
   The fix is upstream in Herdr (a `goose` kind), or a proven `report-agent` path (unknown, see above).
3. **The `DeliveryPolicy` vocabulary cannot honestly describe Goose's best injection surface.**
   - `DeliveryMechanism` is `{PushChannel, HookPull}` and `IdleWake` is `{Native, Channel}`
     (`src/harness/adapter.rs:447-472`).
   - The registry refuses any `(mechanism, provenance)` pair outside
     `PushChannel+cooperative_mod_delivery` and `HookPull+cooperative_hook_delivery+Native`
     (`src/harness/registry.rs:1268-1282,1335-1339`).
   - MOIM delivery is a daemon-written file that the harness pulls before each provider call. It is neither a hook
     output nor a channel, so it needs a new mechanism and a new A3 provenance in TRUST-POLICY (DP9-style edit,
     `docs/superpowers/runs/2026-10-09-agy-harness-adapter/2026-10-09-agy-harness-adapter--delivery-policy-design.md:366`).
4. **Confirmation for MOIM-style delivery has no transcript to scan.**
   - agy confirms by scanning `transcriptPath` (`src/harness/agy/delivery.rs:3-6,57`). Goose hook payloads carry
     no transcript path, only `session_id` (primer §3.3).
   - In 1.36 the MOIM block is ephemeral and is never stored in `sessions.db`.
   - On main the turn-context message is persisted (primer §5), so `sessions.db` could serve as a read-only
     confirm source on main only. INFERRED, not probed.
5. No bead or doc tracks Goose support or the person-attribution gap for unsupported harnesses. VERIFIED
   (`bd search goose`, `bd search "unsupported harness"`: no issues).

## For a harness we do not support yet: what our adapter traits would need, and which primer surfaces fit best

### Trait surface a `GooseAdapter` must fill

| Trait / item | Required? | Goose fit (primer section) |
|---|---|---|
| `metadata()` (`adapter.rs:474-486`): `id: "goose"`, `executable: Path("goose")`, `host_kinds: &["goose"]`, `agent_env_markers` | yes | `host_kinds` only helps once Herdr reports `goose`. An env marker is needed to close limitation 1. Goose 1.36 has none of its own in the shell tool, so the adapter would have to **inject one**: a hook/launch-exported variable, or `AGENT_SESSION_ID` on main (§11). INFERRED. |
| `caller_env()` → `CallerEnvShape` (`adapter.rs:45-50`; `caller_env.rs:1-7` walks ancestry for the innermost harness) | yes, for agent attribution | Ancestry `goose` process plus a session variable. Main exports `AGENT_SESSION_ID` to the shell tool (§11). On 1.36 there is no session id in the shell-tool env, so attribution would have to be pane-based or launch-provided. |
| `contracts()` / `classify` / `decode` / `encode` / `admit` (`adapter.rs:83-111`) | yes | Hook stdin JSON `{event, session_id, matcher_context, tool_name?, ...}` (§3.3). `encode` can only emit `ObserverOnly`, because there is no context field (§3.4). |
| `required_hook_events()` | yes | `SessionStart`, `UserPromptSubmit`, `Stop`, `SessionEnd` (+`PostToolUse`) (§3.2). |
| `setup` / `status` / `unsetup` + `settings_kind` | yes | Plugin directory under `~/.agents/plugins/` and `disabledPlugins` in `~/.config/goose/settings.json` (§3.1, §4). |
| `delivery_policy()` (`adapter.rs:228-231`) | yes, but **the enum must grow** | `None` today means "hooks deliver and native wake applies", which is false for Goose, because its hooks cannot deliver. Options: (a) a new `DeliveryMechanism::ContextFile` with a new A3 provenance; (b) no policy, plus `MARKER` wakes and `inbox` (Codex-hookless parity). |
| `hook_step()` (`adapter.rs:807-829`) | for MOIM | On `UserPromptSubmit`/`PostToolUse`, pull a page (`DeliveryPull`) and write it to the MOIM file. On the next `PostToolUse`/`Stop`, `DeliveryConfirm`. This mirrors agy's `HookStep` (`src/harness/agy/delivery.rs:598`). On 1.36, "the file was read" is only INFERRED from the next hook firing. |
| `composer_policy()` (`adapter.rs:1352-1359`) | yes, for any wake | Read the `> ` row: empty = `>` plus the dim hint (§9). `clear_key` must be inert, and `composer_stash` must stay unsupported, because Ctrl+C deletes the draft and a second one exits (§9). |
| `launch_policy()` (`adapter.rs:1240-1351`) | optional | `goose session [-n NAME] [--with-extension ...]` (§2). It requires `herdr agent start --kind goose`, which Herdr 0.9.5 does not accept (observed help). It is the natural place to set `GOOSE_MOIM_MESSAGE_FILE` for 1.36. |
| `binding_wake_text()` | follows the policy | `MARKER` unless MOIM delivery is in place at turn start, in which case `WAKE_UP` (R18). |

### Which Goose surfaces fit best (ranked)

1. **Injection: `tom` MOIM file (§5).** In-turn, after every tool batch, with no tool call needed. It is the
   closest analogue to Claude's channel / agy's `PreInvocation` systemMessage. Caveats:
   - In 1.36 the env var must be present at launch, and the block is re-sent every provider call until cleared,
     so we must clear promptly or pay k × page.
   - Below a 32k-token context limit it is skipped entirely (§5), so delivery fails silently. The adapter needs a
     `MARKER` fallback.
2. **Busy/idle edges: hooks → `herdr pane report-agent` (§3.2, §10).** `UserPromptSubmit` → working, `Stop` →
   idle. This is unproven: whether it gives `pane.agent = goose` is unknown and must be probed in an isolated
   Herdr session (`scripts/lib/isolated-herdr.sh`). Otherwise the cleaner fix is Herdr adding a `goose` kind
   upstream.
3. **Idle wake: Native typing of `MARKER`/`WAKE_UP` + Enter into the `> ` prompt (§9).** It is the only wake for a
   terminal `goose session`. MCP notifications cannot wake it (§6), and ACP `session/prompt`/`steer` exist only for
   ACP-hosted sessions (§7).
4. **Turn-end delivery on main only: blocking `Stop` hook (§3.4.2).** The reason becomes a hidden user message
   and the turn continues. The framing is fixed ("policy hook denial ... Address this") and contradicts R17
   compact cooperative framing, and blocks are capped at 8. Use it only as a "don't stop yet, mail arrived"
   nudge, if at all.
5. **Avoid:**
   - PreToolUse deny-as-delivery: it destroys the tool call (§3.4.1).
   - Writing into `sessions.db`: it has no lock and gives an unsynchronised second writer (§8).
   - Headless `goose run -r` as a wake while a pane holds the session: it creates two agents on one conversation
     (§8).
6. **Different product shape:** ACP-hosted Goose (`goose serve`, §7) has the cleanest primitives of any harness:
   `session/prompt` for idle wake, `steer` with pickup ids for mid-turn delivery, and active-run updates for
   busy/idle. None of it applies to a Goose CLI in a Herdr pane, and it would need a host other than a Herdr pane
   composer.

## Verification (adversarial re-read)

I re-checked each claim against the code before finishing. These are the corrections made, and what I confirmed:

- I first wrote that a Goose pane's wake fails only at the missing composer policy. In fact three independent
  gates refuse it: no bound harness (`dispatch.rs:325-333`), an unrecognized Herdr agent kind
  (`native.rs:1693-1699`) and the composer read (`composer.rs:58-73`, `native.rs:1876`). The text now says so.
- I first assumed `report-agent` was proven not to set `pane.agent`. The Hermes evidence covers
  `report-agent-session` only, so the claim is now "unknown".
- Confirmed that `None` from `delivery_policy()` means "hooks deliver and the native wake applies"
  (`adapter.rs:228`). It is not a "no delivery" declaration, which is why a Goose adapter with no policy would
  misstate itself. The default wake text is `MARKER` (`adapter.rs:469-471`, `registry.rs:1429`).
- Confirmed the human-attribution chain step by step:
  - `caller.rs:76-77` (`NoHarness` and no marker → `Human`);
  - `output.rs:174-176` (the marker set is the registry's env markers);
  - `authority.rs:127-131` (a Herdr kind must be registered);
  - Goose 1.36 shell env: only `PATH` (grep of `developer/shell.rs`).
- Confirmed that Herdr 0.9.5 has no `goose` kind (help output, `strings` count 0). I did not run `report-agent`
  against the shared Herdr server (AGENTS.md forbids touching it), so its behaviour stays unknown.
