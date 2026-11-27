super-roast verdict: Should-fix (4 confirmed) [low coverage]
mode: PR        iteration: 1 of 3
profile (assumed): Internal developer tooling with production-grade trust rules (TRUST-POLICY is normative). The local daemon's seat continuity decides attribution for receipts and wakes for every agent on the machine. A defect misplaces roles or wastes model context; no network-exposed surface.
inputs: fix/ht-iygh-seat-restore@9ef0a01f9 vs afeed32ac — stated intent: docs/superpowers/specs/2026-11-27-seat-restore-after-restart-design.md (Revision 2), docs/superpowers/reviews/2026-11-27-seat-restore-roast-design-1.md, TRUST-POLICY.md
coverage: unavailable — inline run, no scout or judge fan-out; single reviewer read src/ and migrations/ diff in full, tests selectively; no build or test run (a suite was running in the worktree)
independence: none (inline) · rung: inline

Checked and found sound: migration 0020 (the CHECK list is 0019's list plus `host_session_continuity` and `pane_continuity`; the `_after` swap recreates both indexes that 0001/0019 define; no triggers exist on the table); D1/D3 ordering (each re-pins the seat as unresolved at the judged generation inside its transaction, so the loser gets Conflict); D1 operation key and digest; D2 consumers (liveness gating in bindings.rs:724 and the explicit-hold reads in wake.rs and cooperative.rs are unchanged; ht-0f0r Restore still sees BaselineHeld for a non-waiting unresolved seat at its own pane); D5 call sites (launch and attachment keep `eligible_delivery_recipient`, a pane-form `--existing` still needs a resolved seat).

## Confirmed findings
- [Should-fix] src/harness/codex.rs:1459, src/harness/claude.rs:997, src/notification/dispatch.rs:624 — a wake into a D1-restored seat types the inbox hint (`MARKER`, 56 B) instead of `WAKE_UP`, because the dormant binding's provenance is `host_session_continuity` and both adapters return `WAKE_UP` only for `cooperative_top_level`. This breaks R18 (rule c) and invites the extra `inbox` call that `no_extra_inbox` guards against. The test at tests/store/binding_lazy.rs:910 builds its binding with `enroll_binding` (not the D1 shape: no `registered_at`, no pid, D1 provenance), so it cannot catch this.
  verdict: confirmed (inline reading: wake_target_rows carries the row's provenance into the dispatch reduce)
  evidence: dispatch.rs:624 `registry.binding_wake_text(id, Some(&t.provenance))`; codex.rs:1460 `if provenance == Some(COOPERATIVE_TOP_LEVEL_PROVENANCE) { WAKE_UP } else { MARKER }`
  fix-shape hint: limit HOST_SESSION_BINDING to ended bindings with `observation_provenance='cooperative_top_level'`, then have the Codex and Claude `binding_wake_text` and `binding_turn_delivery` treat `host_session_continuity` as hook-registered; add a wake-text assertion on a binding written by the real decision.
- [Should-fix] src/store/bindings.rs:1495-1500 with src/store/seats/continuity.rs HOST_SESSION_BINDING — D1 accepts any ended top-level binding that is not `managed_launch`, including plain-CLI (hookless) ones. But `plain_reconnect_allowed` reconnects only `liveness_process_exit` and `migrated`, so a `host_session_restore` binding reconnects only through a lifecycle claim. A hookless restored session never makes one: its plain calls just touch a binding that stays disconnected, and the seat reads resolved but unavailable for good. TRUST-POLICY:600 says lifecycle or turn check-in. Only the lifecycle path is tested (host_session_continuity.rs:704).
  verdict: confirmed (code path); runtime impact for hookless Codex not exercised
  evidence: bindings.rs:1497 `Some(reason) if reason == DisconnectReason::ProcessExit.as_str() || reason == DisconnectReason::Migrated.as_str()`
  fix-shape hint: the same provenance restriction as the finding above (simplest, and it matches the policy's hook premise), or admit `HostSessionRestore` in `plain_reconnect_allowed` and record that as an accepted limit.
- [Should-fix] TRUST-POLICY.md:590 vs src/identity/repair.rs judge_host_session_candidate — the policy says a mismatch "(another kind or session, no agent, no session)" is not re-read for 10 minutes. Since 87b508013 the code declines only a reported other session or kind, and retries absence every pass. The normative text is stale. In practice the code polls `agent.get` without end (750 ms timeout each, up to 16 seats per pass, every pass) for any unresolved seat whose saved pane now runs a plain shell, until that seat retires.
  verdict: confirmed
  evidence: repair.rs `if !reported { return; }` before `decline_host_session`; policy line 589-590
  fix-shape hint: update the policy text. Bound the absence retry, e.g. retry absence only within a grace window from the recovery epoch's pass, then decline with the ordinary TTL.
- [Should-fix] src/store/seats/continuity.rs pane_continuity_seat (diff +972..1033) — D3 rests on the pane alone, yet unlike D1 (continuity.rs:1905, `continuity_keys::hold_kind`) it does not refuse a target with a hold-bearing key-conflict row (`key_live_elsewhere`, `key_retired_reversibly`, `session_matches_other_seat`). `keyed_seat` judges only a claim that presents a key, so a keyless fresh session can take X at a pane that a conflict holds.
  verdict: confirmed (no hold_kind on the decide_continuity Pane path); the D3 policy row omits it too
  evidence: resolution.rs:216-224 applies hold_kind only to ordinary resolution; pane_continuity_seat checks only `key_of_seat`
  fix-shape hint: add `continuity_keys::hold_kind(tx, host, target)?.is_none()` to pane_continuity_seat and add the guard to the C1 D3 row.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [Should-fix] migration 0020 loses indexes or triggers — rejected: both indexes are recreated as in 0001/0019, and the table has no triggers.
- [Should-fix] D2 breaks ht-0f0r service restore — rejected for the supported case: the restoring seat is itself the unresolved, non-waiting seat saved at its pane, so BaselineHeld still holds (operator.rs:507).
- [Should-fix] D1 and D3 can both decide one seat — rejected: both re-check `state='unresolved'` at the judged generation inside the writer transaction (guard_continuity_pin; Pane selection requires unresolved).

## Unverified nits (spot-checked)
- [Nit] continuity.rs decide_host_session_continuity (diff +1451) — D1 enrolls a fresh binding (`SeedMode::Fresh`, `first_contact` true, no lineage) instead of reopening or lineage-linking the ended row as C1 does (continuity.rs:1124-1135). Cursors start at the seat maximum, so unread mail survives. The ended binding's above-cursor display marks are lost, though, so those rows can be offered again. Fix: pass `lineage: Some(<ended binding>)`.
- [Nit] repair.rs judge_host_session_candidate — `matched` searches every candidate session, including declined ones, so a declined session can still be decided when another session of the seat is open.
- [FYI] effective.rs baseline_target_held_sql — `recovery_boot IS NULL` reads as held, while `reconciliation_lags` reads NULL as not lagging. The flag is set only with a baseline, so this is likely unreachable.
- [FYI] remote_resolver.rs check_seat — an older home node still answers `AddrTarget::Check` through `eligible_delivery_recipient`, so `handoff --existing` to an unresolved remote seat keeps refusing there until that node upgrades.
- [FYI] operator.rs:507 — a waiting (unretired) seat at its own pane now reads UnambiguousUnclaimed after the pass, so ht-0f0r Restore answers NotFound for it, where the global flag used to give BaselineHeld. This is consistent with D2's waiting exclusion; confirm it is intended.

## Escalations (need human)
- none
