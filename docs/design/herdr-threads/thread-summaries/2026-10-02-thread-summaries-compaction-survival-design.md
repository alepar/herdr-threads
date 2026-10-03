# Thread summaries for compaction survival, and soft-deadline receipt pokes

Root spec for the run `2026-10-02-thread-summaries-compaction-survival`. The requirements were agreed
with the user section by section ([design brief](../../../history/thread-summaries-run/design-brief.md)); this spec turns them into
implementable contracts. [TRUST-POLICY.md](../../../../TRUST-POLICY.md) stays normative and is
amended by this work (§Trust policy amendments).

## Goal

After an agent's context is compacted, resumed or cleared, or the agent joins a long thread, it gets
back the content of its threads as a shared, reusable, structured summary. That makes thread content
the second most durable thing in its context, after user messages. Separately, a seat whose receipt is
close to its deadline gets a safe pane poke before the existing overdue warning goes to the thread.

## Problem description

Thread content reaches an agent only as tool output (`inbox`, `read`, hook `additionalContext` on Bash
PreToolUse). Harness compaction drops tool output first. The only thing that survives as a user turn
is the wake marker, `herdr-threads: attention pending; run herdr-threads inbox`, and it carries no
content. So after compaction an agent forgets the asks and decisions it received over threads,
including asks that relay its user's instructions. Missed receipts are also recovered only by the
hard-deadline warning. Nothing nudges a busy or idle agent before then, so the sender's thread sees
an overdue warning that one prompt could have prevented.

## Main challenges

- **Summarizing without the daemon running a model.** Herdr-threads has no model access. Summaries
  must be produced by the agents themselves, cheaply (small-model subagents), yet stored once and
  shared by every reader of the thread.
- **Keeping specifics across summary levels.** Recursive free-prose summaries lose scattered facts
  and compound errors (research basis in the brief). The design therefore carries what matters most
  as a deterministic ledger that the model cannot drop.
- **A moving target.** New messages arrive while an agent summarizes. Catch-up must freeze the range
  being summarized without letting held messages run into the receipt deadlines the sender froze at
  send time.
- **Poking without harm.** A poke must never answer an approval prompt, never interrupt a person
  watching the pane, and never send or lose text a person had typed.

## Key decisions made

- The daemon owns the plan; agents are workers. `summary <thread>` returns either a ready summary or
  a list of leased jobs. Workers submit results, which the daemon validates deterministically and
  stores as immutable, shared blocks. Chunk and rollup boundaries are pure functions of the immutable
  thread, so every reader converges on the same blocks.
- **Ledger plus narrative.** User instructions and identifiers are extracted by the daemon's own code,
  never by the model.
  - At level 0 the model writes the narrative, proposes new decisions and open items, and proposes
    status transitions. Each proposal cites a message inside the chunk, which is where the raw
    evidence is.
  - Rollup models write only the narrative.
  - The ledger shown to a reader is a deterministic **fold** that the daemon computes over every
    block's records in sequence order (§5). It is never a model's rewrite.
- Catch-up freezes a frontier, holds later messages, and extends effective receipt deadlines only
  while progress is being made (p99 of job duration per completed job). Frozen deadlines are never
  rewritten.
- Pokes reuse the wake dispatcher. Every unsafe state skips the poke, and the hard-deadline warning
  remains the backstop.
- All of this is cooperative and same-user. The new provenance and the new deadline fact are added
  to TRUST-POLICY.md.

## Decision points, by section

### 1. Message authorship and `--relays-user`

**Recommended:** the daemon records two new facts on every ordinary message at send time, in the
deciding transaction: `author_role` and `relays_user`.

`author_role` is a **new column**. `messages.author_kind` already exists (migration 0002, domain
`native|programmatic|built_in`, read by the service substrate, attention and digest code) and is
left untouched. The new column's domain is `human|agent|service`:
- **At send:** `service` when `author_kind = programmatic`. Otherwise it comes from the sender's open
  binding: `human` for `operator_human`, `agent` for `cooperative_top_level`. Seat-less `built_in`
  system events get NULL.
- **Backfill.** Messages written before this migration are backfilled inside the migration, between
  `DROP TRIGGER messages_immutable` and its re-CREATE (the same pattern 0002 uses):
  - `author_kind = programmatic` gives `service`;
  - otherwise the sender seat's occupant binding covering `decision_at` decides: harness `human`
    gives `human`, any other harness gives `agent`;
  - no covering binding, or a seat-less row, stays NULL and reads as `agent`.
- **Marking.** Backfilled rows get `author_role_backfilled = 1`, and `relays_user` is 0 for every
  pre-migration row.
- **`relays_user`** comes from the new `send --relays-user` flag. Service sends always record 0.

A message is **priority** iff `author_role = human` or `relays_user = 1`. Like everything an agent
sends, the flag is a cooperative claim (TRUST-POLICY A1).

**Considered:** model inference of "user asks" (rejected: unreliable, and the signal is lost at every
rollup level); author kind only (rejected: misses relayed asks, the most common case).

### 2. Deterministic chunking

**Recommended:** a level-0 chunk is a contiguous range of thread sequences `[first, last]`.
- **Boundaries.** Chunk 0 starts at sequence 1. Walking forward from a chunk's first message:
  - If the next message alone is at least `chunk_bytes` (default 24 KiB, about 6k tokens):
    - when the current chunk is empty, that message is a chunk of its own;
    - otherwise the current chunk closes before it, and it becomes the next chunk on its own.
  - Otherwise the message joins the current chunk. The chunk closes after the first message that
    brings its accumulated rendered size to `chunk_bytes` or more.
- **Size counted.** Rendered size is the length of the message as rendered into a job bundle (header
  line plus body for ordinary messages, the compact event line for info/warn system messages).
- **Full chunks only.** A chunk is *full* only when its closing message is at or below the thread's
  published head. The partial tail after the last full chunk is never summarized and is returned raw.
- **Determinism.** Messages are immutable and sequences are dense, so every reader computes the same
  chunks.
- **Configuration.** `chunk_bytes` is an installation setting. Changing it starts a new block
  generation (`chunking_version` in the block key) and never mixes with existing blocks.
  `chunking_version` is derived by the chunker module from `chunk_bytes` and the renderer version.

**Considered:** fixed message counts (rejected: wildly uneven sizes); provisional tail summaries
(rejected by the user: they churn and are never reusable).

### 3. Rollups and the displayed cover

**Recommended:** rollup blocks are index-aligned. Level n+1 block k covers level-n blocks
`[8k, 8k+7]` (fan-in 8) and exists only when all eight children exist.

The **displayed cover** of a thread is computed from the full-chunk prefix:
1. Start with every level-0 block.
2. While the cover's **narratives** exceed `display_bytes` (default 40 KiB, about 10k tokens),
   replace the oldest run of eight same-level blocks with their parent. Only narratives are measured
   here, because rolling up never changes the fold.
3. If that parent is not stored yet, the daemon returns its rollup job instead (§4).
4. When no run of eight same-level blocks remains, the narratives are returned as they are.

The fold (§5) is rendered **once** for the whole cover by the single fold renderer, never per block.
`over_budget: true` (with sizes) is set when the narratives still exceed `display_bytes` or the
rendered fold exceeds `fold_display_bytes` (default 24 KiB). The fold is never truncated, because
what it shows in full is open work that must not be dropped.

Recent history therefore stays at level 0, and old history climbs levels. The raw tail is appended
after the cover and does not count against `display_bytes`; it is bounded by `chunk_bytes`.

A rollup's input is:
- its eight child narratives;
- the fold rendered up to the parent's last sequence;
- the raw text behind open pinned user instructions.

It never re-reads raw chunks. **Every job bundle** (level 0 and rollup) aims at `bundle_bytes`
(default 48 KiB), a soft target with one spill rule:
- pinned raw text, and then any fold entries that are not open, become `text_ref` pointers, oldest
  first;
- rendered messages, narratives and open fold entries are never cut. A bundle that is still over
  the target after spilling is emitted oversized and reports its size.

A summary job never refuses to give a worker its input.

**Considered:** fan-in 10 from the brief (replaced by 8 per the research's own recommendation, which
the user approved); a rolling single summary (rejected: not shareable, and errors compound with each
update).

### 4. Summary protocol (daemon-driven)

**Recommended:** three new wire commands, all scoped to threads the caller may read (the same rule as
`history`):
- **`Summary { thread, claim }`** returns one of:
  - `Ready { frontier, cover: [Block], fold, tail: Page<Message>, tail_complete, over_budget }`
  - `Work { frontier, jobs: [JobTicket], leased_elsewhere: [JobRef] }`

  `claim` is the same caller claim every accountable command carries, built from the pane's
  hook-registered context. It identifies the caller seat for entitlement, leasing and catch-up entry.

  **The frontier.** While the caller seat has an active catch-up row for the thread, the frontier
  is that row's F. Otherwise it is the published head at the moment the request is decided. Planning,
  every block, the fold and the tail all stop at the frontier. So within one catch-up the job set is
  fixed and finite, and the Ready tail ends exactly where the hold begins. Messages above F are not
  in Ready. They are offered as ordinary attention after exit (§7), so nothing is delivered twice
  and nothing is skipped.

  **Work and leasing** happen in one place. For each job the current cover needs (missing level-0
  blocks first in ascending chunk order, then rollups whose children all exist):
  - **Already leased to the caller seat and live:** returned again with the same `lease_token`.
    This makes re-polling idempotent, so a recovered parent regains its tickets.
  - **Leased to a different seat and live:** listed under `leased_elsewhere` with `lease_until`, so
    the caller can wait or read raw.
  - **Free:** leased to the caller seat, at most `max_new_leases` (default 8) new leases per Work
    response. The remainder appears in a later Work.

  A JobTicket carries `job_id`, `lease_token`, `lease_until`, `level`, `range` and `budget_bytes`.
  - The lease is *reserved* at Work. Its clock starts at the first `SummaryJob` fetch, and an
    unfetched reservation lapses after 60 s.
  - **Fetching a lapsed reservation:** the fetch is honoured, and the lease starts then, if the job
    is still free. If another seat has leased it since, the fetch returns
    `ReservationLapsed { leased_elsewhere }`.
  - A parent that runs jobs one at a time loops: Summary, one job, Summary. Its own reservations are
    re-returned or re-issued on each poll.
  - The p99 sample is measured from fetch to submit.
  - **Fencing between a seat's parent and its workers:** the token is the seat's. Any invocation of
    that seat holding the token may fetch and submit. The first valid submit stores the block, and
    any later submit for the same job returns the stored block.
- **`SummaryJob { job_id, lease_token }`** returns the job's input bundle (§5):
  - level 0: the rendered messages, plus the fold rendered up to the chunk's last sequence. That
    fold includes:
    - every prefill instruction below the chunk's last sequence, including this chunk's own,
      because prefill items are derived from messages and need no stored block;
    - the model items of blocks stored by fetch time.
  - rollups: the bundle §3 describes.
- **`SummarySubmit { job_id, lease_token, submission }`** validates the submission (§6), then either
  stores the block and returns `Stored { block_id }`, or returns `Rejected { reasons }`.

  A second rejection for the same job stores a **fallback block** instead, which guarantees progress:
  - At level 0 it holds the daemon prefill only: instructions and identifiers, no narrative, no
    model items.
  - At a rollup level it holds an empty narrative.
  - The header carries `fallback: true`, and Ready renders the block with a "(fallback: no
    narrative)" marker.
  - A fallback block is final for its `chunking_version` (an accepted limit). Because prefill items
    are derived from messages, a fallback still holds everything the daemon itself guarantees.

**Entitlement, decided in the daemon (A2):**
- every summary command requires a caller seat that may read the thread's history;
- `SummarySubmit` also requires a live lease issued to that seat;
- entering catch-up requires the seat's accountable claim (§7).
- Leases are issued to the seat, so the seat's summary workers run `SummaryJob` and
  `SummarySubmit` themselves under the seat's claim (A1, cooperative).
- Blocks record the author seat, not an invocation role, because the CLI cannot tell a child from
  its parent and recording a role it cannot know would be dishonest.
- Job and block keys include `chunking_version`. A submit for a job planned under an older version
  is refused as stale. Old-version blocks are kept and never mixed into a cover.

**Version reuse.** The cover uses only blocks whose `chunking_version` equals the current one.
`prompt_version` and `model` never invalidate a stored block.

**Lease duration and uniqueness.**
- A fetched lease lasts `max(2 × p99, 60 s)` from fetch, capped at 10 minutes.
- An expired lease returns the job to the pool, and a later submit with an expired token is
  refused.
- Submit decisions are idempotent per `(job_id, lease_token)`.
- A block is unique per `(thread, chunking_version, level, index)`, with no exceptions.
- A second valid submit for an existing block returns `Stored` with the existing id and is not
  counted as progress twice.

**Considered:**
- Agents choosing ranges themselves (rejected: no convergence, and duplicate blocks);
- one job per call (rejected: the user wants parallel workers);
- the daemon spawning headless models (rejected: herdr-threads has no model access by design, and the
  user chose agent-side workers).

### 5. Block format and the ledger

**Recommended:** blocks store *records*. The ledger a reader sees is a deterministic **fold**
of those records, which the daemon computes in sequence order.

**Block header:**
- `thread`, `chunking_version`, `level`, `index`, `range{first_seq,last_seq}`, `children`;
- `source_hash` (over the rendered inputs), `fallback`;
- `prompt_version` and `model`, which come from the skill's worker procedure. The daemon stores both
  verbatim and only checks that each is non-empty and at most 64 bytes.
- provenance (§11).

**Records a level-0 block stores** (rollup blocks store a narrative only):
- **Items introduced in the chunk.** Each carries its introducing `seq` and a stable id tied to the
  immutable thread, not to block storage:
  - a prefill instruction's id is `i.<seq>`, derived from its message;
  - a model item's id is `<chunking_version>.<chunk index>.<n>`, numbered in submission order;
  - identifiers are keyed by value.
  - **`user_instruction`** `{author_seat, author_role, relays_user, text | text_ref}`: daemon
    prefill, one per priority message in the range. `text` is the verbatim body when it is at most
    2 KiB; otherwise `text_ref` points at the sequence.
  - **`decision`** `{by_seat, text}`: model-proposed.
  - **`open_item`** `{kind: ask|commitment|question|blocker, from_seat, to_seat?, text}`:
    model-proposed.
  - **`identifier`** `{value, kind, seqs[]}`: daemon regex over the range.
- **Transitions accepted in the chunk:** `{target_id, new_status, cite_seq}`. `new_status` is one
  of:
  - for instructions: `done` or `superseded`;
  - for open items: `resolved` or `superseded`;
  - for decisions: `superseded`.
- **`narrative`:** seat-attributed prose of at most `narrative_bytes` (default 3 KiB) at every level.

**Identifier extraction** is precise rather than broad:
- **Patterns:**
  - paths: contain a `/` and an extension, or start with `/`, `./` or `src/`;
  - bead ids: a prefix from the `tracker_prefixes` installation setting (default `["ht-"]`), then
    `[a-z0-9]{2,}` and optional `.N` parts, and the id must contain a digit or be at least 3
    characters after the prefix. `chunking_version` includes a hash of `tracker_prefixes`, so
    changing the setting starts a new block generation instead of mixing extraction rules.
  - SHAs: 7–40 hex characters on word boundaries, containing at least one letter and one digit;
  - URLs: `scheme://`;
  - error strings: backticked spans containing `error` or `failed`.
- **Cap.** At most 64 identifiers per block, with a quota per kind (paths 24, bead ids 16, SHAs 12,
  URLs 8, errors 4). Within a kind, the most recently mentioned win.
- **In the fold**, identifiers are merged by value. The rendered set is capped at 128, again with
  per-kind quotas, keeping the most recent.

**The fold**, computed by the daemon over the cover's level-0 records in ascending sequence order up
to the frontier:
- Every introduced item starts as active or open.
- A transition is applied when all of these hold:
  - its `cite_seq` lies in its own block's range;
  - its target was introduced at a sequence below `cite_seq`, whether in an earlier block or earlier
    in the same block;
  - the target is still open in the fold at that point;
  - for an instruction marked `superseded`, the message at `cite_seq` is a priority message.

  Transitions that fail these checks are dropped, and the drop is logged.

- **Accepted limit:** level-0 jobs run in parallel. A model-proposed item from a chunk that was not
  yet stored when a later chunk was fetched cannot be closed by that later chunk. Instructions are
  never affected, because their prefill ids need no stored block.

Rollup blocks contribute narratives only, so rolling up never changes the fold.

**Rendering:** one fold renderer is used everywhere (Ready, rollup bundles, level-0 bundles):
- Open instructions, open items and active decisions are always shown in full.
- Every closed item, including a `done` instruction, follows the window rule:
  - closed inside the displayed level-0 window: one line each;
  - closed earlier: omitted.
  - In a job bundle, the window is the job's own range. So a level-0 bundle shows only open entries
    from before the chunk, and a rollup bundle shows one line per item closed within its children.

  This keeps the brief's "one line at the next level, dropped the level after".
- An instruction that is still open is never omitted.

**Considered:** free prose only (rejected: specifics are lost across levels); model-extracted
instructions (rejected: identifiers and instructions are exactly where models fail, and the
daemon already knows which messages are priority).

### 6. Validation on submit

**Recommended:** each check is deterministic, and the first failure rejects the submission with a
reason.
The **submission** is versioned by `submission_schema` (an integer starting at 1, refused when
unknown). Its fields:
- `narrative` (string);
- `new_decisions[] {ref, seq, by_seat, text, quote?}`;
- `new_open_items[] {ref, seq, kind, from_seat, to_seat?, text, quote?}`;
- `transitions[] {target, new_status, cite_seq, quote?}`, where `target` is either a bundle id or a
  `ref` from this same submission;
- `prompt_version`, `model`.

Rollup submissions may carry only `narrative`, `prompt_version` and `model`. `budget_bytes` bounds
the whole encoded submission. It is `narrative_bytes + 8 KiB` at level 0 and `narrative_bytes + 1 KiB`
for rollups.

The checks:
1. The submission parses as its `submission_schema`, carries only the fields allowed at its level,
   and fits `budget_bytes` and `narrative_bytes`.
2. Every `seq` and `cite_seq` lies inside the job's range.
3. Every `quote` is an exact substring of the message at the cited sequence.
4. Every `target` was introduced below its `cite_seq` and must be one of:
   - an open id in the bundle's fold, which includes this chunk's own prefill instructions;
   - a `ref` from this submission.

   For `superseded` on an instruction, the message at `cite_seq` must be priority.
5. `new_status` is allowed for the target's kind.

The prefill's 100% instruction recall and the "never dropped" carry-forward are not submission checks.
They are invariants of the daemon's prefill and fold, and they are tested there.

Submissions are data. They are stored and rendered escaped, and no field ever becomes an instruction
to a reader.

**Considered:** an LLM judge (rejected: the research shows LLM faithfulness raters miss many errors,
and the daemon has no model); no validation (rejected: one bad block misleads every reader of the
thread).

### 7. Catch-up mode

**Recommended:** a catch-up row exists per `(seat, thread)`:
`{frontier_seq, binding_generation, execution, entered_at, extension_until, last_progress_at,
state: active|ended, end_reason}`.

- **Entry.** A `Summary` call returning Work, made with the seat's accountable claim (the same
  claim every CLI mutation builds from the pane's hook-registered context), opens or keeps the row.
  - **Accepted limit:** CLI calls cannot tell a seat's top-level agent from its child workers
    (TRUST-POLICY: a child is indistinguishable). Any summary call by the seat may therefore enter
    catch-up.
  - This is harmless: workers exist only after the parent's own `Summary` has entered the row, and
    an active row keeps its frontier, so repeated or worker calls never move F.
- **Hold.** While a row is active, the seat's attention offer and wake selection exclude ordinary
  messages on that thread whose sequence is above F. Excluded: inbox check-in offers, digest
  advancement and wake reasons. Three kinds bypass the hold: warnings, invitations, and priority
  messages (§1). History and explicit `read` are never filtered; the hold only affects pushed
  attention. Held messages stay pending and keep their receipt obligations.
- **Release is a push.** Every row end (ready, stalled or superseded) is a committed attention change
  in the same transaction:
  - it re-publishes the held range for that seat, giving the held items a fresh attention key above
    the seat's attention mark;
  - it re-derives the seat's wake reasons.

  The attention-mark and wake-frontier comparisons (the bridge's `advanced_beyond` and the
  dispatcher's frontier) therefore see the released items as new, and they are pushed. They do not
  depend on an explicit `inbox`.
- **Exit.** The first Ready answer to the same top-level binding for that thread ends the row
  (`end_reason = ready`) and sets `extension_until = now + exit_grace` (default 60 s).
- **Stall.** A due-scan ends an active row whose `extension_until` has passed (`end_reason =
  stalled`). Normal attention resumes and warnings fire on the effective deadline (§8).
- **Binding change.** Replacing the binding generation or execution ends the row (`end_reason =
  superseded`). A successor occupant never inherits a hold.
- **Release on every exit path** (ready, stall or supersession). Held obligations are never dropped.
  They are offered to whichever occupant holds the seat next.
- **Extension hooks.** The lifecycle calls inert contract hooks at entry, progress and exit. The
  deadline-extension work (§8) implements them, so the lifecycle never computes extension values
  itself.
- **Ready without Work.** A `Summary` call that returns Ready on its first call enters no row: the
  agent already holds a summary up to F.

**Considered:** releasing held receipt messages at half their window (replaced by the extension in
§8, at the user's direction); a whole-seat hold (rejected: other threads have nothing to do with this
summary).

### 8. Deadline extension

**Recommended:** a receipt's **effective deadline** is `max(frozen_deadline, extension_until)`, where
`extension_until` comes from the latest catch-up row for that receipt's `(seat, thread)` (active, or
ended with a future `extension_until`). With no row, it is the frozen deadline.

- **Who uses it.** The overdue classification, warning materialization, pending-receipt overdue
  state and the soft point (§10) all use the effective deadline. The frozen deadline is still stored
  and displayed, unchanged.
- **Which receipts.** All of the seat's pending receipts on the thread, both before and after F,
  because reading a summary is not a receipt.
- **Updates.**
  - The first entry, and any entry after a ready or superseded row, grants one p99:
    `extension_until = max(current, entry + p99)`. Re-entry after a stall grants nothing until a
    block is stored: when the most recent row for the `(seat, thread)` ended `stalled` and no block
    has been stored for the thread since it ended, the row keeps the `extension_until` it already
    holds (so a re-entered row whose extension has lapsed stalls again at the next due scan unless a
    block is stored first). A keep-call on an active row grants nothing.
  - Each `Stored` submit that is new progress sets `extension_until = max(current, now + p99)` and
    `last_progress_at = now`. This applies to the row of every seat whose active catch-up includes
    the thread, because shared progress helps every reader.
  - Exit sets it per §7.
- **p99** is computed over the latest 200 recorded fetch-to-submit durations of Stored jobs across
  the instance:
  - with fewer than 20 samples, it is `p99_cold` (default 90 s);
  - the result is clamped to [30 s, 10 min].
- **No hard cap.** Apart from one p99 per entry that follows no stall (the first entry, or one
  after a Ready or a supersession), every extension requires new stored progress (the exit grace
  follows a Ready, which itself requires the summary to be complete), so a stall and re-entry cycle
  cannot extend without progress, and the number of jobs for a thread is finite.
- **Visibility.** Pending-receipts and delivery inspect show both values plus `deferred: recipient
  catching up (until T)` while the effective deadline is later than the frozen one.
- **Due scans.** Both receipt due-scans keep their indexes on the frozen deadline:
  `receipts_pending_unwarned` (legacy rows) and `receipt_state_pending_due` (manifest-era rows,
  `prepared_recipients` plus `receipt_state`). A candidate whose effective deadline is still in the
  future is skipped and rechecked once it falls due. A secondary index on `catch_up.extension_until` drives the rechecks. Warning uniqueness and
  the retirement cutover classification are unchanged, except that they compare against the
  effective deadline.

**Considered:** rewriting stored deadlines (rejected: the store design freezes them at send);
extensions with a fixed cap (rejected: stalls already lapse, and progress is bounded by the job
count).

### 9. Triggers and hook text

**Recommended:**
- **Lifecycle events with a recovery reason:**
  - Codex: SessionStart `compact`, `resume` and `clear`.
  - Claude: SessionStart `resume` and `clear`, plus `compact` once captured evidence for the
    installed version adds it to the Claude recipe (evidence-backed recipe registry). Until then,
    Claude's compaction recovery is the next resume/clear or the on-demand command, and `doctor`
    reports the gap.
- **Hook text** is fixed instruction text plus data:
  - **Hot threads**, at most 8, as id plus a short escaped topic. A thread is hot when the seat has a
    pending receipt or attention on it, or when it has a message newer than `hot_window` (default
    24 h).
    - **Ordering:** threads with pending receipts come first, earliest effective deadline first. Then
      threads with other pending attention. Then recency-only threads, newest first.
    - Hot threads past the eighth go into the overview rows marked `hot`.
  - Every other joined thread goes in the existing overview rows.
  - The instruction: `Context was reset. Run the thread-summary procedure from the herdr-threads skill
    for each hot thread before continuing: herdr-threads summary <id>.`
  - Everything stays within the existing `MAX_CONTEXT` budget, with the overview trimmed first.
- **Top-level only.** Recovery text is emitted only on top-level lifecycle events. Subagent and
  summary-worker events never get hot-thread text, so the procedure cannot recurse.
- **Join.** An accepted invitation whose thread holds at least one full chunk includes `summary
  available: herdr-threads summary <id>` in the result.
- **On demand.** `herdr-threads summary <thread>` works at any time.

**Considered:** compaction only (rejected: resume and clear lose the same context); summarizing every
joined thread (rejected by the user: it blocks the agent on cold threads).

### 10. Soft-deadline poke

**Recommended:** a per-receipt soft point at `effective_deadline − (1 − soft_fraction) × window`,
where `soft_fraction` defaults to 0.6 and `window` is the frozen receipt duration.

- **When.** On each scheduler tick, a seat with receipts past their soft point that have not been
  poked becomes a poke candidate.
  - The soft point is evaluated lazily from the effective deadline on every tick, so an extension
    moves it with no re-arm event.
  - A receipt that has already been poked is not re-armed by a later extension.
  - **During catch-up**, receipts on a thread where the seat has an active catch-up row are not poke
    candidates. The agent is already working toward them, and the hold would hide what the poke
    points at. They become candidates again once the row ends.
- **Text.** One coalesced fixed prompt per seat:
  `herdr-threads: receipt due in <N>s on <thread-ids>; run herdr-threads inbox`.
  - `<N>` is the smallest remaining effective time.
  - Thread ids only, at most 8, then `+<k> more`; no topics or bodies.
- **Eligibility** comes from a fresh target observation, made right before the prompt by the existing
  dispatcher path (reservation, recheck, prompt). All of these must hold:
  - the seat is resolved and bound to a native agent of the bound harness;
  - the occupant is recognized;
  - the pane is **not focused**;
  - the UI state is Idle or ActiveTurn, or HumanInput with a stash-capable recipe;
  - for ActiveTurn, the harness recipe declares `poke_during_turn` from captured evidence.
- **Skipped** (re-evaluated each tick until the hard deadline): ApprovalOrQuestion, Unknown, focused,
  shell-only, unavailable or unresolved targets, and HumanInput without a stash-capable recipe.
  Skipping is the default whenever a poke cannot be made safely.
- **Composer stash.** For HumanInput with a recipe that declares `composer_stash`:
  1. read the composer text through the recipe's read primitive;
  2. clear it;
  3. submit the poke;
  4. retype the saved text without submitting it.

  Any failure in steps 1–2 aborts before the poke is sent. A failure in step 4 is recorded as a
  diagnostic with the saved text kept in the daemon log for operator recovery, never discarded.
- **Spike first.** The recipe capabilities `composer_stash` and `poke_during_turn` start **undeclared**
  for every harness. A native spike captures the evidence (§12) and only then declares them per
  harness and version.
- **Bookkeeping.**
  - Each poked receipt records `soft_poked_at` when the host accepts the prompt (transport success),
    so there is one poke per receipt soft point. A failed or skipped attempt leaves it unset.
  - When a poke and an ordinary wake are both due for a seat, one prompt goes out, using the poke
    text, which already says to run `herdr-threads inbox`.
  - A catch-up extension that moves the soft point later does not re-arm a receipt already poked.
  - Pokes share the wake dispatcher's limits: one in-flight attempt per seat, at most four active
    prompts, and the per-seat elapsed spacing.
- **Focus.** The host observation gains the pane's `focused` flag. The adapter already parses it but
  drops it.
- **Hard deadline.** The existing overdue warning to the thread's joined seats is unchanged, except
  that it fires on the effective deadline.

**Considered:** poking in every state, as the user first framed it (narrowed by the user to "skip
when it can't be done safely"); a separate poke dispatcher (rejected: duplicates the wake safety
machinery).

### 11. Trust policy amendments

**Recommended:** in the same commit as the schema contract, TRUST-POLICY.md gains:

- **A3 provenance `derived_summary`** (summary blocks only). The block was written by an agent acting
  for the seat, possibly a child worker, which the CLI cannot distinguish (accepted limit), using the
  declared model. It is never on receipts, never
  delivery, and never authority for any state change other than storing the block. Validation (§6)
  bounds what a block can claim.
- **A3 note on `author_role` and `relays_user`.** These are recorded claims. `relays_user` is
  cooperative, like everything else an agent sends.
- **A deadline-extension fact**, decided by the daemon from catch-up state the seat itself entered
  through its accountable claim (§7). The policy also gains an accepted-limits entry: any of the
  seat's invocations can enter catch-up. It is extended only by stored progress, and never applied on heuristic
  evidence. Frozen deadlines are never rewritten.
- **A4 *Poke*.** It goes only to an agent of the bound harness, in an unfocused pane, and never in an
  approval or question state.
- **A5 rows.** `summary` / `summary job` / `summary submit` may be called by the seat's binding or its
  children (read-mostly, submit-only), all under the seat's claim.

The scheduler design gets a dated amendment saying that warnings fire on the effective deadline.

**Who writes which part, each in the same commit as the behaviour it describes:**
- The schema contract writes the `derived_summary`, `author_role` (recorded vs backfilled),
  `relays_user`, summary-caller and deadline-extension text.
- The poke dispatch writes the A4 poke rule.
- The stash capability work writes its note, plus an accepted limit when no harness qualifies.
- The Claude compact work writes an accepted limit when no evidence is admitted.

### 12. Validation, testing and evidence

**Recommended:**
- **Unit tests:**
  - chunker determinism, boundary rules and the oversized message;
  - rollup alignment and the displayed cover under budget;
  - every validator rule, including the fallback after the second rejection;
  - the fold: transition guards, sequence ordering across blocks, the display rule, same-chunk closure via submission refs, and a final fallback block with its marker;
  - catch-up entry, hold, bypass, exit, stall and supersession with injected clocks;
  - extension math and the p99 cold start;
  - the soft-point computation;
  - poke eligibility across every UI state × focus × recipe capability combination, with a fake host
    including composer stash success and failure.
- **Native evidence** (configuration smoke, both harnesses):
  - SessionStart compact capture for Claude;
  - hook text, then parallel workers, then Ready on a real daemon;
  - the poke spike: composer read/clear/retype, plus whether input typed during an active turn is
    queued as a user turn.

  Captured evidence files go under `docs/evidence/`, the same as existing recipes.

### 13. Schema and configuration

**Recommended:** one migration, numbered with the next free number when it lands. ht-p03 adds 0011;
ht-xoc may add one; ht-5nb adds none. It adds:
- `messages.author_role` (new; `messages.author_kind` from 0002 is untouched), `messages.relays_user`, `messages.author_role_backfilled`, with the backfill run between DROP and re-CREATE of `messages_immutable`;
- `summary_blocks`, unique on `(thread, chunking_version, level, idx)`. Columns:
  - the header fields and the narrative;
  - `fallback`;
  - provenance: author seat, `model`, `prompt_version` (no invocation role; see §4).
- `summary_items` and `summary_transitions`: the level-0 records of §5, keyed by block, plus an
  index on `(thread, chunking_version, seq)` for the fold;
- `summary_jobs`: reservation, lease token, `fetched_at`, `lease_until`, attempts, rejection count;
- `summary_job_durations`;
- `catch_up`, unique on `(seat, thread)`, with an index on `extension_until`;
- `soft_poked_at` on `receipt_state` (manifest-era receipts) and on legacy `receipts`, read through the effective receipt projection.

Settings, each a positive installation value with defaults as above: `chunk_bytes`, `display_bytes`,
`narrative_bytes`, `bundle_bytes`, `fold_display_bytes`, `max_new_leases`, `tracker_prefixes`, `fan_in` (fixed at 8 in this version),
`hot_window`, `exit_grace`, `p99_cold`, `soft_fraction`.

## Non-goals and follow-on

- Summaries of threads the seat cannot read; cross-thread summaries; semantic search.
- Steering the harness summarizer (PreCompact output is display-only on Codex).
- A Claude compaction trigger without captured evidence.
- Quality evaluation with model probes. The research suggests a sampled probe suite; it is follow-on
  work, run outside this tree after merge.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

**Changes vs. original design (2026-10-02, epic ht-1ip; full record in [report.md](../../../history/thread-summaries-run/report.md))**

- **Migrations.** They landed as `0013_thread_summaries.sql` and `0014_catch_up_release.sql` (schema v14), because main's `0011_cooperative_only.sql` and then `0012_harness_version_evidence.sql` landed first (first renumbered to 0012/0013, then to 0013/0014 when main's harness version evidence merged). The protocol is version 3; it also carries main's capability-gated `HarnessEvidence` and `HarnessStates` commands.
- **Ordinary wakes never read the composer.** Reading it turned Claude's dim prompt suggestion into a "typed draft" and backed wakes off to 300 s (native smoke; ht-jf3 → ht-1ip.46).
- **Claude composer.** Any non-empty Claude composer read counts as "not known empty", so the poke is skipped and Claude's `composer_stash` never runs. This is an accepted limit in TRUST-POLICY.md A4, taken because the capture of typed-draft styling was refused.
- **Codex composer.** Codex pane width is unknown, so Codex drafts are never stashed.
- **Poke admission.** It uses only the §10 limits (in-flight per seat, 4 active, per-seat spacing). It is not gated by the wake retry backoff, and filters run before the 16-seat cut (ht-2i4 → ht-1ip.47).
- **Recovery text and Work output** name `herdr-threads skill`, because a fresh install has no pre-installed skill section (ht-dtq → ht-1ip.50).
- **Dropped fold transitions** are counted and logged at the daemon (ht-1ip.40).
- **Identifiers and fold size.** Identifier values are capped at 160 bytes and multi-line spans are rejected. The fold size counts identifiers (ht-1ip.48).
- **Priority citations** count only ordinary messages (ht-1ip.51).
- **Native evidence gaps.**
  - Not shown natively: the focused-pane skip and a controlled soft-point poke (ht-yuz, open).
  - Codex Q5/Q6 evidence comes from a mock provider.
  - Claude compact evidence is print-mode only.
- **Open escalation (resolved 2026-10-02, ht-hqg).** Whether catch-up entry or re-entry may extend the effective deadline without stored progress (§8, TRUST-POLICY A6 wording). Decided: option A, below.

- 2026-10-02 (ht-1ip.6): catch-up release needs a fresh attention key; migration `0014_catch_up_release.sql` (written as 0012; renumbered with the summary migration to 0013 when main's `0011_cooperative_only.sql` took v11, then to 0014, with the summary migration now `0013_thread_summaries.sql`, when main's `0012_harness_version_evidence.sql` took v12) adds `catch_up.release_seq` (the decision sequence allocated at row end). Released receipts above the row's frontier take `(release_seq, 0)` as their key.
- 2026-10-02 (ht-1ip.18, integration sweep): the join hint ("summary available: herdr-threads summary T") is printed by `accept` only, when the thread already holds a full chunk; a joiner is not hinted anywhere else. Every summary setting has a production read on its spec path and no inert contract stub is left. The cross-flow cases live in `tests/integration/summary_sweep.rs` (join hint to Ready, recovery to Ready with the hold and poke suppression, a chunking-settings change starting a new generation) beside the per-seam `tests/integration/summary_flow.rs`. Observation: the daemon projects `receipt_state` rows (the only source of poke candidates) about one send per second, so a poke is not due for a receipt until its projection lands; the hard-deadline warning does not depend on it (logical receipts).
- 2026-10-02 (ht-hqg, user decision option A on the parked §8 escalation): the first entry, and any entry after a ready or superseded row (including a successor binding's after `/clear`), grants one p99; re-entry after a stall grants nothing until a block is stored. Precisely, `enter_or_keep` skips the entry hook when the most recent `catch_up` row for the `(seat, thread)` ended `stalled` and no `summary_blocks` row for the thread has `created_at >= ended_at` (a block stored before the stall would have extended the active row past the scan, so `>=` cannot count pre-stall progress). Keep-calls on an active row grant nothing; progress extensions and the exit grace are unchanged. A first cut (8a6a5ed0) granted the entry extension only when no row had ever existed, which would have disabled it for every catch-up after the first; corrected the same day. Consequence: a stall and re-entry cycle can no longer postpone an overdue warning past the frozen deadline, and a re-entered row whose extension has already lapsed is ended `stalled` again by the next due scan unless a block is stored first. §8 "Updates" and "No hard cap", TRUST-POLICY A6 and docs/agent-usage.md match. Tests: `first_entry_extends_by_p99`, `reentry_after_stall_extends_nothing_until_progress_is_stored`, `reentry_after_ready_or_supersession_extends_again`, `reentry_after_a_stall_and_a_stored_block_extends_again` (tests/store/catch_up.rs) and `warning_fires_on_the_frozen_deadline_after_stall_and_reentry` (tests/store/receipts.rs).
- 2026-10-02 (ht-6jt): `setup claude` now offers to set Claude's `promptSuggestionEnabled: false` (explain, then `Disable prompt suggestions? [y/N]` on a terminal; advice only without one; `--disable-prompt-suggestions` / `--keep-prompt-suggestions` for scripts), recorded under `<state>/setup/` and reverted by `unsetup` when setup set it; `setup-status` and `doctor` report it. With suggestions off an idle Claude composer reads empty, so the Claude poke path works; the A4 accepted limit now says so. The key name is confirmed against Claude Code's settings reference and the summary smoke's scratch settings.
