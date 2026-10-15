**Verdict: not ready.** I found no code defect that blocks landing. The branch still can't land because no full-suite run exists at HEAD 12d150baa, and the wake e2e has not run since 744ca90a, which predates the ht-yha.62 rewrite of the wake rule. The caller's sweep after this review will be the first full-suite measurement of HEAD, so the branch should not be treated as tested yet.

## Step 1: my own review against the spec

**What changed since the last final review (wf_c04baaa3-4ad, at 5b79084c):**
- the main merge a80ef10e;
- the sweep at a80ef10e;
- ht-yha.73 (bb0f16a41).

I read the spec, design.md D1–D11 plus its post-implementation notes, then reviewed those changes and spot-checked seams across the whole epic.

**Checks I ran at HEAD 12d150baa, all passing:**
- clippy (`--all-targets --all-features -D warnings`): exit 0.
- `scripts/check-default-features`: exit 0.
- `scripts/check-harness-neutral`: 0 violations.
- `cargo fmt --check`: exit 0.
- Focused nextest: `config8_upgrade_from_a_v0_6_0_database`, the `hook_inbox::` tests and the `priority::` tests, 30/30 pass. Then `binary(hook_entrypoint)`, the golden tests and `session_bindings_smoke::config*`, 124/124 pass.
- The leak check (`check-no-leaked-processes --run-id`) was clean after both runs.
- `wake_e2e_selftest.py`: 134 tests OK.

**ht-yha.73:**
- The root cause holds up. A Claude resume with a hot thread fills about 3360 of the 4096 context bytes. ht-yha.60 had made `INBOX_HEADER` 23 bytes longer, which cut the margin to 1 byte. A 4-digit `age_millis_signed` under load then pushed the two-row block over the limit.
- The fix shortens fixed text only (header 315→279, ACK prefix 108→91, the all-shown line 66→34). The commit states the sizes. Page order and the complete-prefix rule are unchanged.
- The new unit tests pin the measured page and a worst case (4077 of 4096).
- No stale copies of the old strings remain in `src`, `tests` or `integrations`.

**Main merge a80ef10e:** the `wake-e2e.py` conflict was resolved correctly, keeping both `nudge_threshold` and main's `shows()`. The other merged files (setup stale-record, `check_failpoints_absent.sh`) are main-reviewed and don't overlap this branch's code.

**Seams I checked:**
- **Mod batch order:** `register.js` `orderKey`/`batchOrder` match `batch_key` in `assemble.rs`: priority, then deadline with none last, with a stable sort so arrival breaks ties.
- **Delivery-mode decisions:** `DeliveryMode` is matched for decisions only in legacy/journaled paths and in the validators.
- **Wake seek:** `wake.rs` no longer reads `lazy_recipients.state`.
- **`drain_pages`:** now emits page by page, so a later page error no longer drops buffered items.

**Findings of my own:**
1. **Sweep at a80ef10e: 405 s against the 300 s budget.**
   - The release-build failpoints test timed out at 300 s (ht-wur). Main's incremental failpoints proof (34f2cd55a) ran cold in that sweep, so the budget is still unmeasured on a warm build.
   - That sweep's config8 FAIL is fixed by ht-yha.73. The topology_handoff `downstream_new_thread_create_links_parent_child_in_same_real_transaction` failure from the 9de1002a sweep passed here; it is now filed as ht-p1eu.
2. **The wake e2e is out of date.**
   - The only full pass is 744ca90a. Since then: ht-yha.62 changed the one-process wake rule, ht-yha.63/.71 changed display marks and deadline keys, and ht-yha.73 changed hook text the model sees.
   - The `side-nudge` sibling phase has never run live.
3. **Minor: `wake-e2e.py:1211`.** `sibling_phase` waits for `SIBLING_SIDE_DONE` with `self.screen()` only. Main's 33d73558f moved such checks to `shows()`, which reads the visible viewport first. This is a Codex-only path, and a stale capture only costs up to 120 s of waiting.
4. **Minor (inherited):** the design spec's ht-yha.45 note (around line 314) still describes the per-binding `lazy` refusal that ht-yha.62 reversed.

## Step 2: ledger triage

**Clusters:**
- **[doc-drift], ×12 across 10 tasks:** this is a pipeline defect. Task file lists left shared normative and model-visible docs (SKILL.md, AGENTS.md, TRUST-POLICY.md, READMEs, the design spec) out of scope, so drift piled up with no owner until dedicated tasks (ht-yha.64, .70) picked it up.
  - Fixed at HEAD: the linux-e2e README case names, the mod README "lazy rows" wording, `help.rs` and SKILL.md on Ensure/pair invitations, the `wake_on: all` comments, and the TRUST-POLICY wording for D9 `--nudge` and the cursor-invariant reliance.
  - Still open, both minor: the spec line around 314, and ht-yha.73's all-shown line ("No other inbox content is pending.") no longer says that no inbox call is needed.
- **[missing-edge-case-test], ×5 across 5 tasks:** one systemic smell. Reviews notice boundary combinations, but no task owns pinning them. None of the instances names a skipped test, a product defect or a data-loss path.

**Parked findings:** none (`parked 0`).

**BLOCKED lines:** ht-yha.5.3, .33, .42 and .44 were all later resolved, re-planned or made moot by protocol 8. Their beads are closed.

**BLOCKED-AUTH lines:** none.

**Metrics:**
- `landed≠completed 57 vs 54`: the three extra landings are ht-yha.1, .5.1 and .47, re-seeded on every Resume, and all are CLOSED. The gap is a ledger-read artifact, not missing work.
- One invocation's metrics are UNAVAILABLE (ledger lines 472–475), so the totals undercount it.
- The one merge failure is ht-xml, which is closed.

**Minors I re-checked for misgrading:**
- **[state-dependent-wake-seek] (ht-yha.4):** a missed-wake defect, but fixed. `wake.rs` no longer reads `state` (ht-yha.44/.54).
- **[error-path-behavior] (ht-yha.9):** fixed. `drain_pages` now emits per page.
- **[missing-capability-probe] (ht-yha.7):** moot under protocol 8.
- **Stall flake (ht-yha.46/.57) in `mod_delivery::stall_with_only_a_p0_delivery_row_hands_over_to_the_native_wake`:** failed in the e6de8153 sweep, passed in 9de1002a and a80ef10e. It is a load flake whose cause was never isolated; watch it in the caller's sweep.
- **[thin-margin] (ht-yha.73):** there is a 63-byte margin under the 4096-byte context limit. The product falls back to a "do not fit" pointer, which is graceful, and a unit test now pins the budget deterministically. Deferred is OK.
- **[unverified-key-transition] (ht-yha.71):** the previous review checked every deadline writer and found no defect beyond the disclosed retired-before-cutover case.

None of the deferred minors needs to be re-graded as must-fix.

## Must fix before landing
1. **Run the full suite at 12d150baa:** `nice cargo nextest run --locked --all-targets --all-features` with `HT_LEAK_RUN_ID` set, then `scripts/check-no-leaked-processes --run-id`.
   - Check the wall-clock against the 300 s budget, with a warm failpoints proof directory.
   - Triage any failure. Expect config8 to pass now.
   - Note whether ht-wur (the failpoints timeout) and ht-p1eu (the topology flake) recur.
2. **Re-run the wake e2e at HEAD**, including the opt-in `--harness codex --cases side-nudge/group,side-draft/group`, or get the user's explicit deferral to the release gate. The wake rule and the hook text the model sees both changed after 744ca90a.

## Untested scope
- **No full-suite run exists at HEAD.** The last sweep (a80ef10e) predates ht-yha.73 and had 1 FAIL and 1 TIMEOUT.
- **The wake e2e** has not run since 744ca90a, and the ht-yha.62 sibling phase has never run live.
- **The deadline-window rework (ht-yha.63/.71)** has only focused-test evidence.
- **Upgrading a real v0.11 state dir** is tested on fixtures only. `migrations/0047` was edited after earlier branch builds, so any state dir migrated by one of those builds should be discarded (acceptable only because v47 is unreleased).
- **The Linux Docker e2e** was not run (optional).
- **No BLOCKED-AUTH coverage loss.**

## Deferred OK
- **Doc wording:** the spec's ht-yha.45 note should say ht-yha.62 reversed it, and the hook all-shown line wording.
- **wake-e2e:** move `sibling_phase` to `shows()` (`wake-e2e.py:1211`).
- **Unpinned edge-case tests:**
  - a bare p0 send to a pending pair invitee;
  - the service-send legacy-receipt resume path (`service_send.rs:357`);
  - the mixed receipt+invitation semantics of `none_offered` (ht-yha.62);
  - the TopologyHandoff route of the M5 test (ht-yha.65).
- **ht-yha.73:** the thin margin, with a structural byte reservation left for later.
- **Speed:**
  - `deadline_traversals_show_every_item_exactly_once` takes 6–36 s.
  - The 0047 cursor triggers cost O(view size) per cursor advance.
- **Test leftovers:** the weak boundary case and the `BoundedP0History` mock-drift risk in `lazy_markers.rs`.
- **Accepted by design:**
  - Service sends with explicit recipients lose the `[p0]` read marker.
  - Deadline-class order is exact only within one 64-item window.
- **Open flake beads stay as filed:** ht-wur, ht-3wm, ht-kk8, ht-92dd, ht-p1eu.
- **ht-esl** (multi-process ordinary wake refused) is a pre-existing main issue, not this branch's.
- **Beads not yet filed:** the previous final review's deferred items have no tracker beads yet.

Files reviewed:
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/review-53e400ed5..12d150baa.diff
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/docs/superpowers/runs/2026-10-13-message-priority-delivery/2026-10-13-message-priority-delivery-design.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/docs/superpowers/runs/2026-10-13-message-priority-delivery/sweep-a80ef10e.log
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/.superpowers/sdd/ht-yha-plan/progress.md
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/cli/hook_inbox.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/delivery/assemble.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/src/delivery/page.rs
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/integrations/claude/mod/hooks/register.js
- /Users/alepar/AleCode/herdr-threads/.worktrees/super-auto/message-priority-delivery/scripts/wake-e2e.py