Machine-specific command paths are redacted as `HEAVY TOOLBOX` and `<worktree>`; review verdicts are unchanged.

# Task 1 scoped re-review, round 1

Spec compliance: Issues found. Task quality: Needs fixes. Original I1 and I2 are ADDRESSED; the I1 fix introduces one Important reload regression.

Scope: `4643aab03..6bcff9047`, supplied fix diff only, original I1/I2 and the current approved design ruling on conservative 256-key novelty-history saturation. No broader branch review.

## Original findings

- **I1 — ADDRESSED.** `integrations/pi/extension/herdr-threads.js:224` checks retained waking witnesses and bounded known keys before admitting new attention, so alternating already known IDs no longer replace the blocked intent. Repeated connected status on a live channel does not reset the fence (`:194`). Rejection/error/abort and no-progress regressions exercise these cases (`integrations/pi/extension/tests/extension.test.mjs:398`, `:422`).
- **I2 — ADDRESSED.** Actual stop work and the enclosing replacement remain tracked through `ownStop`/`stopWatch` (`integrations/pi/extension/herdr-threads.js:116`, `:122`). Memoized close joins outstanding stops and retains the singleton until completion (`:314`, `:333`). Deferred-exit regressions cover shutdown, direct reload and reload joining an already closing owner (`integrations/pi/extension/tests/extension.test.mjs:433`, `:451`).
- **Approved saturation behavior — implemented in the reviewed paths.** Saturation latches a separate wake-only block instead of evicting history to manufacture novelty (`integrations/pi/extension/herdr-threads.js:167`, `:224`); an authentic user transition resets it (`:372`). The added pressure test checks error/no-progress saturation, unchanged reoffers, hundreds of ambiguous keys, settlement silence and a permitted user reset (`integrations/pi/extension/tests/extension.test.mjs:472`). This does not alter ACK authority or canonical pending mail. The reset implementation still has the new-owner regression below.

## Critical

None found.

## Important

### R1: Recovered urgent proposals cannot establish successor wake intent

`integrations/pi/extension/herdr-threads.js:224` requires `!existing?.waking` before creating an intent. Native reload restores proposals and their item-level `waking: true` witnesses (`:357`, `:361`), but initializes the new owner's `intent` to null (`:358`). On canonical replay of a recovered urgent ID, the new check refuses to create intent, then the existing-record path merely calls `wake` (`:232`), which has no intent to act on.

The result is an admitted, live, idle successor with an empty editor and a complete urgent frame accepted on its current branch but never observed in model-bound context. Replaying that frame with `wakes: true` does not wake it, including repeated reoffers. The per-owner novelty history is empty and no rejection or saturation fence justifies the silence. A user run or another unrelated urgent item is needed to recover delivery. This regresses the existing reload recovery path and conflicts with D3/D5's current-owner startup/reload wake handling and the approved new-owner reset.

Reproduced using the real extension callbacks with controlled IO: start a user run; receive p0 mail; persist its completed-boundary proposal without emitting context; emit shutdown with reason reload; register a successor with the shared registry/current branch; admit reload and connected status; replay the same canonical waking item. Output: `exact accepted current-branch frame retained: 1`; `successor idle wakes after canonical urgent replay: 0` (expected one content-free wake).

Keep known-item suppression scoped to the attention generation/owner, or explicitly re-arm recovered unobserved eligible records on an allowed owner transition. Preserve the same-owner alternating-reoffer rejection fence. Add an urgent accepted-but-unobserved reload regression that asserts one content-free wake, no duplicate body proposal, and no ACK before successor context evidence.

## Minor

None recorded.

## Evidence and limits

- Read the complete appended fix report, including separate RED/GREEN for I1/I2, saturation and duplicate connected status, plus final 46/46 passing callback/native-IO/real-SDK output and clean scoped leak evidence. Those runs do not cover urgent accepted-but-unobserved reload.
- Ran only the specific unanswered reload probe via `HEAVY TOOLBOX node --input-type=module`; exit 0, wrapper slot 2 at 16:26:26. It imported the real extension and controlled callbacks/IO without starting subprocesses or services. No suite/build rerun, source/index mutation, credentials, threads operation or outside-diff source inspection occurred.
- Read the fix diff once for review and subsequently extracted exact source-line references from its addition metadata. Only this review report was written.
- Rust/daemon integration and real TUI/provider qualification remain the already assigned later-task gates; this re-review makes no new claims about them.

Task quality: Needs fixes. The two requested repairs are supported by causal regressions, but retained per-item novelty witnesses now survive an owner reset that discards the wake intent they were meant to protect.
