# Comparison: message injection and wake on Goose (Block / AAIF)

Inputs: `ours/goose.md`, `primers/goose.md` and the twelve `alternatives/<slug>/goose.md` reports in this
research directory. Every claim below either cites those reports (as `ours`, `primer §N` or `<slug>`) together with
the clone location they give, or cites code I re-read for this comparison:

- herdr-threads at `2de3da1c4`, paths repo-relative, `file:line`.
- Goose trees from the primer: `goose@1.36` (v1.36.0, `f13f369a`, the installed binary) and `goose@main`
  (1.56.0, `c3bfc5f7`), <https://github.com/block/goose>, under
  `scratchpad/goose-src/{goose-1.36,goose}`.

Tags: **VERIFIED** means read in source, docs or observed `--help` (by me, or by the cited report's own verification
pass). **INFERRED** means reasoning from verified pieces. **unknown** means not determined. Nothing was executed except
`--help` and read-only greps.

## 0. Bottom line

- **No tool supports Goose, and neither do we.** All twelve alternatives give Goose either nothing or a generic
  path (VERIFIED per report). Only Cyclops ships any Goose artifact: a hook-only Open Plugin plus an "UNVERIFIED"
  manifest (`cyclops`: `resources/manifests/goose.toml:1-8`). herdr-threads refuses Goose at the registry
  (`src/harness/registry.rs:1482-1490`, VERIFIED), and Herdr 0.9.5 has no `goose` agent kind (`ours`, observed help).
- **Nobody uses Goose's best surfaces.** No tool uses the `tom` MOIM file, the main-only Stop block, ACP
  `session/prompt` or `steer`, or `AGENT_SESSION_ID` (VERIFIED by absence in every report). The comparison is
  therefore mostly between generic patterns. For a Goose adapter, the useful lessons are about wake safety and
  hookless delivery, not about Goose itself.
- **Where we are already ahead (INFERRED):** we are the only design with all of these at once:
  - a priority threshold, so a default send never wakes;
  - idle-only wakes;
  - a fail-closed unknown composer (`src/harness/composer.rs:58-73` gives `Unreadable`, which every wake path
    skips; `src/notification/policy.rs:385-395`);
  - bounded pages;
  - content-free wake text.

  On Goose, every alternative that types into the pane would clobber drafts or answer approval selects, and we
  would refuse to type.
- **What they expose in us:**
  1. A Goose agent (and any unrecognised agent) is attributed as a person.
  2. Our wake depends entirely on Herdr classifying the pane.
  3. Our `DeliveryPolicy` vocabulary cannot express the injection surfaces Goose actually has.
  4. A hookless caller gets mail only from `inbox` and `accept`, never piggybacked on its other `herdr-threads`
     calls.

## 1. Mechanism table (ours first)

"Support" means shipped Goose support. "Generic" means the alternative's harness-neutral path pointed at Goose by hand.

| Tool | Support on Goose | Injection: what and when | Bounds | Dedupe / ACK | Wake of an idle `goose session` | Idle/busy detection | Draft safety |
|---|---|---|---|---|---|---|---|
| **herdr-threads (ours)** | none; `hook goose` is a usage error (`src/cli/hook.rs:217-224`) VERIFIED | model runs `herdr-threads inbox` in Goose's shell tool; only when the model chooses to (`ours`) | page 20 items / 16 KiB default (`src/protocol/pagination.rs:4,6`) VERIFIED | ACK on full display after write and flush (`integrations/skill/SKILL.md:34`); the call is attributed as Human (`src/cli/caller.rs:74-80`) VERIFIED | **refused**: no bound harness (`src/notification/dispatch.rs:325-333`); Herdr kind not in the registry and status not idle/done (`src/host/native.rs:53,1682-1735`); composer `Unreadable` VERIFIED | Herdr `agent_status` only, and Herdr does not know Goose | fail-closed: `Unknown` UI means skip (`src/notification/policy.rs:395`) VERIFIED |
| agent-talk | none (README lists six agents) | model runs `retalk receive` or reads `inbox.ndjson`; prose "read each turn" | none (spool rotates at 8 MiB) | relay deletes on read; no reader ACK | none; it rejects TUI typing by policy (`agent-talk`: `docs/antigravity-auto-receive.md:76-82`) | n/a | n/a |
| Cotal | none | `cotal_inbox` MCP tool result, model-initiated; the local server joins no mesh for Goose (`cotal`: `app-identity.ts:94-101,146-161`) | 48,000 chars, build-measure-drop-tail (`tool-specs.ts:225,236-258`) | ack on return (JetStream); redelivery marker | none; `cotal_wait` 50 s long-poll only inside a turn (gateway) | none | n/a |
| agmsg | none; the shared Codex-typed skill leaks into Goose through `~/.agents/skills` (`agmsg`: `install.sh:1413-1415`) | `inbox.sh` via shell; a hand-wired Stop hook is wire-compatible with Goose **main** block JSON (`check-inbox.sh:372-380`) | none | marked read after stdout write; **silent loss on Goose 1.36 Stop** (output discarded) INFERRED | `poke`: herdr exits 12/15 (no agent or no marker); tmux poke ungated | none for Goose | fail-closed on herdr; none on tmux |
| ruflo | none (only "mongoose" hits) | `team_inbox` MCP tool, model-initiated | input caps only; drain unbounded | drain-by-rename = consumed | none | n/a | n/a |
| mcp_agent_mail | none | `fetch_inbox` MCP tool; reminder hook output would be discarded (no context field) | `limit` 20 (max 1000), bodies opt-in | does not mark read; explicit ack tool | none (`.signal` files have no consumer) | n/a | n/a |
| hcom | none; classified `adhoc` (`hcom`: `src/shared/tool_detection.rs:79-98`) | (a) blocking `hcom listen` in the shell tool; (b) pending mail **piggybacked on every hcom command** (`src/cli_context.rs:349-382`) | 50 msgs/batch, 1 MiB/msg, no batch byte cap | cursor advanced after `write_all+flush` | none from idle; "CONNECTED MODE" keeps the agent in one endless turn | not needed (busy by construction) | nothing typed; human locked out of pane while listening |
| agentchattr | generic `proxy_flag` + `--with-streamable-http-extension` works (`agentchattr` §1.1) | wake prompt typed ("use mcp to read #ch ..."), then model calls `chat_read` | wake text unbounded (rules); 20 msgs/read, older skipped | server cursor; no ACK | tmux paste + Enter into an owned tmux session; **busy wake likely stuck as draft: LF is Goose's Ctrl+J newline** (`agentchattr` §3.3) | none | none; Enter can **approve a pending tool call** |
| cross-agent-teams (xats) | generic `custom` over MCP HTTP | 200-char hint typed, then model calls `get_inbox` | hint 200 chars; inbox 50-200 msgs, bodies unbounded | cursor in the same transaction; sender gets a real read receipt (`tools.ts:166-178`) | tmux paste + Enter after a 2 s screen-quiescence guard; retries at 30/180/600 s that stop on read | screen unchanged for 2 s | none: a static draft passes the guard |
| agent-peers-mcp | generic stdio MCP, posing as `codex`/`hermes` | `PEER INBOX` block prepended to **every** agent-peers tool result (`codex-server.ts:479-547`); notifications go to the UI only | none per page or body; Goose spills > 200 k chars to a file | draw, then promote, then **confirm on the next call**, with cancel rollback | none; `wait_for_peer_messages` long-poll races Goose's 300 s tool timeout | none | n/a |
| Cyclops | **ships** a hook-only Open Plugin + unverified manifest | one-line doorbell typed, then model runs `cyclops inbox claim` | doorbell ~300 B; claim body unbounded | claim dedupe; UserPromptSubmit exact-payload ACK declared but **unreachable** on Goose (`cyclops` §ACK) | tmux paste + Enter; gate never reads `write_ready`, so effectively raw | hooks wired but no turn end; screen state `Unknown` | none: draft concatenated; never sends Ctrl+C |
| open-cross-session (ocs) | none; skill leaks through `~/.claude/skills` | `ocs read --as` via shell; cmux note with the **body inline** | note ≤ 5120 B, UTF-8-safe | per-consumer cursor; no ACK | cmux only (not Herdr): `send` + Enter | substring `to interrupt` (Goose spinner, hidden during streaming, tools and approvals) | none |
| CAO (awslabs) | none (closed provider enum); Goose is only an MCP Apps host | hypothetical: paste body + suffix as a user turn | 100 msgs/drain, no byte cap | `DELIVERED` before paste; submit confirm skipped on the herdr backend | paste + Enter; on herdr with an unknown kind, only a 30 s reconcile sweep (`cao`: `docs/inbox-delivery.md:111-124`) | provider regex (would need writing) | none; its Ctrl+C interrupt would **delete the draft / exit Goose** |

## 2. Differences from our approach

Each difference is named Dn and analysed in section 3.

- **D1. The message is the wake (body or hint typed as a user turn).** CAO pastes the body as a user turn. ocs
  types the body inline. agentchattr, xats and Cyclops type a hint. We type a content-free marker and deliver the
  bodies separately (R18, `src/notification/policy.rs:14-17`). On Goose today that marker must be `MARKER`,
  which names `inbox` (`src/harness/adapter.rs:466-471`).
- **D2. Ungated or weakly gated typing.** agentchattr and Cyclops type with no gate (Cyclops is effectively raw).
  xats waits for 2 s of screen quiescence. ocs scans for a substring. CAO uses regexes. We require a Herdr kind,
  idle or done status, an empty composer, and a 60 s focused-empty hold (`src/host/native.rs:53,71,1682-1766`).
- **D3. Long-poll blocking tool as the listen loop.** hcom `listen`, Cotal `cotal_wait` and agent-peers
  `wait_for_peer_messages`. We have no wait command, and the skill forbids polling
  (`integrations/skill/SKILL.md:79`).
- **D4. Piggyback on the tool's own call results.** hcom appends pending mail to every hcom command. agent-peers
  prepends it to every tool. Cotal adds an "N new messages waiting" rider. We print pending rows only from
  `inbox` and from text `accept` (`src/cli/accept_delivery.rs`). I found no other piggyback: `grep -rni piggyback
  src integrations` returns nothing.
- **D5. MCP server as the integration surface.** ruflo, mcp_agent_mail, Cotal, agentchattr, xats, agent-peers
  and CAO all use one. Identity is bound per MCP session, per proxy (agentchattr stamps the sender into
  `tools/call`) or per token. We use a CLI plus hooks, with identity from the env and the process ancestry
  (`src/harness/adapter.rs:48`).
- **D6. Stop-hook block-and-continue.** agmsg's Stop output happens to be accepted by Goose main; nobody ships
  it for Goose. We have no Stop-block delivery for any harness.
- **D7. Hook-only Open Plugin for turn edges** (Cyclops). We install nothing for Goose.
- **D8. ACK semantics.**
  - Ack on return or flush: hcom, Cotal, ruflo, agmsg, CAO.
  - Confirm on the next call with cancel rollback: agent-peers.
  - Exact `UserPromptSubmit.message` match: Cyclops.
  - Sender-visible read receipt within 10 s: xats.
  - Ours: ACK on full display after write and flush, plus explicit `--require-ack` receipts.
- **D9. Identity for an unrecognised agent.**
  - The model has to remember a name: hcom `--name`, ocs `--as`, ruflo agent argument, agent-talk `--dir`.
  - Borrowed types: agmsg registers as `codex`, agent-peers as `codex`/`hermes`.
  - An explicit "unknown agent" class: hcom `adhoc`.
  - Ours: **Human** (`src/cli/caller.rs:76-77`).
- **D10. Accidental skill exposure.** agmsg (`~/.agents/skills`), ocs (`~/.claude/skills`) and Cyclops (a
  seeded shared skill) all reach Goose through its global skill roots (`goose@1.36
  crates/goose/src/skills/mod.rs:288-317`, VERIFIED in the `agent-talk`/`agmsg` reports). We install no skill
  anywhere Goose reads, so Goose never learns the protocol.
- **D11. Process ownership.** agentchattr runs Goose inside its own tmux session. xats, Cyclops and CAO bind
  tmux panes. ocs binds cmux surfaces. We use Herdr panes and Herdr's agent API.
- **D12. Retry and backstop.**
  - xats retries at 30/180/600 s and stops once the recipient has read the message.
  - CAO runs a 30 s reconcile sweep.
  - Ours retries at 30/60/120/300 s (`src/notification/policy.rs:49`), with re-offers bounded to 60 s and 3 per
    generation (`src/delivery/ledger.rs:22,24`).
- **D13. Bounds and framing.**
  - Cotal: build, measure, drop the tail, with a redelivery-count marker.
  - ocs: 5120 B notes, with protocol lines neutralised in the body (`ocs`: `src/wake.ts:85-98`).
  - hcom: 50 messages per batch.
  - Ours: 32 blocks / 64 KiB pages and 8 KiB body cuts on hook and channel paths (`src/service/delivery_pull.rs:37,40`,
    `src/protocol/watch.rs:63`), 16 KiB text pages. A per-message header line.
- **D14. Submit confirmation.**
  - CAO waits up to 8 s against a post-clear boundary and retries Enter if it reads the composer.
  - Cyclops checks the readback (which always fails on Goose).
  - Ours re-reads the composer and classifies `HoldingPrompt` when a wake text is still visible after the last
    `>`/`›` prompt row (`src/host/native.rs:2485-2512`, VERIFIED). INFERRED: that parser is generic enough to
    recognise Goose's `"> "` row.

## 3. Pro/con of each difference

Dimensions: latency (L), reliability (R), token cost (T), draft safety (D), setup footprint (S), harness-version
fragility (V), cross-machine (X), trust (Tr). "Theirs" means the alternative's choice and "ours" means our current
choice. All judgements are INFERRED from the cited mechanisms.

**D1. Message-as-wake vs a content-free marker.**
- Theirs, pro:
  - L/T: one provider call per message. ocs's body-carrying note lets Goose act in the same turn.
  - Our `MARKER` costs a wake turn plus an `inbox` tool call plus another provider call, so at least two calls.
  - On Goose, which has no turn-start injection, `WAKE_UP` cannot be used honestly.
- Theirs, con:
  - T: every message becomes a turn; there is no threshold. CAO and xats wake once per send.
  - Tr: the body arrives as an unattributed **user** message (ocs on cmux, CAO). The model cannot tell peer from
    human beyond a text header.
  - D: a longer typed payload widens the window in which it collides with a human.
  - R: multi-line bodies split into several prompts when bracketed paste is off (CAO §byte transport).
- Verdict: keep content-free. If a Goose adapter ever gets MOIM delivery (idea 2), `WAKE_UP` becomes honest and
  the extra round trip disappears.

**D2. Gating.**
- Theirs, pro: S: no dependency on Herdr classification, so they "work" on unknown harnesses.
- Theirs, con:
  - D: concatenated drafts (xats, Cyclops, agentchattr, ocs).
  - R: a wake into a busy Goose sits in the tty and becomes the next prompt (primer §9). With `ICRNL` it may
    become an **unsubmitted draft**, because LF is Goose's Ctrl+J newline (`agentchattr` §3.3; binding VERIFIED at
    `goose@1.36 crates/goose-cli/src/session/input.rs:84-92`).
  - Tr: an Enter typed during a `GOOSE_MODE=approve` select approves a tool call (`agentchattr` §3.4, `xats`,
    `cyclops`).
- Ours, con: L/R: no wake at all on Goose today.
- Verdict: our gate is right. What is missing is the *evidence source* that feeds it (flaw F2).

**D3. Long-poll listen.**
- Theirs, pro: L is sub-second (hcom TCP notify). Nothing is typed, so drafts are safe by construction.
- Theirs, con:
  - T: every delivery, and every empty Cotal return, is a full-context provider call inside one ever-growing turn.
    This counts against Goose's `max_turns` of 1000 (`hcom` §3.2).
  - R: it depends on the model obeying "call again".
  - V: it races Goose's 300 s default extension timeout (`agent-peers`; `goose@1.36
    crates/goose/src/config/extensions.rs:11`).
  - D: the human is locked out of the pane, because Goose reads the tty only for Ctrl+C while busy.
  - It violates delivery-contract rule 2 (idle stays idle).
- Verdict: reject as a default. At most, offer an opt-in `wait` for a human who explicitly says "stand by" (idea 8).

**D4. Piggyback on the tool's own results.**
- Theirs, pro:
  - L: mail arrives mid-turn whenever the agent touches the tool anyway.
  - T: no extra call, and the quiet cost is zero (Cotal's rider is one line).
  - R: it is harness-neutral and needs no hooks, so it works on Goose, hookless Codex and any unrecognised harness.
  - V: no harness-version dependency.
- Theirs, con:
  - Tr/T: per-delivery instruction headers (agent-peers' ~480 B header contradicts R17).
  - R: it fires only while the agent is already using the tool.
  - Machine output needs an opt-out; hcom skips it under `--json` (`hcom`: `src/cli_context.rs:362-364`).
- Ours: delivery to hookless callers happens only on `inbox` or `accept`.
- Verdict: worth adopting in a neutral form (idea 3).

**D5. MCP server vs CLI plus hooks.**
- MCP, pro:
  - S: one `--with-extension`/`--with-streamable-http-extension` flag and no config writes (agentchattr's
    `proxy_flag`).
  - Tr: identity is stamped at the transport, so the model never supplies it (agentchattr proxy). Goose exports
    `AGENT_SESSION_ID` to stdio extensions (primer §6), which solves Goose 1.36's missing shell-tool session id.
- MCP, con:
  - T: tool schemas are paid on every provider call. Estimates: xats 8-12 k tokens, agentchattr ~2.5 k tokens of
    schemas and instructions, ruflo needs `--tools team`.
  - Server notifications never reach a Goose model (primer §6), so MCP adds no push and no wake.
- Ours, pro: zero standing schema cost.
- Ours, con: attribution needs an env marker that Goose does not set.
- Verdict: an MCP surface is not worth its per-call cost for delivery. Its one real benefit, identity at the
  transport, can be had more cheaply through a hook- or launch-exported env marker (idea 4).

**D6. Stop block (Goose main).**
- Pro: L: it keeps a finishing turn alive when mail arrived, with no typing (D-safe).
- Con:
  - Tr/T: fixed framing, "...Address this policy hook denial before trying to stop again."
    (`goose@main crates/goose/src/agents/state_machine/ops_stop_hook.rs:22-24`, VERIFIED). That misstates peer mail
    as a compliance demand and conflicts with R17.
  - It is capped at 8 blocks.
  - V: on 1.36 Stop is fire-and-forget, so a design that marks mail read on stdout write loses it silently (agmsg
    analysis).
- Verdict: at most a "mail arrived, do not stop yet" nudge that consumes nothing. Low priority.

**D7. Hook-only plugin for edges.**
- Pro:
  - S: the footprint is one directory, `~/.agents/plugins/<name>/hooks/hooks.json`, which Goose loads with no
    `plugin.json` (`cyclops`, VERIFIED in `goose@1.36 plugins/formats/open_plugins.rs:14-21`).
  - R: it gives turn edges (`UserPromptSubmit`/`Stop`) and `session_id` on both versions (primer §3.2-3.3).
- Con:
  - V: a new session is needed to load it.
  - The Stop hook is blocking on main, which adds latency at every turn end (`cyclops` limitation 8).
- Verdict: this is the cheapest way to unlock everything else (idea 1).

**D8. ACK semantics.**
- Ack on return or flush (hcom, Cotal) is like our text-inbox rule, and it has the same Goose risk: the shell tool
  truncates beyond 2000 lines / 50,000 bytes to a 50-line preview (`goose@1.36
  .../developer/shell.rs:150-152`, VERIFIED). On main the preview is capped at 10,000 B (`goose@main shell.rs:164`,
  VERIFIED).
  - Our default text page is 16 KiB (`src/protocol/pagination.rs:6`, VERIFIED), below that threshold. So at
    default bounds our ACK-on-flush is sound on Goose. INFERRED. A caller-raised `--max-bytes` could break it.
- agent-peers' confirm-on-next-call and Cyclops' exact-payload hook match are the honest options for a surface
  with no display receipt, such as MOIM (idea 2).
- Sender-visible read receipts (xats) overlap with our `--require-ack`.

**D9. Identity for unknown agents.** This is flaw F1.
- hcom's `adhoc` class is more honest than our Human fallback.
- Borrowed types (agmsg, agent-peers) are worse: they misdescribe the harness and route the wrong wake (agmsg's
  Codex marker poke exits 15).
- Model-remembered names (hcom, ocs) break after compaction or an hour of inactivity (`hcom` §3.2.4).

**D10. Skill exposure.**
- Pro: zero setup. The skill reaches Goose because Goose reads `~/.agents/skills` and `~/.claude/skills`.
- Con (Tr/R): a skill written for another harness gives Goose wrong identity instructions (agmsg's Codex-typed
  skill, ocs's "auto-detected inside Claude, Codex, and Pi").
- Lesson for us: if we ever install into a shared skill root, the text must be harness-neutral and must not assert
  auto-detection.

**D11. Process ownership.**
- tmux ownership gives the alternatives a pane they control.
- Con:
  - X/S: a nested tmux inside a Herdr pane hides Goose from Herdr (`agentchattr` §4).
  - xats has no wake at all without tmux (`xats` limitation 3).
- Ours: native Herdr, but dependent on Herdr's detection (F2).

**D12. Retry.**
- xats stops retrying once the recipient has read (`auto-poke-fanout.ts:141,167`). That avoids stale wakes.
- CAO's sweep is a backstop for missed events. Our ladder plus the store-driven wake frontier already re-evaluates
  per attempt. No change needed. INFERRED.

**D13. Bounds and framing.**
- Cotal's build-measure-drop guards against framing writers that were not budgeted.
- ocs neutralises body lines that mimic its own header.
- I did not check whether our frame neutralises a body line that starts with `[herdr-threads]`. Our bodies are
  indented two spaces (`src/protocol/output_compact.rs:7-15`, per `ours`), which already keeps peer text off
  column zero. INFERRED to be adequate.

**D14. Submit confirmation.**
- Ours reads the composer only.
- A Goose `UserPromptSubmit` hook whose `message` equals our wake text (the field is populated in both versions,
  `cyclops` §ACK) would be a stronger "wake landed" receipt. It costs nothing extra once idea 1 exists.

## 4. Flaws in our approach that the others expose

1. **An unrecognised agent is attributed as a person.** Severity: medium.
   - Evidence: `caller::derive` maps `NoHarness` with no env marker to `CallerOrigin::Human`
     (`src/cli/caller.rs:74-80`, VERIFIED).
   - The A4 guard's only other evidence is a Herdr kind that is a registered harness (`src/cli/mod.rs:355-372`,
     VERIFIED).
   - Goose 1.36's shell tool exports only `PATH` (primer §11).
   - So a Goose model can `human me init` and ACK, send or accept as `operator_human` (`TRUST-POLICY.md:307`).
   - `grep -i "unsupported harness\|unknown harness" TRUST-POLICY.md` finds nothing (VERIFIED), so this is not
     recorded as an accepted limit.
   - Contrast: hcom falls back to an explicit `adhoc` class (`hcom`: `src/shared/tool_detection.rs:79-98`).
2. **Wake is impossible without Herdr classifying the pane.** Severity: high for Goose, and for any harness Herdr
   does not list.
   - Evidence: `cooperative_wake_ready` requires a registry-mapped Herdr kind and `agent_status` ∈ {idle, done}
     (`src/host/native.rs:53,1682-1735`, VERIFIED).
   - CAO shows the failure shape on the same host: on the herdr backend, an unknown kind gets no status events,
     only a 30 s sweep (`cao`: `backends/herdr_backend.py:866-912`).
   - Herdr 0.9.5 exposes `herdr pane report-agent --source ID --agent LABEL --state idle|working|blocked|unknown
     PANE_ID` (observed `--help`, VERIFIED), which a Goose hook could drive. Whether that sets `pane.agent` is
     unknown (`ours`).
   - Nothing in our design treats our own hook edges as idle/busy evidence.
3. **`DeliveryPolicy` cannot describe Goose's in-turn surfaces.** Severity: medium.
   - Evidence: `DeliveryMechanism` is `{PushChannel, HookPull}` (`src/harness/adapter.rs:447-451`, VERIFIED).
   - The registry accepts only `PushChannel+mod` and `HookPull+hook+Native` (`src/harness/registry.rs:1268-1282`,
     VERIFIED).
   - The MOIM file (a pulled context file) and tool-result piggyback fit neither, so a Goose adapter must either
     misdeclare itself or stay at `MARKER` + `inbox`.
4. **Hookless callers get no in-band delivery on their other calls.** Severity: medium.
   - Evidence: delivery outside hooks and channels exists only in text `inbox` and `accept`
     (`src/cli/accept_delivery.rs`; grep for piggyback finds none, VERIFIED).
   - hcom, agent-peers and Cotal show the cheap fix.
   - This affects Goose, hookless Codex bindings (which keep the `inbox` hint, per AGENTS.md) and every unsupported
     harness.
5. **On a hookless harness every wake costs two provider calls.** Severity: low; it is a cost, not a bug.
   - `MARKER` names `inbox` (`src/notification/policy.rs:14`), so the woken turn must call the CLI before the body
     arrives.
   - ocs and CAO deliver in one turn, at the trust cost described under D1.
   - Remedy: idea 2, not body-carrying wakes.
6. **The approval and elicitation exclusion depends on Herdr reporting `blocked`.** Severity: medium once a Goose
   wake exists.
   - Evidence: `ApprovalOrQuestion` is the skip (`src/notification/policy.rs:394`, VERIFIED), and it is fed only
     from Herdr status (`ours`, `src/host/native.rs:2354-2362`).
   - Every typing alternative can approve a Goose tool call (`agentchattr`, `xats`, `cyclops`).
   - CAO's declared adapter flag `blocks_orchestrated_input_while_waiting_user_answer`
     (`cao`: `providers/base.py:390-400`) shows the right shape: a Goose `ComposerPolicy` must classify the
     cliclack select or form screen itself.
7. **No tracking.** Severity: low.
   - No bead covers Goose support or the Human-attribution gap (`ours`, `bd search`).
   - Cyclops at least records its unmeasured state as a backlog item (`cyclops`: `docs/development/NEXT.md:9-11`).

Not flaws (checked):
- Our unknown-composer refusal is fail-closed. Cyclops' manifest *claims* fail-closed, but its gate never reads
  `write_ready` (`cyclops`: `gate.rs:1203-1323`).
- Our default page fits under Goose's shell truncation threshold (D8).
- Our wake texts contain no LF, so the Ctrl+J trap cannot fire from the text itself (`src/notification/policy.rs:14-17`,
  VERIFIED). Whether Herdr's `agent prompt` submits with CR or LF is unknown and must be probed.

## 5. Better integration ways, ranked by value/cost

1. **Goose hook-only Open Plugin for turn edges, plus Herdr self-report** (Cyclops footprint, CAO lesson). Value:
   high. Cost: medium.
   - Install `~/.agents/plugins/herdr-threads/hooks/hooks.json` with `SessionStart`, `UserPromptSubmit`, `Stop`,
     `SessionEnd` (+`PostToolUse`). Hooks inherit `HERDR_PANE_ID` (primer §3.3, INFERRED).
   - On `UserPromptSubmit`/`Stop`, call `herdr pane report-agent --agent goose --state working|idle`. If Herdr does
     not then set `pane.agent`, accept our own hook edges as idle evidence.
   - That second option is a trust change: client hook edges are hints, and the daemon decides (A2). It needs a
     TRUST-POLICY entry.
   - `UserPromptSubmit.message == wake text` doubles as a wake-landed receipt (D14).
   - First step: probe `report-agent` in an isolated Herdr session (`scripts/lib/isolated-herdr.sh`). Fixes F2 and
     strengthens D14.
2. **A Goose `ComposerPolicy`.** Value: high. Cost: low-medium. A prerequisite for any wake.
   - Empty means the `> ` row plus the dim `Enter to send · Ctrl+J newline` hint (primer §9).
   - Never declare `clear_key`/`composer_stash`: Ctrl+C deletes the buffer, and a second one exits.
   - Classify cliclack approval selects and elicitation forms as `ApprovalOrQuestion` without relying on Herdr
     (fixes F6).
   - Check that the foreground process is `goose` before typing (`cao`: `tmux.py:1781-1783`; `ocs`:
     `src/roster.ts:180-224`).
   - Treat `GOOSE_PROMPT_EDITOR` screens as Unknown.
3. **Neutral CLI piggyback for hookless callers** (hcom, Cotal rider, agent-peers). Value: medium-high. Cost:
   low-medium.
   - When a bound agent caller with no hook or channel delivery runs any text-mode `herdr-threads` command, append
     its pending rows, framed as `inbox` rows and within the same page bounds. ACK after write and flush, as text
     `inbox` does.
   - Or, cheaper still, append one `pending: N [ids]` + `next: herdr-threads inbox` line.
   - Skip it under `--json`/`--machine`.
   - Harness-neutral, so no adapter branch. Fixes F4 for Goose, hookless Codex and unknown harnesses.
   - Needs its byte size stated per token rule (f).
4. **An honest "unrecognised agent" caller class** (hcom `adhoc`). Value: medium (trust honesty). Cost: medium.
   - Option (a): a launch- or hook-exported env marker for Goose, `HERDR_THREADS_AGENT=goose`, set by the managed
     launch or found through ancestry of a `goose` process.
   - Option (b): as a policy, refuse `human me init` when the process ancestry contains a known agent binary that
     is not registered.
   - Either needs a TRUST-POLICY edit (A3/A4) and a new provenance if it attributes. Fixes F1 for all unsupported
     harnesses.
5. **An MOIM pointer before an MOIM page.** Value: medium. Cost: low once ideas 1-2 exist.
   - A managed launch sets `GOOSE_MOIM_MESSAGE_FILE` to a per-binding file (it must be in the process env at launch
     on 1.36; `goose@1.36 .../platform_extensions/tom.rs:63-110`, VERIFIED `std::env::var`).
   - The daemon writes one id-only line, `herdr-threads: N pending [p0] ids...; run herdr-threads inbox`
     (mcp_agent_mail's count-only reminder, R17-compact), and removes it when the receipts settle.
   - Consumption stays with `inbox`, so no new ACK provenance is needed. Peer bodies stay out of the context file,
     so trust is unchanged.
   - Gives a busy Goose a mid-turn hint after every tool batch, at about 60-100 B per provider call while pending.
   - Caveats:
     - it is skipped below a 32 k-token context limit (`goose@1.36 extension_manager.rs:1894-1901`, VERIFIED);
     - in 1.36 it is dropped silently if `fix_conversation` reports unexpected issues
       (`goose@1.36 crates/goose/src/agents/moim.rs:36-47`, VERIFIED);
     - the `MARKER` wake remains the fallback.
   - Needs `DeliveryPolicy` growth or a non-delivery "hint" declaration (F3).
6. **Full MOIM page delivery** (`DeliveryMechanism::ContextFile`). Value: high. Cost: high.
   - The daemon atomically renames a bounded page into the MOIM file (ruflo's rename trick).
   - It is additive only, never consuming (agmsg's PostToolUse stance). Confirm on the next `PostToolUse`/`Stop`
     for that `session_id` (agent-peers' confirm-on-next-call), then clear.
   - Mark redelivered items (Cotal).
   - Watch the costs:
     - 1.36 re-sends the block on every provider call (k tool batches cost k × page);
     - `read_bounded` keeps the **first** 64 KiB (`tom.rs:15,27-47`, VERIFIED), so the file must be a rewritten
       "pending now" view, never a log;
     - a new A3 provenance and a TRUST-POLICY DP9-style edit are needed.
   - Makes `WAKE_UP` honest (R18) and removes the second provider call (F5).
7. **Opt-in bounded `wait`** (Cotal `cotal_wait`, agent-peers sender filter). Value: low-medium. Cost: low.
   - Only on an explicit human "stand by for X", with a timeout below the harness tool timeout. Never the default
     (rule 2).
8. **Goose-main Stop block as a non-consuming "mail arrived" nudge.** Value: low. Cost: low.
   - The framing is fixed and misleading, there is a cap of 8, and it is main-only. Consider it only behind a
     per-seat opt-in.
9. **ACP-hosted Goose launch** (`goose serve`/`goose acp`: `session/prompt` wake, `steer` with pickup ids,
   `SessionInfoUpdate` busy/idle; primer §7). Value: high in isolation. Cost: very high. It is a different product
   shape with no Herdr pane composer, and no alternative uses it. Track it; do not build it.

Rejected:
- MCP server delivery (standing schema cost, no push; D5).
- Body-carrying or ungated typed wakes (D1/D2).
- `goose run -r` headless relaunch while a pane holds the session (two agents on one conversation, primer §8).
- Writing to `sessions.db` (no lock, primer §8).
- PreToolUse deny-as-delivery (destroys the tool call, primer §3.4).

## 6. Primer surfaces nobody uses yet

None of the thirteen designs, ours included, uses any of these (VERIFIED by absence per report; Cyclops' grep covers
MOIM, steer, `session/prompt` and `goose run`):

| Surface (primer §) | Potential | Note |
|---|---|---|
| `tom` MOIM file / `GOOSE_MOIM_MESSAGE_TEXT` (§5) | in-turn, after every tool batch, no tool call | ideas 5-6. Noted as an idea by `agent-talk`, `cotal`, `agmsg`, `mcp-agent-mail`, `agent-peers`, `cyclops` and `xats`; nobody built it |
| Stop block-and-continue (main, §3.4) | turn-end delivery | agmsg's output is accidentally compatible; not shipped |
| SessionStart `banner` (main, §3.4) | once-per-session text | model visibility unknown |
| ACP `session/prompt`, `_goose/unstable/session/steer` + pickup ids, `system-prompt/set`, active-run `SessionInfoUpdate` (§7) | cleanest wake, mid-turn and busy/idle API of any harness | ACP-hosted sessions only |
| `AGENT_SESSION_ID` to stdio extensions (§6) and in the shell tool (main, §11) | identity binding | nobody reads it (`mcp-agent-mail`, `ocs` grep) |
| Hook `session_id` + inherited `HERDR_PANE_ID` (§3.3) | pane ↔ session binding | Cyclops uses ancestry instead |
| `goose session list --format json` (§8, §11) | session discovery by name or working dir | none |
| MCP sampling (1.36 only, §6) and elicitation (§6) | out-of-session completion; a structured question to the user | unsuitable for delivery (INFERRED) |
| `notifications/tools/list_changed` (main, §6) | dynamic tool surface | none |
| `goose term log` → session-store messages (§8) | Goose's own between-turn injection | unsafe for a live pane (no lock) |
| Herdr `pane report-agent` (not Goose; observed help) | gives Herdr a classification for an unlisted harness | idea 1 |

Surfaces in use, for contrast:
- Global skill roots: agmsg and ocs by accident; Cyclops seeds a shared skill; hcom ships a Gemini-format skill
  extension.
- Open Plugin hooks: Cyclops only.
- `--with-streamable-http-extension`: agentchattr; xats by hand.
- MCP server instructions in the system prompt: agentchattr, agent-peers, mcp_agent_mail.
- Keystroke typing: agentchattr, xats, Cyclops, ocs (cmux) and CAO (hypothetically).

## 7. Verification

What I re-checked for this comparison, beyond the input reports' own verification passes:

- **herdr-threads, by reading at `2de3da1c4`:**
  - `builtins()` (`src/harness/registry.rs:1482-1490`);
  - hook usage error (`src/cli/hook.rs:217-224`);
  - Human fallback (`src/cli/caller.rs:74-80`) and A4 evidence (`src/cli/mod.rs:355-372`);
  - `DeliveryMechanism`/`IdleWake`/`wake_text` (`src/harness/adapter.rs:447-472`) and pairing rule
    (`registry.rs:1268-1282`);
  - `MARKER`/`WAKE_UP`/`WAKE_TEXTS` (`src/notification/policy.rs:14-21`), retry `STEPS_MS` (`:49`), poke decisions
    (`:385-395`);
  - `WAKE_READY_STATUSES`, `FOCUSED_EMPTY_MILLIS`, `cooperative_wake_ready`, the `agent prompt` call and the
    submission parser (`src/host/native.rs:53,71,1682,1898,2485-2512`);
  - `read_composer_in` (`src/harness/composer.rs:58-73`) and the no-bound-harness refusal
    (`src/notification/dispatch.rs:325-333`);
  - page defaults 20 / 16,384 (`src/protocol/pagination.rs:4,6`); hook page bounds (`delivery_pull.rs:37,40`) and
    8 KiB cut (`watch.rs:63`); re-offer constants (`ledger.rs:22,24`);
  - no piggyback, no MOIM, no wait command (grep);
  - no TRUST-POLICY entry for unsupported harnesses (grep).
- **Goose, by reading:**
  - `tom.rs` 64 KiB `MAX_BYTES`, first-bytes `read_bounded`, `std::env::var` in 1.36;
  - `inject_moim` before every provider call (`agent.rs:1671-1682`), insertion before the last assistant message,
    and silent drop on unexpected `fix_conversation` issues (`moim.rs:20-50`);
  - 32 k context skip (`extension_manager.rs:1894-1901`);
  - Stop-block framing (`goose@main ops_stop_hook.rs:22-24`);
  - Ctrl+J newline key (`input.rs:84-92`);
  - shell limits 2000 lines / 50,000 B / 50-line preview (1.36) and 10,000 B preview (main).
- **Herdr:** `herdr pane report-agent --help` (observed; no server state touched).
- **Taken from the input reports without re-reading the alternative clones:** every alternative-side citation in
  sections 1-3. Those reports each record their own adversarial pass.
- **Corrections relative to the inputs:**
  - `ours` lists the inbox bounds only as "ordinary page bounds". I pinned them to 20 items / 16 KiB, which makes
    our ACK-on-flush safe under Goose's shell truncation at default bounds.
  - New here: the 1.36 MOIM silent-drop path.
