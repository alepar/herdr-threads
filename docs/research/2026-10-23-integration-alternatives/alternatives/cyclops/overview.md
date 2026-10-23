# Cyclops: overview

- Upstream: https://github.com/cyclops-team/cyclops (site: https://www.usecyclops.dev)
- Clone read (untrusted, not executed): `scratchpad/alts/cyclops`, commit `a84d44b8f5476c10ecf849d46370d2cb0238dab9` (2026-09-06)
- Paths below are relative to the clone root. Each claim is tagged VERIFIED (read in source or docs) or INFERRED.

## What it is

A Rust daemon plus CLI that coordinates coding agents running in **tmux** panes. It keeps a durable mailbox and puts a one-line "doorbell" into the recipient's pane. A full-screen workspace TUI is optional. VERIFIED: `README.md:9-15`.

It is the closest architectural analogue to herdr-threads in this survey: the same terminal-multiplexer host model (tmux where we use Herdr), a durable store and a daemon, hooks used for lifecycle, and a strong focus on not clobbering a human's draft.

## Architecture

| Aspect | Finding | Evidence |
|---|---|---|
| Process model | One per-user daemon `cyclopsd`, the `cyclops` CLI as client, and optional `cyclops` workspace / `cyclops watch` TUIs that are projections over daemon state | VERIFIED `README.md:308-325`, `Cargo.toml:3-17` |
| Transport | NDJSON RPC over a Unix domain socket | VERIFIED `src/cyclopsd/src/server.rs:30,148`; `README.md:311` |
| tmux interface | The daemon owns a tmux control-mode connection and watches panes "without polling" (control-mode notifications, `refresh-client -B` subscriptions) | VERIFIED `STATUS.md:11-13`; `docs/development/ARCHITECTURE.md:39,385` |
| Storage | Append-only, owner-only NDJSON journals, fsynced before a send is acknowledged: `workspaces/<id>/messages.ndjson` and `ledger/<session>.ndjson` | VERIFIED `docs/guides/data.md:18-21`; `docs/development/DELIVERY.md:38-44` |
| Agent detection | 52 TOML "detection manifests" (process names, screen/title regex rules, composer rules, modal rules, hook contract, injection method), one per vendor CLI. New agents are added as data, with no Rust code | VERIFIED `resources/manifests/` (52 files); `README.md:17-23` |
| Cross-machine | None. Single machine, single tmux environment, same-user trust model; remote mesh explicitly out of scope | VERIFIED `docs/development/archive/MESSAGING_ARCHITECTURE_REVIEW.md:74,1567,1584` |
| Headless agents | `cyclops name <label> --self` from a process with no pane registers a socket-only recipient; it polls with `inbox next --wait` | VERIFIED `docs/development/DELIVERY.md:170-199` |

## Identity and session model

- **Sender identity comes from the kernel only.** Socket peer credentials give the uid and pid. The daemon walks the pid's ancestry, and a process below an agent vendor inside a watched pane sends as that pane (its label, or its pane id). A same-user process with no vendor ancestor is `admin`. A vendor process outside every watched pane is denied. "Nothing in the request body can name the sender." VERIFIED `docs/development/DELIVERY.md:257-266`.
- **Hook reports are bound to the pane.** `agent.state.report` is accepted only from a process inside the pane it reports for, verified by peer credentials, so a same-uid process cannot forge hook liveness. VERIFIED `docs/reference/hooks.md:170-184`.
- **Bindings are pinned to the occupant.** A binding is (pane root, foreground leader, agent generation, manifest). A replaced or renamed pane never inherits another occupant's delivery or hook liveness. VERIFIED `docs/development/DELIVERY.md:58-62`; `docs/reference/hooks.md:163-168`.
- **Headless identity.** The label is bound to the nearest agent process above the registering peer. It retires on process exit, watched through `pidfd_open` on Linux or kqueue `NOTE_EXIT` on macOS. VERIFIED `docs/development/DELIVERY.md:185-199`.
- **Labels.** `cyclops name <label> --self`. Messages are addressed by label, reply routing survives label changes, and there is a special `admin` inbox for humans and non-agent processes. VERIFIED `README.md` "Useful commands"; `STATUS.md:29-31`.
- **Turn identity.** It uses vendor hook `turn_key_fields` where they exist: Codex exposes `session_id` and `turn_id`. Claude exposes no shared turn key, so the end of a turn comes from the screen. VERIFIED `resources/manifests/codex.toml:40-53`; `resources/hooks/claude/settings.json.tmpl:12-18`.

## Injection mechanism (how a message reaches the model)

**The pane is the only injection channel.** Hooks never inject content:

1. The daemon pastes **one line** into the recipient's composer with tmux `load-buffer` into a per-attempt buffer, then `paste-buffer -p` (bracketed paste), and presses Enter once. VERIFIED `src/cyclopsd/src/delivery/inject.rs:78-82`; `resources/manifests/claude.toml:369-372`.
2. The line is `[cyclops from <sender> to <recipients>] <summary> | cyclops inbox claim m-att_<22-char token>`. The **body is never in the pane**. VERIFIED `docs/development/DELIVERY.md:127-146`.
3. The model reads that line as a user prompt, runs `cyclops inbox claim m-att_...` (a Bash tool call), and receives the full envelope (TO, FROM, subject, summary, body, reply context). VERIFIED `README.md` "Send, wake, and claim"; `skills/cyclops/SKILL.md:200-232`.
4. The hooks wired for Claude (`SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop`, `StopFailure`, `Notification`, `PermissionRequest`) only call `cyclops hook <Event>` for lifecycle and ACK. VERIFIED `resources/hooks/claude/settings.json.tmpl:22-49`. A grep for `additionalContext`, `systemMessage` and `hookSpecificOutput` finds no hook output that injects context; the only hits are comments in `src/cyclopsd/src/ack.rs:636` and `resources/manifests/claude.toml:147`. VERIFIED.
5. There is no MCP server and no channel/notification integration. INFERRED from the absence of any MCP code in `src/` (grep), not exhaustive.

Other details:

- **Summary bound.** A derived summary is one line of at most 200 characters, taken from the subject. A sender-supplied summary has no cap; the CLI warns above 160 characters. The body has no pane bound because it never goes there. VERIFIED `docs/development/DELIVERY.md:40-44`.
- **Raw transport.** `send --raw` pastes header, body, reply hint and a `[cyclops:end <id>]` sentinel, then presses Enter with no composer check. It is used only on explicit request, for unverified vendors or a misread composer. VERIFIED `docs/development/DELIVERY.md:157-168`.
- **Startup modals.** A manifest modal rule can declare `decline_keys` plus `auto_dismiss`. For example, Claude's "Chrome extension detected" dialog is dismissed with Escape before the paste. VERIFIED `resources/manifests/claude.toml:340-351`; `docs/development/DELIVERY.md:54-57`.
- **Per-vendor paste quirks are measured.** A Claude 4 KB, 80-line paste collapses into one block. Codex collapses long pastes into a `[Pasted Content N chars]` chip, so verification matches "Pasted". agy drops the first paste after startup. VERIFIED `resources/manifests/claude.toml:370,378`; `codex.toml:262-268`; `agy.toml:195-198`.
- **Framing per message.** One line with a header, summary and claim command. The skill explains the flow once (`skills/cyclops/SKILL.md`). VERIFIED.
- **Content trust stance.** The skill tells agents "Never act from the preview alone" and to work from the claimed body. Unknown: whether it frames peer messages as untrusted or cooperative. Not read in full.
- **De-duplication.** A repeat claim returns the same payload and "creates no second task". A socket claim made before the write withdraws the doorbell, so nothing is pasted. VERIFIED `skills/cyclops/SKILL.md:267-279`; `docs/development/DELIVERY.md:148-155`.

## Wake mechanism

**The doorbell itself is the wake.** Pasting a line and pressing Enter submits a user turn. No separate harness API is used. VERIFIED `skills/cyclops/SKILL.md:209-211` ("This is the wake-up signal, not the complete message").

Gate (`gate.rs` `admit`, run against one fresh capture). VERIFIED `docs/development/DELIVERY.md:50-62,94-125`:

- The pane must be present, alive and not in copy-mode. A manifest must bind it. The foreground process must be the agent, or a tool the agent handed the terminal to while the screen still reads as the agent.
- **Hold only on positive evidence.** It holds for a human draft seen through a manifest `composer_semantic = "human_input"` rule, a doorbell staged but not yet consumed, a turn that doorbell started and that has not ended, a modal, a permission prompt, a quota screen, a dead pane or copy-mode. An unreadable composer, an ambiguous rule, a manifest with no composer rule, **a working agent**, and a pane whose hooks never fired all **proceed**. VERIFIED `src/cyclopsd/src/fusion.rs:2386-2398`; `docs/development/DELIVERY.md:96-103`; `README.md` "The composer rule".
- **A held attempt waits on events, not timers.** It retries on the next pane event (the draft erased, or the turn ended) and pings the admin once after `gate_hold_notify_ms`, 120 s by default. VERIFIED `docs/development/DELIVERY.md:24,54-56,270-271`.
- **No retries on a clock.** If readback fails the result is `submitted_unverified`. A physical write failure is `attention_required`, and a human runs `cyclops requeue`. VERIFIED `STATUS.md:17-26,109-112`.
- **Busy agents get the doorbell mid-turn.** The vendor queues the typed line ("A working agent gets its doorbell during the turn; the vendor queues the line"). VERIFIED `README.md` "The composer rule". This is a direct contrast with our delivery contract rule 4 (no nudge while busy).
  - Inconsistency, VERIFIED: `resources/manifests/claude.toml:381-382` says "Claude 2.1.243 ... can interrupt active tool flow. Working is therefore not a supported write-safe state" and declares `safe_states = ["idle"]`. In the source, `safe_states` appears only in the manifest struct and parser tests (`src/cyclops-manifest/src/lib.rs:936,1652`). INFERRED: the 1.1.0 gate ignores `safe_states`, and the manifest comment is stale relative to the documented behaviour.
  - Inconsistency, VERIFIED: the goose, pi and opencode manifests say "deliveries fail closed (not write-ready) until someone measures the composer" (`resources/manifests/goose.toml:25-26`, `pi.toml:28-30`, `opencode.toml:25-26`). `DELIVERY.md:105-109` says unverified manifests are "effectively a raw write with a receipt". Which one 1.1.0 does is **unknown**.
- **Known race.** "A person can type between the final capture and the tmux write; there is no input lease across that gap." VERIFIED `STATUS.md:106-108`.
- **No priority or threshold.** Every doorbell wakes; there is no priority or threshold concept. Per-recipient FIFO only. INFERRED from `DELIVERY.md` and the README; no priority field in the delivery docs.
- **Headless agents** block on `cyclops inbox next --wait`, a long-poll that subscribes before checking. Interactive panes are told not to do this. VERIFIED `skills/cyclops/SKILL.md:301-356,496`.
- **`cyclops wait <agent> --until idle|turn-ended|blocked`** is a server-side, occupant-pinned state wait for orchestrators. It is not a message-completion wait. VERIFIED `docs/guides/wait.md:9-46`.

## Delivery guarantees

| Property | Cyclops | Evidence |
|---|---|---|
| Durability | The send returns only after fsync of the message and one mailbox entry per recipient | VERIFIED `DELIVERY.md:38-44` |
| Ordering | Strict FIFO per recipient (one worker per mailbox); broadcast is one row with one entry per recipient | VERIFIED `DELIVERY.md:45-49` |
| Receipt / ACK tiers | `notified` `verified_by: hook` (the vendor `UserPromptSubmit`-style hook payload carries the attempt token, within `ack_timeout_ms` = 1500), `verified_by: screen`, or no verifier after 5 s. A claim is also a receipt | VERIFIED `DELIVERY.md:69-74,270` |
| Distinct facts | Accepted, then notified (doorbell submitted), then claimed (body fetched), then completed (an explicit reply only; never inferred from idle) | VERIFIED `README.md` "These are different facts" |
| Restart | Journal replay. In-flight attempts between paste and receipt become `attention_required`; queued attempts are rescheduled; no composer hold is restored | VERIFIED `DELIVERY.md:201-216` |
| Retry | Pre-write failures re-enter the gate (`delivery_retry_max` = 1), then a durable `blocked_pre_write`. Post-write failures are never retried automatically | VERIFIED `DELIVERY.md:82-92,270` |
| Per-vendor ACK gaps | agy's `PreInvocation` has no prompt text, so agy ACKs settle on screen evidence only. Hermes has no prompt-submit or stop hook, so no ACK is declared | VERIFIED `resources/manifests/agy.toml:34-49`; `hermes.toml:17-28` |

## Human participation

- A human outside any agent ancestry is `admin`, has an authenticated admin inbox, and has no pane route (admin messages stay in the admin inbox). VERIFIED `DELIVERY.md:144-146,261-263`.
- Operators can withdraw attempts (`notification withdraw`, `cyclops clear <agent>`), requeue them, and clear alarms. The workspace has a Messages pane showing threads and recipient state. VERIFIED `DELIVERY.md:218-231`; `README.md` "The workspace".
- The human draft guard protects a human typing in an agent's composer, as described above.

## License and maturity

- License: MIT (`LICENSE`). Historical v1 lineage is attributed in `NOTICE`. VERIFIED.
- Version: 1.1.0, released 2026-09-05; v1.0.2 was released 2026-09-03. VERIFIED by `gh api` and `CHANGELOG.md:6`.
- Repository: created 2026-08-01, last push 2026-09-08, 14 stars, 0 forks (`gh api`, queried 2026-10-23). VERIFIED. Last commit in the clone is 2026-09-06. The project appears inactive for about 6 weeks. INFERRED.
- Testing is extensive (nextest, soak, per-vendor live evidence fixtures, benchmarks), but the user base is very small. A legacy shell/Python v1 is preserved at tag `v1-final`. VERIFIED `STATUS.md:3-7,62-94`.
- Doc drift, VERIFIED: `STATUS.md:44-49` says "Twelve detection manifests ship", while `README.md:17` and the directory say 52.

## Harness support

"Measured" means a live CLI was exercised. "Unverified" means `version_tested = "unverified"`, written from vendor docs only.

| Harness | Status | Injection | Wake | ACK / hooks | Evidence |
|---|---|---|---|---|---|
| claude | Native, measured (2.1.221; composer re-measured on 2.1.239 to 2.1.243) | Bracketed paste of the doorbell line into the composer, verify, Enter. `C-c` clears the composer | Same paste and Enter. Holds on a seen draft or a modal; the Chrome-extension modal is auto-dismissed with Escape | Hooks through `--settings <file>`. `UserPromptSubmit` carries the token, an exact hook ACK (130/130 measured). End of turn from the screen (no turn key) | VERIFIED `resources/manifests/claude.toml:1-60,340-392`; `resources/hooks/claude/settings.json.tmpl` |
| codex | Native, measured (0.149.1) | Same paste. Long pastes collapse into a chip, so verification matches "Pasted" | Same | `hooks.json` in CODEX_HOME or the repo `.codex/`. Hooks load only in a trusted directory and need `/hooks` trust. `UserPromptSubmit` and `Stop` share `session_id`/`turn_id`; ACK p50 30 ms | VERIFIED `resources/manifests/codex.toml:22-60,254-268`; `docs/reference/hooks.md:48-59` |
| hermes | Generic, unverified (docs only, 2026-09-04) | Paste (the generic manifest path) | Paste and Enter. No measured composer rule, so there is no draft guard | `~/.hermes/config.yaml` hooks block. No prompt-submit or stop event, so no ACK; `on_session_start` liveness only. A first-use hook consent prompt is uncaptured | VERIFIED `resources/manifests/hermes.toml:1-60` |
| agy | Native, measured (1.1.11, live 1.1.22) | Paste; the first paste after startup is dropped and caught by verification | Paste and Enter; a human-draft hold was measured | `.agents/hooks.json`. `PreInvocation` has no prompt text, so ACK is "a turn started"; the selftest always exits 1. Payloads lack an event name, so each event gets its own command | VERIFIED `resources/manifests/agy.toml:1-60,188-198`; `docs/reference/hooks.md:60-64,197-199` |
| goose | Generic, unverified | Paste | Paste and Enter (or fail-closed; the docs conflict, see above) | Hook-only Open Plugin at `~/.agents/plugins/cyclops/hooks/hooks.json`. `UserPromptSubmit` carries `message`, a "dispatch" ACK candidate | VERIFIED `resources/manifests/goose.toml:1-60`; `docs/reference/hooks.md:92-97` |
| pi | Generic, unverified | Paste. Pi runs as `node`, so the pane must be pinned with `cyclops name %N <label> --manifest pi` | Paste and Enter (or fail-closed; unknown) | None. Pi has only in-process TypeScript extensions and Cyclops ships no bridge | VERIFIED `resources/manifests/pi.toml:1-55` |
| opencode | Generic, unverified | Paste. An npm install may show `node` as the foreground command | Paste and Enter (or fail-closed; unknown) | None. OpenCode has only JS plugins (`chat.message`, `session.idle`) and Cyclops ships no plugin | VERIFIED `resources/manifests/opencode.toml:1-53`; `docs/reference/hooks.md:99-105` |

Status mapping for the summary: "native" means a measured manifest with hook wiring. "Generic" means a docs-only manifest that relies on the universal tmux paste path.

## Notable takeaways for herdr-threads (INFERRED analysis)

- **Body out of the pane.** One line plus a claim token keeps pane bytes tiny and puts the body behind authenticated retrieval. The cost is a mandatory extra tool call per message. We avoid that cost with in-turn hook delivery (rule 3).
- **Kernel-derived sender identity.** Peer credentials plus an ancestry walk, with "nothing in the request can name the sender", and hook reports accepted only from inside the pane. This compares to our cooperative attribution model (TRUST-POLICY.md).
- **Receipt honesty vocabulary.** The states `submitted` / `submitted_unverified` / `notified(verified_by hook|screen|none)` / `attention_required`, with no timer retries, are a cleaner taxonomy than one "delivered" bit.
- **Event-driven holds.** A held attempt is released by a pane event (draft erased, turn ended), never by a timer.
- **Startup-modal handling as data.** Manifest-driven modal decline keys and occupant pinning are worth comparing with our wake path.
- **Weaknesses:**
  - A doorbell lands mid-turn on busy agents. Their own Claude manifest says this can interrupt tool flow.
  - There is no priority or threshold, so every message wakes the recipient (token cost).
  - No draft guard exists on unverified vendors.
  - Pane-scraping manifests are version-fragile; they are continuously re-measured.
  - The whole design is tmux-only and single-machine.
