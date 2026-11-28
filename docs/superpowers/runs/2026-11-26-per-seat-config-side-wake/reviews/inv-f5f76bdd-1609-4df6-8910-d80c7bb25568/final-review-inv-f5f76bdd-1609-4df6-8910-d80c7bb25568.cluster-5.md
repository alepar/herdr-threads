# Final review, cluster 5 (docs-validation)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Caller checkout observed clean on branch super-auto/per-seat-config-side-wake at the start; nothing was modified apart from this file and the bind-review-source ledger append.

Cluster input: `docs/validation/herdr-release-matrix.md`, slice `slices/000051.diff`. It is one changed row, introduced by commit 64d684da7 ("Codex native wake defers on a /side chat composer").

## What changed

The Wake row of the coverage table used to read "p0 under Codex's `/side` placeholder", which meant the matrix expected a wake while `/side` was displayed. It now reads "no wake under Codex's `/side` placeholder, then a p0 wake once it closes".

## Trace against the spec

The requirement baseline in `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` was checked. The only baseline row this cluster touches is B9 ("Stop native wake typing into a side chat. Remove the side prompt from the list of empty composers." / "lgtm").

- Production path at head: `src/harness/codex/composer.rs` `CODEX_PLACEHOLDERS` now holds only `"Ask Codex to do anything"`. The `/side` placeholder `Ask a follow-up question` therefore reads as `ComposerRead::Text`. The native wake gate refuses to type into a composer that is not empty, and the refusal backoff retries the wake. So no wake is typed while the side chat is displayed, and the next attempt after it closes wakes the session. The unit test `captured_side_placeholder_is_not_empty_and_a_side_draft_is_text` runs this against the captured evidence (`docs/evidence/ansi-composer/`). The module doc in `src/harness/composer.rs` was updated to match.
- The validation row matches its test: `tests/integration/herdr_release_smoke.rs` `herdr_release_wake` now submits `/side`, sends `--nudge` (p0) and waits 10 s. It asserts that no wake was typed, sends Ctrl-C, waits for the stand-in's `side-close` event and then waits up to 60 s for the wake. `tests/fixtures/herdr-matrix/codex.js` gained a Ctrl-C branch. With the side placeholder showing, Ctrl-C restores the main placeholder and logs `side-close` with no Interrupt hook. The fixture's docstring describes this.
- The row's wording therefore matches what the production code does and what the test asserts. It also uses the same terms as the "Wake" column entries next to it (priority/threshold phrasing, "p0").
- B9 is delivered by the production composer change. The matrix doc only records how it is checked. Composition with the queue-wake and side-routing work (B8) belongs to the seam reviewer and is not judged here.

## Accuracy of the rest of the document

- The Wall time paragraph says "one release is about 130 s ... about 65 s for the others, which run alongside it". The wake test now adds a fixed 10 s sleep plus a wait after the side chat closes that can last up to the per-seat wake spacing. The test runs in parallel with the move test, so the 130 s per-release figure probably still holds. The "about 65 s" for the other tests may now understate the wake test. Nobody has measured it at head.
- The stand-in description (paragraph 2) does not mention `/side` or Ctrl-C. The authoritative description is in the fixture's own docstring, which was updated, so leaving it out here is acceptable.

## Verdict

ready

## Must fix before landing

- none

## Untested scope

- `herdr_release_wake` (ignored, needs a private Herdr through `scripts/herdr-release-smoke`) has not run at head c3e9ab0b. The inverted /side expectation that this doc row describes and the new codex.js Ctrl-C side-close branch have therefore never run against a real Herdr release. The run is tracked as an open gate in ht-z41e.10 (live qualification at the final SHA) and in the roast-pr-1 final-review key "run ignored herdr_release_wake on the final SHA". The row's claim of coverage only holds once that evidence is recorded.

## Deferred OK

- The Wall time paragraph's "about 65 s for the others" may understate the wake test now that it has a 10 s no-wake hold and a post-close wake wait. Refresh the figure from the ht-z41e.10 matrix run instead of estimating it.
