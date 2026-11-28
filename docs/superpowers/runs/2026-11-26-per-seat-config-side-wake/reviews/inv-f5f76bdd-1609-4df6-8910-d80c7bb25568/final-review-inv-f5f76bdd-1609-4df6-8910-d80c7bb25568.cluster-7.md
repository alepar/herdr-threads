# Final review, cluster 7 (integrations-codex)

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). Cluster inputs: `integrations/codex/README.md`, slice `slices/000059.diff`. The slice is byte-identical to `git diff base..head -- integrations/codex/README.md`.

## What the cluster changes

One added paragraph in the "Interactive `/side`" section of the Codex README. It says that a native wake is never typed into a displayed side chat, because the side composer placeholder (`Ask a follow-up question`) is not treated as an empty composer and the wake is refused before typing. With `programmatic_wake` off (the default), the wake defers on the refusal backoff (every <= 30 s, no give-up) until the side chat closes. A sender's overdue line shows the refused wake (`wake=`).

## Claims checked against code at head

- Placeholder: `src/harness/codex/composer.rs` at head has `CODEX_PLACEHOLDERS = &["Ask Codex to do anything"]`. The side placeholder was removed, and the unit test now asserts `ComposerRead::Text("Ask a follow-up question")` for the captured side screen. This matches design section 3 and baseline row B9.
- No typing through a stash path: the poke policy (`src/notification/policy.rs`) stashes non-empty input only when `composer_stash == Supported`. The only Codex recipe (`src/harness/codex.rs` `RECIPES`, `codex-hooks-v1`) declares `composer_stash: NativeSupport::Unsupported`, so a `Text` read cannot lead to a stash-and-type into the side chat. "Refused before typing" holds.
- Backoff: `src/notification/dispatch.rs` records every pre-submit exit as `WakeOutcome::Refused`, and the scheduler restores the ladder on `Refused` (`refused_restore`). `src/service/pacer.rs` `Backoff` caps the delay at `CAP_MS = 30_000`. So "every <= 30 s, no give-up" matches. Nothing counts refusals toward a limit.
- Default: `src/protocol/seat_config.rs` registers `programmatic_wake` with `default: "off"` and tier Human. "Off (the default)" is correct, and the README does not mention the removed node setting anywhere.
- `wake=` on the sender's overdue line: this behavior already existed and is what the `side-draft` wake e2e expectation in AGENTS.md asserts. The paragraph adds no new model-visible text.

## Requirement baseline trace for this cluster

- B9 (stop native wake typing into a side chat): the production path is the daemon wake lane, then the native dispatcher, then the Codex `NativeComposer::read`, then a `Text` read and a pre-send refusal. The path is reachable from the real scheduler. The README documents it accurately.
- B8 (wake reaches the root session while `/side` is open, with queue wake on): this README does not describe that case. The behavior lives in other clusters (selection, Codex adapter, AGENTS.md, TRUST-POLICY), so tracing it is the seam reviewer's job. Leaving it out of this README does not mean the row is undelivered.
- B1–B7, B10: no README content in this cluster touches them.

## Findings

- nit: the paragraph covers only the `programmatic_wake` off case. A Codex user reading the `/side` section is not told that with the seat key on (set through `ht human config seat set programmatic_wake on`) the idle wake goes to the root session through the queue while the side chat stays displayed. The doc is incomplete, but nothing in it is wrong (`integrations/codex/README.md:276-280`).
- nit: "With `programmatic_wake` off (the default) it defers" suggests that with it on nothing defers. With it on, a queue-selected attempt that falls back to native also defers on a displayed side composer. Examples are a side that is not yet enrolled being judged by the native gate, bindings across processes, or a platform without queue wake. A clause such as "or whenever the wake falls back to native" would make the paragraph exact.
- nit: "a sender's overdue line shows the refused wake" applies only to items with a deadline (ACK requests with `--deadline`). A plain p0 send shows no overdue line. The current wording reads as if every sender sees it.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The README's description of the native defer and the root wake after the side chat closes depends on the live `side-native-defer/group` wake e2e case (ht-z41e.10). This cluster review found no live run of it at head c3e9ab0be; whole-epic live qualification belongs to the seam and whole-epic reviewers.

## Deferred OK
- Add the `programmatic_wake on` behavior (queue wake to the root session while `/side` is displayed, enabled per seat via `ht human config seat set`) to the Codex README `/side` section.
- Tighten the paragraph to say native fallback also defers, and that the overdue `wake=` line appears only for deadline-bearing items.
