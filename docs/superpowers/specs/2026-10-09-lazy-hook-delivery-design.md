## Goal

A direct user request amends lazy delivery. Pending lazy messages reach the seat's top-level agent at its
next eligible standard hook opportunity (native SessionStart lifecycle, Bash PreToolUse, and qualified
turns), including when lazy mail is the only thing pending. The user's follow-up clarification (relayed
verbatim by Main): "yes, let hook inject the inbox contents, with instructions to keep retrieving if
necessary. lets fix the trigger. attention digest should change on lazy msgs too. just nudge is skipped."

So: (1) a lazy arrival changes the attention digest the hook compares, so the next hook presents it;
(2) the presenting hook injects a bounded page of the actual inbox contents, with exact retrieval
commands when more remains; (3) lazy mail still never wakes or nudges an idle session and still creates
no receipt, ACK obligation, deadline or warning.

This supersedes the 2026-10-07 lazy design's rules "Never add lazy rows to attention counts, tokens,
ready commands, Current/Startup hook offers" and "No hook emits an inbox reminder solely because lazy
content exists", for the hook's digest only. Wake batching, host prompt/poke queues, warnings, deadlines
and the wake frontier stay lazy-free.

## Why this cannot wake

Standard hooks run only while the harness is already active: a session start, a turn, or a tool call the
agent is about to make. Wake decisions read `wake_seat_attention` and the scheduler's logical frontier,
neither of which reads lazy rows. The lazy part of the digest exists only in a digest the hook explicitly
asks for; nothing schedules work, pokes a pane or prompts a session because of it.

## Decisions

- **Digest trigger, opt in.** `AttentionDigestQuery` gains `lazy: bool` (omitted when false). A daemon
  advertising `hook.lazy_delivery_v1` then adds `AttentionDigest.lazy` (a bounded class: count, newest
  IDs, `has_more`) and the token's lazy key: the newest pending published lazy row's (publication
  decision sequence, recipient ordinal). The key advances `advanced_beyond` and joins like the other
  components, so a lazy-only arrival moves the hook's mark, and completing rows never makes an old key
  look new. A token without a lazy key keeps its `v1.` spelling; one with it is `v2.INV.REC.WARN.LAZY.EP`.
  Old clients never ask and see the unchanged wire shape; old daemons are never asked. The walk takes
  one `WINDOW` of the pending seat/ordinal index (newest ordinal first) before testing publication, so
  an unpublished backlog cannot stretch it; a row published so late that more than `WINDOW` newer
  pending rows exist is still found by inbox, only not by the marker (accepted, bounded).
- **Inbox contents in the hook.** When a top-level hook presents (the tool boundary's marker advanced,
  or a lifecycle/qualified-turn check-in), it reads one bounded read-only v2 inbox page (8 rows) and
  appends the complete items that fit the bytes left under the 4096-byte context bound after the
  ordinary context, unchanged. The rows are exactly what `herdr-threads inbox` prints, JSON-escaped
  inside `inbox_peer_data` under a fixed plugin-authored header. Only a complete prefix of the page is
  shown: never a body prefix, never an item after one that did not fit. When anything remains, one line
  gives the exact `inbox` command to keep retrieving; otherwise one line says nothing else is pending.
  No page is read while a Claude mod channel is live (the mod delivers), for subagent events, or when the
  marker did not advance.
- **No ACK by the hook.** ACK-required messages shown get one exact `herdr-threads ack ID...` command
  for the agent to run after reading (receipt only). The hook never ACKs and defines no receipt
  provenance; `inbox` remains the display-ACK path and repeats what is not yet ACKed.
- **Honest lazy bookkeeping.** Lazy messages shown whole are completed with the existing
  `CompleteInboxDelivery` under the registered execution's top-level claim, with a fresh operation and
  `via: hook_context` (`cooperative_hook_context` in TRUST-POLICY), only after the hook wrote and flushed
  stdout and the adapter reported that the output carries context. A failed, timed-out or unknown
  completion leaves the rows pending (shown again by inbox or the next presenting hook; at least once).
  Nothing is written to the intent or context journals for this.

## Harness coverage

- Claude: SessionStart and Bash PreToolUse (`additionalContext`). With the bundled mod live, the hook
  adds no page and the mod keeps appending lazy rows.
- Codex: SessionStart (all sources) and Bash PreToolUse through the same path.
- Hermes: `pre_llm_call` qualified turns and lifecycle callbacks take the shared check-in path and deliver
  `context`. `post_tool_call` and `on_session_reset` are observer callbacks that carry no context and
  never complete anything.

## Compatibility

Without `hook.lazy_delivery_v1` (or the v2 inbox) the hook behaves as before. `via` is omitted for inbox
completion, so existing completion payload digests are unchanged. Stored attention marks without a lazy
key keep their v1 spelling; an older binary that meets a v2 mark treats it as absent and re-presents once.

## Verification

Protocol: v2 token spelling, canonical decode, advance/join with the lazy key; digest lazy class bounds.
Store/integration: a lazy publication moves the lazy-requesting digest and nothing that wakes (wake
attention, wake candidates and attention producers unchanged; the default digest unchanged).
Hook (real hook process and daemon): lazy-only arrival is presented and its row injected at the next
Claude and Codex PreToolUse, completed only after delivery, then quiet; ordinary and lazy rows together
with an exact ack command and no hook ACK; subagent calls quiet; an oversized body moves the marker but is
never shown or completed. Hermes qualified turn: page injected, only whole lazy IDs deferred to delivery.
Unit: page fitting (whole items only, page order, bound, retrieval line, escaped peer text).
