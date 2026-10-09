**Closing whole-epic review at b7a408c0.** Independent opus reviewer, static; mod JS tests 84/84.

**Verdict: ready.** No must-fix defects.

- Review 10's finding (ht-j16.34) is fully resolved by 2945d1ee:
  - `wake_warnings` is shared with native wake.
  - `mod_seat_view` narrows its warning count with it.
  - The daemon sets `informational`, and the drain leaves those warnings out of attention.
  - The new field is wire-compatible.
  - Three tests cover it, and TRUST-POLICY A7 and the spec note record the rule.
  - No regression in ModChannels, AckModDelivered, wake suppression, the hook live paths, setup or register.js.
- `git diff 573841ed b7a408c0` touches only docs, CHANGELOG and run records. The code is what the suite (4032/4032) and the live stress (14/14 scenarios 3/3) ran against.
- Whole-surface re-scan: no new must-fix beyond the items already routed.

**Escalation triage**

1. **Idle check vs submit not atomic** (design roast 1). **Open: a human decides. Low severity.**
   - register.js re-checks `busy()` and the hold right before a synchronous submit, so the window is narrow.
   - Whether the engine keeps a plugin prompt queued behind a user Enter after an Esc was never checked live.
   - Only the general limit "Engine-reported idleness is trusted" covers it. Options: add a named accepted limit, or run the spike check.
2. **`$.session.id()` read inside `session.end`.** **Resolved.**
   - The read happens after `session.end`, and `sidSuspect` forces a re-read on restart (register.js 228-233, 721-726). This was live defect D1, fixed by ht-j16.28.
   - `clear_rebind` passed 3/3 at b562d1e4, 73a75cbf and 573841ed.
3. **Frame marker spoofing.** **Accepted limit:** TRUST-POLICY "Mod frames keep peer text off the header column" (~557-561).
   - Body indentation and quoted, single-line names harden it.
   - What remains is marker-like words inside the quoted names on the header line.
4. **D3 `isError`/`deny` heuristic.** **Accepted, documented heuristic:** spec note "Permission-dialog Esc (ht-j16.30)" and CHANGELOG.
   - Worst case is a submit delayed by up to 120 s. Nothing is lost or delivered twice.

**Minors**
- CHANGELOG.md:8 still cites the 13/14 run; the final run was 14/14.
- CHANGELOG.md:13 does not say the mod channel follows the focused-warning narrowing.
- With a saturated walk, the fingerprint and the drain disagree. This is harmless: the effect is extra frames with no attention line.
