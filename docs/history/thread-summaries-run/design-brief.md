# Design brief: thread summaries for compaction survival + soft-deadline ACK poke

Agreed with the user in a Mode A brainstorm on 2026-10-02 (every section approved). This is the design input for super-auto: write two specs from it under one epic. Do not re-ask any decision recorded here.

## Goal

After an agent's context is compacted (or the session is resumed or cleared, or the agent joins a long thread), the content of its threads comes back as a shared, reusable, structured summary. Thread content then survives compaction second only to user messages. Separately, receipts close to their deadline get a safe poke before the hard-deadline warning fires.

## Problem today

- The wake path (`src/notification/policy.rs` MARKER) types only a fixed marker into the pane as a user turn.
- Message bodies arrive as tool results from `inbox`/`read`, and compaction drops tool results first.
- Codex already parses SessionStart `compact` (`EventKind::Compact`, coalesce=false re-presents pending attention). The Claude adapter rejects `source: compact` (`src/harness/claude.rs` ~154).

## Spec 1: Thread summaries and catch-up mode

### Triggers (option D)
- SessionStart `compact` on both harnesses. For Claude, add it to the recipe only with captured evidence (evidence-backed recipe registry rule).
- Resume and clear.
- Join or accept of a thread longer than one chunk: the join result points to `summary <thread>`.
- On demand: `summary <thread>`, documented in the bundled skill.

### Which threads after compaction (option B)
- "Hot" threads are summarized: pending receipts or attention, or a message in the last 24h.
- Every other thread gets a one-line overview row in the hook text, with `summary <thread>` on demand.

### Protocol (daemon-driven)
- `summary <thread>` returns one of:
  - `Ready{blocks, tail, frontier}`: the daemon concatenates the stored blocks, choosing display levels to fit about 10k tokens, and appends the raw tail.
  - `Work{jobs[]}`: each job has a lease token, input, target level and budget. Only jobs whose inputs exist are returned, so rollups follow their children.
- `summary job <id>` returns the job's input bundle. `summary submit <id>` takes JSON on stdin (ledger proposals plus narrative).
- The parent agent runs jobs in parallel through cheap subagents: the Agent tool with Haiku on Claude, a small-model subagent on Codex. If neither is available, the parent runs them sequentially itself. It then repeats `summary <thread>` until Ready.
- Chunking is deterministic: a chunk closes at the first message boundary after about 6k tokens, and messages are never split. An oversized single message is its own chunk.
- Only full chunks are summarized. The partial tail is always returned raw (option A).
- Rollups are index-aligned: level n+1 block k covers children [8k, 8k+7] (fan-in 8). The ~10k budget only decides which levels are displayed.
- A lease prevents duplicate work, and an expired lease returns the job to the pool. Blocks are immutable and shared by every reader of the thread. Reading blocks follows the same visibility rule as reading thread history.

### Block format
- **Header:** range{first_msg, last_msg}, level, children, model, prompt_version, source_hash.
- **Ledger:**
  - `user_instructions[]`: extracted by the daemon from human-authored or `--relays-user` messages, verbatim, or a msg-id pointer if over the cap. The model only marks status.
  - `decisions[]`: {id, msg_id, by, text}.
  - `open_items[]`: {id, kind: ask|commitment|question|blocker, from, to, msg_id, text, status, resolved_by_msg}.
  - `identifiers[]`: paths, bead ids, SHAs, URLs, error strings, extracted by regex in the daemon.
- **Narrative:** about 600–800 tokens of seat-attributed prose (roughly 10x).
- **Carry-forward:**
  - The daemon carries the ledger forward deterministically. Open items and unsuperseded user instructions are never dropped.
  - The model only proposes status changes, which the daemon accepts only when they cite a msg-id inside the block's range.
  - A resolved item becomes one line at the next level and is dropped the level after.
- **Rollup input:** child narratives, the merged ledger and the raw text of pinned quotes. Raw chunks are never re-read.

### Validation on submit (deterministic)
1. Quotes are exact substrings of the cited message.
2. msg-ids fall inside the block's range.
3. Every human or relayed message in the range appears in `user_instructions`.
4. Every child open item appears in the parent.
5. The output is within budget.

On failure, reject and retry once, then fall back to a ledger-only block with no narrative. Summary text is presented escaped and labelled as derived from peers.

### `send --relays-user` (option B)
An agent flag on `send` marking that the message forwards a user ask. It is a cooperative claim, and these messages get the same priority as human-authored ones.

### Catch-up mode
- `summary <thread>` enters catch-up at a fixed frontier F, the newest message id at entry.
- While catching up, the daemon does not offer or wake for messages after F on that thread. They stay pending.
- Warnings, invitations and human or relayed messages bypass the hold.
- Catch-up ends when the agent collects Ready. Held messages are then offered normally.

### Deadline extension (replaces any early-release rule)
- Frozen deadlines are never rewritten. An effective deadline is `max(frozen, extension)`, and warnings and overdue status use it.
- The extension covers every pending receipt the seat holds on threads it is catching up on, before and after F. Reading a summary is not a receipt.
- Entering catch-up sets the extension to entry + p99(job duration). Each completed job sets it to completion + p99. p99 comes from recorded lease-to-submit durations, with a cold-start default of about 90s.
- On exit, the extension is exit + grace (about 60s).
- On stall, the extension lapses: catch-up ends, held messages are released and warnings fire. There is no separate hard cap, because the job count is finite.
- The sender sees `deferred: recipient catching up (until T)` in delivery inspect and pending-receipts.

### Trust policy (update TRUST-POLICY.md in the same commit)
- New provenance `derived_summary`: author seat, child invocation, model. Summaries carry no receipts or ACKs and are never delivery. Children may submit them.
- New fact `deadline_extension`: decided by the daemon and triggered only by catch-up state the seat itself entered. Never applied on heuristic evidence.
- The scheduler design is amended so that warnings fire on the effective deadline.

### Store
- New tables: summary_blocks, summary_jobs (leases), catch_up (seat, thread, frontier, extension_until) and job durations.
- Coordinate the migration number with ht-xoc and ht-5nb.

### Errors
An expired lease is re-queued. A rejected submit is retried once, then gets the ledger-only fallback. If the daemon is down, the agent reads raw.

### Testing
- Unit tests:
  - chunker determinism and message-boundary cuts;
  - index-aligned rollups;
  - each validator rule;
  - ledger carry-forward;
  - catch-up hold, bypass and extension with injected clocks;
  - p99 cold start.
- Native validation on both harnesses: compaction hook, then parallel workers, then Ready.

### Research basis
The background deep-research found:
- Free-prose recursive summaries lose specifics and compound errors (Wu et al. 2021; BooookScore; FABLES; Context-Aware Hierarchical Merging).
- Structured, anchored sections help (Factory compaction evaluation).
- Identifier tracking is weak in every method.
- LLM faithfulness checkers miss many errors, so deterministic checks are preferred.

Numbers for Haiku-class models on multi-author threads are extrapolated, so make chunk size, fan-in, budget and ratio config values and plan an evaluation pass.

## Spec 2: Soft-deadline poke

- **Hard deadline:** the existing scheduler overdue warning to joined seats, unchanged except that it fires on the effective deadline.
- **Soft point:** configurable, defaulting to 60% of the receipt window, computed on the effective deadline.
- **Eligibility:** a running native agent whose pane is not focused, in any UI state except approval/question.
  - Skip when the seat is shell-only, unavailable or unresolved, or when the pane is focused. The hard-deadline warning covers those.
  - User rule: skip the poke whenever it cannot be done safely.
- **Text:** fixed, with no peer data: `herdr-threads: receipt due in <N>s on <thread-ids>; run herdr-threads inbox`. One poke per seat per soft point, coalesced across threads.
- **Input box:**
  - If the box holds text, stash it (read it, clear it), send the poke, then restore the text.
  - If the box cannot be read and restored reliably for that harness or version, skip the poke.
  - The not-focused rule makes stashing safe.
  - A spike is needed before implementation: can herdr read and clear the input box reliably (multi-line text, pasted images, Codex vs Claude)?
- **Testing:** fake-host eligibility tests across every state, focus and input-box case, plus a native stash/restore spike.

## Run preferences seen this session
- The user has run prior super-auto runs autonomously with both roasts on.
- Project bd memories: be light on reviews in the super-code phase, merge at 0 Blocking, and run one full serial suite before the super-roast.
