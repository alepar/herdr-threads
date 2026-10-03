# Remaining herdr-threads findings (B1–B4, B6–B10) — design

## Goal

herdr-threads at the integration tip is ready to tag v0.1.0: the daemon stays fast and quiet over weeks
of retained history and through Herdr outages, tells the operator what went wrong and where to look,
runs only the cooperative design it ships, admits newer Claude Code / Codex versions honestly with a
scheduled canary that pins down the first breaking version, and is backed by a parallel-clean test suite,
a hardened validator and a fresh native matrix run on that tip.

## Problem description

The first run (`main@cf8c83d7`) shipped a working plugin but left about 165 open findings, bucketed in
`docs/history/remaining-findings-2026-10-01.md`. Each bucket has one shared root: no retention model
(B1), timer-only loops with no wakeup or backoff (B2), no end-to-end failure-to-operator path (B3), a
ports layer that still encodes the pre-cooperative adversarial design (B4), version support asserted by
hard-coded strings and refusals instead of re-observation (B6), a release path that never ran (B7),
evidence and a validator that lag the code (B8), shared mutable test infrastructure (B9), and
presentation layers with no per-field contract (B10). B5 (the trust model) is owned by another session.

## Main challenges

The buckets touch nearly every module, so the work must be sequenced to avoid churn: B4 deletes code
that B1/B9 would otherwise retrofit, and B8's native rerun is only meaningful after every behavior
change has landed. Several fixes cannot be proven by unit tests alone (Herdr-down loops, installer
upgrade with Herdr down, live harness behavior), yet the shared Herdr server and the owner's harness
profiles are off-limits, so every live scenario needs an isolated Herdr session and isolated harness
homes. B6's canary must work in CI without the owner's machine and still be runnable locally, and model
access in CI is not guaranteed. Finally, ~116 Minor items must be closed or explicitly deferred without
inflating each bucket into a rewrite.

## Key decisions made

Each bucket gets exactly one design move, and every member finding is either closed by that move or
listed as an explicit deferral with a reason. The verification layer is deleted, not adapted (owner
decision). Unknown newer harness versions are admitted with an `optimistic` admission label that
Health and doctor report verbatim; only older-than-supported and unparsable versions, and versions
inside a recipe's `known_broken` range, are refused (B6 Decision 2 ladder). A
scheduled GitHub Actions canary calls `scripts/harness-canary.sh`, which installs harness versions into
a throwaway prefix, runs a no-model compatibility tier (plus a model tier when secrets exist) and bisects
the first failing version. The release workflow becomes push-tag-only with SHA-pinned actions and an
idempotent publish; the installer's final status becomes truthful; Linux stays a declared platform.
B8 closes with one late leaf that reruns the native matrix on the integration tip with the hardened
validator; B9 root-causes the `hook_entrypoint` parallel crash and drops the serial pin; B10 adds a
per-result-type presentation contract with golden tests and one shared escaping function.

## Scope

Buckets B1, B2, B3, B4, B6, B7, B8, B9 and B10 of the findings doc, each through the design move below.
Includes code, tests, CI workflows, scripts, the installer and the design/operator docs that describe
them (design docs under `docs/design/herdr-threads/`, `docs/install.md`, `docs/operations.md`,
`docs/release.md`, `docs/compatibility/harnesses.md`, the bundled SKILL text).

## Non-goals

- **B5 (identity, seat continuity, cooperative trust edge)** — owned by a separate session. Where a B5
  file is touched incidentally (for example `me init` output in B10) the change is presentation-only and
  must not alter attribution or continuity semantics. **Carve-out (amended: run.md
  amendment-2026-10-01, human-approved relay from the trust-model session):** B5 items P10 (the
  EmptyShell / verified-execution branches) and W5-1 (`decision_fence`) were decided there as
  deletions and are folded into B4's removal (ht-p03.2). Deleting them is the B5 owner's decision, not
  a B5 semantics change made by this run (policy: only evidence the production adapter can produce may
  drive a transition, `TRUST-POLICY.md` on branch `trust-model-invariants`).
- Pushing the v0.1.0 tag, running the live release, or publishing anything (follow-on, human).
- New features beyond what a finding requires (no new commands, no new harnesses).
- A token-overhead benchmark (B8 gap) beyond recording one measured number per harness from the
  native rerun; a proper benchmark harness is deferred.
- Windows or any platform outside the four release targets.

## Hard constraints

- **Never restart, stop or reconfigure the shared Herdr server.** Every Herdr-down or Herdr-restart
  scenario (B2 hot loop, B3 stale socket, B7 installer upgrade with Herdr down, B8 ht-910 restart)
  runs against an isolated named Herdr test session with its own config, endpoint and state, created
  and torn down by the test itself.
- **Never touch `~/.claude`, `~/.codex`, or aisw profiles** (no writes, no setup into them). Harness
  setup and canary installs use throwaway `HOME` / `CLAUDE_CONFIG_DIR` / `CODEX_HOME` / npm prefix
  directories. Native runs may only *read* credentials (Claude's real config dir for auth, the default
  Codex aisw profile's credentials) exactly as `scripts/validate-native-demo.sh` already does, and
  write only into their run root.
- **No `git push`**, no tag push, no release creation from this run.
- Never `git stash`; set work aside with WIP commits.
- B5 semantics are frozen (see Non-goals), except the P10/W5-1 deletions the B5 owner decided and
  folded into B4 (amended: run.md amendment-2026-10-01; see the Non-goals carve-out).

## B1. Retention and pending-only discovery

**Design move.** Define *live* versus *settled* per table, make every background discovery query read
only live rows through an index, and add one cleanup lane that prunes settled rows on a retention policy.

**Decision 1 — how discovery reaches live rows.** Options: (a) filter in Rust after a full scan (status
quo, rejected: it is the bug); (b) separate pending tables that rows move out of when they settle
(rejected: doubles write paths and needs a data migration of every table); (c) **partial indexes**
`WHERE <live predicate>` plus queries that name their index with `INDEXED BY`. Chosen: (c) — additive,
one schema migration (v9 → v10), and the `INDEXED BY` clause turns a missing index into a prepare-time
error instead of a silent scan. (Amended: B1 nested spec D1 — `INDEXED BY` is a query-side clause; the
partial-index predicates are literal, parameter-free `WHERE` clauses restated in that spec; the
earlier "the same way `work_jobs_ready` already does" wording was loose, since `work_jobs_ready` is a
plain index.) Live predicates (amended: B1 nested spec D1/D3): seats not retired
(`seats_live_ordinal`); work jobs pending or failed (`work_jobs_live`); receipts/ACKs pending and
warnings unresolved reuse the existing v1 partial indexes and v8 pending projections (no new index);
snapshot generations get **no** live index — "the current published one plus at most one staged"
(Amended by coverage r2, 2026-10-01: superseded — the keep set is the B1 nested D3 keep set.)
cannot be a static partial-index predicate, so readers reach live generations by primary-key pointer
joins and the retention keep set (Decision 2) bounds the table.

**Decision 2 — retention.** Options: (a) never delete, only index (rejected: the snapshot table grows a
row per pane every 5 s); (b) **time-and-count retention** in a cleanup lane. Chosen: (b) with fixed
defaults, not user-configurable in v0.1.0. The lane is a fifth Pacer lane with a 60 s safety tick and
an empty kick set (amended: B1 nested spec D4; the earlier "generalised from the digest pruning path"
is superseded). Superseded snapshot generations and their target rows are pruned on the retention
lane's next 60 s tick after a newer generation publishes, keeping the nested keep set — current,
previous published, recovery baseline, generations pinned by unresolved seats or current-baseline
releases, and in-flight stages (amended: B1 nested spec D3/D4; the earlier "pruned immediately …
keep current + previous" is superseded: a per-publish prune would cost a retention commit every 5 s,
and FK-pinned generations cannot be pruned); completed work jobs pruned after 24 h, except
`preparation_cleanup` jobs, whose completion row is the operation-key reuse marker (amended: B1 nested
spec D4, deferral row there); settled warnings retained (amended: B1 nested spec D4; see §Explicit deferrals); retired seats and
their settled receipts are **kept** (they are user-visible history and identity tombstones that B5
reasons about) but excluded from every discovery path by the partial indexes. Cleanup runs in bounded
batches (≤ 256 rows per transaction) on the B2 primitive so it never starves writers.

**Decision 3 — attention rebuild (`src/store/mod.rs:838`, `effective.rs:493`).** The wake worker
re-derives attention only for seats with a pending row in the live index, not from full history; the
O(seats × warnings) fold is replaced by a join over unresolved warnings only. (Amended: B1 nested spec
D2 — wake attention is derived from the v8 pending projections with O(1) per-seat pending probes over
live, non-human seats; `scan_effective_seat_attention` leaves the production wake path and stays as
the test oracle.)

**Closes:** `seats.rs:1217` (observation walk skips retired seats), `mod.rs:838` (+ effective.rs:493),
`seats.rs:702` (snapshot pruning), `mod.rs:689` (work-job discovery uses the new partial index
`work_jobs_live` — amended: B1 nested spec D1; the plain `work_jobs_ready` index named earlier is the
rejected alternative, since it orders by status first and would change the cursor shape),
`queries.rs:990` (page sizing computed once per page with a running byte budget instead of
re-encoding per candidate, all 13 sites through one helper), Wave 18 (human seats with pending ACK are
not wake candidates — amended: B1 nested spec D2; wake discovery excludes human-bound seats with a
`NOT EXISTS` probe on the unique `occupant_bindings_current` binding, O(1) per seat; no index can
exclude them, because "human" lives on the current binding, not on `seats`), W6-R1 (show/participants
reuse the caller's connection and a single bounded seat page), Wave 27 (human `read` batches
SeatInspect/`pane_names` per page and fetches clipped-preview bodies in one query).

**Acceptance.** A cost-flatness test per discovery path (extending the existing flatness tests, made
isolated by B9) proving per-pass work is flat as settled history grows 10×; a migration test v9 → v10
on a fixture database; a pruning test proving the snapshot table stays bounded across 1,000 publishes
(≤ |keep set| + 1 generations — amended: B1 nested spec Acceptance).

## B2. Shared wakeup-and-backoff primitive

**Design move.** One small primitive, `Pacer` (in `src/service/`), adopted by every daemon loop: a
`tokio::sync::Notify`-backed cancellation/kick, a per-lane `kick()` called from commit paths, and capped
exponential backoff on consecutive failures that resets on success.

**Decision 1 — event plumbing.** Options: (a) shorten timers (rejected: more idle CPU); (b) a
channel per lane carrying work items (rejected: duplicates the durable work queue, which is already the
source of truth); (c) **a level-triggered Notify kick** — the store's `after_committed_change` hook
(already defined, never called in production per `workers.rs:857`) is wired into commit paths and
kicks the lanes whose tables changed; the lane then drains durable work as today. Chosen: (c). Timers
remain only as a slow safety net (idle tick 5 s, unchanged reconciliation cadence). (Amended: pacer
nested spec D1 — the mechanism is table-level change capture on the single writer connection; the
writer turn takes its sealed change set **while still holding the writer guard** and kicks after
releasing it, so a commit by another thread can never be attributed to the wrong origin; the
`Retention` kick row maps no table and Retention-origin commits kick no lane, B1 nested spec D4.)

**Decision 2 — backoff shape.** Capped exponential, base 100 ms, factor 2, cap 30 s, ±20 % jitter,
reset on first success; the attempt count and next retry time are exposed to Health (B3). The Herdr-down
observation lane (`reconcile.rs:885`) also stops writing durable rows for repeated identical failures:
it records the first failure and a counter, so with Herdr down **the observation lane** costs at most
one durable commit (the admission fence) per backoff step. (Amended: pacer nested spec D4 — the
repeat-invalidation skip is a complete contract: it applies only after the previous invalidation's
unresolved-marking pass completed, returns a dedicated outcome that keeps the invalidated Health
classification and host-evidence recording, starts no reconciliation pages and counts as a backoff
failure.) **Scope of this bound:** it covers the observation lane only. The wake lane's durable
commits for refused wakes while Herdr is down (a reserve plus a fenced completion per refused attempt,
pacer D5) are not bounded by it; whether that cost is acceptable or needs its own bound is the parked
round-1 escalation (pacer D5 vs this Decision) and remains open for the human — this spec does not
resolve it. (Cross-reference, design roast round 2: the task tree already pins that cost as currently
specified — ht-p03.9.4's outage test asserts **≤ 2 durable commits per seat per refusal-backoff step**
(the reservation plus the restoring completion). That criterion records D5's cost; it does not accept
it. The escalation stays parked, and if the human resolves it with a different bound, ht-p03.9.4's
criterion is revised to match.)
(Superseded wording: the earlier "zero commits per retry" and "idle: no durable commits" are replaced by the nested spec D4 resolution: on an idle daemon, zero durable commits from the deadline and wake lanes and from request paths; the retention lane zero absent publication and ≤ 1 commit per 60 s tick while observation publishes at idle (skip-unchanged deferred); the observation lane at most one cycle's commits per 5 s cadence; with Herdr down, the observation lane at most 1 commit per backoff step. See `pacer-adoption-design.md` D4.)

**Decision 3 — cancellation.** The 10 ms `AtomicBool` sleep-poll in `daemon/transport.rs:67/68` is
replaced by the same Notify-backed cancellation. `SessionLease::drop`
(`service_connection.rs:213`) must not take a blocking `std::sync::Mutex` on the current-thread runtime:
it hands the release to a non-blocking path (`try_lock`, falling back to cancelling the session and
handing `revoke_exact` to `tokio::task::spawn_blocking` — amended: pacer nested spec D3; the earlier
fallback to a release message processed by the owning task is superseded, since the blocking pool is
that owner for blocking work).

**Decision 4 — W9-1 wake ladder.** Pre-send refusals (target not ready) no longer climb the 30 s → 300 s
reminder ladder; they are retried on the Pacer backoff (cap 30 s) because they are transient host
states, and only a delivered-but-unacknowledged wake advances the ladder. (Amended: pacer nested spec
D5 — a refused completion restores the pre-reservation ladder fields and offered frontier in its fenced
update; when a concurrent path already cleared the reservation, the restore matches no row and the
refused attempt keeps its one advanced step, an accepted few-millisecond window that errs toward the
slower ladder.) This is expected to turn R20 from MET-WITH-GAP into MET in the B8 rerun; the MET
closure holds only if R20's backing cells PASS on the final SHA (B8 Decision 3), otherwise R20 stays
open.

**Closes:** `reconcile.rs:885`, `workers.rs:857`, `transport.rs:67/68`, W9-1,
`service_connection.rs:213`.

**Acceptance.** A test against an isolated Herdr session that is stopped mid-run proving the
observation lane makes ≤ 1 durable commit per backoff step and reaches the cap; a latency test proving
a committed send is picked up in < 100 ms without waiting for the tick; an idle-daemon test proving zero
durable commits from the deadline and wake lanes and request paths, the retention lane zero absent publication
and ≤ 1 commit per 60 s tick while observation publishes at idle (skip-unchanged deferred), the observation lane at most one
cycle's commits per 5 s cadence, and ≤ 1 wakeup per safety-net tick per lane (the stricter "no durable commits"
wording is superseded by nested spec D4). Observation commits kick no lane: neither `host_instances`
(admission fence, publication pointer) nor `snapshot_generations` (stage, seal, publish) is in any kick
set (amended by design roast round 2: pacer nested spec D1 — `snapshot_generations` left the Wakes set
because one publish is several separate commits on it).

## B3. Diagnostics contract

**Design move.** One failure-to-operator contract: every daemon failure lands in the existing rotating
`daemon.log` (`src/daemon/logs.rs`) at a path `doctor` and Health print; Health says "degraded: see
<path>" whenever a lane has failed since its last success; errors carry a four-way taxonomy checked
before decode; remedy strings are generated from the taxonomy, not hand-written per call site.

**Decision 1 — pre-election startup errors (`main.rs:52`, `lifecycle.rs:237-239`).** The detached
child's stdio goes to `/dev/null`, and the log sink is installed only after election. Options: (a)
install the sink before election (rejected: the sink's ownership checks assume an elected owner, and
before election every racing child would write the one owner's `daemon.log`, so a starter could not
tell its own child's error from another's); (b) **redirect the detached child's stderr to a
per-attempt startup log** and have `ensure` read and print its tail when the child exits or the start
times out. Chosen: (b) — no ownership change, and the starter that waits is exactly the process that
can report. Because two `ensure` calls can both spawn a child (`ensure_running_with_timeout` probes and
drops the owner lock before `spawn_detached`), the file is **per attempt**, never a shared fixed path:
the starter creates `<state>/logs/startup-<starter pid>-<nonce>.log` with `O_CREAT|O_EXCL` and the same
private-file checks as `daemon.log`, size-capped, hands it to its child as stderr, and prints only that
file's tail — so a starter never prints another child's output (for example a "lost election" line).
Each starter prunes `startup-*.log` files older than 24 h and keeps at most the 8 newest; a failed
attempt's file stays for the operator, and the tail message names its path. (Amended by design roast
round 1: the earlier single `startup.log` "truncated per start attempt" had the same race it rejected
(a) for.)

**Decision 2 — lane errors and status (`workers.rs:625`, `workers.rs:599`, W5-3, Wave 26 ×3).** Every
lane error goes to `daemon.log` rate-limited (first occurrence, then one summary line per backoff cap
window with a count — sharing B2's attempt counter); the 8-entry ring stays as the Health source.
A poisoned status mutex reports `Degraded("status poisoned")`, never Ready. `transitions_refused` is
surfaced in Health and `last_reconciliation_at` advances only on a successful pass; `record_tick`
counts only passes without `work_error`; the `observe_harness_admissions` spawn result is checked and
a failure logged and surfaced. Doctor's "(cooperative mode)" label is applied only to Cooperative, and
Unsupported/Refused print their own reason.

**Decision 3 — taxonomy (`connection.rs:550`, `lifecycle.rs:181`, Wave 25 stale socket).** Four
classes: `Transient` (busy, locked, interrupted I/O; retried by B2), `Unavailable` (daemon/Herdr not
running — including a stale socket file whose connect returns ECONNREFUSED, reported as "server not
running"), `Corrupt` (only SQLITE_CORRUPT / SQLITE_NOTADB and failed integrity checks), `VersionSkew`.
Unlisted SQLite errors map to Transient-with-detail, never Corrupt. Skew is detected **before** decode:
the client reads the handshake's `protocol_version` first, and on mismatch reports
"daemon is version X, CLI is Y: run `herdr-threads daemon stop` then `ensure`" instead of a 5 s timeout
or a `deny_unknown_fields` decode error. That remedy works from the CLI that printed it because
**`daemon stop` is skew-tolerant** (amended by design roast round 2; today `request_stop`,
`control.rs:218-228`, refuses on a protocol mismatch, so the remedy would loop): on a protocol mismatch
stop sends no wire `Stop`; it confirms the owner lock is held and the endpoint descriptor's `pid` and
`boot_id` match the running daemon, sends that pid `SIGTERM`, and waits for the owner lock to be
released (an old daemon without a handler exits by the default action, which SQLite's WAL tolerates).
With a matching protocol, stop keeps the wire `Stop`. Old CLIs talking to a new daemon receive a handshake shaped
for the old decoder (the daemon answers a mismatched-version hello with the minimal skew error the old
version understands).

**Decision 4 — remedies (`lifecycle.rs:273`, `exit.rs:21`).** A single `remedy(class, context)`
function yields the remedy text, always naming the log path when logs are the next step, and, for exit 3,
naming `ensure` for an unavailable daemon and `stop` then `ensure` for version skew (ht-p03.107,
resolving the parked ht-p03.46 escalation). Health's 16-line cap gets a test asserting the worst-case line count
and a 4-line headroom by folding per-lane lines into one summary when more than two lanes are degraded
(Wave 16).

**Closes:** all 13 B3 findings.

**Acceptance.** A startup-failure test (unwritable state dir) asserting `ensure` prints the startup
log tail and exit code; a two-starter test (two concurrent `ensure` calls, one child failing) asserting
each starter prints only its own attempt's tail; a skew test with a fake old-version daemon that also
runs the printed remedy (`daemon stop` with this CLI stops the old daemon, then `ensure` starts this
version — design roast round 2); a SQLite error mapping table test;
a stale-socket test in an isolated Herdr session; a Health line-budget test.

## B4. Remove the pre-cooperative verification layer

**Design move (owner decision 2026-10-01).** Delete the verification layer that only tests reach,
collapse `src/ports.rs` to the surface production calls, and rewrite the design docs to describe the
cooperative system that ships. This lands **first** among the code buckets, because B1 and B9 would
otherwise retrofit code that is about to disappear.

**Decision 1 — what "unreachable" means.** The removal list is determined mechanically, not by
judgment: a port method, trait or store path is removed when it has no caller reachable from
`main.rs` (daemon, CLI, hook entrypoints) — verified by deleting it and confirming that only tests
and the `test_support` module fail to compile. Known members: `allocate_seat`, `revoke_registration`,
`CallerVerifier`, `MutationPermit::new`, and their store paths in `store/{seats,control,receipts,
messages,mod}.rs` and `protocol/authority.rs`. Also known members (amended: run.md
amendment-2026-10-01, the B5 owner's P10/W5-1 decision folded into this removal — see the Non-goals
carve-out): **P10**, the EmptyShell / verified-execution branches the native adapter can never feed —
the `MarkOccupantUnavailable` emission and its store application, `proven_empty_shell_bridge`,
Reconfirm-with-execution and Replace in `identity/reconcile.rs`, and
`NativeLaunchCapability::ProvenEmptyShell` (live continuity becomes `ReconfirmStructure` only); and
**W5-1**, `decision_fence` in `store/mod.rs`, consumed only by native permits. If either deletion proves
unsafe, ht-p03.2's completion report says so rather than forcing it. Tests that exercise only the removed paths are deleted;
tests that use them as setup are rewritten onto the cooperative production path
(`tests/store/cooperative_checkin.rs` is the model). Schema tables used only by removed paths are
dropped in the same v10 migration as B1; tables shared with the cooperative path are kept. **Which
tables those are is decided mechanically too** (amended by design roast round 2: SQL table names are
string literals the compiler never sees, so the compile check cannot decide it). After the code
deletion, a table is dropped only when `rg -w <table> src/` finds it nowhere except `migrations/` and
the schema verification lists in `src/store/schema.rs` (`required_tables` and its siblings, which the
same change edits). A table any kept statement still names is kept. The trust-model carve-out applies
unchanged: tables used only by the folded P10 / W5-1 paths follow the same rule, and if ht-p03.2's
completion report records one of those deletions as unsafe, the tables its kept code reads stay;
tables the kept cooperative and B5-frozen continuity paths read (for example `recovery_holds`) are
kept.

**Decision 2 — port shape after collapse.** Options: (a) keep traits and only delete methods (rejected:
leaves the ~250-line forwarding traits and Unsupported defaults that let fakes diverge); (b) remove
ports entirely and test against concrete types (rejected: `HostPort` and the clock are genuine seams
for Herdr-down and time tests); (c) **keep a trait only where a production-relevant fake exists**:
`HostPort` (fake host), `Clock`, `NotificationPort`, `LocalClient/LocalService`. `StorePort` loses
every default body (no `Unsupported` defaults — a method is either implemented by `SqliteStore` or
gone) and the duplicate `_admitted` entry points fold into one. `DeadlinePort`'s blanket impl and
fake-friendly defaults go; scheduler tests use `SqliteStore` on a temp root (cheap, and it is what
production runs). The forwarding traits in `identity/reconcile.rs:24` become direct calls. Concrete
`FairWriter` and `LiveServiceGate` move out of `ports.rs` into `service/` and are referenced by type.
Chosen: (c).

**Decision 3 — `ApiError` literals (`results.rs:150`).** Add constructors per `ErrorCode`
(`ApiError::not_found(..)`, `::conflict(..)`, ...) carrying B3's class, and convert the ~167 literal
sites mechanically. Done here because B3's taxonomy needs a single construction point.

**Decision 4 — docs.** The root design and the store, identity, harness and caller-attribution
designs gain a "Cooperative reality (2026-10)" section and have adversarial-verification text marked
superseded (not deleted — history stays readable); rank/self/alias are documented (W6-R5); the INDEX
statuses move from `draft` to `implemented` for designs that shipped. The "local compositions" error
is reworded to name Skill (Wave 20).

**Closes:** all 9 B4 findings, plus the folded B5 items P10 and W5-1 (amended: run.md
amendment-2026-10-01).

**Acceptance.** `cargo build` with no dead-code allowances added; `rg 'allocate_seat|revoke_registration|
CallerVerifier|MutationPermit::new|decision_fence|ProvenEmptyShell|proven_empty_shell_bridge|MarkOccupantUnavailable'`
returns nothing in `src/` and `tests/` (the last four per the amendment, unless ht-p03.2's completion
report records that deletion as unsafe); for each table the v10 migration drops, `rg -w <table> src/`
finds nothing outside `migrations/` and the schema verification lists (design roast round 2; the
dropped-table list is recorded in ht-p03.2's completion report); full suite green.

## B6. Optimistic harness admission and the version canary

**Design move.** One source of truth for harness versions, optimistic admission of anything newer
than it, honest labels everywhere the version is shown, runtime re-observation, and a scheduled
canary that finds the first breaking version so an adapter recipe can be added for that range.

**Decision 1 — source of truth.** Options: (a) a TOML/JSON registry loaded at runtime (rejected: a
runtime file can drift from the compiled adapters); (b) **the compiled recipe tables
(`harness/claude.rs::RECIPES`, `harness/codex.rs::RECIPES`) stay authoritative**, gaining a per-version
evidence level (`live` — native receipt run; `no_model` — canary tier 0 or schema capture only;
`none`), and a checked-in `docs/compatibility/harness-versions.json` is generated from them and
guarded by a test that fails when the two differ. Chosen: (b). `COOPERATIVE_RECEIPT_LINE`
(`health.rs:131`), the setup sandbox constants and the docs' version lists are derived from the same
tables (closes Wave 16 and the hard-coded strings). The canary script reads the JSON.

**Decision 2 — admission ladder.** Options: (a) keep refusing unlisted versions (rejected by owner);
(b) admit everything including older versions (rejected: older-than-supported versions predate the hook
contracts we rely on and are known not to work); (c) **admit newer, refuse older**. Chosen: (c), with
this ladder, applied identically to both harnesses. **Rows are evaluated top to bottom and the first
match wins** (amended by design roast round 1: the order, the in-span row and the assumed-recipe
definition were previously unstated). `known_broken` is checked before every admitting row, so a
version inside a broken range is refused even when it is also listed or newer than every max. Every
version reaches exactly one row: after rows 1–5, a parsed version that is neither listed nor older than
every min is unlisted, either inside the supported span or above it, and row 6 admits it — consistent
with the rule that only older-than-supported and unparsable versions are refused (plus `known_broken`).

| # | Observed version | Admission | Health/doctor label |
|---|---|---|---|
| 1 | unparsable | `Refused` | existing message naming supported recipes |
| 2 | inside any recipe's `known_broken` range | `Refused` | names the broken range and the newest working version |
| 3 | in a recipe | `Listed` | `claude 2.1.286: listed recipe <id> (evidence: live)` |
| 4 | Codex, unlisted, **not older than every recipe's min**, hook-schema fingerprint matches a recipe (qualifier added by design roast round 2: Codex 0.155.1 embeds byte-identical schemas but is below the min 0.157.1, so without it row 4 would admit an older-than-supported version that row 5 must refuse) | `SchemaMatched` | existing `schema-matched, live-unverified` |
| 5 | older than every recipe's min | `Refused` | existing message naming supported recipes |
| 6a | unlisted, newer than every recipe's max (Codex: fingerprint differs or unreadable) | `Optimistic` | `claude 2.1.301: optimistic — newer than verified 2.1.286, assumed compatible with recipe <id>; run doctor after issues, report at <issues URL>` |
| 6b | unlisted, inside the supported span (not older than every recipe's min and not newer than every recipe's max: a gap between recipes or inside a recipe's version set — e.g. Codex 0.157.5 against `VersionSet::Exact(&[0.157.1, 0.158.0])`; Codex: fingerprint differs or unreadable) | `Optimistic` | `codex 0.157.5: optimistic — unlisted within the supported span, assumed compatible with recipe <id>; run doctor after issues, report at <issues URL>` |

**Assumed recipe `<id>`** (rows 6a/6b; 6b restated by design roast round 2, because the earlier "newest
covered version ≤ the observed version" selected no recipe for the 6b example above):
- 6a: the recipe with the greatest max.
- 6b: the recipe whose span `[min, max]` contains the observed version (Codex 0.157.5 → the
  `[0.157.1, 0.158.0]` recipe); if the version falls in a gap between recipes, the nearest recipe below
  it, i.e. the one with the greatest min ≤ the observed version.

The `major version change` flag compares the observed major version with that recipe's max.

`Optimistic` maps to `HarnessStatus::Cooperative { live_unverified: true }` — never `Supported` — and
Health shows it as an informational note, not as `degraded` (it is not a failure), and doctor as a warning
check. A major-version bump is still admitted optimistically but the label says `major version
change`. Recipes gain an optional `known_broken: VersionSet` field, filled from canary results until an
adapter exists. Hook entrypoints already fail open; under `Optimistic` a hook parse failure is logged
(B3) and counted in Health so the operator sees "N hook payloads not understood" instead of silence.
The latent Wave 26 bug (`codex_status` would report Supported for any recipe declaring native receipt)
is closed by requiring native receipt **and** `Listed` admission for Supported.

**Decision 3 — re-observation (Wave 25).** The daemon re-runs admission when the resolved harness
binary's path, inode, size or mtime changes, checked on the existing admission tick; the cached result
is replaced atomically and the change logged. Doctor additionally checks the `claude` on PATH
(Wave 26), mirroring the Codex check.

**Decision 4 — the canary.** `scripts/harness-canary.sh` is the single implementation; the workflow
only calls it.

- *Inputs:* `--harness claude|codex|both`, `--versions latest|since-verified|<list>`, `--bisect`,
  `--model-tier auto|off|required`, `--out <dir>`. It needs a built `herdr-threads` (or builds one).
- *Install:* each version is installed with `npm install --prefix <tmp>/<harness>-<ver>` from
  `@anthropic-ai/claude-code` / `@openai/codex`, run with a throwaway `HOME`, `CLAUDE_CONFIG_DIR`,
  `CODEX_HOME` and `XDG_*` under `<tmp>`. It never reads or writes the real home, `~/.claude`,
  `~/.codex` or aisw profiles, and never contacts the shared Herdr server.
- *Tier 0 (no model, always):* `--version` parses and admission classifies as expected; `setup` installs
  and `unsetup` removes cleanly on a fresh isolated home (closes Wave 25 "no fresh-install 0.159.3 setup
  test" for every version tested); the harness loads the written config without error using a
  model-free command recorded per harness in the script; Codex hook-schema fingerprint extraction
  (`codex_schema.rs`) succeeds and is reported as match/drift; the launch flags `herdr-threads launch`
  passes appear in the harness's `--help`.
- *Tier 1 (model, when `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` are present, or `required`):* a one-turn
  headless run (`claude -p`, `codex exec`) with capture hooks (a tiny script that writes stdin to the
  capture dir) for SessionStart and Bash PreToolUse, plus a nonce injected through `additionalContext`
  that the model must echo. A gated integration test (`tests/harness/canary_payloads.rs`, active only
  when `HT_CANARY_CAPTURE_DIR` is set) parses every captured payload through the production parsers.
  No product-surface change is needed for capture.
- *Bisect:* candidate list = stable `X.Y.Z` versions from `npm view <pkg> versions --json` newer than
  the verified max. Test the newest; if it passes, report all-pass. If it fails, binary-search the list
  for the first failing version, then confirm that version fails and its predecessor passes. Each
  probe is retried once (tier 1 twice) before counting as a failure, and a non-monotone result is
  reported as `inconclusive` with every probe's outcome rather than guessed.
- *Output:* `canary-report.json` + `summary.md` with per-version per-check results, `first_bad`, the
  failing checks, and a suggested action: "add recipe for `>= first_bad`" (an adapter) or "mark
  `[first_bad, …)` known_broken". Exit 0 on all-pass, 1 on a located break, 2 on infrastructure error.
  (Amended: canary nested spec D3/D7/D8 — the tier-0 "model-free command" is realized as
  `t0.config-load` + `t0.hook-fires` + `t0.payload-parse`; `inconclusive` means an *observed*
  contradiction (unprobed non-monotone regions are reported as `assumes_monotone`, not detected) and
  exits 1.)

Options considered for the CI side: (a) Renovate/Dependabot-style bump PRs (rejected: they do not
bisect and need write access); (b) **a scheduled workflow** `.github/workflows/harness-canary.yml`,
daily cron plus `workflow_dispatch`, on `ubuntu-24.04`, `permissions: {contents: read, issues: write}`,
SHA-pinned actions (B7). It runs the script with `--harness both --versions since-verified --bisect
--model-tier auto`, uploads the report, and on exit 1 opens or updates one issue per
`(harness, first_bad)` (deduplicated by title). Chosen: (b). Adding the adapter itself is a human or
agent follow-up driven by that issue — automating code changes from CI is out of scope. (Amended:
canary nested spec D9/D10 — an `inconclusive` result also files one issue under its own title, a
`pull_request` trigger runs the offline self-test job only, and an open issue gets a comment only when
the report digest changed.)

*Liveness of the schedule* (amended by design roast round 1). GitHub disables scheduled workflows in a
public repository after 60 days without repository activity, and a disabled canary looks exactly like a
quietly passing one. The canary docs (`docs/compatibility/harnesses.md`) and the release checklist
(`docs/release.md`, §Follow-on item 2) state the rule and the check: `gh workflow view
harness-canary.yml` shows the workflow `active` with a scheduled run in the last few days, and `gh
workflow enable harness-canary.yml` re-enables it. An automated keepalive is deferred (§Explicit
deferrals).

**Decision 5 — setup and launch correctness (the remaining members).** Each gets the minimal fix that
makes the behavior match its doc, with a test:
- Wave 17: setup never widens an existing `features.network_proxy` allowance — it records the
  pre-existing keys and doctor warns; launch checks the `CLAUDE_CONFIG_DIR`/`CODEX_HOME` it will hand
  the pane, not the launcher's; `codex_install` writes config and hooks to temp files and renames
  config last, rolling back hooks on failure; the quiet gate is evaluated before parse; unsetup after
  plugin uninstall and symlinked dotfiles (edited at the link target) are documented.
- Wave 25: the legacy `~/.local` state dir is chosen only when it holds a store and the XDG dir does
  not, and doctor says which and why; `HERDR_SESSION` handling is consistent and the fast path
  checks that the plugin is installed.
- Wave 30: `plan_remove` removes only entries whose fingerprint the manifest recorded; the v2-manifest
  downgrade refusal stays (safe) and is documented in release notes (B7).
- Wave 20: the hook hint fires on SessionStart only, matching the docs.
- P1: setup adds `--no-daemon` once and only where absent. P2: `codex_layer_paths` stops at the
  project (git) root, matching Codex's own layering, else at the cwd. P29: the moved-binary conflict
  names both paths and the fix.
- W6-C4: managed Codex launch detects an rc-5 early exit inside the launch observation window and
  reports it. W6-R3: a `None` digest still emits the D2 procedure line.
- P19/P20/W6-D5: one context-budget function with a documented trim order and a final fit check;
  long run roots are abbreviated rather than failing S15.
- Wave 21: output mode prefers harness markers (`CLAUDECODE`, Codex env) over `isatty`, so a PTY
  harness gets agent output.
- Wave 31: launch passes the agent name to Herdr so it does not show `None`. Wave 28 (Codex profile):
  launch prints the effective profile/config path; docs answer the question.
- Wave 28 (wake prompt left unsent in the Claude composer): wake delivery verifies submission via the
  pane's agent state and re-sends the submit key once; verified in the B8 rerun — closed only when the
  Wave 28 Claude wake-submission cell PASSes on the final SHA (B8 Decision 3), otherwise it stays open.
- FIX-NOW: `install.md:119` list fixed, launch cache warm-up documented as best effort; the
  `-c`/`-i` refusal (`launch.rs:333`) applies only to options before `--`/the first positional.

**Closes:** all 28 B6 findings, the two evidence gaps through the B8 rerun (current versions) and
canary tier 0 runs of the older listed versions (evidence level recorded honestly as `no_model` where
no live run exists). The current-version evidence gaps close only when their backing cells — the four
core-flow cells (Claude manual and managed on the installed Claude, Codex manual and managed on the
installed Codex) — PASS on the final SHA with the recorded harness versions (B8 Decision 3); a
NOT_EXERCISED or stale backing cell leaves the gap open.

**Acceptance.** Admission ladder table-driven tests for both harnesses, including the first-match order
(a `known_broken` version that is also listed, or newer than every max, is Refused; a Codex version
older than every min whose fingerprint matches — 0.155.1 — is Refused, not SchemaMatched), the in-span
row 6b and the assumed-recipe choice (0.157.5 → the recipe whose span contains it; a gap version → the
nearest recipe below); the JSON-vs-recipes guard test;
`scripts/harness-canary.sh --harness both --versions latest --model-tier off` passes locally; a
bisect self-test (`scripts/harness-canary.sh --self-test`) using a stub installer with a planted break
proves the bisect finds it and reports `inconclusive` on a planted non-monotone sequence.

## B7. Release readiness for v0.1.0

**Design move.** Make the release path rehearsable and truthful *before* the tag: every target
compiles in CI on every PR, the release workflow publishes only from a pushed tag and can be re-run,
actions are SHA-pinned, and the installer's final status and hints are computed from what actually
happened. The tag push and the live clean-machine release test are post-merge follow-on (human).

**Decision 1 — platforms.** Options: (a) keep `platforms = ["macos"]` and stop building Linux
(rejected: Linux musl builds already exist and Herdr runs on Linux; dropping them loses users for no
gain); (b) **declare `["macos", "linux"]`** and prove Linux in CI. Chosen: (b). If Herdr nevertheless
refuses the link, the installer reports it truthfully (Decision 4), and README states the Linux link is
unverified until the follow-on rehearsal passes.

**Decision 2 — never-compiled targets.** `ci.yml` gains a `build-targets` job compiling all four
release targets (`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-musl`,
`aarch64-unknown-linux-musl`) with `--locked --release` on the same runners the release uses, plus
`clippy` once with default features and once with `--all-features` (P25). In-run proof is local: the
Intel macOS target is built on the owner's machine, and the musl targets through the repo's
`scripts/package-release.sh` path in a disposable Linux container if one is available; otherwise the
first CI run after merge is the proof and the spec records that as a follow-on check.

**Decision 3 — release workflow (`release.yml`).** Options: (a) keep dispatch-creates-release with
guards (rejected: two triggers with different semantics is the bug); (b) **tag push is the only
publishing trigger**; `workflow_dispatch` builds and uploads workflow artifacts only. Chosen: (b).
Publishing is idempotent: create the release as a draft if absent (`gh release view` first), upload
assets with `--clobber`, verify checksums of every uploaded asset, then flip draft → published as the
last step, so a re-run after a partial failure converges. Every `uses:` in both workflows (and the B6
canary) is pinned to a full commit SHA with the tag in a trailing comment; job-level permissions
narrow `contents: write` to the publish job only. `actionlint` runs in CI.

**Decision 4 — installer truthfulness (`scripts/install.sh`).** Options: (a) patch each message
(rejected: the inconsistencies come from hints being printed at many points); (b) **record outcomes
and print one final status and one `next_steps()` block computed from them**. Chosen: (b). Specifics:
- Herdr refusing the link (or `--no-herdr`) ends with exit 3 "installed, not linked: <reason>" and the
  exact register command, never exit 0 with success text.
- Upgrade with Herdr down stops the old daemon with the binary's own `herdr-threads daemon stop` (no
  Herdr needed) before replacing it; the misleading message at :385 is replaced.
- Uninstall without `herdr` on PATH prints the exact unregister command and exits 3 (partial).
  Uninstall prompts before `unsetup` on a TTY; non-interactive runs require `--yes` and otherwise skip
  unsetup and print the command.
- `herdr_action` stops splitting JSON on `{`: it uses Herdr's exit status plus a fixed-string match on
  the documented field, and where a value must be extracted it uses the just-installed binary's hidden
  `herdr-threads internal json-field <path>` helper (no `jq`/`python` dependency for curl|bash).
- Replacement is described as "crash-safe" (rename old aside, rename new in, restore on failure),
  not "atomic", in code comments and docs.
- Setup reminders are correct when no harness is found or one is declined.
- `tests/release/install_test.sh` gains a partial-setup-failure case and a Herdr-down upgrade case run
  against an isolated named Herdr test session (never the shared server).

**Decision 5 — docs and notes.** `docs/release.md` drops the false "unchanged since d635aca" claim and
gains a release checklist (the follow-on steps below). A `CHANGELOG.md` `v0.1.0` entry records the
removed STATUS constants, the `inv-` prefix, the manifest v2 downgrade refusal, optimistic admission
(B6) and the platforms list. README "Try it" and demo text are refreshed against current output.

**Closes:** all 17 B7 findings in code/docs; the "never compiled" gap is closed by CI on the first
post-merge PR/branch run if not provable locally (recorded as a follow-on check).

**Acceptance.** `actionlint` clean; `rg 'uses: [^@]+@v[0-9]'` finds nothing in `.github/`; installer
tests green including the new cases; local Intel macOS build succeeds.

## B8. Validator hardening, review debt and the native rerun

**Design move.** Trust the validator only as far as it is tested, review the code that was reviewed
least, and stamp the native evidence on one SHA — the integration tip after every other bucket lands.

**Decision 1 — validator false-PASS hardening (`scripts/validate-native-demo.py`,
`scripts/reconcile-validation.py`).** Options: (a) fix each path ad hoc (rejected: the next wrapper
form slips through again); (b) **a negative corpus**: one synthetic transcript fixture per known
false-PASS/false-FAIL path, each asserting the exact verdict, run in CI with the Python suites (B9).
Chosen: (b), plus two structural fixes. A call counts as model-issued only when it is a top-level
tool call of the root model whose argv, after peeling wrappers (`env`, `sudo`, `exec`, `command`,
`nohup`, `timeout`, `sh -c`/`bash -c`, backticks and `$()`), is a direct `herdr-threads` invocation and
not inside a nested `claude -p` / `codex exec` (P15, P31). Children and readers are counted by stable
identity (session/agent id), not by spawn events or per-read lines (W10-E6, W10-E7); SL1 checks every
expected sequence number (W10-E1). The six label/false-FAIL items (W6-D3, W6-D4, W10-E2, W10-E3,
W10-E5, W10-E8) and W9-5 (R13 asserts the Unsafe row) get a fixture each. The FIX-NOW quoted-literal
false positive gets a fixture; the `native.rs` incarnation-recheck test is rewritten so the fake host
actually changes server incarnation between observations.

**Decision 2 — review debt.** One review leaf, not a re-review of everything. Its scope is fixed:
(1) the regression-pass fixes never re-roasted; (2) the light-review lanes — user-level setup, short
IDs, token diet, human output, release/installer, `read --follow` — limited to code not already
rewritten by B1–B7/B10 in this run (rewritten code is reviewed by this run's own code review);
(3) re-check of the 17 DONE_WITH_CONCERNS reports; (4) enumerate P41–P44 parked items from tag
`archive/herdr-threads-run-2026-09-26` and triage each into a bucket fix, a filed `bd` issue or an
explicit accept; (5) P45: the human goal-versus-tree read-through is performed by the review leaf as a
written goal-vs-tree table and flagged for the owner's read at finish. Findings at Should-fix or above
are fixed in-run; below that, filed as `bd` issues. The two remainder-capped roast candidates were
never recorded; they are closed as unrecoverable unless the archive tag contains them.

**Decision 3 — the native rerun leaf (G0).** Options: (a) rerun per bucket (rejected: evidence on a
moving SHA is exactly G0); (b) **one late leaf on the integration tip**, after all code buckets and the
validator fixes, using the hardened validator. Chosen: (b). It runs the full matrix in
"Configurations to exercise" in an isolated named Herdr test session, with the default Codex profile
and `validate-native-demo.sh`'s existing isolation (setup into run-root copies of `HOME`,
`CLAUDE_CONFIG_DIR` and `CODEX_HOME`; real credentials read, never written), and regenerates `docs/validation/report.md` and the sweep statuses from
the results (closes Wave 15 precision items and Wave 16 R20/R23 status consistency — R20 should become
MET after B2). A cell that cannot run (e.g., no model access) is reported `NOT_EXERCISED` with the
reason, never PASS.

*Evidence provenance* (amended by design roast round 1; replaces the earlier "the affected cells rerun
on the new tip; the report records the final SHA", which left "affected" undefined and stamped one SHA
on cells that never ran on it):
- **Per-cell record.** Every cell records, in `docs/validation/report.md`, the tip SHA it ran on, the
  output of `claude --version` / `codex --version` taken immediately before and after the cell, and its
  outcome. Both harnesses run from a **fixed versioned path** (amended by design roast round 2:
  `DISABLE_AUTOUPDATER=1` is per-process, and the shared launcher `~/.local/bin/claude` can be
  repointed by any other Claude Code process mid-matrix). Claude: the run resolves
  `~/.local/share/claude/versions/<v>` once at run start and exposes it as `claude` on a run-root PATH
  entry ahead of `~/.local/bin`, with `DISABLE_AUTOUPDATER=1` kept in the run-root settings `env`.
  Codex: a fixed install path, as before. A cell whose before/after versions differ is invalid and
  reruns. Every cell of the final matrix reports the same version per harness, and that version equals
  the installed version's committed recipe row.
- **One final SHA.** Any commit after a cell ran (a fix for a failing cell, the recipe-row commit, any
  other change except an evidence-only commit, below) makes every cell that did not run on the final SHA stale. The **full matrix** reruns on
  the final SHA; the report's evidence SHA is the one SHA every reported cell ran on. Commits that are
  known before the run (the recipe rows for the installed versions) land before the final matrix run,
  so the cells run under the admission they certify.
- **Evidence-only commits** (amended by design roast round 2; without this the leaf's own report commit
  would stale every cell). A commit that touches only evidence paths — `docs/validation/**` (report.md,
  integration-sweep.md and the sweep statuses) and this run's directory (the findings closure ledger,
  run evidence) — does not stale any cell. A change to any other path (`src/`, `scripts/`, `tests/`,
  the recipe tables, `docs/compatibility/**`, workflows) forces the full-matrix rerun. The evidence SHA
  is the code SHA the cells ran on, which is the parent of the first evidence commit. A mechanical check
  enforces it: `git diff --name-only <evidence SHA>..HEAD` lists only evidence paths.
- **Pre-landed recipe rows** (amended by design roast round 2). The installed versions' Listed rows are
  committed at evidence level `live`, and the compiled `RECIPES` and the generated
  `harness-versions.json` change in the same commit. A row may stay only if all of its backing cells
  PASS on the evidence SHA. If any backing cell is FAIL or NOT_EXERCISED, the row is removed. That
  removal is one commit, which triggers one more full-matrix rerun under Optimistic/SchemaMatched
  admission, and the row is not re-added in this run. So no shipped row claims `live` evidence it lacks,
  and the row loop ends after at most one removal per harness.
- **Retries and convergence** (amended by design roast round 2). A cell that FAILs may be retried on
  the same SHA up to two more times (three attempts in all; a retry is not a commit). A PASS after a
  failed attempt is recorded as PASS (flaky) with its attempt count, and its closures hold. A cell that
  fails all three attempts needs a fix commit, which starts a new full-matrix run. The leaf runs at
  most **three full-matrix runs**. At that cap it exits: the report is written on the last full-matrix
  SHA, cells that still FAIL are recorded as FAIL, their closures stay open in the findings closure
  ledger, and the leaf's completion report names them. FAIL is a reportable end state only at the cap.
- **Closures bind to backing cells.** Each closure that depends on the rerun names its backing cells
  and holds only if every one of them PASSes on the final SHA; a NOT_EXERCISED or stale backing cell
  leaves its finding or R-item open (recorded as open in the findings closure ledger, never closed).
  Backing cells: R20 MET (W9-1) ← SW2 coalesced warning wake (Claude, Codex) and the Wave 28
  wake-submission cell; Wave 28 ← the Wave 28 Claude wake-submission cell; B6 current-version evidence
  gaps and the installed versions' Listed recipe rows ← the four core-flow cells on the installed
  versions; ht-910 ← ht-910 daemon restart (Claude, Codex); P40 ← the P40 crash-fix3 cell; the three
  B3 NOT_EXERCISED cells ← concurrent children (Claude, Codex), SW2 coalesced warning wake (Claude,
  Codex), Codex TUI children write-absence; G0/B2 ← every cell.

**Closes** (each rerun-dependent item subject to its backing cells, above)**:** G0/B2, the three B3
NOT_EXERCISED cells, ht-910, P40, the validator false-PASS (5) and label (6) items, W9-5, FIX-NOW (2),
Wave 15 (7), Wave 16 (2), process debt (10).

**Deferred:** the token-overhead benchmark — the rerun records per-turn injected-context bytes per
harness as one measured number, but a benchmark harness is new tooling outside this run.

**Acceptance.** Validator negative corpus green in CI; every matrix cell PASS or explicitly
NOT_EXERCISED-with-reason on one recorded SHA (or, only when the three-run cap is reached, FAIL recorded
with its closures open), each cell's row carrying that SHA, its before/after harness versions (one
version per harness across the matrix) and its outcome, plus its attempt count when retried (no stale
cell in the report: `git diff --name-only <evidence SHA>..HEAD` lists only evidence paths); every
rerun-dependent closure lists its backing cells and is marked closed only if they all PASS; a
pre-landed recipe row survives only with all its backing cells PASS; review leaf's findings table
attached to the run.

## B9. Test isolation

**Design move.** One isolation pass so plain `cargo test` (default parallel harness) passes
repeatedly, and the CI `--test-threads=1` pin (`ci.yml:108`) is dropped — not confined to a named
group.

**Decision 1 — `hook_entrypoint` exit 101 in parallel (ht-4is.11.8, P12, P13).** It was waived, never
root-caused. Approach: reproduce first (`cargo test --test hook_entrypoint` under the default harness
in a loop, and with `--test-threads=16`), capture the panic message and backtrace (exit 101 is a Rust
panic), then classify. Candidate causes to check in order: a shared fixed path (state root, socket,
lock or capture file) between tests; environment inherited from the invoking shell (`HERDR_*`,
`CLAUDE*`, `CODEX*` — the suite must run identically inside a Herdr pane); the `Race` mutex in
`tests/hook_entrypoint.rs:55` being poisoned by one test and unwrapped by another; a real race in the
production hook path. If the root cause is in production code, it is fixed there with a regression
test; a test-only fix is accepted only when the evidence shows the race cannot occur in production.
No fix that claims a root cause lands without the reproduced panic message recorded in the commit.

*If it does not reproduce* (amended by design roast round 1; plausible after B4's deletions, which
land first). The attempt budget is bounded: 200 runs of `cargo test --test hook_entrypoint` under the
default harness plus 200 with `--test-threads=16`, both inside a Herdr pane environment and with a
scrubbed one, on the current tip; if none reproduces, the same budget on the pre-B4 base
(`main@cf8c83d7`). A reproduction only on the old base is classified against the B4 diff and recorded
with the deletion that removed it. With no reproduction anywhere, the leaf (1) records the attempt
counts, environments and SHAs in the commit and in the Post-Implementation Notes, (2) audits the four
candidate causes above statically and fixes any it finds with a regression test, and (3) records the
outcome as "not reproduced; mitigated by Decision 2 isolation" — never as root-caused. The serial pin
is still removed only through the 10-consecutive-run acceptance below, which governs either way.

**Decision 2 — shared infrastructure.** Every test gets a private state root from one helper in
`src/test_support` (temp dir per test, never a fixed path, and a clean env with `HERDR_*`/harness vars
removed for spawned binaries). The process-global `UNITS` counter (`tests/store/attention.rs:277`) is
replaced by a counter carried in a per-test context passed to the instrumented code (thread-local is
rejected: the store runs work on other threads). Socket path length (P37) is kept under the Unix limit
by a short temp base rather than depending on fixture path length.

**Decision 3 — time-based assertions.** Options: (a) widen sleeps/slack (rejected: still flaky, just
slower); (b) **deterministic synchronization**: the fake clock already in `ports` for logic, explicit
barriers/notifications (B2's `Pacer` exposes a test hook "lane went idle") for "nothing happened"
assertions, and bounded polling with a generous deadline only for real-process tests. Chosen: (b).
Applies to the five named flakes (`concurrent_publishers_receive_distinct_ordered_keys`,
`async_accept_listener_keeps_owner_lock_until_server_stops`, `pending_invitation_axis_is_flat`,
`digest_cost_is_flat…`, host-seats service timing), `host_adapter.rs:487`, `resolution.rs:2001`,
and the sweep's fixed 2 s late-ACK sleep (Wave 16, also renaming "rejected").

**Decision 4 — missing and weak tests.** Add: recipients backfill (`attention.rs:481`), publish-time
archived recheck (`messages.rs:521`), wake suppression and hook-path takeover (Wave 18), "healthy" in
cooperative setup sweep (Wave 26), latency test asserting prompts > 0 with stderr drained (Wave 28).
Strengthen `receipts.rs:1455` to assert the error code. Deduplicate the payload pasted seven times in
`hook_entrypoint.rs:1015` into one fixture builder. Python demo/validator suites run in CI
(`ci.yml:105`), alongside B8's negative corpus.

**Closes:** all 15 B9 findings.

**Acceptance.** `cargo test --locked --all-targets --all-features` (no thread pin) passes 10
consecutive runs locally (in-run) and on CI's two OSes (post-merge, §Follow-on item 6); the serial pin is removed from `ci.yml`; the
`hook_entrypoint` root cause — or, when the bounded attempts did not reproduce it, the recorded
"not reproduced; mitigated" outcome with its attempt counts and static audit — is written in the
Post-Implementation Notes.

## B10. Presentation contract and shared escaping

**Design move.** One presentation contract per result type, and one escaping function used by every
output path.

**Decision 1 — contract form.** Options: (a) a prose doc only (rejected: it drifted once already);
(b) a declarative schema/DSL for renderers (rejected: heavy for ~15 result types); (c) **a
per-result-type table in `docs/agent-usage.md` (must-keep fields and next-step argv for each of
compact/agent and human modes) enforced by golden tests** — one golden file per (result type, mode)
under `tests/cli/golden/`, plus a test that every argv continuation listed in the table appears in the
compact output. Chosen: (c). Agent-facing continuations are the priority: `checked_in` keeps the
warnings continuation and `offered_through`; compact `thread()` keeps `topic_detail_argv`; inbox keeps
topic and invitation id in human mode; `event_row` keeps the dropped fields; the already-joined invite
prints its specific form; timestamps older than today carry a date (`MM-DD HH:MMZ`).

**Decision 2 — escaping.** Options: (a) per-page escaping as today (rejected: Wave 31); (b) **one
`escape_for_terminal(&str, Context)`** in `src/view` covering C0/C1, DEL, bidi controls (LRM, RLM,
LRE..PDF, LRI..PDI, ALM), and all Unicode `Cf` format characters (zero-width etc.), rendering them as
visible `\u{…}` escapes; `Context` only selects whether newlines are allowed (multi-line bodies) or
escaped (single-line fields, argv, follow notices). `is_unsafe` is replaced by it. Column alignment
uses display width (East Asian wide = 2) via `unicode-width`. Forged-line protection no longer
relies on irc indentation: every body line is prefixed by the renderer and embedded newlines in
single-line fields are escaped. Chosen: (b).

**Decision 3 — follow and misc.** `follow` treats one slow connect as a retryable `Transient` (B3)
and only prints "lost the daemon" after the backoff budget is exhausted; the fatal notice is printed
once; Ctrl-C is honoured during a call (cancellable await), and NickCache no longer takes
`context.lock` on the render path. The dead `--offset` fallback is removed. `write_selected`'s doc is
corrected. SKILL.md stops overclaiming `--json` and moves the digest example. Pane-name precedence is
documented and `pane_not_found` gives a hint naming the precedence. Autodetect uses a short bounded
probe so a command never waits the full `HERDR_QUERY_TIMEOUT` (Wave 25).

**Closes:** all 15 B10 findings.

**Acceptance.** Golden tests per result type and mode; an escaping table test covering every class
above; a follow test with an injected slow connect.

## Sequencing

1. **B4** first (deletes code others would otherwise touch; owns the v10 migration skeleton and the
   `ApiError` constructors B3 builds on).
2. **B9** isolation helpers and the `hook_entrypoint` root cause next, so every later bucket is
   developed against a parallel-clean suite.
3. **B1, B2, B3** — B2's `Pacer` before B3's rate-limited logging and B1's cleanup lane, which both use
   it; B1 adds its partial indexes to the v10 migration.
4. **B6, B7, B10** — independent of each other; B7's SHA-pinning convention also applies to the B6
   canary workflow.
5. **B8** validator hardening and the review leaf in parallel with step 4; the **native rerun leaf runs
   last**, on the integration tip after every other leaf has merged.

## Configurations to exercise

**Release targets** (B7) — all four must compile with `--locked --release` (CI `build-targets` job;
local proof where possible):

| Target | Runner | In-run proof |
|---|---|---|
| `aarch64-apple-darwin` | macos-15 | local build + full test suite |
| `x86_64-apple-darwin` | macos-15 (cross) | local cross build |
| `x86_64-unknown-linux-musl` | ubuntu-24.04 | Linux container if available, else first CI run (follow-on) |
| `aarch64-unknown-linux-musl` | ubuntu-24.04-arm | Linux container if available, else first CI run (follow-on) |

**Harness version set** (B6 canary and B8 rerun):

| Harness | Versions | Exercised by |
|---|---|---|
| Claude Code | 2.1.286 (newest listed; installed today is 2.1.287, unlisted → the rerun adds its recipe row, ht-p03.20, committed at evidence `live` before the final matrix and kept only if its backing cells PASS on the evidence SHA, else removed and not re-added; every cell runs the fixed versioned binary `~/.local/share/claude/versions/2.1.287` with `claude --version` recorded before and after and auto-update disabled — §B8 Decision 3, design roast round 2) | B8 native rerun (live) + canary tier 0 |
| Claude Code | 2.1.283, 2.1.284, 2.1.285 (listed, older) | canary tier 0 (evidence level `no_model` where no live run exists) |
| Claude Code | newest published > 2.1.286 | canary tier 0 → must classify `Optimistic` |
| Codex | 0.159.3 (installed) | B8 native rerun (live) + canary tier 0 |
| Codex | 0.157.1, 0.158.0 (listed), 0.159.2 (schema-matched) | canary tier 0 |
| Codex | newest published > 0.159.3 | canary tier 0 → `SchemaMatched` or `Optimistic` |
| either | a version older than every recipe (including Codex 0.155.1, whose hook-schema fingerprint matches — design roast round 2) | unit test → `Refused` |

Tier 1 (model) canary runs only where API keys exist; locally it is `--model-tier off`, and the B8
rerun supplies live evidence for the installed versions.

**Native matrix** (B8 rerun, one SHA — the full matrix reruns on the final SHA after any later non-evidence commit, at most three full-matrix runs,
each cell recording its SHA, harness versions and outcome, §B8 Decision 3 — isolated named Herdr
session, default Codex profile): Codex
manual, Codex managed, Claude manual, Claude managed (prelaunch flow, one omitted-initial-prompt flow
per harness); plus the owed cells — concurrent children (both), SW2 coalesced warning wake (both),
Codex TUI children write-absence, ht-910 daemon restart with the agent staying in its pane (both),
P40 crash-fix3 real-host acceptance, Codex sandbox run with a non-tmp (XDG default under the run's
`HOME`) state dir covering pane resolution, hook and daemon start inside the sandbox, and the Wave 28
Claude wake-submission check.

**Herdr up/down states** — each against the isolated named Herdr test session, never the shared
server:

| State | Exercised by |
|---|---|
| Herdr up, plugin linked | normal suites, native matrix |
| Herdr stopped while daemon runs | B2 backoff/commit-count test, B3 Health degraded wording |
| Herdr never started (no socket) | B3 "server not running" |
| Stale Herdr socket file, server dead | B3 stale-socket classification |
| Herdr restarted under a running daemon | B8 ht-910 / P40 cells, B2 recovery-to-normal cadence |
| Installer: fresh install, upgrade, uninstall × Herdr up / down / absent from PATH / `--no-herdr` | B7 `install_test.sh` |

## Explicit deferrals

Rows marked **Amended** record a root line superseded by a nested-spec decision or by run.md
amendment-2026-10-01; the root text carries an "(amended: …)" marker at that line.

| Item | Bucket | Reason |
|---|---|---|
| **Amended** — §B1 D1 live predicate "snapshot generations = the current published one plus at most one staged" as a partial index | B1 | Not expressible as a static partial-index predicate (no subqueries/other tables); B1 nested spec D1/D3: pointer joins plus the retention keep set |
| **Amended** — §B1 D1 "`INDEXED BY` the same way `work_jobs_ready` already does" | B1 | Loose wording: `work_jobs_ready` is a plain index; B1 nested spec D1 restates every partial-index predicate literally |
| **Amended** — §B1 D1 live predicates for receipts/ACKs and warnings as new partial indexes | B1 | Existing v1 partial indexes and v8 pending projections already serve them; B1 nested spec D1 adds only `seats_live_ordinal` and `work_jobs_live` |
| **Amended** — §B1 D2 "pruned immediately after a newer generation publishes (keep current + previous)" and the cleanup lane "generalised from the digest pruning path" | B1 | B1 nested spec D3/D4: a fifth Pacer lane on a 60 s tick with an empty kick set prunes outside the nested keep set; a per-publish prune would cost a retention commit every 5 s |
| **Amended** — §B1 D2 "completed work jobs pruned after 24 h" for every kind | B1 | B1 nested spec D4: `preparation_cleanup` completion rows are the operation-key reuse marker and are retained (pruning needs a separate marker or reuse window) |
| **Amended** — §B1 D3 attention rebuild as "a join over unresolved warnings" | B1 | B1 nested spec D2: derived from the v8 pending projections with O(1) per-seat probes; the old scan stays as the test oracle |
| **Amended** — §B1 Closes "work-job discovery uses `work_jobs_ready`" | B1 | B1 nested spec D1: `work_jobs_live`, keeping the ascending-ordinal cursor |
| **Amended** — §B1 Closes Wave 18 "the live wake index excludes human seats" | B1 | B1 nested spec D2: a `NOT EXISTS` probe on `occupant_bindings_current`; "human" lives on the binding, so no index on `seats` can exclude it |
| **Amended** — §B2 D1 commit-kick mechanism and the Retention kick row | B2 | Pacer nested spec D1 (sealed set taken under the writer guard); B1 nested spec D4 (Retention maps no table) |
| **Amended** — §B2 D2 Herdr-down bound | B2 | Pacer nested spec D4: the bound is the observation lane's, with a complete repeat-invalidation skip contract; the wake-lane cost while Herdr is down is the parked round-1 escalation (human) |
| **Amended** — §B2 D3 `SessionLease::drop` fallback "release message processed by the owning task" | B2 | Pacer nested spec D3: cancel the session and hand `revoke_exact` to `spawn_blocking` |
| **Amended** — §B4 D1 known members, §Non-goals and §Hard constraints (frozen B5) | B4 / B5 | run.md amendment-2026-10-01: the B5 owner decided P10 and W5-1 as deletions folded into ht-p03.2 |
| **Amended** — §B6 D4 tier-0 "model-free command", `inconclusive` semantics, issue filing | B6 | Harness-canary nested spec D3/D7/D8/D9/D10 (deviations table there) |
| Automated keepalive for the scheduled canary (GitHub disables scheduled workflows after 60 days without activity in a public repo) | B6 | A keepalive commit needs `contents: write` on a scheduled job; the liveness check and re-enable command are documented in the canary docs and the release checklist instead |
| Skip-unchanged-snapshot / remove the last idle commit (observation admission + generation every 5 s with an unchanged host; identity fencing, adjacent to frozen B5) | B2 | Needs a change to the identity fencing protocol, which B5 owns; B1 bounds generation growth. Surfaces in report Remaining |
| B5 entirely (F6, `me init` guards, trust invariants, etc.) | B5 | Owned by a separate session; this run must not change its semantics |
| Token-overhead benchmark harness | B8 | New tooling; the rerun records one measured number per harness instead |
| Auto-generated adapter PRs from the canary | B6 | Needs write permissions and code synthesis; the canary files an issue with the located range instead |
| Canary tier 1 in CI | B6 | Requires repository API-key secrets the owner must add; the workflow auto-enables when present |
| Tag push, live GitHub release, clean-machine install/upgrade/uninstall rehearsal | B7 | Owner decision: human, post-merge |
| Linux musl compile and Linux Herdr link proof, if no local Linux container | B7 | Needs CI/Linux host; becomes the first follow-on check |
| Pruning settled warnings after 7 days (§B1 Decision 2) | B1 | Deviation approved in the B1 nested spec (D4, deviations table): warnings are retained; coverage r1 recorded it here. (The `work_jobs_live` amendment has its own row above.) |
| User-configurable retention periods | B1 | Fixed defaults suffice for v0.1.0; config surface can follow real usage |
| Review-leaf findings below Should-fix | B8 | Filed as `bd` issues rather than fixed in-run |
| The two remainder-capped roast candidates | B8 | Content was never recorded; closed as unrecoverable unless the archive tag holds them |

## Follow-on (post-merge)

Performed by the human after this run merges; listed in `docs/release.md`'s checklist:

1. Watch the first CI run on `main`: `build-targets` (all four targets), parallel tests, Python suites,
   `actionlint`. Any Linux failure blocks tagging.
2. Add `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` repository secrets if the canary's model tier is wanted;
   trigger `harness-canary.yml` once by `workflow_dispatch` and confirm the report artifact. Keep the
   schedule alive: in a public repository GitHub disables scheduled workflows after 60 days without
   repository activity, so periodically check `gh workflow view harness-canary.yml` (state `active`,
   a recent scheduled run) and re-enable with `gh workflow enable harness-canary.yml` when needed.
3. Push tag `v0.1.0`; confirm the release workflow publishes a draft, uploads and checksums all assets,
   then publishes.
4. Clean-machine rehearsal (macOS, and Linux): curl|bash install, upgrade from the tag's own artifact,
   uninstall — each with Herdr up and down; confirm final statuses and exit codes match the installer
   contract.
5. If the Linux Herdr link is refused, flip `platforms` back or file an upstream Herdr issue and update
   README accordingly.
6. B9's CI half: confirm `cargo test --locked --all-targets --all-features` with no thread pin passes on CI's
   two OSes (macOS and Linux). The in-run acceptance is 10 consecutive local parallel runs (bead ht-p03.26);
   a CI failure here is filed as a test-isolation bug against the merged suite.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*

### 2026-10-01 — hook_entrypoint root cause (ht-p03.8)

Reproduced on the tip (a18a6815), so this is root-caused, not "not reproduced".

- **Reproduction.** `cargo test --test hook_entrypoint` under the default harness exited 101 in about 12 s
  with the libtest summary and every later line missing (the serial run takes about 286 s and passes).
  20 of 20 runs failed before the fix, 5 per cell: default and `--test-threads=16`, each inside a Herdr pane
  environment and under `env -i` (scrubbed), so inherited `HERDR_*`/`CLAUDE*`/`CODEX*` is not the cause. The
  pre-B4 base cf8c83d7 budget was not needed. There is no panic message to quote: the process loses its own
  stdout and stderr (`panicked` never appears even with `--nocapture`, `RUST_BACKTRACE=1`).
- **Root cause (classified: a shared process-global resource, a fifth case beyond the four candidates).**
  The in-process fixture started the daemon through `run_elected_with_diagnostics`. Its `ChildOwnedLogSink`
  dup2()s the process-wide fd 1 and 2 onto a pipe for the daemon's lifetime and restores them on close
  (`src/daemon/mod.rs`, `install`/`restore_output`). Two fixtures overlapping in one test process save each
  other's pipe as the "original", so the last restore leaves fd 1/2 on a dead pipe: libtest's summary and
  every panic message are lost and the process exits 101. Not a production race: production runs the sink
  once, in the dedicated daemon child (`main.rs` -> `run_elected`), never beside another sink.
- **Fix (test-only).** The fixture uses `run_owner_with_factory` with `WriterSink(io::sink())`, which never
  touches the process descriptors. Regression test `overlapping_in_process_daemons_leave_process_stdio_intact`
  (RED before the fix: the run dies with no summary; GREEN after). The stale "quiet panic hook" comment and
  `set_hook` were removed (nothing in `src` installs a hook).
- **Second layer, visible once the summary survived.** Wall-clock hook budgets (`TOOL_BUDGET` 1.5 s,
  `LIFECYCLE_BUDGET` 5 s) failed under parallel load in the same file: `stalled_stdin_*` (2.55 s and 3.4 s
  against a flat 2.5 s ceiling), `twenty_thousand_*` and `*_pending_*` (`UnknownOutcome` or a fail-open
  SessionStart: "no registered execution"), and `installed_*`/`rejection_*` ("installed claude version:
  Unavailable"). All are fail-open exit-0 hooks starved of CPU by the other concurrent fixtures (and a
  shared host at load average 20+). Intermediate fixes (an exclusive gate for scale tests only, a SessionStart
  retry in setup) still failed 4 of 20 runs. Final fix: every test that spawns the hook holds one
  process-wide `serialized()` guard; the 20 tests overlapped for no wall-clock gain (parallel 200-390 s
  against 286 s serial). The stalled-stdin ceiling is also the midpoint of the tool and lifecycle budgets,
  which still fails a watchdog started at 5 s. Acceptance on the final code: 10/10 default and 10/10
  `--test-threads=16`.
- **Static audit of the four candidates.**

  | Candidate | Finding | Action |
  | --- | --- | --- |
  | Shared fixed path | `private_root()` is `/private/tmp/hk-<uuid12>`; no fixed state, socket, lock or capture path | none |
  | Inherited environment | identical 20/20 failure scrubbed and unscrubbed; `run_hook` sets `HERDR_*` explicitly | none |
  | `Race` mutex poisoned | the lock is released before the armed action runs (`Counting::after`); the `Race` is per fixture, so no cross-test poisoning | none |
  | Production race in `src/cli/hook.rs` | none found; the failures reproduce with the hook binary unchanged | none |
- **Payload fixture builder.** The pasted payload literals are replaced by one `Payload` builder
  (`"hook_event_name"` now appears once in `tests/hook_entrypoint.rs`).
