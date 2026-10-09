# Build and lint speed

Budgets (AGENTS.md "Speed budgets"): an incremental build and the per-change lint each stay under
1 minute. This file records how the current settings were chosen and the measurements behind them.

## Summary

| | Before | After |
|---|---|---|
| Per-change check (edit) | check + 2 clippy: 21-32 s | 1 clippy: ~8.3 s |
| Merge check (edit) | check + 2 clippy: 21-32 s | clippy + `scripts/check-default-features`: ~12 s |
| `cargo test --all-features --no-run` (edit), CPU | ~21.7 s | ~15.5 s |
| `cargo build` (edit) | 3.3-3.5 s | 3.0 s |
| Cold build (all targets) | 33.1 s wall, 126 s CPU, 2.0 GB | 29-31 s wall, ~104 s CPU, 1.6 GB |
| Test binaries | 10 | 5 |
| `cargo test --lib --no-run` (edit to `handoff.rs`), test `opt-level = 1` (item 7) | 54-150 s wall, ~400 s CPU | 28-64 s wall, 66-76 s CPU |

Edit-and-rebuild wall time was already well under budget and is now mostly rustc's front end over
the one crate. The changes cut duplicated work, CPU and disk (a busy machine with several
worktrees is the common case here), not the critical path. Item 6 (crate split) is the only lever
left on that, and it is not worth its cost yet.

## How it was measured

- Machine: Apple M2 Max, 12 cores, 96 GB. Rust 1.94.0. Numbers marked with a load figure were
  taken with other worktrees building on the same machine; compare rows taken at similar load.
- **Edit**: insert (or remove) a comment line at the top of `src/store/queries.rs` (4.5k lines),
  then run the command. That shifts every span in the file, so it re-runs codegen for the file's
  functions; a bare `touch` lets rustc's incremental cache skip nearly everything and understates
  a real edit. Three timed runs after one warm-up run.
- **Cold**: an empty `CARGO_TARGET_DIR`, `cargo build --locked --all-targets --all-features`
  (every dependency, the library, its unit tests, the binary and every test target).

## Baseline (before any change; load ~5)

| Command | Edit, s (3 runs) |
|---|---|
| `cargo check --all-targets --all-features` | 3.6 3.7 3.5 |
| `cargo clippy --all-targets --all-features -D warnings` | 8.3 8.3 8.4 |
| `cargo clippy --all-targets -D warnings` (default features) | 8.5 8.1 8.3 |
| `cargo check --all-targets` (default features) | 3.3 3.3 3.3 |
| `cargo build` | 3.5 3.4 3.3 |
| `cargo test --all-features --no-run` | 6.0 6.0 6.0 |
| merge check: check + clippy all-features + clippy default (load ~7) | 20.8 32.0 27.0 |

Cold build: 33.1 s wall, 126 s CPU (user), target dir 2.0 GB.

`cargo build --timings` (cold): the critical path is the library compiled with its unit tests
(`lib (test)`, 44 s of a 53 s build at load ~25). The ten test targets start only when the plain
library finishes and build in parallel (2-7 s each, mostly linking), finishing before `lib (test)`.
On an edit the shape is the same: `lib (test)` 5.3 s, the test targets 1.5-2 s each in parallel.

## Changes

### 1. No separate `cargo check`

`cargo clippy` runs the full type check of every target it lints, so the `cargo check` in front of
it repeated work. The per-change check is now one command:
`cargo clippy --locked --all-targets --all-features -- -D warnings`.

| Sequence (edit) | s |
|---|---|
| before: check + clippy all-features + clippy default (load ~7) | 20.8 32.0 27.0 |
| clippy all-features + clippy default (load ~5) | 16.5 17.0 16.5 |
| after, per change: clippy all-features (load ~5) | 8.3 8.3 8.4 |

### 2. Default-feature guard: `scripts/check-default-features`

Release builds use the default feature set; the per-change lint uses `--all-features`. The
default-feature lint exists for two breakages only the default set shows:

- a test target that uses `herdr_threads::test_support` without
  `required-features = ["test-support"]` (a compile error), and
- items only the `test-support` surface uses, left unused in a default build (a warning; the
  integration sweep hit two of these).

`cargo check --all-targets` catches the first but only warns on the second. Cargo's
`build.warnings = "deny"` is unstable on 1.94 (ignored without `-Z`), and `RUSTFLAGS=-Dwarnings`
would rebuild every dependency whenever it toggles. So the script runs the default-feature
`cargo check` and fails when cargo reports any rustc warning (cargo replays cached warnings, so an
up-to-date build still fails). Both breakages were probed: an unused `#[cfg(not(feature =
"test-support"))]` item and a test file importing `test_support` with no `[[test]]` entry each
fail the script, on a fresh and on a cached build.

What it gives up: clippy lints on code that compiles only without the feature (three
`cfg(not(... feature = "test-support"))` sites in `src/`). CI keeps the full default-feature clippy
job (`.github/workflows/ci.yml`, asserted by `tests/release/workflows_test.sh`), so both feature
sets are still linted before anything lands on main.

| Command (edit, load ~5) | s |
|---|---|
| `cargo clippy --all-targets -D warnings` (default features) | 8.2 8.2 8.0 |
| `scripts/check-default-features` | 3.4 3.3 3.3 |
| merge check before: check + clippy all-features + clippy default (load ~7) | 20.8 32.0 27.0 |
| merge check after: clippy all-features + `scripts/check-default-features` (load ~6) | 11.6 12.2 12.2 |

### 3. Dev profile debug info

`[profile.dev] debug = "line-tables-only"`, `[profile.dev.package."*"] debug = false` (the test
profile inherits dev). Release is unchanged.

| Measure (load ~5) | Before (full debug info) | After |
|---|---|---|
| Cold build, wall | 33.1 s | 31.0 s |
| Cold build, CPU (user) | 126 s | 109 s |
| Cold target dir | 2.0 GB | 1.6 GB |
| `cargo build`, edit | 3.5 3.4 3.3 | 3.2 3.3 3.4 |
| `cargo test --all-features --no-run`, edit | 6.0 6.0 6.0 | 5.9 5.9 6.1 |
| `cargo clippy --all-targets --all-features`, edit (unaffected; check builds emit no debug info) | 8.3 8.3 8.4 | 8.9 8.7 8.6 (load ~8) |

The gain is in cold builds (a new worktree, a profile or toolchain change), CPU and disk. Incremental
edits barely move: rustc's incremental codegen reuses unchanged codegen units either way, and the
macOS linker is not the bottleneck at this size. (The handoff's "34 s -> 6 s" compared a near-cold
rebuild with a warm one.)

Backtraces still carry file, line and column for our frames; inlined frames get short names
(`new<String>` rather than the full path). Checked with a panicking `HostTargetId::new("")` under
`RUST_BACKTRACE=1`:

```
   4: new<alloc::string::String>
             at ./src/protocol/ids.rs:20:36
   5: bt_probe
             at ./tests/zz_bt_probe.rs:3:13
```

What it gives up: a debugger shows no local variables. Set `CARGO_PROFILE_DEV_DEBUG=true` for a
debugging session (a full rebuild of the crate).

### 4. Fewer test binaries

Every test target links the whole library. Six suites that keep no process-wide state
(`contracts`, `host_adapter`, `lifecycle_ux`, `local_endpoint`, `setup_cli`, `view`) are now
modules of one `combined` target (`tests/combined.rs`). The files did not move: `combined.rs`
includes each with `#[path]`, so concurrent edits to them still merge. Ten targets became five.

The other four keep their own binaries because they change process-wide state that would leak into
unrelated tests running in parallel in the same process:

| Target | Process-wide state |
|---|---|
| `hook_entrypoint` | `dup2` stdio redirection, panic hook, in-process daemon |
| `integration` | panic hooks, in-process daemon (`ONE_DAEMON`) |
| `service` | `dup2`, in-process daemon (`IN_PROCESS_DAEMON`), failpoints |
| `package` | `umask` |

`autotests = false` is needed so cargo does not also build each included file as its own target.
Its cost is that a new top-level `tests/*.rs` file would silently not be built, so `combined` has a
guard test, `every_top_level_test_file_is_built`, which fails naming any such file (probed with an
empty `tests/zz_orphan.rs`).

Test names gain the suite prefix: `--test setup_cli foo` is now `--test combined setup_cli::foo`.
`combined` lists the same 162 tests the six binaries listed (plus the guard) and passes in parallel
in one process (162 passed, 1 ignored as before, 9.7 s). `contracts`, `host_adapter` and `view`
used to build under default features too; they now build only with `test-support`, which
CI and AGENTS.md always use for tests.

Measured back to back (edit; wall s / CPU s, user+sys):

| Command | Before (10 targets) | After (5 targets) |
|---|---|---|
| `cargo test --all-features --no-run` (load ~14-21) | 6.7/21.6 6.5/21.5 6.4/21.9 | 6.5/18.0 6.0/17.8 6.1/18.2 (load ~27-41) |
| `cargo clippy --all-targets --all-features` | 8.8/18.3 8.7/18.1 8.6/17.9 | 8.5/16.1 8.3/16.0 8.3/15.9 |
| Cold build (after item 3) | 31.0 s wall, 109 s CPU | 29.9 s wall, 104.5 s CPU (load ~20) |

About 17% less CPU per test build, while wall time stays at about 6 s, because the test binaries link in
parallel and finish before `lib (test)`, which remains the critical path. The CPU saved matters
most when several worktrees build at once (load is routinely 20-40 on this machine).

### 5. Other settings

**`codegen-units = 64`** in `[profile.dev]` (default 256 with incremental). Each value was measured
in its own empty target dir: one cold build, then edits (wall s / CPU s, user+sys):

| codegen-units | `cargo test --all-features --no-run`, 5 edits | `cargo build`, 3 edits | load |
|---|---|---|---|
| 256 (default) | 5.2/18.3 5.2/18.1 17.3/19.7 13.4/18.9 6.2/18.0 | 3.4/4.3 3.4/4.5 3.3/4.3 | ~13 |
| 128 | 5.4/16.2 6.0/16.1 5.2/16.0 5.3/16.0 5.4/16.0 | 3.1/4.0 3.3/4.2 3.0/4.1 | ~5 |
| **64** | 5.5/15.7 5.5/15.9 5.4/15.6 5.2/15.4 5.2/15.3 | 3.0/3.7 3.0/3.8 3.0/3.8 | ~5 |
| 32 | 6.0/16.4 6.2/16.4 6.3/16.6 5.9/16.1 5.7/16.0 | 3.2/4.0 3.2/3.9 3.8/4.2 | ~16 |

A first pass (load ~7-11) agreed: 256 18.2-18.8 s CPU, 64 15.3-15.5 s, 16 16.9-17.2 s. 64 is about
14% less CPU per test build and 0.3 s off `cargo build`; cold builds are unchanged (29-31 s wall,
102-106 s CPU at every value). Clippy does no codegen and is unaffected.

**`[profile.dev.package.sha2] opt-level = 3`: kept.** It is a functional requirement (the Codex
schema fingerprint hashes a ~240 MB binary inside the observation deadline), and it costs only
sha2's own cold compile (1.5 s, in parallel, off the critical path; nothing on an edit).

**The linker is not the bottleneck.** macOS uses Apple's ld-prime (ld-1267). Relinking the binary
alone (its output deleted, nothing else changed) takes 0.5-0.7 s. On an edit, `cargo check --bin
herdr-threads` takes 2.6-3.1 s and `cargo build` 3.0-3.4 s: an edit's time is mostly the rustc
front end (parsing, type and borrow checking) over the whole 71k-line crate, which no profile
setting reduces. Only the crate split (item 6) changes that.

Not pursued: a different linker (lld or mold: link is under a second), `opt-level` for build
scripts (`libsqlite3-sys` compiles bundled SQLite once per cold build, 5 s, in parallel off the
critical path), `split-debuginfo` (macOS dev already defaults to `unpacked`, no dsymutil).

### 6. Splitting the crate (evaluated, not done)

**Shape.** `src/` is 54.6k lines of code and 28.1k lines of unit tests in one crate. By top-level
module (code / unit tests, thousands of lines): store 12.8/11.6, cli 10.5/5.2, harness 7.4/3.1,
protocol 5.3/0.4, daemon 4.4/0.4, service 3.5/0.4, host 2.3/1.7, test_support 2.6/0.2, ports
0.1/3.4, client 0.5/1.6, identity 1.4, scheduler 1.3, app 1.1, notification 0.8, view 0.6.

A plausible layering:

| Crate | Modules | ~Code k | ~Tests k |
|---|---|---|---|
| core | protocol, ports, view::escape, daemon::paths, service::{kicks, fair_writer}, failpoints | 6-7 | 4 |
| store | store | 12.8 | 11.6 |
| runtime | service, daemon, scheduler, notification, identity, host, client, harness | 21 | 7 |
| herdr-threads (bin + tests/) | cli, app, view, test_support | 14 | 5.5 |

The modules form cycles today, but every cycle edge is thin: protocol -> harness (1 reference) and
-> view (2); harness -> cli (3) and -> daemon::paths (6); store -> service::{kicks, fair_writer}
(5); notification -> service (1); daemon -> service::kicks (6); daemon/host -> client (1 each);
service <-> store (1 each way). Moving those few items down into core breaks them.

**Expected gain.**

- An edit's cost is mostly rustc's front end re-verifying the whole crate (item 5): `cargo check`
  2.6-3.5 s, `lib (test)` 5.3 s. An edit in a leaf crate (cli/app, ~1/5 of the code) would
  re-verify only that crate plus relink: estimated `cargo check` ~1-1.5 s and `cargo test
  --no-run` ~3 s, about half of today's. An edit in store or core gains little or nothing:
  every dependent crate is rechecked as well (mostly incremental cache hits, but not free).
- Cold builds gain more: today `lib (test)` (the whole crate with all unit tests) is one
  serial rustc invocation and the critical path (44 s of 53 s at load ~25). Four crates' unit
  tests build in parallel; estimated 25-40% off cold wall time. This matters for new worktrees
  and CI more than for the edit loop.
- Clippy splits the same way: leaf edits relint one crate.

**Cost.**

- Mechanical but wide: moving the cycle-breaking items, then rewriting `crate::` paths across
  ~80 files. 227 `pub(crate)` and 16 `pub(super)` items need a visibility decision at each new
  crate boundary (most become `pub`, widening the API surface the crates show each other).
- `cfg(test)` does not cross crates. The 52 `cfg(any(test, feature = "test-support"))` sites and
  the `failpoint!` macro (10 uses in store, scheduler, service) need a `test-support` feature per
  crate, forwarded down the chain, with dev-dependencies enabling it; unit tests in one crate that
  use another crate's test-only helpers move or switch to the feature.
- Paths change: TRUST-POLICY.md and AGENTS.md name `src/identity/`, `src/store/seats.rs` and others;
  scripts, CI and docs name `src/...` paths. `tests/` can keep `herdr_threads::store::...` through
  re-exports (`pub use herdr_threads_store as store;`).
- Every in-flight branch (the main run, the flakiness tab, parked beads) conflicts with a move of
  most of `src/`. It needs a quiet point with no open branches touching `src/`.

**Recommendation: not now.** After items 1-5 an edit costs 3-6 s to build and ~8 s to lint, about
a tenth of the 1-minute budget, and the split would roughly halve only the leaf-crate share of that.
Revisit when an edit-and-rebuild passes ~20 s or the crate roughly doubles. If it is done, the cheap
first step is a core crate (protocol + ports + the cycle-breakers): it removes the cycles, and
everything else can follow one crate at a time.

### 7. Test profile: no crate-local ThinLTO, 256 codegen units for our crate

`[profile.test.package.herdr-threads] opt-level = 1` (added for test run time) made an edit
cost a near-full LLVM rebuild of the library. After a one-file edit to `src/protocol/handoff.rs`,
`cargo test --locked --all-features --lib --no-run` took 54-150 s wall and ~400 s CPU (all runs
in the tables below plus 66-90 s in earlier runs); the same edit at `opt-level = 0` took 17-19 s
(one loaded run 52 s) and 40-51 s CPU. Two mechanisms, each measured by counting the
regenerated objects in the incremental session directory:

- **`#[inline]` local copies.** Optimized, rustc gives every `#[inline]` function a private copy
  in each codegen unit that uses it (at -O0 there is one shared copy). Derived `Clone` and
  `PartialEq` impls are `#[inline]`: 192 functions from `handoff.rs` had copies in 2-21 units
  (44 of them in 13 or more; `-Z print-mono-items`). A comment line at the top of `handoff.rs`
  (shifting every span below it, no code change) re-generated 36 of 64 units; the same line at
  the end of the file, 0. Debug info is not the carrier: with `debug = false` the same edit still
  re-generated 35.
- **Crate-local ThinLTO.** Optimized with more than one codegen unit, rustc runs ThinLTO across
  the crate's units unless `lto = "off"`. In incremental mode a unit's post-LTO object is redone
  when anything it imports changed, so 36 re-generated units became 64 of 64 re-optimized, and a
  top-of-file comment in `store/queries.rs` (10 units) became 62. `-Z time-passes`: 44 s of a
  62 s rebuild was `LLVM_thinlto`.

Gate transition (`handoff.rs` with Task 21's `RecoverBootstrap::decision_operation` body swapped,
in both directions), same machine and edit, rustc flags checked in `cargo -v` output:

| Test profile | Regenerated | → current body, wall s | CPU s | Runs, load |
|---|---|---|---|---|
| as before (thin-local LTO, 64 units) | 64 / 64 | 54-90 | ~400 | 6, 15-36 |
| thin-local LTO, 256 units | 254 / 256 | 50 | 352 | 1, ~14 |
| `lto = "off"`, 64 units | 41 / 64 | 45 57 67 | 180-188 | 3, 13-31 |
| **`lto = "off"`, 256 units** | 68 / 256 | 32-64 | 67-76 | 7, 15-36 |

ThinLTO off is the larger lever. 256 units (rustc's own default for incremental builds) cut CPU
by another 2.6x, and smaller units shorten the single-threaded tail, which matters on a shared
machine: interleaved
with `lto = "off"` at 64 units, the current-body edit took 48.5/35.5/39.9 s against 67.2/57.1/44.9 s.

Interleaved A/B of the exact gate command, one target dir per side, four pairs (A is the
previous profile; load average from other worktrees 13-45 on 12 cores):

| Edit | Before, wall / CPU s | After, wall / CPU s |
|---|---|---|
| → historical body | 53.6/399 89.7/415 86.3/415 150.1/424 | 28.1/66 28.7/66 35.2/70 37.0/68 |
| → current body | 53.6/393 72.6/401 87.8/409 89.9/402 | 32.1/70 64.3/76 54.5/74 55.0/74 |

The build-speed method above (a comment at the top of `store/queries.rs`, `cargo test
--all-features --no-run`, every test target), interleaved at load 35-103:

| Edit | Before, wall / CPU s | After, wall / CPU s |
|---|---|---|
| add comment | 110.0/490 89.9/492 88.2/459 | 18.3/64 23.1/68 44.0/74 |
| remove comment | 94.2/473 74.8/470 195.2/511 | 33.9/73 18.6/63 59.1/77 |

The same profile on a later base (Task 22, an edit to `src/host/transport.rs`, interleaved, three
rounds each way): `--lib --no-run` 14-44 s / 45-52 s CPU against 55-178 s / 414-473 s, and
`--test integration --no-run` 8-37 s / 12-16 s against 26-63 s / 152-168 s.

A comment line at the top of `handoff.rs` is the worst edit measured: 69.9 s / 162 s CPU adding
it and 49.1 s / 154 s removing it (load 24-32), against 92.8-93.9 s / 424-430 s before.

CPU per edit is down 2.7x (top-of-`handoff.rs` comment), ~5.7x (gate edit) and 6-8x, ~7x on average (`queries.rs` comment). Wall time
now follows the serial part of a rebuild: the front end, building LLVM IR on rustc's main thread,
writing the incremental dep graph (1-23 s, depending on load) and the largest unit. That is
`serde_json::de`'s "volatile" unit (10.5 MB of object code: every `serde_json` deserializer
instantiation in the crate), which any edit to a type deriving `Deserialize` re-generates and
which one LLVM thread optimizes. No profile setting splits it. **The budget is met at light load
but not always under heavy load:** the gate edit took 64.3 s once at load ~26-29, and the
top-of-`handoff.rs` comment 69.9 s at load ~27-32. A nearly full disk inflates wall time further
(a no-op rebuild took 26 s at 9 GB free).

Run time: the page-fit and page-cursor unit tests (`cargo test --lib store::page --
--test-threads=1`, the tests `opt-level = 1` was added for) took 11.5/11.4/11.9 s before and
11.5/11.6/12.0 s after, interleaved; at `opt-level = 0` they take 20.6-22.8 s. One full nextest
run each, back to back: 468 s after and 503 s before (both over the 5-minute suite budget on
this loaded machine, whichever profile). Both had failures the profile does not cause: two
`archival_legacy` tests in each (all 32 pass alone, three times, either profile), and before
also three timing-sensitive tests (`lanes_latency`, `summary_flow`, `topology_activation`).

`lto` is a whole-profile setting (it cannot be set per package), so `lto = "off"` also covers
dependencies in the test profile: they are -O0 except `sha2` and `libsqlite3-sys` (whose C is not
LTO'd by rustc). `cargo build` (dev), release builds and clippy are unaffected. The dev profile
keeps 64 units (item 5), which suits -O0.
