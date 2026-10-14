# Cross-node sync: forks, restore and incarnations — design

Parent: [root spec](2026-10-14-cross-node-sync-design.md) · Bead: ht-5fr.6

## Goal

A node restored from a backup, a node whose store was wiped while its key survived, and a log that two holders of one key extended differently all heal without losing a fact. Three outcomes are observable. A restored node rotates to a new incarnation and pulls its lost tail back without waking anyone for settled history. A forked log ends frozen on every node that saw the fork, and its owner re-issues the union of every branch's tail under a new incarnation, with the original fact IDs, so that all nodes converge to the same replicated projection. An operator can rebuild a damaged store from a peer with `sync resync`, after a `VACUUM INTO` backup.

## Problem description

The journal core (ht-5fr.3) reports forks but does not resolve them. Its ingest returns `IngestOutcome::Fork{evidence}` with `ForkEvidence::{AheadOfSelf, Conflict}`. It exposes `begin_incarnation`, `store_forked` and `freeze`, and it leaves `journal_forked` unused. The peer link (ht-5fr.5) pauses a forked log and hands it to a `ForkSink` whose only implementation logs a line and never resumes. Root §10 fixes the policy: detect forks at `Have` time and at ingest; third parties freeze at the last common counter and link evidence forward; the owner mints a new incarnation and re-issues every branch's tail; a restore without a fork rotates and pulls back; corrupt bytes penalize the source; `sync resync` rebuilds. It leaves open the wire frames, the Have-time comparison, how a node learns a fork it never saw, where the frozen counter comes from when only a hash mismatch is known, how the owner finds and fetches branches, the re-issue envelope (which fields are kept, what deps it carries), delivery on the owner during re-issue, persistence and restart of the repair, how a wiped store avoids re-using incarnation 1, and what `sync resync` does mechanically.

## Main challenges

A fork must be proven before anyone acts on it: a different head hash in a `Have` is only a claim, and a mismatched entry with a bad signature is corruption. The frozen counter must be safe without global agreement, because a third party sees two branches at most while the owner may see more. Re-issue has to be kind-agnostic, because the payloads of unknown kinds are relayed and re-issued without being understood. It also has to keep each branch's internal order, keep LWW and "sent at" stable (so re-issued facts keep their HLC), and never stall on a branch that is fetched only partly. The owner rotates while local commands keep authoring in parallel; it must never author into a frozen log, and its rotation must not wait for the network. Delivery has to stay inside the contract: repairs wake no one for history that was already settled, and they hide no obligation that is still open. Fork-related data can be large (two entries of 512 KiB each), so it cannot ride inside `Have`. Finally, a wiped store bootstraps from nothing, so it must not author a counter-1 genesis that collides with its own past.

## Key decisions made

- **Proof before action.** Every fork path acts only on two entries that are both validly signed by the log's key and are proven distinct at one counter. The proof is either two different entries at the same counter, or an entry whose `prev_hash` differs from the held entry at `counter-1`, which is a "prev-link" proof. A `Have` mismatch triggers a single-entry fetch, and the fetched entry goes through the same proof check.
- **Freeze at the proof counter minus one.** A node freezes at `x-1`, where `x` is the counter the proof covers. It lowers the freeze when it gets more evidence or when the owner declares a freeze, and it never raises one. Convergence does not depend on nodes agreeing on the freeze counter: it rests on the union re-issue and on idempotent apply by fact ID.
- **Three `fork.v1` frames and one `Have` field.** The new frames are `branch_want`, `branch_entries` (credit-gated) and `branch_none`, and `Have` gains `forks: [ForkRef]`. Evidence travels as references to `(counter, hash)` pairs, and the entries themselves are fetched through `branch_want`.
- **The owner rotates at once.** When its active incarnation is proven forked or ahead of its store, the owner rotates in one background writer quantum. It then pulls branches from `frozen_at+1`, searching backwards exponentially when a branch diverges earlier, and re-issues the union through a persisted, kind-agnostic job.
- **What a re-issue keeps.** A re-issue keeps `kind`, `kind_version`, `hlc`, `creates`, `needs` and the payload bytes. It gets a new log, counter, prev_hash and signature, and its deps are the element-wise maximum of the owner's applied vector and the original deps.
- **Delivery during re-issue on the owner.** Delivery is suppressed only for settlements that the owner authored in any of its own logs, including `journal_forked`, which is root §7's "already settled" rule. Open obligations are still offered.
- **Random first incarnation.** A fresh store starts at a random incarnation instead of 1, so a wipe with the key intact is a restore with no fork.
- **`sync resync`.** It is a restart-mode rebuild driven by a marker file. It backs up with `VACUUM INTO`, moves the old files aside, starts a fresh store, re-adds the dial targets, salvages verified entries from the backup and pins the rx source to the named peer until its first round completes.

## Decision points

### 1. Module layout and ownership

**Recommended.**

| File | Contents |
|---|---|
| `src/sync/incarnations.rs` (new) | `IncarnationsSink` (implements `ForkSink`), the `IncarnationsWorker` thread, `Job`, `Proof`, `classify_*` pure functions, the request table with timeouts and source penalties |
| `src/sync/resync.rs` (new) | `ResyncMarker`, `ResyncPhase`, `request`, `resume_at_start`, `ResyncPlan`, the salvage reader |
| `src/store/incarnations.rs` (new) | the only owner of `sync_forks`, `sync_branches` and `sync_reissues`: `record_fork`, `owner_rotate`, `restore_rotate`, `next_incarnation`, `upsert_branch_tip`, `branch_progress`, `reissue_pass`, `adopt_declared_freeze`, `status` |
| `src/store/journal.rs` (changed) | random bootstrap incarnation, `Predecessor` with optional `frozen_at`, lower-only `freeze`, `serve_branch`, `reissue`, open invariant "active self log is never frozen" |
| `src/store/apply.rs` (changed) | `ApplyMode::Reissue{of}`, `ApplyContext::is_own_history()`, the re-issue suppression rule |
| `src/sync/frame.rs` (changed) | `BranchWant`, `BranchEntries`, `BranchNone`, `ForkRef`, `Have.forks` |
| `src/sync/link.rs` (changed) | `branch_entries` joins `entries` on the credit-gated data queue |
| `src/sync/coordinator.rs` (changed) | the `ForkSink` extension, routing of the fork frames, `serve_branch` serving, `ResyncPin` in `reselect` |
| `src/sync/journal_port.rs`, `src/sync/memory_journal.rs` (changed) | `serve_branch` on both implementations |
| `src/sync/facts/node_genesis.rs` (changed) | `predecessor.frozen_at: Option<u64>`, and adopting a declared freeze on apply |
| `migrations/node/0001_node_baseline.sql` (changed) | three tables, one trigger, one index (editable until release) |
| `src/service/kicks.rs` (changed) | the three new tables in `KNOWN_UNMAPPED` |
| `src/app.rs` (changed) | resync start step, sink and worker composition, shutdown |

The bead names `src/sync/link.rs` for the frames, but ht-5fr.5 put `PeerFrame` in `src/sync/frame.rs`, so the variants go there, and `link.rs` changes only for credit classification. There are no new dependencies: randomness comes from `getrandom`, which ht-5fr.1 already pins.

**Considered.**
- Putting the store functions inside `src/store/journal.rs`: Journal owns the three journal tables, while fork records, branch progress and re-issue progress are Incarnations state (root: "state in `sync_logs`; the re-issue job's progress is persisted"). A separate store module keeps the owners distinct.
- A new `Lane::Repair`: the repair is driven by sink jobs and its own tick, not by table kicks. A lane would add a pacer and a mapped table for no gain. Failures report through the existing `Lane::Sync` status instead (§12).

### 2. Schema additions

**Recommended.** Added to the baseline, all `STRICT`:

```sql
CREATE TABLE sync_forks (
  fork_ix INTEGER PRIMARY KEY,
  log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
  origin TEXT NOT NULL CHECK(origin IN ('evidence','declared')),
  proof_counter INTEGER CHECK(proof_counter IS NULL OR proof_counter>=1),
  held_counter INTEGER, held_hash BLOB CHECK(held_hash IS NULL OR length(held_hash)=32),
  conflicting_counter INTEGER, conflicting_hash BLOB CHECK(conflicting_hash IS NULL OR length(conflicting_hash)=32),
  frozen_at INTEGER NOT NULL CHECK(frozen_at>=0),
  source_node TEXT CHECK(source_node IS NULL OR length(source_node)=32),
  detected_at INTEGER NOT NULL,
  CHECK((origin='evidence') = (proof_counter IS NOT NULL AND held_hash IS NOT NULL AND conflicting_hash IS NOT NULL)),
  UNIQUE(log_ix, held_counter, held_hash, conflicting_counter, conflicting_hash)
) STRICT;

CREATE TABLE sync_branches (            -- owner only
  branch_ix INTEGER PRIMARY KEY,
  log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
  tip_counter INTEGER NOT NULL CHECK(tip_counter>=1),
  tip_hash BLOB NOT NULL CHECK(length(tip_hash)=32),
  next_from INTEGER NOT NULL CHECK(next_from>=1),
  search_span INTEGER NOT NULL DEFAULT 0 CHECK(search_span>=0),
  state TEXT NOT NULL CHECK(state IN ('pulling','complete','unavailable')),
  attempts INTEGER NOT NULL DEFAULT 0,
  next_attempt_at INTEGER,
  last_error TEXT CHECK(last_error IS NULL OR length(last_error)<=512),
  first_seen_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE(log_ix, tip_hash)
) STRICT;

CREATE TABLE sync_reissues (            -- owner only, append-only
  source_log_ix INTEGER NOT NULL REFERENCES sync_logs(log_ix),
  source_counter INTEGER NOT NULL,
  source_hash BLOB NOT NULL CHECK(length(source_hash)=32),
  target_ingest_seq INTEGER NOT NULL REFERENCES journal(ingest_seq),
  reissued_at INTEGER NOT NULL,
  PRIMARY KEY(source_log_ix, source_counter, source_hash)
) STRICT, WITHOUT ROWID;

CREATE INDEX journal_forked_creates ON journal_forked(creates) WHERE creates IS NOT NULL;
-- the backward walk of §4 uses journal_forked's existing UNIQUE(log_ix, counter, hash)
```

Triggers, each `RAISE(ABORT, '<name>')`:
- `sync_logs_freeze_lowers_only`: BEFORE UPDATE OF `frozen_at_counter` WHEN `OLD.state='frozen' AND NEW.frozen_at_counter > OLD.frozen_at_counter`.
- `sync_forks_append_only` and `sync_reissues_append_only`: BEFORE DELETE.
- `sync_reissues_immutable`: BEFORE UPDATE.

`sync_branches` is mutable progress state, and rows are never deleted.

Invariants that the store functions keep, and that `sync verify` (ht-5fr.8) and the tests check:
- **I1.** The active self log (`local_node.active_log_ix`) is never `frozen`. `journal::open` adds this to its startup invariants and refuses the start with `StoreCorrupt "journal invariant: active self log frozen"`.
- **I2.** Every `journal_forked` row of a log has `counter > frozen_at_counter` of that log.
- **I3.** No `sync_reissues` row names a `node.genesis` entry.

`journal_forked` (ht-5fr.3) is unchanged apart from the index. `sync_forks`, `sync_branches` and `sync_reissues` go into `KNOWN_UNMAPPED`: nothing kicks on them. The `sync_logs` updates that a freeze makes already kick `Lane::Sync` (ht-5fr.5 table map), so wantability and the advertised `frozen_at` refresh on their own.

**Considered.**
- Storing the evidence entries inline in `sync_forks`: the held entry is already in `journal` and the conflicting one goes to `journal_forked`, so a row references them by `(counter, hash)`.
- A per-log `retired` state for a log rotated away from during a restore: an own log is "not active" exactly when `active_log_ix` names another row, so a state column would duplicate that.

### 3. Proofs: what counts as a fork

**Recommended.** One pure function in `src/sync/incarnations.rs`:

`classify_pair(pubkey: &[u8;32], a: &Envelope, b: &Envelope) -> Classified::{Proof(Proof), Same, NotProof, Corrupt{which, err}}`:
1. `a.log == b.log`, or the result is `NotProof`.
2. `a.verify(pubkey)` and `b.verify(pubkey)` must both pass; a failure is `Corrupt` for that entry.
3. If both entries are at the same counter, equal hashes give `Same` and different hashes give `Proof{x: counter}`.
4. Order the pair so that `lo.counter + 1 == hi.counter`. If `hi.prev_hash != lo.hash`, the result is `Proof{x: lo.counter}`, because two different entries at `lo.counter` are signed. Otherwise it is `NotProof`.
5. Any other counter distance is `NotProof`.

`Proof { log, x, held: Envelope, conflicting: Envelope }`. "Held" is whichever entry this node stores in `journal`; when it stores neither, held is the lower `(counter, hash)`. The freeze bound is `x - 1`.

`classify_ahead_of_self(my_head: (u64, [u8;32]), my_hash_at: impl Fn(u64) -> Option<[u8;32]>, pubkey, entry) -> AheadClass` covers `ForkEvidence::AheadOfSelf`. Journal's ChainRules raises that variant before verifying, so this function verifies first:
- `entry.verify` fails → `Corrupt`;
- `entry.counter ≤ my_head.0`: an equal stored hash gives `NotFork`; otherwise `Fork(Proof{x: entry.counter})`, with held being my entry at that counter;
- `entry.counter == my_head.0 + 1`: `entry.prev_hash == my_head.1` gives `Restore`; otherwise `Fork(Proof{x: my_head.0})`, with held being my head entry (a prev-link proof);
- `entry.counter > my_head.0 + 1` → `NotFork` (a gap; the coordinator re-Wants from `contiguous + 1`).

`ForkEvidence::Conflict{held, conflicting}` from ingest is classified with `classify_pair`. Journal has already verified the conflicting entry, so `Corrupt` cannot occur here, but the same function runs anyway.

**Considered.**
- Acting on a `Have` head mismatch directly: a mismatch is an unsigned claim, and a corrupted peer store would freeze healthy logs (root §10 separates corrupt from fork).
- Probing for the exact divergence point at every third party: third parties do not need it (§5), and a probe frame would add wire surface.

### 4. Wire: `fork.v1`

**Recommended.** Every node built with this subepic advertises the cap `"fork.v1"`. Fork content is sent only on established, active links whose effective caps include it, which is ht-5fr.5's rule for non-base frames. Additions to `src/sync/frame.rs`:

| `t` / field | Fields | Notes |
|---|---|---|
| `have.forks` | `[ForkRef{node, inc, held: {counter, hash}, conflicting: {counter, hash}, frozen_at}]`, ≤64, `#[serde(default, skip_serializing_if = "Vec::is_empty")]` | carried in full `Have`s and in the next delta after a new fork record; a peer without the cap ignores the unknown field |
| `branch_want` | `node, inc, tip: {counter, hash}, from` | asks for the chain that ends at `tip`, counters `from..=tip.counter`, ascending; requires `1 ≤ from ≤ tip.counter`, else `protocol_error` |
| `branch_entries` | `node, inc, tip_hash, entries: [WireEnvelope]` (≥1, ≤256), `more: bool` | one reply per `branch_want`; credit-gated exactly like `entries`; split in halves above `max_frame` |
| `branch_none` | `node, inc, tip_hash` | the server does not hold that tip, or the walk is over its bound; on the control queue |

`link.rs`: the data queue and the credit counting cover `entries` and `branch_entries` alike. The receiver returns `Credit{len}` for a `branch_entries` frame once the frame has been handed to the sink, or dropped.

**Serving** (`JournalPort::serve_branch(log, tip: (u64, [u8;32]), from, limits, budget) -> Result<Option<Vec<Envelope>>, ApiError>`, on a query connection):
1. If `journal` holds `(log, tip.counter)` with hash `tip.hash`, it serves `journal` rows `from..=tip.counter`, using `serve`'s 256-entry / 256 KiB limits with an upper bound.
2. Else, if `journal_forked` holds `(log, tip.counter, tip.hash)`, it walks backward by `prev_hash`. At each step `c-1` it looks for a `journal_forked` row `(log, c-1, prev)` or a `journal` row `(log, c-1)` whose hash is `prev`. Once it reaches `journal`, the remainder is a `journal` range. It stops at `from`, then returns the first batch ascending from `from`. The walk is bounded at 65 536 steps, and over the bound the result is `None`.
3. Otherwise the result is `None`.

`MemoryJournal` implements the same rules over its main chain plus a forked map, and test-support `store_forked` fills that map.

The coordinator serves `branch_want` like `want`, through `spawn_blocking` and the data queue, except that it keeps no subscription: one request gets one reply. It handles at most 4 outstanding `branch_want`s per link in FIFO order, and answers further ones with `branch_none`.

**Considered.**
- Inlining evidence entries in `Have`: two 512 KiB payloads can exceed the 1 MiB frame cap.
- A separate `fork_evidence` frame: `branch_want` with `from = tip.counter` already fetches one entry by `(counter, hash)`, so evidence and branch pulls share one serving path.
- A `fork_probe` frame (hashes at chosen counters) for locating divergence: the owner's exponential back-search (§7) uses only `branch_want` and is needed rarely.

### 5. Third-party path: freeze and forward

**Recommended.** All of this runs on `IncarnationsWorker` (§12), in background writer quanta.
- **Ingest evidence.** `ForkSink::on_fork(log, ForkEvidence::Conflict{..}, source)` returns `ForkAck::Paused` and queues `Job::Fork`. The worker classifies the pair (§3). On `Proof` it calls `store::incarnations::record_fork(tx, &proof, Some(source))`, which in one transaction:
  1. calls `journal::store_forked` for whichever proof entries are not in `journal` (exact duplicates are ignored);
  2. inserts a `sync_forks` row (`origin='evidence'`), deduplicated by the UNIQUE;
  3. calls `journal::freeze(tx, log, x - 1)` (lower-only, §8).

  It returns `ForkRecord{new: bool, frozen_at}`. After the commit, the sink adds `log` to `resumed()`, and when `new` it sets `refs_changed` so that the coordinator sends a `Have` delta with `forks`. The coordinator then finds the log unwantable at or beyond `frozen_at` (ht-5fr.5 §9).
- **Have-time detection.** `ForkSink::on_have(source, fork_cap, logs, forks)` queues `Job::Have`, coalesced per source to the latest. For each `HaveLog` of a held log where `peer.counter ≤ local.contiguous`, the worker reads `stored_hash(log, peer.counter)` on a query connection. A per-`(source, log)` memo of the last checked `(counter, head)` skips repeats. On a mismatch it queues the request `branch_want{tip: (peer.counter, peer.head), from: peer.counter}` to `source`. The reply runs through `classify_pair` with my stored entry, and a `Proof` goes to `record_fork`. Logs where the peer is ahead are checked by the peer from my `Have`, or at ingest (root §10). A mismatch with a peer that lacks `fork.v1` writes one log line per log per hour, `sync: log <uuid8>/<inc> head differs at <c> from <uuid8> (no fork.v1)`, and nothing else.
- **Forwarded evidence.** For each `ForkRef` received for a held log that has no matching `sync_forks` row, the worker requests whichever of `held` and `conflicting` it does not store by hash, from the advertising peer. Once both are present, `classify_pair` → `record_fork(source = advertiser)`. A `ForkRef` for a log this node does not hold is ignored; it comes back with the next full `Have`.
- **What stays.** The held branch stays in `journal`, applied or pending, and keeps being served to peers that want it. Pending entries of the held branch beyond `frozen_at` still apply (root: "keep the branch they hold"). The readiness cap at `frozen_at` (ht-5fr.3 §7) makes entries of other logs that depend on a different branch ready, and their `needs` park them until re-issued creators apply.
- **Declared freeze.** A `node.genesis` whose `predecessor.frozen_at` is `Some(k)` makes its handler call `adopt_declared_freeze(tx, predecessor_log, k)`. If that log is held, this inserts `sync_forks(origin='declared', frozen_at = k)` and lowers the freeze to `k`; if the log is not held, it does nothing.

**Considered.**
- Third parties computing the exact divergence counter: a freeze above the true fork point only means this node also keeps some of its own branch, and those facts are part of the owner's union anyway. The owner's declared counter and later evidence lower it.
- Applying the conflicting branch at third parties: that is first-seen-wins in another form (root rejected it), and the entries are re-issued by the owner.

### 6. Owner path: detection and rotation

**Recommended.** The owner is the node whose `node_uuid` equals the log's. Each of the following triggers `Job::OwnerFork(proof)`:
- `on_fork` with `AheadOfSelf` classified as `Fork`;
- a `Proof` from `classify_pair` about any own log (from ingest, from a Have mismatch, or from a forwarded `ForkRef`).

`AheadOfSelf` classified as `Restore` triggers `Job::Restore`. `NotFork` resumes the log. `Corrupt` makes `on_fork` return `ForkAck::Corrupt`, which the coordinator handles like `IngestOutcome::Corrupt`: a 10-minute penalty for `(link, log)` and a reselect. `on_fork` verifies `AheadOfSelf` synchronously (one signature check, about 50 µs), so that it can return the verdict.

`store::incarnations::owner_rotate(tx, ctx, decision, proof)`, in one background writer quantum:
1. **Re-check inside the transaction.** Local commands may have authored since detection, so the worker re-reads the active log head and reclassifies. A former `Restore` that the node has since authored past becomes a `Fork`.
2. If `proof.log` is the active self log: `inc = next_incarnation(tx)`, then `journal::begin_incarnation(tx, inc, Some(Predecessor{incarnation: proof.log.incarnation, frozen_at: Some(x - 1)}))`. That authors and applies the new genesis and moves `active_log_ix`, which keeps I1.
3. `record_fork(tx, &proof, source)`, which freezes the old log at `x - 1` (lower-only).
4. `upsert_branch_tip(tx, log, conflicting.counter, conflicting.hash, source)`.

For a fork of an own log that is **not** active (an incarnation already rotated away from by a restore, or an older one), steps 1 and 3-4 run with no rotation: the node does not author there anyway.

`restore_rotate(tx, ctx, decision)` re-checks as in step 1, then calls `begin_incarnation(tx, next_incarnation(tx), Some(Predecessor{incarnation, frozen_at: None}))`. The old incarnation is not frozen. It becomes an ordinary own log, which the coordinator wants as soon as the sink resumes it. Ingest appends it normally (ChainRules raises `AheadOfSelf` only for the active log), and ApplyEngine applies it with own-old-incarnation suppression (ht-5fr.3 §8 (a); ht-5fr.4 §12).

`next_incarnation(tx) -> u32` is a uniformly random value in `1..=u32::MAX` that no `sync_logs` row of this node already uses. It is drawn from `SyncState.incarnation_rng` (an xorshift seeded from `getrandom` at `journal::open`; the test-support `StoreContext::seed_incarnations(u64)` fixes it).

The window between detection and the rotation commit is one background writer turn. Commands in that window author into the old log, and those entries are part of the owner's branch: they are re-issued (§7), never lost.

`ForkSink::resumed()` returns the log after the commit. The coordinator keeps it paused only until then.

**Considered.**
- Holding all local authoring until the first sync round after start: research 09 F2 rejects it because it violates AP, and incarnations make it unnecessary.
- A sidecar head file written after commits (research 09 F1 (a)): restores of the whole node directory, which is the usual backup unit, copy the sidecar too, and the Have and ingest checks catch the same case once any peer is reachable.
- Rotating on a fork of an old, non-active own log: the node does not author there, and the active log is unaffected. A rotation would only add a genesis.

### 7. Owner path: branch discovery and pulls

**Recommended.** These steps run only on the owner, and only for frozen own logs.
- **Tips.** On each `Job::Have` for a frozen own log `L`, a peer head `(c, h)` is a branch tip unless `c ≤ own contiguous` and `stored_hash(L, c) == h`. Evidence entries and `ForkRef` sides that are not on the owner's chain are tips too. `upsert_branch_tip` inserts a `sync_branches` row (`state='pulling'`, `next_from = frozen_at + 1`). The sink keeps an in-memory `tip → {advertising sources}` map.
- **Pulls.** For each `pulling` row with no outstanding request and `next_attempt_at` past, the sink sends `branch_want{tip, from: next_from}` to one advertising source that holds no live penalty for `L`. It prefers the source with the oldest last request.
- **Reply handling** (`branch_progress(tx, branch_ix, source, entries, more)`, one quantum):
  1. Verify each entry with `L`'s pubkey. The first entry's `prev_hash` must be the hash of a known predecessor at `counter - 1`: a `journal` row, a `journal_forked` row, or the zero hash at counter 1. Each later entry must chain on the previous one.
  2. If the first entry does not chain and `next_from > 1`, the branch diverges below `next_from`: set `search_span = max(256, 2 × search_span)` and `next_from = max(1, next_from - search_span)`, and request again. This is an exponential back-search that costs at most twice the divergence distance in pulled entries.
  3. An entry whose hash equals the owner's stored hash at its counter is common and is skipped. Every other entry goes to `journal_forked` with `source_node`. The first off-chain counter `d + 1` lowers the owner's freeze to `min(frozen_at, d)`, which keeps I2 and exposes the owner's own rows above `d` to re-issue.
  4. Set `next_from = last + 1`. The row is `complete` when `last == tip.counter`, or when the reply has `more = false` and `last < tip.counter`, in which case the source held less. A shorter source later re-advertises its head, and the head creates its own row.
- **Failures.** A bad signature or a broken chain penalizes `(source, L)` for 10 minutes (root: corrupt bytes penalize the source), counts `attempts` and records `last_error`. `branch_none`, or no reply within 30 s, moves to another source. With no source left the row becomes `unavailable` with `next_attempt_at = now + 10 min`. It returns to `pulling` when a `Have` advertises the tip again.

**Considered.**
- Pulling every branch from counter 1: a fork after a week of history would re-transfer that week for nothing.
- Deleting `complete` rows: `sync status` reports them, and the table is tiny.

### 8. Journal changes

**Recommended.** These refine ht-5fr.3:
- **Bootstrap incarnation.** `open` step 3 calls `begin_incarnation(tx, next_incarnation(tx), None)` instead of incarnation 1. A wiped store with the key intact therefore starts a new log. Peers advertise its old incarnations, which are ordinary own logs here, so they are pulled back and applied with suppression. That is root §10's "same case from counter 0", with no fork (research 09 F2). A collision with an earlier own incarnation (probability 2⁻³² per earlier incarnation) surfaces as a fork at counter 1 and goes through §6.
- **`Predecessor { incarnation: u32, frozen_at: Option<u64> }`** replaces `(u32, u64)` in `begin_incarnation`. The `node.genesis` payload's `predecessor` becomes `{incarnation, frozen_at: null | n}`. The baseline is unreleased, so this is an in-place v1 definition, not a version bump. `None` means the node rotated away after a restore and the old log was not forked.
- **`freeze(tx, log, at) -> FreezeOutcome::{Frozen, Lowered, Unchanged}`**. It is lower-only, matching the trigger. It refuses (`Conflict "cannot freeze the active self log"`) when `log` is the active self log, which keeps I1. `owner_rotate` therefore rotates before it freezes.
- **`reissue(tx, ctx, decision, source: &Envelope, source_where: ReissueSource) -> Result<Authored, ApiError>`**, Journal's second authoring entry point:
  1. Read the active self log (`counter = contiguous + 1`, `prev_hash = head_hash`).
  2. `deps` = the element-wise maximum of `applied_vector(tx)` and `Deps::decode(source.deps)`, minus the active self log. It never lists the new log, and an entry for the source's own log cannot occur, because `Deps` excludes it.
  3. Copy `kind`, `kind_version`, `hlc`, `creates`, `needs` and the payload bytes unchanged. The HLC is not ticked: re-issued register facts keep their `(hlc, node)` and lose LWW to themselves (ht-5fr.4 §11), and `sent_at` is stable. Self-log HLCs need not be monotone (ht-5fr.3 §3).
  4. Seal, insert as `pending`, advance the head, then call `apply::apply_one(tx, ctx, ingest_seq, ApplyMode::Reissue{of: source.log})`, which records any outcome (`applied`, `parked_*` or `failed`).

  Unlike `author`, a rejection does not abort, because the fact was validly authored and other nodes need it. The commit guard holds, since parked and failed entries also advance `applied_through`.
- **`serve_branch`** (§4) and `JournalPort::serve_branch`.

**Considered.**
- Keeping incarnation 1 and resyncing before authoring: the bootstrap genesis would collide at counter 1 with the old incarnation, and the node's whole history would then be re-issued.
- An incarnation derived from Unix seconds: it is readable, but devices without a real-time clock boot at the same epoch second, and random needs no clock.
- Re-ticking the HLC on re-issue: the re-issued register value would then win LWW against concurrent later writes it never saw.
- Deps equal to the owner's applied vector alone: a receiver could then apply a re-issued fact before facts its original causally followed.

### 9. Re-issue job

**Recommended.** `store::incarnations::reissue_pass(tx, ctx, decision, limits: ReissueLimits{entries: 64, wall_ms: 100}) -> ReissueProgress{reissued, waiting, has_more}`.
- **Candidates** for every own log that is `frozen`, ordered by `(sync_logs.first_seen_at, log_ix, counter, source kind [journal first], journal ingest_seq | forked_ix)` and excluding rows already in `sync_reissues` and every `node.genesis`:
  - `journal` rows with `counter > frozen_at_counter`, which is the owner's own branch;
  - all `journal_forked` rows, which are the other branches (I2 puts them above the freeze).
- **Branch order.** A `journal_forked` candidate is ready only when its `prev_hash` names a `journal` row of the log at `counter - 1`, or a `journal_forked` row at `counter - 1` that is already in `sync_reissues`. A candidate that is not ready is skipped and counted in `waiting`, so one stalled branch never blocks the others. `journal` candidates are always ready, because ordering by counter re-issues their predecessors first. The one exception is the rows exposed by a lowered freeze (§7 step 3): they re-issue after higher counters, and receivers rely on `needs` parking for any entity those lower rows create.
- **Per candidate:** `journal::reissue`, then `INSERT INTO sync_reissues`, in the same transaction.
- **Driver.** The worker runs passes while `has_more`, after every `record_fork` or `branch_progress` commit, at start, and every 5 s while `waiting > 0`. Every pass is idempotent: a crash between passes leaves `sync_reissues` and `journal` consistent, because they commit together.
- **Receivers.** Every node receives the re-issued entries as ordinary entries of the owner's new incarnation. Entries it already applied are no-ops by fact ID (root §5; ht-5fr.4 §3 idempotence list). The rest are new facts and deliver normally under their own suppression rules.

**Considered.**
- Re-issuing branch by branch: interleaving by counter keeps every branch's internal order and finishes the owner's own branch in step with the others.
- Waiting for every branch to be `complete` before re-issuing anything: an unreachable branch holder would block the repair indefinitely, and branches found later are appended in the same way.

### 10. Apply on the owner during re-issue

**Recommended.** These are the changes in `src/store/apply.rs`:
- `ApplyMode::Reissue{of: LogKey}`. Readiness is not checked: the entry is the head of the active log, and its deps are allowed to exceed the owner's applied vector (§8). `needs` parking applies as usual.
- `ApplyContext::delivery_suppressed(tx, key)` in this mode is true when a row with `creates = key` exists in `journal` or `journal_forked` in any log of this node. That row is a settlement this node authored in any branch of its past. Root §7 suppresses "receipts that log already settled", and the settlement of an earlier branch usually sits in `journal_forked` when the message is re-issued. An unsettled obligation, such as a message from another branch to a local seat that was never acknowledged, is offered normally through DeliveryTail. This is deliberately narrower than ht-5fr.3 §8 (a)'s blanket rule for ingested own-old-incarnation entries, which this subepic leaves as ht-5fr.3 and ht-5fr.4 define it.
- `ApplyContext::is_own_history() -> bool` is true for an own-old-incarnation origin (ht-5fr.3 §8 (a)) and in `Reissue` mode. ht-5fr.4's `attribution::take` uses it to record `local_journal` provenance when no staged row exists. That is the value ht-5fr.4 §3 defines for own history.
- Provenance stays `Local` (the origin is this node). On other nodes a re-issued fact is `replicated_from:<owner uuid>`.

**Considered.** Suppressing every re-issued fact on the owner: open obligations from a lost branch would never be offered, while the deadline lane would still raise them as overdue to their senders.

### 11. `sync resync --from <node> [--log <node>]`

**Recommended.** The CLI and the operator gate belong to ht-5fr.8. This subepic owns the mechanism in `src/sync/resync.rs`.
- **`request(store, paths, sync: &SyncHandle, from: NodeUuid, logs: Option<Vec<NodeUuid>>, operator: &str) -> Result<(), ApiError>`.** It checks the following in order and refuses on the first failure:
  - `from` is a known node other than self, else `InvalidRequest "unknown node"`;
  - at least one dial target has learned `from`, or a link to it is active, else `Unavailable "no path to <tag>"`;
  - free space is at least the database size plus the headroom, else `StoreFull "resync needs <n> MiB free for the backup"`.

  It then writes `<node_dir>/resync.json` atomically (temp file, fsync, rename, dir fsync):
  `{"v":1,"from":"<uuid>","logs":[…]|null,"operator":"operator:local-user:<uid>","requested_at":<ms>,"phase":"requested","backup_dir":null,"targets":[{"ssh_dest","remote_command"}]}`.
  Finally it triggers the daemon's existing graceful stop. The CLI waits for the old daemon to exit, then calls `ensure_running`.
- **`resume_at_start(paths, clock) -> Result<Option<ResyncPlan>, StartError>`.** This is a new step 0 in `run_elected_impl`, after election and before ht-5fr.1's step 1:
  - `requested`:
    1. create `<node_dir>/backups/<unix_ms>/` (mode 0700);
    2. open the old database read-only and `VACUUM INTO '<dir>/herdr-threads.db'`; a failure is logged and the step continues;
    3. rename `herdr-threads.db`, `-wal` and `-shm` into `<dir>/orig/`, then fsync;
    4. set `phase = "moved"` and `backup_dir`.

    Nothing is deleted. The old bytes are preserved in two forms whenever the vacuum succeeds.
  - `moved` or `salvaging`: it skips straight to the plan. `ResyncPlan{from, logs, backup_db}` uses `backup_db` = the vacuumed copy if it exists, else `orig/herdr-threads.db`.

  ht-5fr.1's steps then run against a missing store. The key is loaded, not created, and `journal::open` bootstraps a new random incarnation (§8), which is root §10's "rebuilds under a new incarnation".
- **After `journal::open`, before `SyncCoordinator::start`:**
  - `add_explicit` for each marker target, which restores the path to `from`;
  - `CoordinatorDeps.resync_pin = Some(ResyncPin{from, logs})`;
  - `phase = "salvaging"`.
- **Pin.** In `reselect(L)`, a pinned log (every log when `logs` is `None`, else the logs of the named nodes) has only the link to `from` as a candidate while that link is active. The pin lifts once a round with `from` completes (`SyncSignals`), or after 10 minutes. That is the meaning of `--log`: which logs must come from the named peer.
- **Salvage.** This is a `Job::Salvage(backup_db)` on the worker:
  1. Open the backup read-only.
  2. For each log in its `sync_logs`, in `LogKey` order, stream `journal` rows ascending in batches of 256 into `JournalPort::ingest`, under the same background writer turns as sync ingest.
  3. Stop that log at the first outcome other than `Stored`, or at a SQLite error, and continue with the next log.

  Ingest verifies every entry, so corrupt rows are never admitted. This recovers facts the damaged store authored that never reached a peer. Its own old incarnations are ordinary logs in the new store, so they apply with own-history suppression. The marker is deleted when salvage finishes. The backup directory is kept, and the operator deletes it.
- **What a rebuild loses.** Node-local state that is not in the journal is lost: bindings, `operations`, wake and attention bookkeeping, `herdr_hosts` (rebuilt on attach), and service-managed threads (`journaled = 0`, ht-5fr.4 §2). Own seats come back from `seat.created` as `unresolved` (ht-5fr.4 §4) until C1 continuity or an operator rebinds them. The command is operator-gated, and the losses are listed in its help text and in the TRUST-POLICY repair entry.

**Considered.**
- Rebuilding in the running daemon: it would need to swap a live `SqliteStore` under every worker. The marker and restart reuse the existing start path.
- `--log` as a per-log re-pull into the existing store: the journal is append-only (triggers forbid deleting or updating envelope rows), so a damaged log cannot be replaced in place.
- Skipping the salvage: facts that never left the damaged node would be lost even though their bytes are often intact and verifiable.

### 12. Runtime: sink, worker, coordinator changes

**Recommended.**
- **`ForkSink` extension** (in ht-5fr.5's `src/sync/coordinator.rs`). It is fixed here because ht-5fr.5 is not yet implemented:

```rust
pub enum ForkAck { Paused, Corrupt }
pub trait ForkSink: Send + Sync {
    fn on_fork(&self, log: LogKey, evidence: ForkEvidence, source: NodeUuid) -> ForkAck; // was ()
    fn resumed(&self) -> Vec<LogKey>;
    fn on_have(&self, _source: NodeUuid, _fork_cap: bool, _logs: &[HaveLog], _forks: &[ForkRef]) {}
    fn on_branch(&self, _source: NodeUuid, _reply: BranchReply) {}          // Entries{..} | None{..}
    fn outbox(&self) -> ForkOutbox { ForkOutbox::default() }               // {sends: Vec<(NodeUuid, BranchWant)>, refs_changed: bool}
    fn fork_refs(&self) -> Vec<ForkRef> { Vec::new() }                     // ≤64, newest first
}
```

  `LogOnlyForkSink` returns `Paused` and keeps the defaults. All methods are non-blocking. `IncarnationsSink` only enqueues jobs, and reads state it shares with the worker behind a `Mutex`.
- **Coordinator additions:**
  - call `on_have` after each `have` merge;
  - route `branch_want` to serving (§4), and route `branch_entries` and `branch_none` to `on_branch` after a `TryFrom<WireEnvelope>` shape check (a shape error drops the frame and counts against the source), then grant credit;
  - on the 1 s tick, drain `outbox()`: each send goes to the active `fork.v1` link to that node, or is dropped when there is none (the sink's 30 s timeout retries elsewhere), and `refs_changed` sends a `Have{full: false, logs: [], forks}` delta to every active `fork.v1` link;
  - full `Have`s carry `fork_refs()` to `fork.v1` peers;
  - treat `ForkAck::Corrupt` as `IngestOutcome::Corrupt`;
  - `resync_pin` in `reselect`.
- **`IncarnationsWorker`** is a std thread, `herdr-incarnations`, owned by the daemon worker slot and stopped by `factory_stop`.
  - Its bounded job queue holds 256 jobs. `Have` jobs coalesce per source, and branch replies are capped at 8 MiB in total. Overflow drops the job with one log line per minute, and requests are retried by timeout.
  - Each job is one quantum under `writer.enter_background(5 s budget)`, run through `execute_budgeted_decision`.
  - At start it re-runs `reissue_pass` until done, resumes the `pulling` branches, and runs the salvage if a `ResyncPlan` is present.
  - Failures call `status.record_failure(Lane::Sync, &e)`, so Health shows the existing `lane sync failed: …` line. A pacer-style backoff (base 1 s, cap 60 s) applies before the job is retried.
- **Composition** (`src/app.rs`): `IncarnationsSink::start(deps)` runs before `SyncCoordinator::start` and replaces `LogOnlyForkSink` in `CoordinatorDeps`. On shutdown the worker is cancelled after the coordinator stops, with a 3 s join bound.
- **Status for ht-5fr.8.** `store::incarnations::status(conn) -> IncarnationsStatus { logs: Vec<LogRepair{log, frozen_at, origin, evidence_count, branches: Vec<BranchStatus>, reissued, pending}>, own_unknown_newer: Vec<LogKey> }`. `own_unknown_newer` lists own-node logs that are not active, are not named as a predecessor by any own genesis, and have a genesis HLC above the active genesis. That is a clone suspect (research 09 F6). It is detection only: no action is taken, and the operator repairs it.

**Considered.**
- Running repairs on the coordinator's ingest worker: it would serialize fork repair with ordinary relay, and the coordinator must not resolve forks (root).
- A new `IncarnationsPort` trait for the coordinator: the coordinator already has `ForkSink` for repair and `JournalPort` for serving, so a third seam would have one implementation.

### 13. Trust policy

**Recommended.** ht-5fr.9 amends TRUST-POLICY.md with fork quarantine and re-issue as defined repairs (root §12). The task that lands `owner_rotate` and `reissue_pass` checks that amendment and adds, in the same commit, any of these accepted limits that it does not already state (AGENTS.md trust rule):
1. **Dead owner.** When a forked log's owner never returns, the facts on divergent tails stay only on the nodes that hold each branch.
2. **Same-ordinal facts.** Facts from different branches that decide the same per-home ordinal or one-shot state (a membership transition `n`, an invitation answer, a receipt's settlement `how`) resolve on each node to whichever it applied first. The seat's next transition makes them converge again.
3. **Unresolved seats after a rebuild.** A resync rebuild loses node-local bindings and managed threads, and leaves own seats `unresolved`.
4. **Re-issued facts are attributed to their origin.** They keep their original fact IDs and HLC, and they are attributed `replicated_from:<owner>`, never to the node that held the branch.

The resync marker records `operator:local-user:<uid>` provenance, and the daemon log line names it.

## Testing

- **`src/sync/incarnations.rs` unit tests** (pure, seed identities from `NodeIdentity::from_seed`):
  - `classify_pair` for each outcome: a same-counter proof, a prev-link proof in both argument orders, `Same`, `NotProof` (distance 2, different logs, consistent prev link), and `Corrupt` (a flipped byte, a foreign key);
  - `classify_ahead_of_self` for each outcome: `Restore`, `Fork` at a shared counter, `Fork` by prev link, `NotFork` on an equal hash, `NotFork` on a gap, `Corrupt`;
  - the sink request table, with paused time: a 30 s timeout moves to another source, a penalty lasts 10 minutes, outstanding requests are capped, and `Have` jobs coalesce per source.
- **`tests/store/incarnations.rs`** (a module of `src/store/incarnations.rs`, with two or three `SqliteStore`s as nodes):
  - `record_fork` freezes at `x - 1`, stores the missing entry in `journal_forked`, deduplicates repeated evidence, and lowers but never raises the freeze (the trigger fires on a raise);
  - I1 (`freeze` refuses the active log; `open` refuses a store whose active log is frozen) and I2;
  - `next_incarnation` never returns a used own incarnation (fixed seed with a forced collision), and a fresh store bootstraps a non-1 random incarnation;
  - `owner_rotate` writes a genesis whose `predecessor.frozen_at` is `Some(x - 1)`, moves `active_log_ix` and freezes the old log; its re-check turns a `Restore` into a `Fork` when a command authored in between;
  - `restore_rotate` leaves the old log active-state and non-active, and a later ingest of its tail is `Stored` and applied with suppression (no `wake_work` and no lazy rows for settled and unsettled own-history recipients, per ht-5fr.3 §8 (a));
  - `branch_progress` skips common entries, stores off-chain ones, lowers the owner's freeze to the first divergence, runs the back-search when the first entry does not chain, and rejects a broken chain with a source penalty;
  - `reissue_pass`:
    - re-issued envelopes keep kind, version, HLC, creates, needs and payload bytes byte-for-byte;
    - their deps are the element-wise maximum of the two vectors and never include the active log;
    - `node.genesis` is skipped;
    - a `journal_forked` row waits for its predecessor;
    - a second pass is a no-op;
    - a handler rejection is recorded as `failed` without aborting;
    - a crash between passes (simulated by dropping the store mid-sequence) resumes correctly;
  - re-issue suppression: a settlement found only in `journal_forked` suppresses, an unsettled message to a local seat writes its lazy row and `wake_work`, and `is_own_history` drives the `local_journal` attribution;
  - `serve_branch` from `journal`, from a `journal_forked` walk into `journal`, `None` for an unknown tip, and `None` over the walk bound;
  - `adopt_declared_freeze` lowers the freeze, and ignores a log that is not held.
- **JournalPort contract** (`tests/sync/journal_port_contract.rs`): `serve_branch` cases run against both `SqliteStore` and `MemoryJournal`.
- **Frames and link** (`src/sync/frame.rs`, `src/sync/link.rs`):
  - round trips of the three frames and of `Have.forks`;
  - a peer without the cap decodes a `Have` with `forks` (field ignored) and `branch_*` as `Unknown`;
  - `branch_entries` consumes and returns credit like `entries`.
- **Coordinator** (`src/sync/coordinator.rs` tests, `MemoryJournal` plus a recording fake `ForkSink`):
  - `on_have` is called per `have`;
  - `branch_want` is served, and over the 4-request cap it gets `branch_none`;
  - replies reach `on_branch` with credit granted;
  - the outbox sends reach only `fork.v1` links;
  - `refs_changed` produces one delta;
  - `ForkAck::Corrupt` penalizes and reselects;
  - `resync_pin` restricts the candidates until the round completes, then lifts.
- **Convergence scenarios** (`tests/sync/incarnations_scenarios.rs`, a module of `tests/combined.rs`). These drive three to four in-process stores through `JournalPort` and the sink and worker with an in-memory frame shuttle, with no daemons. Each ends with ht-5fr.3's oracle comparing `replicated_projection` across all nodes, and with every old incarnation frozen wherever a fork was seen:
  1. **Restore without fork.** A is restored to an earlier copy, B holds A's tail, A rotates, pulls back, and creates no wake rows for settled history.
  2. **Restore, then author.** A forks; C (which holds the old branch) freezes from ingest; D learns the fork only from C's `ForkRef`; A rotates, pulls both branches and re-issues; everything converges.
  3. **Wiped store with the key intact.** The store bootstraps a new random incarnation, no fork is detected, and the old incarnation is pulled back.
  4. **Have-time mismatch.** Two nodes at equal counters with different heads detect the fork with no ingest.
  5. **Corrupt branch reply.** It penalizes that source, and the branch completes from another.
  6. **Unknown kind.** An entry kind A does not know is re-issued unchanged and applies on a node that knows it.
  7. **Three branches.** The owner's freeze lowers to the minimum divergence.
- **Resync** (`tests/daemon/sync_resync.rs`, test-support, its own `[[test]]` because it runs daemons; two daemons linked with `sync.dial_command`, all processes owned and stopped, followed by `scripts/check-no-leaked-processes`):
  - a request with no path refuses;
  - a request writes the marker and stops the daemon;
  - the restart produces `backups/<ts>/herdr-threads.db` and `orig/`, a new incarnation, the dial target restored, salvage recovering a fact authored while the link was down, the peer's facts pulled, and the marker removed;
  - a crash injected after `moved` resumes without a second backup.
- **Checks:** `nice cargo clippy --locked --all-targets --all-features -- -D warnings`, `scripts/check-harness-neutral`, `nice scripts/check-default-features`, focused `nice cargo test --locked --all-features incarnations resync journal_port fork`.

Suggested task cut (files touched):
1. Schema, triggers and the store table owner: `migrations/node/0001_node_baseline.sql`, `src/store/incarnations.rs` (`record_fork`, `next_incarnation`, `adopt_declared_freeze`, `status`), `src/store/mod.rs`, `src/service/kicks.rs`.
2. Journal refinements: `src/store/journal.rs` (random bootstrap, `Predecessor`, lower-only `freeze`, I1, `serve_branch`), `src/sync/facts/node_genesis.rs`, `src/sync/journal_port.rs`, `src/sync/memory_journal.rs`, `tests/sync/journal_port_contract.rs`.
3. Frames and credit: `src/sync/frame.rs`, `src/sync/link.rs`.
4. Proof classification: `src/sync/incarnations.rs` (pure functions).
5. Re-issue authoring and apply mode: `src/store/journal.rs` (`reissue`), `src/store/apply.rs`, `src/store/incarnations.rs` (`reissue_pass`), plus TRUST-POLICY.md accepted limits if missing.
6. Owner and restore rotation and branch pulls: `src/store/incarnations.rs` (`owner_rotate`, `restore_rotate`, `upsert_branch_tip`, `branch_progress`).
7. Sink, worker and coordinator wiring: `src/sync/incarnations.rs`, `src/sync/coordinator.rs`, `src/app.rs`.
8. Resync: `src/sync/resync.rs`, `src/app.rs`, `src/sync/coordinator.rs` (pin).
9. Scenario and daemon tests: `tests/sync/incarnations_scenarios.rs`, `tests/combined.rs`, `tests/daemon/sync_resync.rs`, `Cargo.toml` (`[[test]]`).

Order: 1 → 2 → (3, 4) → 5 → 6 → 7 → 8 → 9.

## Key abstractions

**ForkProof (`classify_pair`, `classify_ahead_of_self`)** — new pure functions plus the `Proof` value struct and the closed enums `Classified::{Proof, Same, NotProof, Corrupt}` and `AheadClass::{Restore, Fork, NotFork, Corrupt}` (in `src/sync/incarnations.rs`). Together they are the one statement of what counts as a fork.
- Owns: the rule that a proof is two validly signed entries proven distinct at one counter (same counter, or a prev-link mismatch), the choice of held versus conflicting entry, the proof counter `x` and so the freeze bound `x - 1`, and the corrupt-versus-fork split (a bad signature is never a fork).
- Hides: the order of the verify, counter-distance and prev-link checks.
- Must not: touch the store, the clock or the network, act on a verdict, or read payloads.
- Relationships: a sibling of ChainRules, which raises `AheadOfSelf` before verifying, so this verifies first. It depends only on Envelope and LogKey. Its callers are `IncarnationsSink::on_fork` (synchronously, for `AheadOfSelf`), the worker (ingest `Conflict`, Have-time and forwarded `ForkRef` replies), and the re-check inside `owner_rotate`/`restore_rotate`. Those pure functions are the only part of `sync::incarnations` that `store::incarnations` uses.
- Errors: none of its own. `Corrupt{which, err}` carries the `EnvelopeError`, and callers `match` every variant.

**IncarnationStore (`store::incarnations`)** — new store module, the only owner of `sync_forks`, `sync_branches` and `sync_reissues`, and the single place where repair decisions become rows.
- Owns: `record_fork` (store the missing proof entries, insert the evidence row, freeze lower-only), `owner_rotate`/`restore_rotate` (the in-transaction re-check, and rotation before freeze so I1 holds), `next_incarnation`, `upsert_branch_tip`, `branch_progress` (chain checks, skipping common entries, the exponential back-search, lowering the freeze to the first divergence), `reissue_pass` (candidate order, branch readiness, `waiting` counts, idempotence through `sync_reissues`), `adopt_declared_freeze`, `status` (including `own_unknown_newer`), and invariants I2 and I3.
- Hides: the three tables, their triggers and the candidate query's ordering.
- Must not: write `journal`, `journal_forked` or `sync_logs` itself. It goes through Journal's `store_forked`, `freeze`, `begin_incarnation` and `reissue`. It must not hold network or timing state (sources, timeouts and penalties live in the sink), or interpret a payload (re-issue is kind-agnostic).
- Relationships: called only by IncarnationsWorker inside background writer quanta, by the `node.genesis` handler (`adopt_declared_freeze`), by `journal::open` (`next_incarnation` at bootstrap), and by ht-5fr.8's status. It depends on Journal, ForkProof and `SyncState.incarnation_rng`.
- Lifecycle: stateless functions over persisted state. Every function is one transaction, so a crash between quanta resumes from the rows.
- Errors: a `branch_progress` verdict (`Corrupt` chain or signature, which penalizes the source; `NeedsEarlier`, which triggers the back-search; progress) is a value, not an `Err`. `Err` stays reserved for store errors, which roll the quantum back. A handler rejection inside a re-issue is recorded as `failed` and is not an error (§8).

**IncarnationsSink / IncarnationsWorker** — new struct pair (`src/sync/incarnations.rs`). The sink is the `ForkSink` implementation and the non-blocking front. The worker is the `herdr-incarnations` std thread that runs repair jobs one writer quantum at a time.
- Owns: the bounded `Job` queue (a closed enum: `Fork`, `Have`, `Branch`, `OwnerFork`, `Restore`, `Salvage`, plus the re-issue tick) with per-source `Have` coalescing and the 8 MiB reply cap. It also owns one `Mutex`-guarded shared state with the resumed set, the `ForkOutbox`, the `fork_refs`, the `tip → sources` map, the per-`(source, log)` Have memo, and the branch-request table (at most one outstanding request per branch, a 30 s timeout, failover to the next source, 10-minute `(source, log)` penalties, oldest-last-request preference). Finally it owns the triggers that drive `reissue_pass` (after a commit, at start, every 5 s while `waiting`) and the failure path (`Lane::Sync` status plus a 1 s → 60 s backoff).
- Hides: all fork policy from the coordinator, and when and from whom branches are pulled.
- Must not: block in any `ForkSink` method (the one synchronous signature check in `on_fork` excepted), write SQL outside `store::incarnations` and Journal, send frames itself (the coordinator drains the outbox), or share its pull penalties with the coordinator's rx-source penalties.
- Relationships: chosen once in `CoordinatorDeps` in `app.rs`, replacing `LogOnlyForkSink`. The worker uses `store::incarnations`, ForkProof and, for salvage, `JournalPort::ingest`. Per spec §12 there is no `IncarnationsPort`: `ForkSink` and `JournalPort` are the two seams.
- Lifecycle: started before `SyncCoordinator::start`. The worker is owned by the daemon worker slot, is cancelled after the coordinator stops (3 s join bound), and is stopped by `factory_stop`. The in-memory state is rebuilt empty at start, and the worker re-derives its pulls and re-issues from the store.
- Errors: `ForkAck::Corrupt` (the coordinator penalizes and reselects) is kept apart from `Paused`. A dropped job on overflow is logged and recovered by timeout or by the next `Have`.

**Resync (`src/sync/resync.rs`)** — new module: `ResyncMarker` (`resync.json` v1), the closed `ResyncPhase::{Requested, Moved, Salvaging}`, `ResyncPlan` and the salvage reader. It is the restart-mode rebuild state machine.
- Owns: the request preconditions and their order, the atomic marker writes, the step-0 phase transitions (backup by `VACUUM INTO`, moving the old files into `orig/`, choosing `backup_db`), the plan handed to composition, and deleting the marker when salvage ends.
- Hides: the marker format, the backup directory layout, and that a rebuild is a restart against a missing store.
- Must not: delete the old database or the backups, swap a store under a running daemon, or admit salvaged rows except through `JournalPort::ingest` (which verifies every entry).
- Relationships: `request` is called by ht-5fr.8's operator-gated CLI path, and it uses the existing graceful stop. `resume_at_start` is step 0 of `run_elected_impl`. `app.rs` turns the plan into `add_explicit` targets, `CoordinatorDeps.resync_pin` and `Job::Salvage`.
- Lifecycle: the marker file is the persisted state. Each phase is restart-safe, and `moved`/`salvaging` never back up a second time.
- Errors: request refusals are kept apart: `InvalidRequest` (unknown node), `Unavailable` (no path) and `StoreFull` (no room for the backup). A failed vacuum is logged and the rebuild continues. A start failure is a `StartError`.

**Incarnations** — extended: realized as ForkProof, IncarnationStore, IncarnationsSink/IncarnationsWorker and Resync. Freezes are lower-only and set at the proof counter minus one, and convergence rests on the union re-issue plus idempotent apply by fact ID, not on nodes agreeing on a freeze counter.

**ForkSink** — extended: `on_fork` returns `ForkAck::{Paused, Corrupt}`, and the trait gains `on_have`, `on_branch(BranchReply)`, `outbox() -> ForkOutbox` and `fork_refs()`, all with no-op defaults, so `LogOnlyForkSink` is unchanged apart from its return value.

**Journal** — extended: a random bootstrap incarnation through `next_incarnation`; `Predecessor { incarnation, frozen_at: Option }`; a lower-only `freeze` returning `FreezeOutcome::{Frozen, Lowered, Unchanged}` that refuses the active self log; startup invariant I1; `serve_branch`; and `reissue`, the second authoring entry point. `reissue` copies the source fields, deps are the element-wise maximum, the HLC is not ticked, and a rejection never aborts.

**JournalPort / MemoryJournal** — extended: `serve_branch` with the same rules on both implementations (a `journal` range, a bounded `journal_forked` walk back into `journal`, or `None`), covered by the contract suite.

**ApplyEngine / ApplyContext** — extended: `ApplyMode::Reissue{of}` (no readiness check, `needs` parking kept); the re-issue suppression rule (suppress only when this node authored a settlement for the key in any of its logs, in `journal` or `journal_forked`); and `is_own_history()`, which Attribution reads to record `local_journal`.

**PeerLink (`PeerFrame`)** — extended: the `BranchWant`, `BranchEntries` and `BranchNone` variants, `ForkRef`, and `Have.forks` behind the `"fork.v1"` cap. `branch_entries` shares the credit-gated data queue with `entries`.

**SyncCoordinator** — extended: it calls `on_have` after each merge, routes `branch_*` frames (with a shape check before `on_branch`, then grants credit), serves `branch_want` with no subscription and at most 4 outstanding per link, drains the outbox on the 1 s tick to `fork.v1` links only, carries `fork_refs` in full `Have`s and in the `refs_changed` delta, treats `ForkAck::Corrupt` as `IngestOutcome::Corrupt`, and applies `ResyncPin` in `reselect` until a round with the pinned peer completes or 10 minutes pass.

**`node.genesis` (FactKind)** — extended: `predecessor` becomes `{incarnation, frozen_at}`, and a `Some` freeze calls `adopt_declared_freeze`.

**SyncState** — extended: `incarnation_rng` (an xorshift seeded from `getrandom` at `open`, fixed by the test-support `seed_incarnations`).

**Lane (`service/kicks.rs`)** — extended: `sync_forks`, `sync_branches` and `sync_reissues` go into `KNOWN_UNMAPPED`. Freezes kick `Lane::Sync` through the existing `sync_logs` mapping.

Variation: how forks are handled varies (log-only, or Incarnations), so that stays the ancestor's `ForkSink` trait, chosen once in `CoordinatorDeps`, and no new trait is added (spec §12 rejects `IncarnationsPort`). Fact kinds vary, but repair is kind-agnostic: re-issue copies kind, version, HLC, creates, needs and payload bytes, so a new kind needs nothing here. Everything else is a fixed set and is a closed enum or CHECK handled with `match`: `Classified`, `AheadClass`, `ForkAck`, `Job`, `FreezeOutcome`, `ApplyMode`, `ResyncPhase`, fork `origin` (`evidence`/`declared`), branch `state` (`pulling`/`complete`/`unavailable`) and the new `PeerFrame` variants behind `fork.v1`. Per spec §6, the owner path and the third-party path are one worker that branches on "is this my log", which is the fixed ownership test (`node_uuid` equality), not a kind switch.

Walk-through: A is restored from backup and authors one command. C holds A's old tail. C's ingest of A's new entry returns `Fork{Conflict}`, so the coordinator pauses the log and calls `on_fork`, and the sink queues `Job::Fork`. The worker runs `classify_pair`, gets `Proof{x}`, and `record_fork` stores the conflicting entry in `journal_forked`, inserts the evidence row and freezes at `x - 1`. After the commit, the sink resumes the log and sets `refs_changed`. The coordinator sends a `Have` delta with `forks`. D receives the `ForkRef`, fetches the side it lacks through `branch_want`, and records the same fork. A sees C's `Have` head mismatch or ingests C's entry as `AheadOfSelf`. `on_fork` classifies `Fork`, and the worker's `owner_rotate` re-checks, mints a random incarnation with `predecessor.frozen_at = Some(x - 1)`, freezes the old log and upserts C's tip. Branch pulls fill `journal_forked`, `reissue_pass` interleaves both branches by counter through `journal::reissue`, and the re-issued entries apply with re-issue suppression. Every node then applies them as entries of A's new log, already-held facts are no-ops by ID, and the replicated projections converge. Likely future change: a new fact kind (for example ht-5fr.4's `seat.renamed`) adds one `FactKind` module and one registry entry. Its forked entries are pulled, stored and re-issued byte for byte, and ForkProof, IncarnationStore and the sink are untouched.

Digest:
- ForkProof: owns what counts as a fork, the proof counter and corrupt-versus-fork; a new proof shape is a `Classified`/`AheadClass` case here;
  callers never act on an unsigned `Have` mismatch or compare entry hashes ad hoc.
- IncarnationStore (`store::incarnations`): owns `sync_forks`, `sync_branches`, `sync_reissues`, rotation, branch progress and the re-issue pass; new persisted repair state goes here;
  callers never write those tables or freeze, rotate or re-issue through Journal directly.
- IncarnationsSink/IncarnationsWorker: own the fork job queue, branch-request sources, timeouts and penalties, the outbox and fork refs; new repair triggers are `Job` variants here;
  the coordinator never decides, retries or penalizes a repair, and sink methods never block.
- Resync: owns the resync marker, its phases, the backup and move, and salvage; new rebuild steps are phases here;
  nothing deletes old store files or rebuilds the store inside a running daemon.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
