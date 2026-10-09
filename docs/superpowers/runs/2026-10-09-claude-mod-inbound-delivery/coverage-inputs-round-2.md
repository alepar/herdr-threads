## Goals

### ht-j16 (root)
A Claude Code session in a Herdr pane receives its herdr-threads messages from a bundled Claude Code mod, without a terminal send-keys nudge:
- between tool calls while a turn runs;
- as a new turn when the session is idle;
- passively for lazy messages.

Each fully delivered message settles its receipt. Whenever the mod is not connected, delivery falls back to today's hooks plus send-keys wake, losing nothing.

Mode B (one-shot), autonomous super-auto run. Inputs:
- spike: [`docs/research/claude-mod-delivery-spike/README.md`](../../../research/claude-mod-delivery-spike/README.md)
- research: `~/Documents/Claude_Code_Mods_Delivery_Research_20261008/report.md`
- normative: [TRUST-POLICY.md](../../../../TRUST-POLICY.md)

## Task tree

- ht-j16 · Claude Code mod inbound delivery (epic) · Root epic for super-auto run 2026-10-09-claude-mod-inbound-delivery. · deps: none
  - ht-j16.1 · Seam contract: mod watch protocol, ack command, provenances · Compilable inert boundary for the watch protocol, AckModDelivered, JSON-line schema, ModChannels trait, provenances, stub mod files and the TRUST-POLICY amendme · deps: none
      owns: watch wire types, AckModDelivered shape, watch JSON-line schema, provenance constants, ModChannels trait, mod file layout.
      owns: watch/watch ack CLI invocation contract (argv: watch --harness claude --session <id>; watch ack --session <id> --via context|submit|append <ids...>; env H
  - ht-j16.2 · Daemon watch connection and ModChannels registry · Daemon watch connection kind and the ModChannels registry: A2 registration, push frames, close, replace, stall state, status read, disconnect kick. · deps: ht-j16.1
      owns: ModChannels implementation (is_live, register/unregister, notify).
      consumes: watch wire types and ModChannels trait (ht-j16.1).
  - ht-j16.3 · AckModDelivered deciding transaction · Daemon AckModDelivered: settle fully delivered ordinary receipts as cooperative_mod_delivery and complete lazy rows, refusing per id. · deps: ht-j16.1
      owns: AckModDelivered semantics.
      consumes: AckModDelivered shape, provenance constants, ModChannels trait (ht-j16.1).
  - ht-j16.4 · Route wakes around live mod channels (suppress, stall, disconnect) · Wake routing: suppress native prompts, pokes and hook digests while a mod channel is live; let the native ladder run while stalled. · deps: ht-j16.1
      owns: wake routing policy for mod channels, mod_channel_live check-in flag.
      consumes: ModChannels trait (ht-j16.1).
      owns: mod_channel_live check-in result field AND its sole consumer, the PreToolUse hook digest omission in src/cli/hook.rs (producer and consumer both in this b
  - ht-j16.5 · herdr-threads watch and watch ack CLI · The herdr-threads watch CLI that streams bounded JSON lines of pending items, and watch ack that reports mod deliveries. · deps: ht-j16.1
      owns: watch CLI behaviour.
      consumes: watch wire types, JSON-line schema, AckModDelivered shape (ht-j16.1).
  - ht-j16.6 · Claude mod: delivery state machine and plugin tests · The Claude mod: watch child lifecycle, turn-id busy tracking, context/submit/append delivery with post-abort hold, acks, ledger, plugin tests with randomized st · deps: ht-j16.1
      owns: mod behaviour.
      consumes: watch JSON-line schema and mod file layout (ht-j16.1).
      consumes: watch/watch ack CLI invocation contract (ht-j16.1).
  - ht-j16.7 · Install the mod via setup claude (CLAUDE_CODE_PLUGIN_DIRS) · setup claude installs the embedded mod and appends it to env.CLAUDE_CODE_PLUGIN_DIRS under the owned manifest; unsetup and setup-status. · deps: ht-j16.1
      owns: mod install/uninstall.
      consumes: mod file layout (ht-j16.1).
  - ht-j16.8 · Gate: fix loop exited · Released by super-auto when the phase-5 code fix loop exits. · deps: none
  - ht-j16.9 · Live race stress: mod delivery in real Claude TUI sessions · Gated final-SHA live stress of mod delivery in real Claude TUI sessions with evidence, read-only fallback to unit stress. · deps: ht-j16.10, ht-j16.2, ht-j16.3, ht-j16.4, ht-j16.5, ht-j16.6, ht-j16.7, ht-j16.8
  - ht-j16.10 · Integration sweep: mod delivery end to end (daemon, watch CLI, mod, ack, fallback) · Automated non-live end-to-end proof of the goal's main flows before the gated live stress. · deps: ht-j16.1, ht-j16.2, ht-j16.3, ht-j16.4, ht-j16.5, ht-j16.6, ht-j16.7
