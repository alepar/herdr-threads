super-roast verdict: Nit (1 confirmed)
mode: PR        iteration: 1 of 3
profile (assumed): Local developer tool (herdr-threads CLI/daemon) shipped to external users, under a same-user cooperative trust model. Defects here break setup or daemon startup for users. At worst they could let a client or setup delete or adopt another root's or daemon's artifacts. The bar is production-grade for local tooling.
inputs: fix/resume-device-change@633549399 vs main@92dff69f8
coverage: scouts 13/13 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, observability, testing, test-causality-lifecycle, test-isolation-environment, test-suite-cost, hygiene-docs, deploy-safety) · raw 47 → deduped 26 → panel 7 · spot 19 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "git diff main...fix/resume-device-change at 633549399, 2 commits, 4 files"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 7 · rr 0.86 · rg 0.43 · fg 0.29 · unanimous 0.29 · ground-loo 0.33 (n=6) · reproduce 1/6/0 · refute 2/5/0 · ground 5/2/0
lane-yield (found/confirmed/unique/refuted): correctness 6/1/0/5 · security 4/0/0/4 · premortem 4/1/0/3 · simplicity-design 4/0/0/3 · hot-path-perf 3/0/0/3 · concurrency-async 3/1/0/2 · observability 4/0/0/4 · testing 6/0/0/4 · test-causality-lifecycle 2/0/0/2 · test-isolation-environment 3/0/0/3 · test-suite-cost 0/0/0/0 · hygiene-docs 5/0/0/3 · deploy-safety 3/0/0/3

No route was overruled and no severity was changed from the seat ratings. No severity floor applies: the one confirmed finding fails closed, loses no data and crosses no privilege boundary. The only pre-existing packet is an unverified nit, reported as FYI. For the rejected test-coverage finding (tests/daemon/ownership.rs:1091), the two tests the majority seats cite were checked to exist at 633549399 with the owner lock held and `socket_device ^ 1`; the dissenting seat's evidence does not address either, so the rejection stands.

## Confirmed findings            ← consumed by super-design, one task per finding
- [Nit] src/harness/permissions.rs:341; src/harness/permissions.rs:345 — renew_renumbered_marker rewrites the marker non-atomically (write_all_at, then set_len). An interruption between the two, when the new marker is shorter, leaves the new bytes plus a stale tail. No later acquire renews or accepts that file, so setup is refused on every run, with no documented recovery. [lanes: correctness, premortem, concurrency-async]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/harness/permissions.rs:344-346 does `file.write_all_at(&marker, 0)?; file.set_len(marker.len() as u64)?; file.durable_sync()?;` as separate syscalls.
    marker() prints the device in decimal (`{dev}:{inode}`), so a renumbering such as 66307 -> 2049 shortens the marker.
    In the torn file, line 3 parses as the current device, so renumbered_marker returns false (`device != self.device` fails) and nothing is rewritten.
    revalidate then compares the bytes with identity.marker(), sees the tail, and refuses with "foreign or edited config lock" on every acquire.
    The lock name is never unlinked by design, and docs/operations.md gives no recovery for the setup lock.
    All three seats rate it Nit: the window is two adjacent syscalls, it needs a shorter new device number, it fails closed, and deleting the lock file recovers it (acquire creates a fresh lock on NotFound).
    The seats read the code; none ran a reproduction.
  fix-shape hint: make renumbered_marker accept this root's current marker followed by trailing bytes and truncate it, or encode the device in a fixed-width field; add a test with a longer stale device (see the unverified nit at permissions.rs:540) and a one-line recovery note in docs/operations.md.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/daemon/ownership.rs:390-396 — read_descriptor now takes the owner flock for a moment (renumbered_without_owner -> previous_owner_released -> try_lock), against ensure's rule that nothing probes the lock while a child elects.
  reason: panel 2 REJECT / 1 CONFIRM (Nit).
    Within one ensure, `lock.remove_stale_endpoint()` runs before `spawn_detached`, so later handshakes get NotFound from parse_descriptor and never reach the probe. The rule holds for ensure's own children.
    A foreign reader must stall between its re-stat and its try_lock across another ensure's unlink, fork and exec.
    A lost election is absorbed by ensure's existing "loser of a transient probe: launch again" path. Clients get NotFound and delete nothing.
    Client-side probes of the same lock already exist on base (live_skew at lifecycle.rs ~184, stop at control.rs ~440).
    The confirming seat concedes each of these and rates it a doc/test nit. The side effect is undocumented; no stated requirement is contradicted.
- src/harness/permissions.rs:338-342 — renew_renumbered_marker decides from bytes read by path but writes through the locked fd, with no fd-vs-path identity check in between.
  reason: panel 2 REJECT / 1 CONFIRM (Nit).
    The write can only land on the fd's inode, never on the file at the path.
    guard.revalidate() -> check_lock_metadata requires same_inode(fd, path) and nlink == 1, so a swapped path is refused and no guard is handed out.
    A swap needs a non-cooperating same-user actor who also supplies this root's renumbered marker at the path. AGENTS.md and TRUST-POLICY.md put that outside the cooperative model.
    The confirming seat agrees the protection holds and calls it hygiene. Reading through the fd (`read_at`) would be tidier.
- src/daemon/ownership.rs:392 — the client-side stale verdict never checks that the socket refuses connections, so a live non-owner listener on a renumbered socket reads as NotFound.
  reason: panel 2 REJECT / 1 CONFIRM (Nit).
    docs/operations.md:53 states this client behaviour for the whole class: "A client that finds such a socket while no process holds the owner lock reports the daemon as not running (exit 3), as for any stale endpoint, and removes nothing".
    The "live listener holds the daemon socket pathname without the owner lock" refusal is documented for `daemon ensure`, and renumbered_device_never_reclaims_live_or_unsafe_artifacts covers it.
    The client path removes nothing. The only effect is a less specific first diagnostic for a state that needs a stray listener after a renumbering.
- src/harness/permissions.rs:211 — the adoption rule matches backend, root path and inode and ignores device, so a mount-point config root (fixed root inode) adopts a marker from another volume mounted at the same path.
  reason: panel 3 REJECT.
    Dropping the device from the identity is the stated fix (ht-1nva). A same-path, same-inode marker from another volume cannot be told apart from a renumbered resume.
    A different root path, inode or backend still refuses, and the new test covers those three.
    Adoption only rewrites the user's own flock'd, owned, 0600 lock file. No privilege or data boundary is crossed.
- src/daemon/ownership.rs:396 — the probe drops a plain File without an explicit flock(LOCK_UN), so a concurrent fork can hold a copy of the lock until exec and an electing daemon can see WouldBlock (the ht-w7n pattern).
  reason: panel 2 REJECT / 1 CONFIRM (Nit).
    previous_owner_released is unchanged and already on base (92dff69f8, ownership.rs:523) in the control and live_skew paths.
    The new caller runs only when a device-only mismatch is published with the lock free, not on every read_descriptor.
    The worst outcome is a transient WouldBlock that ensure relaunches. Nothing is reclaimed or deleted.
    Not reproduced by any seat. An explicit unlock in the probe would be cheap hardening.
- tests/daemon/ownership.rs:1091 — no test covers a renumbered socket (inode equal, device different) while a live process holds the owner lock still reading as substituted.
  reason: panel 2 REJECT / 1 CONFIRM (Nit).
    tests/daemon/ownership.rs:948-962 (dead_boot_socket_with_renumbered_device_is_reclaimed) keeps `owner` held, rewrites the descriptor with `socket_device ^ 1`, and asserts PermissionDenied, is_socket_substituted and the exact text from read_descriptor.
    tests/daemon/lifecycle.rs:1534 (live_owner_with_mismatched_socket_is_kept_and_reported) holds the OwnerLock with `socket_device ^ 1` and asserts the "a live process holds the owner lock: stop it" error from ensure.
    Deleting the lock term from renumbered_without_owner would fail both. The confirming seat's list of live-owner read_descriptor tests (ownership.rs:382, 486, 589) omits these two.

## Unverified nits (spot-checked)
- [Nit] src/harness/permissions.rs:540 — the ht-1nva test builds the stale marker with `device ^ 1`, which keeps the digit count, so deleting `file.set_len(marker.len())` would still pass. (spot: confirmed; closes with the confirmed finding's fix if a longer stale device is tested)
- [FYI] docs/operations.md:53 — the client's "run `herdr-threads daemon ensure` to reclaim it" text and the doc sentence "`daemon ensure` (or the startup hook) reclaims it" now apply only to cases ensure cannot reclaim (live lock holder, or inode differs). (spot: confirmed at Nit; pre-existing on base: docs/operations.md, same paragraph, "Clients still refuse a mismatched socket (...); `daemon ensure` reclaims it"; the seat notes the reworded sentence in this diff reads as self-contradictory)
- [FYI] src/daemon/ownership.rs:376; src/daemon/ownership.rs:398 — the socket check and the owner-lock probe are separate reads, so a race with an electing daemon yields a transient NotFound reported as an unexplained exit 3. (spot: rejected; within the documented contract, no caller deletes on it, a retry clears it)
- [FYI] src/daemon/ownership.rs:393 — renumbered_without_owner re-stats the socket; if the elected owner removes it between the two stats, the client briefly reports "socket pathname was substituted". (spot: rejected; the owner holds the lock during removal, so the lock term would return false anyway)
- [Nit] src/harness/permissions.rs:315 — no test holds the flock on a renumbered marker and asserts the bytes are unchanged after a refused acquire, so moving the renewal ahead of acquire_config_lock would pass the suite. (spot: confirmed; production ordering is correct)
- [Nit] src/daemon/lifecycle.rs:203 — the `handshake` doc comment still says only the elected owner can tell a dead boot's renumbered socket from a substituted one; read_descriptor now does so when the lock is free. (spot: confirmed; stale comment, no behaviour affected)
- [FYI] src/daemon/ownership.rs:377 — callers match on NotFound and drop the new "stale endpoint descriptor" text, so `doctor` and the CLI show plain "not running". (spot: rejected; exit 3 / unavailable is the stated intent of ht-x772, recovery is unchanged)
- [FYI] src/harness/permissions.rs:340 — the marker rewrite emits no log line or setup output. (spot: rejected; a quiet successful recovery, and new setup output is model-visible text that AGENTS.md asks to justify)
- [Nit] src/daemon/ownership.rs:376 — the "inode equal, device differs" rule is written out in both renumbered_without_owner and remove_dead_boot_socket_as, with a second stat and a downcast of the module's own error. (spot: confirmed; maintainability only)
- [FYI] tests/daemon/ownership.rs:1096 — the new stale test could see the lock held after `drop(owner)` if a sibling test thread forks at that moment. (spot: rejected; the lease's Drop does an explicit flock(LOCK_UN), ownership.rs:27-31, and the test at tests/daemon/ownership.rs:917 covers it)
- [FYI] src/daemon/ownership.rs:392 — a lock-probe I/O error is mapped to `false` and surfaces as the substituted message. (spot: rejected; fail-closed, documented in the doc comment, ensure then surfaces the real error)
- [FYI] src/harness/permissions.rs:212 — renumbered_marker hand-parses the layout that marker() defines. (spot: rejected; acceptance is a byte comparison against marker(), a format change fails safe and fails the renewal test)
- [FYI] src/harness/permissions.rs:552 — the "other root" fixture changes root path and device together; no case pins a malformed device line. (spot: rejected; the finding concedes the logic is correct)
- [FYI] src/harness/permissions.rs:568 — the negative cases assert only `acquire(...).is_err()`, not the error kind or text. (spot: rejected; the unchanged-marker assertion covers the property, and the file's other tests use the same style)
- [FYI] docs/operations.md:53 — the ht-1nva doc sentence ships in the ht-x772 commit (633549399), not with its code (47a2cf601). (spot: rejected; the commit message says so, docs match behaviour at head, no rule requires per-commit docs)
- [FYI] tests/daemon/ownership.rs:1091 — the exit-3 / doctor "not running" outcome is asserted only at the io::ErrorKind level of read_descriptor. (spot: rejected; tests/lifecycle_ux.rs:908 already asserts exit 3 for every descriptor-reading CLI path on NotFound)
- [FYI] src/daemon/ownership.rs:393 — the substituted path costs roughly 2 stats, 3 opens and a flock per call inside the 25 ms and 10 ms poll loops. (spot: rejected; only in the rare mismatch state, microseconds against the poll interval)
- [FYI] src/harness/permissions.rs:340 — every acquire does one more by-path read of the marker before revalidate reads it again. (spot: rejected; setup-only path, and the read is what decides the renewal)
- [FYI] src/harness/permissions.rs:315 — neither the NotFound reclassification nor the marker rewrite has a kill switch. (spot: rejected; no stated requirement asks for one, both triggers are narrow, a downgrade accepts the rewritten marker)

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none