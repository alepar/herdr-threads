# Final review, cluster 1 (docs-evidence), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`; review source bound through `dispatch-text.mjs bind-review-source`, exit 0). Cluster inputs: `docs/evidence/ansi-composer/README.md`, slice `slices/000007.diff`. The file was changed by one commit, 64d684da7 "Codex native wake defers on a /side chat composer" (bead ht-z41e.1, closed).

## Scope and spec mapping

- The cluster's only change edits one bullet of the Codex findings in the ansi-composer evidence README. It adds a parenthetical saying that since ht-z41e.1 the `/side` placeholder reads as text, not empty, because a side chat is not the wake target.
- Spec: design doc `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md`, Section 3 and baseline row B9 ("Stop native wake typing into a side chat. Remove the side prompt from the list of empty composers."). The doc is supporting evidence for B9, not a producer of it.
- B9 delivery (checked beyond the cluster, since it is a named baseline risk): at head, `src/harness/codex/composer.rs` `CODEX_PLACEHOLDERS` holds only `Ask Codex to do anything`, and its doc comment explains why `Ask a follow-up question` is absent. The unit test `captured_side_placeholder_is_not_empty_and_a_side_draft_is_text` reads the byte-exact capture `codex-0.163.0-side` from this evidence directory and asserts `ComposerRead::Text("Ask a follow-up question")`. The production path (native composer read → pre-send refusal → retry) is in `src/host/native.rs`, outside this cluster; the code and seam reviewers trace it.
- No other baseline row touches this file.

## Accuracy of the README at head

- The edited bullet is now correct. Read together with the original sentence, it says the plain read's `Text` classification under `/side` is again the result, and now on purpose.
- The next bullet still ends "So \"dim means empty\" is not safe for Codex, and the `/side` placeholder is added to the known placeholders instead." At head the placeholder has been taken out of `CODEX_PLACEHOLDERS`, so that clause describes the pre-ht-z41e.1 code and contradicts the bullet above it.
- The capture table row "`codex-0.163.0-empty` / `-side` | dim placeholders | Empty | n/a" gives the plain read of `-side` as `Empty`. At head the classifier, and the unit test that loads this exact capture, give `Text("Ask a follow-up question")`. The row should be split, or `-side` marked `Text`. The table's Plain read column records how the current classifier reads each capture, not a 2026-10-23 historical result: the original finding says the first plain read gave `Text` for `-side`, and `Empty` appeared only after the placeholder was added. So the row is stale, not historical.
- The commit's `Outside-adapter` trailer names this file, so the R13 accounting holds.

## Findings

- docs/evidence/ansi-composer/README.md, Codex findings, third bullet: "and the `/side` placeholder is added to the known placeholders instead" is stale at head (ht-z41e.1 removed it from `CODEX_PLACEHOLDERS`) and contradicts the edited bullet directly above it.
- docs/evidence/ansi-composer/README.md, capture table: the row `codex-0.163.0-empty` / `-side` lists the plain read as `Empty` for both captures, while at head `-side` reads `Text("Ask a follow-up question")` (asserted by `captured_side_placeholder_is_not_empty_and_a_side_draft_is_text` against this capture).

## Verdict
not ready (evidence README still says the /side placeholder is a known placeholder and that its capture reads Empty, contradicting ht-z41e.1)

## Must fix before landing
- docs/evidence/ansi-composer/README.md: replace the stale clause "and the `/side` placeholder is added to the known placeholders instead" with the current behaviour (the `/side` placeholder is deliberately not a known placeholder, so a side composer reads as text and native wake defers until the side chat closes).
- docs/evidence/ansi-composer/README.md: split the `codex-0.163.0-empty` / `-side` table row so `-side` shows plain read `Text` (matching `src/harness/codex/composer.rs` and its capture test), keeping `-empty` as `Empty`.

## Untested scope
- none

## Deferred OK
- none
