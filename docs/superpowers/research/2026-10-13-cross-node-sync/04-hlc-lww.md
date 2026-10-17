# 04 — Hybrid logical clock and clock-skew handling

Scope: research question 4 of the vision spec (`docs/superpowers/specs/2026-10-13-cross-node-sync-vision-design.md`). Covers HLC encoding, crate choice, drift bounds, persistence, and three uses: LWW registers (topic, name, archived), ingest interleaving, and the sender-side ACK clock.

## Recommendations

1. **Write a small HLC in-tree; do not take a crate.** It needs about 60 lines over the existing `Clock` port (`context.clock().utc_now()`, `src/store/schema.rs:3655`), so tests can inject time and the clock state can live in the same SQLite transaction as the journal append. `uhlc` takes a `fn() -> NTP64` clock, embeds a u128 ID in every timestamp, and defaults to a 500 ms reject window [S2]. `hlc-gen` uses a 2024 custom epoch with a 42+22 split and errors on drift [S3]. Neither persists state, and neither has the "absorb only within bound, never reject" behaviour recommended below.
2. **Encoding: one `u64`, stored as SQLite `INTEGER`, with 48 bits of Unix milliseconds in the high part and a 16-bit counter in the low part.** This is the split the HLC paper proposes (it uses µs-rounded NTP rather than ms) [S1]. 48-bit ms lasts until about year 10889, and the sign bit stays clear for i64. The tie-break is `(hlc, origin_node)`, compared as an integer pair, not packed. If the counter reaches 65535, borrow by setting l += 1 instead of failing. Long's implementation throws `OverflowError` at 65535 [S6]; a daemon must not fail a write over this.
3. **Never reject a replicated entry because of its HLC.** Rejecting one would stall that origin's per-origin FIFO forever, which contradicts "nothing silently lost, only delayed". The uhlc docs advise "refuse or drop the incoming event" [S2]. That advice suits pub/sub, not an append-only log. Instead:
   - On ingest, **ratchet** the local HLC with the remote `hlc` only when `remote.l - now <= MAX_ABSORB`. When the gap is larger, store the entry as is and leave the clock untouched. This matches uhlc's behaviour on `Err` [S2] without dropping anything.
   - **`MAX_ABSORB` = 60 s.** Production defaults span a wide range: CockroachDB 500 ms [S4], uhlc 500 ms [S2], Long's CRDT 60 s [S6], MongoDB 1 year [S5]. Laptops drift by seconds after sleep and SSH-linked boxes have no tight-sync guarantee, so sub-second bounds would trip constantly. Make it a config knob, not a version-dependent gate.
   - **Measure skew at link handshake** (each side sends `utc_now` in the hello and computes the offset with RTT/2). Expose it in `peer status`. Emit a node-local warning (not replicated) when |offset| exceeds 10 s, and a stronger one when an ingested entry exceeds `MAX_ABSORB`. **Do not shut down.** CockroachDB self-terminates at 80% of max-offset [S4], but that is a CP choice and the spec is AP.
4. **LWW writes must dominate what the writer has seen.** A locally authored topic, name, or archive write uses `hlc = max(local_hlc.next(), register.hlc + 1)`, where `+1` increments the counter with the borrow rule. This keeps a "poisoned" far-future register (one that was not absorbed into the node clock, per rec. 3) from silently overriding a user's later edit. It also leaves the node-wide HLC unaffected. It is causally correct, because the writer read that value. Store the winning `(hlc, origin)` per register, and apply the rule on every node in materialization: higher pair wins, regardless of arrival order.
5. **Persistence: the HLC never regresses across restarts.** Every HLC-stamped fact is a journal row, so on startup set `last = max(MAX(hlc) over the whole journal, all origins absorbed within bound, utc_now)`. A single-row `hlc_last` column that is updated in the same transaction as each append avoids the scan. Absorbed remote values have to be included: otherwise, after a restart, a local LWW write could sort below a remote write that this node had already shown. Rec. 4 also covers this case per register.
6. **Ordering stays out of the HLC.** The dense per-thread `sequence` assigned at ingest is the only display order, as the spec says. Show the HLC's l as "sent at" (it can be ahead of the sender's true time by up to `MAX_ABSORB`). Do not re-sort by it.
7. **ACK clock: the sender's local wall clock is enough, provided the sender's node also judges overdue.** Today deadlines are absolute UTC milliseconds compared against `decision.utc` (`src/store/receipts.rs:575`). If the deadline is set on the sender's node and evaluated there, skew between nodes cancels out. Only the sender's own clock steps matter, and those only move the deadline by the step. Recipient nodes should treat `deadline` vs their own clock as advisory display only.
8. **Sleep and wake.** Keep deadlines in UTC, not `Instant`. `Instant` uses `CLOCK_UPTIME_RAW` on macOS, and the Rust docs leave unspecified whether suspend counts [S7][S9]. Detect a wake by `Δutc − Δmonotonic` exceeding a threshold. After a wake, hold overdue evaluation briefly (for example until the first vector exchange with reachable peers completes, capped at about 30 s), so ACKs authored elsewhere during the sleep land first. Without the hold, a laptop that wakes after 8 hours emits a burst of overdue warnings and then "late" settlements. The existing receipt path already handles a late ACK after overdue (`src/store/receipts.rs:539`). The hold only reduces churn.

## Findings

### HLC algorithm and encoding
HLC keeps `l` (the max physical time seen) and `c` (a counter). It guarantees `e hb f ⇒ hlc.e < hlc.f` and `l.f ≥ pt.f`, and keeps `|l − pt|` bounded by clock-sync error [S1]. The paper packs `l` into the 48 most significant bits of the 64-bit NTP timestamp and leaves 16 bits for `c`, which "allows it room to grow up to 65536, which is more than enough" [S1, §6.2]. uhlc keeps full NTP64 with only 4 counter bits (`CSIZE = 4`) and resolves sub-ns ticks [S2]. hlc-gen uses 42-bit ms since 2024-01-01 and a 22-bit counter, lock-free on `AtomicU64` [S3]. At 1–10 msg/s total, millisecond precision with 16 counter bits is far from any limit. A counter of 65536 would only fill if the wall clock stepped back by about 1.8 h while the node sustained 10 writes/s, and the borrow rule handles even that case.

### Drift bounds in practice
- CockroachDB: default max offset 500 ms. A node that detects drift over 80% of that against half the other nodes shuts itself down. Leap-second smear or slew is preferred because steps cause "nodes may spontaneously exit" [S4].
- MongoDB: `maxAcceptableLogicalClockDriftSecs` defaults to one year and caps how far cluster time can advance in one step [S5].
- uhlc: an incoming timestamp more than delta (500 ms, set by `UHLC_MAX_DELTA_MS`) ahead returns `Err` and the clock is not updated [S2].
- Long's CRDT HLC: `maxDrift: 60000`, and it throws `ClockDriftError` [S6].

These systems agree on one point: a far-future timestamp must not drag the local clock forward without limit. A system that orders strictly (CockroachDB) refuses it. A system with an append-only log (this one) can only decline to *absorb* it.

### Clock sources on laptops
`SystemTime` "is not monotonic" and can step in either direction [S8], which is why the HLC exists. `Instant` never goes backwards, but on Darwin it is `CLOCK_UPTIME_RAW`, and whether it counts suspend "varies across platforms and Rust versions" [S7]. Rippling saw Windows count sleep while macOS did not, and wrote a crate to normalise this [S9]. A proposal to switch Linux `Instant` to `CLOCK_BOOTTIME` was closed without merging [S10]. So a deadline timer built on `Instant` or tokio sleeps will not fire across a sleep. Re-check deadlines against UTC on wake.

### Current code fit
Each decision already captures `DecisionInstant { utc, monotonic }` from an injectable clock (`src/store/schema.rs:3655`), and receipt deadlines are absolute UTC compared to `decision.utc` [S11]. The HLC can be computed in the same place, replacing `utc` as the stamp for journal entries.

## Risks to spec decisions (flagged separately)

- **R1, the overdue author is not named in the spec.** "The ACK clock starts at send time on the sender's node" does not say which node declares overdue. If the recipient's node evaluates it, skew between nodes moves the deadline (by minutes with a bad clock). Recommendation: the sender's home node authors the overdue timeline event. This fits "one authoring node per fact". The cost: while the sender's node is offline, nobody declares overdue, but the sender's agents are offline then too.
- **R2, poisoned LWW registers.** With plain `(hlc, node)` LWW, one write from a node whose clock is far ahead wins over every honest write until real time catches up. Rec. 4 fixes this for writers who have seen the poisoned value. Two writers on opposite sides of a partition still resolve by raw HLC. That is acceptable, and it is the nature of LWW.
- **R3, "send time" display.** A late message shows its original send time while sitting at the bottom of the list. If the sender's clock is wrong, that time is wrong too. Consider showing the skew-annotated time, or the ingest time, when they differ by more than `MAX_ABSORB`.
- No evidence argues against using LWW for the three multi-writer fields at this scale.

## Limitations and retrieval stop

Stopped with every facet covered by primary sources. Gaps: hlc-gen's support for an injected clock was not checked. There is no field data on how large laptop clock skew gets right after a wake, so the 60 s and 10 s thresholds are judgement calls.

Method: `search` CLI 0.9.0 (serper and perplexity failed on quota; other providers answered), 7 query attempts, about 37 result entries considered, and 13 documents retrieved directly with curl, the r.jina.ai reader, and pdftotext. Ledgers are in the session scratchpad (`r4/`).

## Sources

- [S1] Kulkarni, Demirbas et al., *Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases*, UB tech report 2014-04. https://cse.buffalo.edu/tech-reports/2014-04.pdf
- [S2] atolab/uhlc-rs, README, `src/lib.rs` (L78–87, L287–303), `src/ntp64.rs`. https://github.com/atolab/uhlc-rs
- [S3] hlc-gen documentation. https://docs.rs/hlc-gen
- [S4] Cockroach Labs, *Clock Management in CockroachDB*. https://www.cockroachlabs.com/blog/clock-management-cockroachdb/
- [S5] MongoDB Server Parameters, `maxAcceptableLogicalClockDriftSecs`. https://www.mongodb.com/docs/manual/reference/parameters/
- [S6] J. Long, crdt-example-app `shared/timestamp.js`. https://github.com/jlongster/crdt-example-app/blob/master/shared/timestamp.js
- [S7] Rust std `Instant`. https://doc.rust-lang.org/std/time/struct.Instant.html
- [S8] Rust std `SystemTime`. https://doc.rust-lang.org/std/time/struct.SystemTime.html
- [S9] Rippling, *Suspend-unaware cross-platform monotonic time in Rust*. https://www.rippling.com/blog/rust-suspend-time
- [S10] rust-lang/rust PR #88714, "use CLOCK_BOOTTIME in Instant::now" (closed). https://github.com/rust-lang/rust/pull/88714
- [S11] herdr-threads repository: `src/store/schema.rs:3655`, `src/store/receipts.rs:539,575`.
