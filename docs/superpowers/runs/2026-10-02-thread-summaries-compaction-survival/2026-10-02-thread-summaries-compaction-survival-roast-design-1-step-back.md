decision: redesign
summary: Five of the 15 r1 findings come from one §5 decision: each block's ledger is closed over its own range and merged as a parent-by-parent union. Replace it with a daemon-side, sequence-ordered fold of item and transition records across blocks. Patch the remaining findings in two clusters plus standalone nits.
pattern: The ledger findings share one cause. §5 makes each block's ledger the union of its children's ledgers, and a status transition is accepted only when its cited seq lies in the same block's range. That one rule produces these effects:
- The union never shrinks, and decisions have no status, so the ledger only grows (r1 [Should-fix] §3 displayed cover steps 1-3…; r1 [Nit] §5 decisions[]…).
- An item opened in chunk k and answered in chunk k+1 cannot be closed by either level-0 job, and nothing reconciles status across blocks in Ready (r1 [Should-fix] §4 SummaryJob level-0 bundle…).
- Closures move into rollups, where the model never saw the raw messages, so a "superseded" is grounded only by "seq in range" (r1 [Should-fix] §3 'It never re-reads raw chunks'…).
- The submission is left half-defined: is it the echoed ledger or only proposals? This leaves rules 4-5 as checks on something the daemon itself merged (r1 [Should-fix] §4 'SummarySubmit…).

Each fix-shape hint adds its own machinery around the union: a per-level cap, cross-block transition acceptance, guards on rollup transitions, a decision status field. Together these rebuild a thread-wide fold piece by piece. The lease findings form a second, independent cluster (Work hands out every lease at once and says nothing about a re-poll by the same seat). Everything else is unrelated.
clusters:
- C-ledger: r1 [Should-fix] §3 displayed cover steps 1-3; §5 Carry-forward; §6 rule 1; §4 JobTicket budget_bytes, r1 [Should-fix] §4 SummaryJob level-0 bundle ('the rendered messages, plus the ledger skeleton the daemon pre-filled'); §5 transition range rule; §6 rule 6; §3 'It never re-reads raw chunks' / 'Recent history therefore stays at level 0', r1 [Should-fix] §3 'It never re-reads raw chunks'; §5 'The model may only set status (open | done | superseded) and must cite a sequence in range'; §5 'resolved or superseded in a child keeps one line in the parent and is dropped from the parent's own parent'; §6 rules 2-6, r1 [Nit] §5 decisions[] {id, seq, by_seat, text}; §6 rules 2-3; §5 Carry-forward ('every decision are copied verbatim'), r1 [Should-fix] §4 'SummarySubmit { job_id, lease_token, submission }'; §5 'Carry-forward (merge) is daemon code, not model output'; §6 rules 1, 4, 5; §2 chunk closing rule | rule: The ledger becomes a fold over records in sequence order.
  - Each block stores what it adds: items and transitions.
  - The ledger shown at any frontier is the daemon's fold of those records across blocks, in seq order.
  - A transition cites a seq inside its own block and may target any earlier open item.
  - That rule applies to every ledger entry, decisions included.
- C-lease: r1 [Should-fix] §4 Summary/Work ('jobs lists every ready job ... Each ready job not under a live lease is leased to this caller'); §4 Leases ('max(2 × p99, 60 s), capped at 10 minutes ... a later submit with an expired token is refused'); §8 p99 'lease-to-submit', r1 [Should-fix] §4 Work two-case rule ('not under a live lease' / 'leased to someone else'); §4 'Leases are issued to the seat'; §4 Leases idempotency; §7 Exit 'first Ready answer' | rule: Define lease issuance in one place.
  - Work returns three cases:
    - jobs under the caller seat's own live lease are re-returned with the same token (an idempotent re-poll);
    - jobs leased to a different seat are listed as `leased_elsewhere`;
    - free jobs are leased.
  - Each Work leases at most a capped number of new jobs.
  - The lease clock, and the p99 sample, start at the `SummaryJob` fetch, not at Work.
  - Write the fencing rule between a parent seat and its workers into this same paragraph.
changes: §5 "Carry-forward (merge) is daemon code, not model output. A parent's ledger is the union of its children's ledgers plus every proposed transition the daemon accepted …". Related pieces change with it:
- the transition range rule in §5;
- the level-0 bundle and rollup bundle contents in §3 and §4;
- §6 rules 4-6;
- the `decisions[]` schema in §5;
- the bead that owns the §5 merge in the ht-1ip tree.
to: The ledger state becomes a deterministic fold, computed by the daemon in sequence order over records the blocks contribute.
- **What a block stores.** Each block stores two lists:
  - (a) the items it introduces:
    - level 0: daemon-prefilled `user_instructions` and identifiers, plus model-proposed `decisions` and `open_items`;
    - every item has a stable id;
  - (b) the transitions it accepted: `{target_id, new_status, cite_seq}`.
- **Transition rule.**
  - `cite_seq` must lie in the submitting block's own range.
  - `target_id` may be any item introduced at a seq ≤ `cite_seq` that is open in the fold up to that block's first seq.
  - `decisions` gain `status: active | superseded` and follow the same rule.
- **Level-0 bundle.** It carries the rendered messages, the prefill, and the currently-open ledger items folded up to the chunk's first seq. Level-0 workers can therefore close earlier items with in-range evidence, which is where raw evidence exists.
- **Rollup jobs.**
  - They write only the narrative.
  - They propose no transitions, which removes ungrounded transitions.
  - Their bundle is the child narratives, the open ledger folded up to the parent's last seq, and the raw text behind open pinned instructions.
- **Rendering (Ready and rollup display).**
  - Ready renders the fold up to F: open items, open instructions and active decisions in full.
  - Items closed within the displayed window get one line.
  - Items closed earlier are omitted, which keeps the brief's "one line at the next level, dropped the level after" as a display rule.
- **Submission schema.**
  - Fields: `{narrative, new_decisions[], new_open_items[], transitions[], prompt_version, model}`.
  - `budget_bytes` bounds exactly this payload.
- **Validator rules.** Rules 4-5 become daemon invariants of the fold rather than checks on the submission.
- **Transition guard.** Rule 6 also requires `cite_seq > target.seq`. For an instruction marked superseded, the citing message must be priority.
- **Blocks stay immutable.** The fold is recomputed from them. They remain unique per `(thread, chunking_version, level, index)` and keep their deterministic boundaries.
dissolves:
- r1 [Should-fix] §4 SummaryJob level-0 bundle ('the rendered messages, plus the ledger skeleton the daemon pre-filled'); §5 transition range rule; §6 rule 6; §3 'It never re-reads raw chunks' / 'Recent history therefore stays at level 0'
- r1 [Should-fix] §3 'It never re-reads raw chunks'; §5 'The model may only set status (open | done | superseded) and must cite a sequence in range'; §5 'resolved or superseded in a child keeps one line in the parent and is dropped from the parent's own parent'; §6 rules 2-6
- r1 [Nit] §5 decisions[] {id, seq, by_seat, text}; §6 rules 2-3; §5 Carry-forward ('every decision are copied verbatim')
- r1 [Should-fix] §3 displayed cover steps 1-3; §5 Carry-forward; §6 rule 1; §4 JobTicket budget_bytes. The ledger-growth and `budget_bytes` parts dissolve. One sentence is still needed for the §3 step-2 case where no run of eight remains while the cover is still over `display_bytes`.
- r1 [Should-fix] §4 'SummarySubmit { job_id, lease_token, submission }'; §5 'Carry-forward (merge) is daemon code, not model output'; §6 rules 1, 4, 5; §2 chunk closing rule. The schema and rules 4-5 parts dissolve. The §2 rule for an oversized message still needs one sentence choosing its boundary.
remains:
- The C-lease cluster: r1 [Should-fix] §4 Summary/Work ('jobs lists every ready job ... Each ready job not under a live lease is leased to this caller'); §4 Leases ('max(2 × p99, 60 s), capped at 10 minutes ... a later submit with an expired token is refused'); §8 p99 'lease-to-submit' and r1 [Should-fix] §4 Work two-case rule ('not under a live lease' / 'leased to someone else'); §4 'Leases are issued to the seat'; §4 Leases idempotency; §7 Exit 'first Ready answer'. Patch them together under the C-lease rule.
- r1 [Should-fix] §4 Summary frontier ('frontier is the published head sequence at the moment the request is decided. Every block and the tail stop at it') vs §7 Entry ('An existing active row keeps its frontier, so repeated calls never move F')
- r1 [Should-fix] §4 'A second rejection for the same job stores a ledger-only fallback block'; §4 'prompt_version and model never invalidate a stored block'; §4 block uniqueness; §5 L0 pre-fill. The fold limits the damage, because later blocks can still close items, but the fallback block still needs a marker in its header and a way to be superseded.
- r1 [Should-fix] §5 identifiers[] ('bead ids (`[a-z]+-[a-z0-9]+(\.[0-9]+)*`), 7-40 hex SHAs ... capped at 64 entries per block, keeping the most-mentioned')
- r1 [Nit] §3 'A rollup's input is its eight child narratives, the children's merged ledger, and the raw text of the messages behind every pinned user instruction that is still open'; §4 SummaryJob rollup bundle
- r1 [Nit] §10 When/Eligibility; §7 Hold ('exclude ordinary messages ... above F' from inbox check-in offers); §10 soft point formula
- r1 [Nit] §9 Hot threads ('at most 8 ... hot when the seat has a pending receipt or attention on it, or when it has a message newer than hot_window')
- r1 [Nit] §13 migration list vs §1 ('Backfilled rows are marked author_kind_backfilled = 1') and §4/§10 ('Each block records whether it came from a top-level or a child invocation') / §11 ('author_kind (recorded vs backfilled)')
- r1 [Nit] §4 Summary protocol 'Summary { thread, claim }'
scope: inside — The Goal stands unchanged: "a shared, reusable, structured summary". The brief decisions stand as well:
- The daemon still extracts the instructions and identifiers.
- "The model only proposes status changes, which the daemon accepts only when they cite a msg-id inside the block's range" still holds: the cited seq stays in the block's own range, and only the target may come from an earlier block.
- "Open items and unsuperseded user instructions are never dropped" still holds.
- "A resolved item becomes one line at the next level and is dropped the level after" is kept as a display rule.
- Rollup input still includes the child narratives, the merged (now folded) ledger and the pinned raw quotes, and raw chunks are never re-read.

The only brief-adjacent change is that rollup models stop proposing transitions. The brief allows that ("the model only proposes status changes"; it does not require them at every level). The spec's own "At every level it may propose status changes" in Key decisions needs a one-line edit.
recommendation: The fix-shape hints for the four core ledger findings each patch around the per-parent union: a cap, transitions that cross blocks, guards on rollup transitions, a status on decisions. Taken together they already need cross-block reconciliation at Ready time, which is the fold. Writing the fold once into §5 settles those findings in one place. It also stops the next round from finding the same union problem in Ready assembly, fallback recovery or display sizing.
