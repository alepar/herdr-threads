# CLI Agent Orchestrator (CAO, awslabs) x Goose (Block)

- Upstream: https://github.com/awslabs/cli-agent-orchestrator, depth-1 clone at
  `69909e98035905b612fd6b117abff8bf8876220f` (2026-10-23 18:49 +1100). Paths below are relative to that clone
  root unless prefixed `goose@1.36` / `goose@main` (the Goose trees named in `../../primers/goose.md`).
- Read with: `overview.md` (this directory) for CAO's general mechanism, `primers/goose.md` for Goose surfaces.
- Tags: VERIFIED = read in source, docs or the GitHub API at the cited place. INFERRED = my reading.
  "unknown" = not determined.

## Support status

**Not supported.** CAO has no Goose provider, and nothing in the repository launches, detects or delivers to
Goose.

- The provider set is a closed enum plus a hard-coded factory map. Members: `kiro_cli`, `claude_code`,
  `codex`, `kimi_cli`, `copilot_cli`, `opencode_cli`, `hermes`, `cursor_cli`, `antigravity_cli`, `omp`,
  `grok_cli`, `mcode`, `mock_cli`. VERIFIED `src/cli_agent_orchestrator/models/provider.py:4-20`,
  `src/cli_agent_orchestrator/providers/manager.py:39-59` (unknown id raises
  `ValueError("Unknown provider type: ...")`, line 59). `ls src/cli_agent_orchestrator/providers/` has no
  goose file. VERIFIED.
- Every "goose" hit in the tree (excluding `.git/`) is about **Goose as an MCP Apps host** that can render CAO's
  fleet dashboard, or about where Goose keeps skills. None is a provider. Examples (not exhaustive; the same
  text recurs in mirrored `skills/` and `src/.../skills/` copies, `cao_mcp_apps/README.md:9`,
  `examples/mcp-apps/README.md:30`, `docs/knowledge-graph-viewing.md:378`):
  `README.zh-CN.md:233`, `docusaurus/docs/features/mcp-server.md:82`, `docs/mcp-apps.md:7,33`,
  `src/cli_agent_orchestrator/ext_apps/widget.py:12`, `src/cli_agent_orchestrator/skills/cao-mcp-apps/SKILL.md:3`,
  `skills/mcp-apps-builder/SKILL.md:47-48` and `skills/mcp-apps-builder/references/installation-per-agent.md:98-106`
  (`~/.config/goose/skills/`). VERIFIED (`grep -rni goose`).
- Issues: no "add Goose provider" issue. GitHub search `repo:awslabs/cli-agent-orchestrator goose` returns 5
  (fetched 2026-10-23). VERIFIED:
  - #649 (open, "Expose CAO as an ACP server ..."): lists Goose among the ACP-registry CLIs. It does not ask
    CAO to drive Goose over ACP. It asks CAO to *be* an ACP server.
  - #788 (open, 2026-09-14): a user on CAO 2.5.0 with the **herdr backend** lists their "Seats: claude_code,
    codex, deepseek, groq, mistral, kimi_cli, goose, antigravity_cli". `goose`, `deepseek`, `groq` and
    `mistral` are not upstream providers. INFERRED: the user runs a locally patched provider set. #147
    (open) says adding a provider means editing 7 hard-coded files, and that one team keeps "a 426-line Python
    patcher". How that user's goose seat detects status or receives input is unknown.
  - #331/#332/#347: MCP Apps work, Goose named as a host.
- No generic provider exists. `mock_cli` is a credential-free test double (`models/provider.py:19-20`), not a
  "run any CLI" adapter. VERIFIED.

So this report covers two things: (a) the only real Goose touchpoint, Goose as an MCP host for CAO's MCP
server, and (b) what CAO's generic mechanism would do if someone added a Goose provider. That is CAO's
pattern of paste plus Enter into the TUI, screen-regex idle detection and an MCP `send_message` tool, checked
against the Goose primer. Everything in (b) is INFERRED unless a line is cited.

## Injection mechanism

### (a) Goose as a plain MCP host, outside CAO

A user can add `cao-mcp-server` to Goose as a stdio extension, because Goose is a standard MCP client
(primer section 6). That route sends messages only. **Nothing can be received**:

- Every orchestration tool resolves the caller from `CAO_TERMINAL_ID` in the MCP subprocess env. It must be 8
  lowercase hex characters (`src/cli_agent_orchestrator/utils/orchestration.py:217-235`). VERIFIED.
- `_send_to_inbox` raises `ValueError("CAO_TERMINAL_ID not set - cannot determine sender")` when it is unset.
  VERIFIED `orchestration.py:1391-1416`. A Goose session that CAO did not launch has no terminal id, so its
  `send_message` fails.
- A Goose session has no terminal row, so no inbox can be addressed to it. Delivery types into the pane of a
  CAO-owned terminal (below), so there is nothing to type into. VERIFIED by the delivery path
  (`services/inbox_service.py:205-300`). INFERRED that this rules out receiving.
- The MCP server has no tool for reading the inbox. Its ~40 tools are `handoff`, `assign`, `assign_elastic`,
  `send_message`, `complete_assignment`, `emit_ui`, `answer_user_prompt`, `load_skill`, `delete_terminal`,
  `get_handoff_result`, `list_siblings`, `update_metadata`, `find_profiles`, `memory_*`, `report_outcome`,
  `list_outcomes`, `store_lesson` and a `workflow_*` family (`run`, `resume`, `cancel`, `start`, `status`,
  `result`, `list`, `wait`, `events`, `create`, `update`, `get`, `validate`, ...). VERIFIED
  `src/cli_agent_orchestrator/mcp_server/server.py:333-3587` (`@mcp.tool` decorators; "inbox" appears only in
  docstrings). An
  HTTP `GET /terminals/{terminal_id}/inbox/messages` exists (`src/cli_agent_orchestrator/api/main.py:8450`),
  but no tool exposes it.
- What does work from Goose is the **operator surface**: launching or inspecting CAO fleets, and rendering the
  MCP Apps fleet UI (`docs/mcp-apps.md`). This is monitoring. It is not messaging.

### (b) Hypothetical CAO Goose provider: what the generic path does to Goose

CAO's single inject path for every provider: the sender calls MCP `send_message`, whose `_send_to_inbox` POSTs
to `cao-server` (`orchestration.py:1391-1453`), which stores a `PENDING` row (`api/main.py:8373`) and attempts
immediate delivery (`docs/inbox-delivery.md:12`). `InboxService._deliver_pending_locked` then checks status, marks
rows `DELIVERED`, joins each contiguous same-sender run with `\n` and calls `terminal_service.dispatch_input`.
VERIFIED `services/inbox_service.py:205-295`:

```python
limit = num_messages if num_messages > 0 else 100
messages = get_pending_messages(terminal_id, limit=limit)
...
if status not in (TerminalStatus.IDLE, TerminalStatus.COMPLETED):
    ...  # eager path only if provider.accepts_input_while_processing
for message in messages:
    update_message_status(message.id, MessageStatus.DELIVERED)
for sender_id, group in groupby(messages, key=lambda m: m.sender_id):
    combined = "\n".join(m.message for m in batch)
    boundary = terminal_service.dispatch_input(terminal_id, combined, ...)
```

**Byte transport.**

- tmux: `load-buffer` then `paste-buffer`. Below tmux 3.7 the message is wrapped in literal
  `\x1b[200~...\x1b[201~` and pasted with `-r`. On tmux 3.7 and later it is pasted with `-p`, which emits
  markers **only if the pane enabled DECSET 2004**. Then comes a provider-specific delay, a copy-mode cancel and
  `send-keys Enter`. VERIFIED `src/cli_agent_orchestrator/clients/tmux.py:1695-1860` (wrap/`-r` at ~1801-1812,
  `-p` at ~1813-1821, cancel at 1777 and 1850, Enter at 1855). Default `paste_submit_delay` is 0.3 s
  (`providers/base.py:348-358`).
- herdr: `herdr pane send-text` (optionally with hand-made bracket markers) then `pane send-keys Enter`.
  VERIFIED `src/cli_agent_orchestrator/backends/herdr_backend.py:645-705`.

**What Goose does with it.** The target is the `rustyline` `> ` prompt of `goose session` (primer section 9).

- If rustyline has bracketed paste on (its default, primer INFERRED), the whole payload lands as one buffer,
  and the trailing Enter submits it as **one user message**. That message becomes a new turn.
- If bracketed paste is off, the result depends on the tmux version. On tmux 3.7 and later, `-p` adds no
  markers, and `paste-buffer` without `-r` turns LF into CR. Every line of the message, and CAO adds
  `"\n\n" + suffix` by default (skipped only when injection is disabled or for an `@`-led structured prompt to
  a provider in `STRUCTURED_PROMPT_PROVIDERS`, `orchestration.py:187-207`), would then be submitted as a **separate prompt**: the first line starts a turn, and the
  rest wait in the tty until the next prompt. INFERRED from `tmux.py:1813-1821` and primer section 9 (Enter
  submits, Ctrl+J is newline). This is the #230 failure CAO saw with kiro-cli (`tmux.py:1726-1729`). On older
  tmux the hand-made wrap works only if rustyline parses the markers. That is unknown for the case where 2004
  was not enabled.

**Payload and framing.** The model sees the raw body, a blank line, then a trailing suffix. There is no
header line, no message id, and no priority or thread. VERIFIED `orchestration.py:125-138` (templates),
`183-207` (`append_sender_suffix`):

```
<body>

[Message from terminal 1a2b3c4d. Use send_message MCP tool for any follow-up work.]
```

`assign` uses `[Assigned by terminal X. When done, send results back to terminal X using send_message]`
instead. Suffix injection is on by default (`CAO_ENABLE_SENDER_ID_INJECTION`, `orchestration.py:127`).

**When it fires.** Only when CAO thinks the receiver is `IDLE` or `COMPLETED`. Eager mid-turn delivery
requires both the global `CAO_EAGER_INBOX_DELIVERY=true` (default `false`, `constants.py:322`) and the provider's
`accepts_input_while_processing`, which is `False` by default (`providers/base.py:376-387`,
`inbox_service.py:220-247`). VERIFIED. For Goose that default is correct: the CLI does not read the terminal while busy, and injected bytes
are echoed by the cooked tty and become the *next* prompt (primer section 9). CAO cannot reach Goose mid-turn
at all, because the CAO extension's MCP notifications never reach the model (primer section 6) and CAO does
not use Goose's MOIM file or its ACP `steer`. INFERRED.

**Size bounds.** None found on the inbox POST or the paste (overview item 6: unknown). A drain batch is capped
at 100 messages (`inbox_service.py:212`), with no cap in bytes. VERIFIED for the count. INFERRED: a large paste
goes into rustyline as is, and Goose has no input size cap of its own for typed prompts (unknown).

**Dedupe and ACK.**

- Rows are marked `DELIVERED` **before** the paste, so a re-entrant status event cannot type them twice.
  VERIFIED `inbox_service.py:250-257`.
- Status has three values: `PENDING`, `DELIVERED` or `FAILED` (`models/inbox.py:17-22`). There is no
  receiver ACK.
- Submit confirmation (#78, still open) waits up to 8 s (`constants.py:378`) for a status newer than the
  dispatch boundary. When none arrives, it retries up to 2 times (`constants.py:383`): bare Enter if the text is
  visible in the composer, a full re-paste only if the provider read its composer and the text is absent and it
  has a status probe. So recovery needs `extract_current_composer` (`providers/base.py:340-347`, default `None`;
  `inbox_service.py:413-480`, `services/terminal_service.py:3465-3474`). A Goose provider without a composer
  reader would log a drop and leave the row `DELIVERED`. **On the herdr backend confirmation is skipped
  entirely** (it would block herdr's event loop), so a drop there goes undetected for every provider
  (`inbox_service.py:296-312`). VERIFIED for the code paths. INFERRED for Goose.

## Wake mechanism

Wake and inject are one act: the paste plus Enter into an idle `> ` prompt submits a new turn.
`docs/inbox-delivery.md:5-15`, VERIFIED. There is no content-free wake. The wake *is* the message.

**Idle/busy detection.**

- tmux: a Goose provider would have to implement `get_status(buffer)`
  (abstract, `providers/base.py:192-218`) with regexes over the raw pipe-pane stream. Candidates from the primer: idle is the `> ` row with the dim
  hint `Enter to send · Ctrl+J newline`, or after an interrupt `Interrupted, what should goose work on
  instead?`. Busy is the spinner or "thinking" lines (primer section 10). Goose has no machine-readable state
  in the CLI, so this is pure screen scraping, like CAO's Codex provider (~3k lines of TUI parsing, overview).
  INFERRED.
- Waiting-for-user: Goose's tool-approval selects (`GOOSE_MODE=approve|smart_approve`) and MCP elicitation
  forms take keystrokes as answers (primer section 9). A Goose provider would need
  `blocks_orchestrated_input_while_waiting_user_answer = True` (`providers/base.py:390-400`) and regexes for
  those widgets. Otherwise an inbox paste plus Enter answers an approval. That is CAO's #896 lesson
  (`inbox_service.py:232-243`). With CAO's default launch, a `--yolo`-equivalent (`GOOSE_MODE=auto`, which is
  Goose's default), these prompts mostly do not appear, but elicitation still can. INFERRED.
- **herdr backend: likely broken for Goose.** Herdr appears to have no Goose detection, so `agent_status` would
  be `unknown` (INFERRED; primer section 10 bases this on a `strings` check of the herdr binary, itself INFERRED). CAO maps `unknown` to `None`
  (`backends/herdr_backend.py:866-912`) and falls back to provider regexes over a live `get_history()` read
  (`providers/base.py:541-560,649-679`). Herdr would never emit `pane.agent_status_changed` idle/done events
  for the pane, though, and those events are what `HerdrInboxService` delivers on (`docs/herdr.md:97-99`).
  VERIFIED for the mapping and the event trigger. INFERRED: on herdr, a message reaches Goose only through the
  immediate attempt at send time (if the regex says idle then) or through the **30 s reconcile sweep**
  (`docs/inbox-delivery.md:111-124`). Wake latency would be up to ~60 s (30 s grace plus a 30 s interval).
  This is the situation of the #788 user's "goose" seat, unless their patch does something else (unknown).

**Draft safety.** None. CAO has no check for human-typed composer text (overview, "Draft clobbering"). In
Goose that hurts more than in Ink-based TUIs:

- A paste into a `> ` line holding a human draft merges with it, and Enter submits both. INFERRED.
- CAO's interrupt verb sends `C-c` (`orchestration.py:2741-2745`, ops UI `mcp_server/app_tools.py:396,478`;
  herdr `herdr_backend.py:714-715`). In Goose, Ctrl+C with text in the buffer **deletes the buffer**. Ctrl+C on
  an empty line arms exit, and a second one **ends the session** (primer section 9, `goose@1.36
  crates/goose-cli/src/session/input.rs:56-81,154-173`). Two cancels in a row against an idle Goose would kill
  the worker. VERIFIED for CAO's key and Goose's behaviour. INFERRED for the combination.
- The input guard refuses delivery when the pane is back at the shell or in `ERROR`
  (`_refuse_if_provider_gone`, `services/terminal_service.py:4492-4524`). That covers a Goose that has exited, but not one that
  is merely holding a draft.

**Latency.**

- tmux: delivery on the next status event. The watchdog polls log files every 5 s
  (`docs/inbox-delivery.md:13,77-79`), plus the 0.3 s paste delay. INFERRED: about 0.3-5 s.
- herdr: see above, up to ~60 s.

**Failure modes.** INFERRED:

1. A multi-line payload splits into several prompts when bracketed paste is off.
2. Bytes sent while Goose is busy are echoed into the output and corrupt the screen regexes, then become the
   next prompt.
3. An Enter swallowed during the rustyline redraw leaves text in the composer. With no composer reader there
   is no recovery, and the row still says `DELIVERED`.
4. An approval or elicitation select gets answered by an inbox message.
5. A handoff result is scraped from the TUI (`extract_last_message_from_script`, abstract,
   `providers/base.py:449-458`). Goose renders markdown with no stable response marker, so extraction would
   be fragile.
6. `GOOSE_PROMPT_EDITOR` replaces the inline composer with `$EDITOR`, which breaks every assumption above
   (primer section 9).

## Setup/installation footprint on the harness

For (a), Goose as an MCP host: the user adds `cao-mcp-server` as an extension in Goose's config (or uses
`--with-extension`). Nothing else. CAO writes no Goose config. VERIFIED (no Goose code in CAO).

For (b), a hypothetical provider, CAO's style would be **per-launch flags and zero config writes**. INFERRED,
by analogy with Claude's `--mcp-config ... --strict-mcp-config` and Antigravity's per-launch MCP registration
(overview, harness table):

- `goose session --with-extension 'cao-mcp-server'`. `CAO_TERMINAL_ID` already reaches the extension through
  pane-env inheritance: tmux sets it at `clients/tmux.py:1252,1416,1598`, and the primer says Goose stdio
  extensions inherit the env. A `--with-extension 'CAO_TERMINAL_ID=<id> cao-mcp-server'` form also exists
  (primer section 2).
- The agent profile or system prompt is a gap. `goose session` has no `--system` flag (it exists only on
  `goose run`, primer section 2). Options are a first message (as CAO does for agy's `-i`), a `.goosehints`
  file, or `GOOSE_MOIM_MESSAGE_FILE`. INFERRED.
- Compare Hermes, the one CAO provider needing a hand-edited harness config: the user adds `cao-mcp-server` to
  the Hermes profile by hand (`docs/hermes.md:92-100`). VERIFIED.

## Token cost per message and per quiet turn

**Per message.** The full body, plus the ~85-byte suffix (template at `orchestration.py:136-138`), plus
**a whole new model turn**. Goose resends the system prompt and the full tool list, and in 1.36 the MOIM
block, on every provider call (primer sections 1 and 5). One message costs one turn. With the default
`num_messages=1`, N pending messages cost N turns unless drain mode is used. VERIFIED for batching
(`inbox_service.py:212-214`). INFERRED for the Goose turn cost.

**Per quiet turn.** Zero injected bytes; CAO adds nothing on turns with no mail. The **standing cost** is the
cao-mcp-server tool schemas, about 40 tools (`server.py` decorators), sent with every Goose provider call. Their
size in tokens is unknown, but they are long docstrings. INFERRED several KB, minus whatever the allowlist
middleware hides per profile (`server.py:1573,1691`).

## Limitations and bugs you can see

1. No Goose support, and no way to add a provider without patching 7 files (#147, open). Users already patch
   (#788). VERIFIED.
2. Goose as an MCP host cannot take part in messaging without a CAO-launched terminal id
   (`orchestration.py:1413-1416`). VERIFIED.
3. The herdr backend plus a harness Herdr does not recognise gives no event-driven delivery, only the 30 s
   sweep. This is a general CAO gap, not specific to Goose. INFERRED from `herdr_backend.py:866-912` and
   `docs/herdr.md:97-99`.
4. Paste safety depends on the target enabling DECSET 2004 on tmux 3.7 and later, and the suffix guarantees a
   multi-line payload. INFERRED.
5. A Ctrl+C-based cancel is destructive in Goose (draft loss, session exit). INFERRED.
6. Without a composer reader, submit-confirmation recovery is off, and `DELIVERED` can mean "sitting unsent
   in the composer". On herdr, confirmation does not run at all. VERIFIED (`providers/base.py:340-347`,
   `inbox_service.py:296-312`).
7. No draft check at all. VERIFIED (overview).

## Verification

Adversarial pass, 2026-10-23, against the clone at `69909e9` and the GitHub API. Checked: provider enum and
factory (`models/provider.py:4-20`, `providers/manager.py:41-59`), `MessageStatus` values, all goose grep hits,
issues #147/#649/#788/#230/#896/#78 and the 5-result goose search, `CAO_TERMINAL_ID` resolution and
`_send_to_inbox`, suffix templates and `append_sender_suffix`, `_deliver_pending_locked`, tmux paste paths
(wrap/`-r` 1810-1811, `-p` 1821, cancel 1777/1850, Enter 1855), herdr `send_keys`/`C-c`/`get_native_status`,
base-provider defaults, `docs/herdr.md:97-99`, `docs/inbox-delivery.md` (5 s watchdog, 30 s/30 s sweep),
`docs/hermes.md`, the input guard, and primer sections 2, 6, 9, 10.

Changed: the goose-hit list is now marked non-exhaustive; the MCP tool list was ~20 tools ending at line 2325,
it is ~40 including `store_lesson` and `workflow_*` (standing-cost estimate updated); the PENDING row is stored
by `cao-server` (`api/main.py:8373`), not by `orchestration.py`; the suffix is default, not "always"; eager
delivery also needs `CAO_EAGER_INBOX_DELIVERY`; the submit-confirmation citation was wrong (now
`inbox_service.py:413-480`) and the fact that herdr skips confirmation was added; `get_status` citation fixed
(192-218); Herdr's lack of Goose detection downgraded to INFERRED; foreground-check citation fixed and the
`_refuse_if_provider_gone` guard named.

## Clever ideas worth stealing

INFERRED unless cited.

1. **Capability flags on the adapter for input hazards.** `accepts_input_while_processing` and
   `blocks_orchestrated_input_while_waiting_user_answer` (`providers/base.py:376-400`) make "never type into
   a picker" a declared property that every delivery path reads. A Goose adapter for us needs the same: the
   approval and elicitation selects must block a Native wake.
2. **Re-check the pane's foreground command before every paste.** `tmux.py:1781-1783` (rationale in the
   docstring at 1710-1722) checks it fresh on every call and degrades to plain keys at a shell, and
   `_refuse_if_provider_gone` (`terminal_service.py:4492-4524`) refuses typed input outright at a bare shell. For Goose, typing the wake marker into a pane where Goose has
   exited would run it as a shell command. We should check the foreground process before every Native wake.
3. **Confirm the submit against a boundary sampled after the buffer is cleared.** `inbox_service.py:265-275`
   fits Goose's rustyline, where a swallowed Enter leaves the marker in the composer. Our wake path should
   confirm a post-wake transition (a `UserPromptSubmit` hook edge is available in both Goose versions, primer
   section 10) instead of trusting the keystroke.
4. **Degrade on herdr `unknown` instead of erroring** (`herdr_backend.py:877-884`). This is consistent with our
   rule of degrading only the broken capability. CAO's resulting gap, no events and so a 30 s sweep, is the
   warning: a Goose adapter for us should not depend on Herdr `agent_status` for Goose. Our own hook edges
   (`UserPromptSubmit`/`Stop`) should drive idle/busy.
5. **The reconcile sweep as a backstop** (`docs/inbox-delivery.md:111-124`): a cheap query when nothing is
   stuck, and only backlogged receivers cost work. This is a reasonable safety net under event-driven wakes.

What not to copy for Goose: message-as-user-turn (it breaks our rule that idle sessions stay idle and costs a
turn per message), Ctrl+C as an interrupt, and result scraping. For Goose the better surfaces are in the
primer: the `tom` MOIM file for in-turn delivery after every tool batch, the main-only Stop block for turn-end
delivery, and a content-free Native wake into an empty `> ` composer, with a Goose composer reader added to
`src/harness/composer.rs` (ht).
