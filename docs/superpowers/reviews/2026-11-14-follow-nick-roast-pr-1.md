super-roast verdict: Should-fix (6 confirmed)
mode: PR        iteration: 1 of 3
profile (assumed): Not supplied by the caller; inferred from the repository's AGENTS.md and the diff. herdr-threads is a released Rust CLI and Herdr plugin (v3.1.2, public README and CHANGELOG) that carries message threads between coding agents on one user's machine, under a cooperative same-user trust model. The changed surface is the human-facing, read-only `follow`/`read` transcript display, so a defect here misleads a reader but does not lose data and is easy to roll back. Display defects are therefore weighed as cosmetic, while the accuracy of the documented selector contract still matters because the tool has outside users.
inputs: feat/follow-nick@febee63c9 vs main@611fade0f
coverage: scouts 12/12 (correctness, security, premortem, simplicity-design, hot-path-perf, concurrency-async, api-contract, observability, testing, test-causality-lifecycle, test-isolation-environment, hygiene-docs) · raw 40 → deduped 27 → panel 8 · spot 19 · promoted 0 · judge completion 100% · remainder-capped: 0 · review readiness: coverage complete, provenance available — basis: "git diff main..febee63c9 in the captured worktree"
independence: same-family (Claude) — seat-differentiated panel · rung: Workflow
seat-agreement: panels 8 · rr 0.63 · rg 1.00 · fg 0.63 · unanimous 0.63 · ground-loo 1.00 (n=5) · reproduce 6/2/0 · refute 3/5/0 · ground 6/2/0
lane-yield (found/confirmed/unique/refuted): correctness 6/3/2/3 · security 4/2/1/2 · premortem 2/1/0/1 · simplicity-design 5/1/0/3 · hot-path-perf 0/0/0/0 · concurrency-async 0/0/0/0 · api-contract 7/3/1/4 · observability 5/1/0/4 · testing 7/0/0/5 · test-causality-lifecycle 0/0/0/0 · test-isolation-environment 0/0/0/0 · hygiene-docs 4/1/0/2

## Confirmed findings            ← consumed by super-design, one task per finding
- [Should-fix] src/cli/irc.rs:150 — For a remote (synced) agent seat whose pane has no label, `follow` shows the pane id as the pane segment (`ws/tab/w1:p3`, `ws/tab/core:t4`). The stated rule is that the pane segment appears only when the pane is labelled. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `pane_is_id` is `target.is_some_and(...)` (src/cli/irc.rs:150), so it is false whenever `target` is None.
  Remote seats are inserted with no `target_id` column (src/store/seats/publication.rs:214), so `SeatSummary.target` is None.
  The publisher sends the pane as label-else-id (src/service/seat_publisher.rs:220, `pane_label.unwrap_or_else(...)`).
  follow.rs (~616) builds the triple from `row.directory` and passes `row.target`, so the id is pushed as a segment.
  The new tests pass `Some(target)` only (tests/cli/irc.rs:149-166).
  All three seats confirmed by reading the code; none ran it.
  fix-shape hint: have the published directory say whether the pane is labelled (an optional label beside the id; a new wire field needs `#[serde(default)]`), so the client no longer infers it from the local target.
- [Nit] src/cli/irc.rs:178 — Label components are not checked for `/`, so a tab or pane label containing `/` mimics the `workspace/tab/pane` structure. A tab labelled `tryout/alice` with an unnamed pane renders the same as pane `alice` in tab `tryout`, and the pasted string can select that other pane. [lanes: security]
  verdict: confirmed (reproduce ✓ at Should-fix / refute ✗ REJECT, outvoted / ground ✓ at Nit)
  evidence: `nick_label` (src/cli/irc.rs:178) applies only `escape_for_terminal(SingleLine)` and `one_line(NICK_MAX)`; both nick builders then `parts.join("/")`.
  The base nick ended in `/SEAT_ID`, and the nickname path rejected a prefix that did not match the seat's live location; the head nick has no trailing ID.
  CHANGELOG.md:7 says the name "is also a `--seat SPACE/TAB/PANE` selector when the pane is named".
  A branch-style tab label such as `feat/x` is enough to trigger it (reproduce seat).
  The refute seat's dissent rests on the cooperative same-user trust model and on the selector returning `conflict` for several matches; it concludes "at most ... a documentation or hardening Nit", which matches the final severity.
  demoted: the profile is a cooperative same-user local tool where the user sets these labels on their own Herdr, the output is read-only display, and `--show-ids` gives exact IDs, so a misleading `/` in a label is a Nit here and not a Should-fix. No security floor applies: nothing reaches data the invoker could not already reach.
  fix-shape hint: replace or escape `/` inside each nick component, or fall back to the host id for a label that fails the paste-safe rules.
- [Nit] src/cli/irc.rs:150; src/cli/irc.rs:152 — `agent_seat_nick_named` guesses from string shape (`pane == target`, or a `:<target>` suffix) whether the published label-else-id pane string is an id. A real label of that shape is dropped, and the hc `<session>:tN` address format is duplicated in the CLI. [lanes: correctness, security, premortem, simplicity-design, api-contract, observability]
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: src/service/seat_publisher.rs:218-231 merges label and id into one string; src/cli/irc.rs:150-152 reverses the merge by shape only.
  A label such as `deploy:p2` on target `p2` is treated as an id and omitted.
  The refute seat's dissent says a directory entry with no target "is not a realistic input"; the unanimous panel on the Should-fix finding above shows remote seats arrive with `target == None` (src/store/seats/publication.rs:214), so the dissent's premise does not hold.
  The no-target half of this finding is the same defect as the Should-fix finding above; the remaining half is the dropped real label and the duplicated address format.
  fix-shape hint: the same directory change as the Should-fix finding (carry label presence) removes the heuristic; both nick builders can then apply one rule.
- [Nit] src/cli/follow.rs:487 — With `--show-ids`, the ids line for a remote agent seat carries only the seat id and no pane id, while the CHANGELOG promises each author's pane ID and seat ID. [lanes: correctness]
  verdict: confirmed (reproduce ✓ / refute ✗ REJECT, outvoted / ground ✓)
  evidence: `nick.ids = Some(irc::seat_ids_line(target, seat))` uses the local target, which is None for remote seats; `seat_ids_line(None, seat)` returns only `seat.as_str()` (src/cli/irc.rs:166).
  The refute seat cites the doc comment "pane ID when mapped" and the docs line "an unavailable host leaves only the full seat ID". The reproduce seat addresses both and finds they cover the remote case only loosely.
  The directory pane element is a pane id only when the pane is unlabelled, so the fix proposed in the finding is incomplete (reproduce, refute and ground seats agree on this).
  The seat id, which is the exact selector, is always on the line.
  fix-shape hint: state in the CHANGELOG and docs that a remote seat's line shows the seat ID only, or publish the remote pane id explicitly; do not print the directory pane string in the id slot.
- [Nit] src/cli/commands.rs:1988 — Plain human `read` also stops showing seat IDs in agent nicks, but `--show-ids` is `requires = "follow"`, and the CHANGELOG describes the change only as a `follow` change. [lanes: api-contract]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: src/cli/mod.rs:1178 builds the History `NickCache` with `.with_caller(caller_pane)` and no `.with_show_ids`; the only `with_show_ids` call is at src/cli/follow.rs:1181.
  docs/agent-usage.md says "Plain `read` uses the same IRC transcript in its human form".
  `read --json`, machine output and `read --follow --recent N --show-ids` still carry the IDs, so impact is small.
  fix-shape hint: either accept `--show-ids` on plain `read`, or say in the CHANGELOG that plain `read` names change too.
- [Nit] CHANGELOG.md:7 — The CHANGELOG and docs claim the agent nick is a `--seat SPACE/TAB/PANE` selector and drop the earlier caveat, but the nick is rendered with `nick_label` (terminal escape, 32-character clip) and not the paste-safe renderer, so clipped, escaped, `/`-containing, ID-shaped or sibling-colliding labels do not round-trip. [lanes: api-contract, hygiene-docs]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: `nick_label` (src/cli/irc.rs:178-186) runs `escape_for_terminal` then `one_line(.., NICK_MAX)` with NICK_MAX = 32; name matching is exact and case-sensitive (docs/agent-usage.md:81).
  The diff removed "a `read`/`follow` nick verifies only when its labels were shown unaltered, so clipped (32 chars) or escaped labels give that conflict" and added the unconditional "a `read`/`follow` agent nick is a `SPACE/TAB/PANE` name".
  The claim is correct for ordinary short, safe labels; the defect is a documentation overclaim.
  fix-shape hint: qualify the selector sentence (label shown unaltered and paste-safe), or render the nick with the paste-safe renderer.

## Not verified (beyond panel cap)   ← severe candidates the panel cap left unverified — listed, never dropped
- none

## Not verified (dedupe failed or judge lost)   ← findings a pipeline failure left unjudged — listed, never dropped
- none

## Beyond remainder cap (count only)   ← low-severity candidates the dedupe remainder cap dropped; the count survives, the claims do not
- none

## Rejected (with reason)        ← so re-roasts don't re-litigate
- src/cli/irc.rs:127; src/cli/irc.rs:144 — Two agent seats in one workspace and tab with unnamed or identically labelled panes render identical nicks and colours by default.
  reason: rejected 3-0. The mechanism is accurate, but the stated intent (bead ht-io66) is exactly this: pane segment only when labelled, no seat id by default, `--show-ids` as the disambiguator.
  The CHANGELOG ("The seat ID is no longer part of the name") and docs ("two unnamed panes of one tab read the same") state the tradeoff.
  The deliberate-collision variant is adversarial verification, which AGENTS.md and TRUST-POLICY exclude (cooperative, same-user, labels advisory).
- src/cli/irc.rs:120 — A nick pasted back from `read`/`follow` no longer selects the author's exact seat; it resolves to the pane's current occupant.
  reason: rejected 3-0. This follows directly from "no seat id by default".
  docs/agent-usage.md:81 was updated in the same change: the nick is a `SPACE/TAB/PANE` name and the `--show-ids` pane or seat ID "selects the seat exactly"; the `resolved:` stderr line reports name resolution.
  The "breaking, needs a major version" point is a release-policy opinion about human display text, not a defect against the spec.

## Unverified nits (spot-checked)
- [FYI] src/cli/follow.rs:486 — `--show-ids` also adds an ids line to person seats and to the unresolved fallback nick, where it repeats the seat id. Spot check: REJECT; the bead's "unchanged" refers to the default name column, and the CHANGELOG says "each author".
- [FYI] src/cli/irc.rs:386; src/cli/irc.rs:384 — `-!-` join/leave and warning notices pass `ids = None`, so `--show-ids` does not disambiguate them. Spot check: REJECT; a notice has no name column, and the spec promises the line under an author's name only.
- [FYI] src/cli/irc.rs:166 — The ids line prints a bare host target id with no host qualifier, while the docs say the shown pane ID selects the seat exactly. Spot check: REJECT; the seat ID on the same line is exact, and host-scoped resolution of a bare pane ID was inferred from a comment, not run.
- [FYI] src/cli/follow.rs:498 — The directory naming path (`agent_seat_nick_named`) has unit tests on hand-written strings only, with no follow-level test of the unnamed-pane inference. Spot check: REJECT; the published fallback is built from the same target, so the spelling mismatch cannot occur with the current publisher. The spot seat did not consider the `target == None` remote case confirmed above.
- [Nit] docs/agent-usage.md:275 — The paragraph still says human transcripts name authors relative to the invoking pane and that "the pane component stays visible", which is now false for agent authors. Spot check: CONFIRM; the neighbouring paragraph at line 129 was updated and this one was not.
- [FYI] src/cli/irc.rs:497; src/cli/irc.rs:495 — The ids line has no width bound and sits one column right of the body indent when a wide nick forces indent 8; these branches and the folded-body case are untested. Spot check: REJECT; the column under the nick text is deliberate and the effect is cosmetic.
- [FYI] tests/cli/cooperative.rs:4737 — The round-trip test now checks name resolution instead of the seat-ID-verified nickname form, and no test pastes the unnamed-pane `SPACE/TAB` form back. Spot check: REJECT; the change follows the new contract and generic name-resolution tests cover conflicts.
- [FYI] tests/cli/read_cost_names.rs:1077 — The negative guard `!shown.contains("project/alice")` can no longer fire, because the pooled fallback now renders `project/tryout/alice`. Spot check: REJECT as immaterial; the assertion is dead, but the two positive assertions on lines 1075-1076 cover each header.
- [FYI] src/cli/follow.rs:1181 — `--show-ids` is accepted with `--json`/`--machine` or non-TTY output and silently has no effect. Spot check: REJECT; the flag is presentation-only and the JSON already carries the seat id.
- [Nit] src/cli/commands.rs:556 — No test checks that `show_ids` survives the picker path (`follow --show-ids` with no THREAD); dropping the field in `PickerRequest::selected` would pass the suite. Spot check: CONFIRM; the only Picker assertion (tests/cli/commands.rs:1677) ignores `show_ids`.
- [FYI] src/cli/follow.rs:672 — In the resolve fallback, `--show-ids` prints only the seat ID, so an unmapped pane and a failed lookup look the same. Spot check: REJECT; this is the documented degraded path ("pane ID when mapped").
- [FYI] src/cli/irc.rs:120 — The `workspace/tab[/pane]` format is built by two functions with different pane-omission rules. Spot check: REJECT; the inputs differ (raw optional labels versus resolved label-else-id strings), so only the final join could be shared.
- [FYI] docs/agent-usage.md:129 — The docs record the identical-name outcome as fact, without the rationale for accepting it. Spot check: REJECT; the docs state the limit and the `--show-ids` remedy together.
- [Nit] src/cli/irc.rs:493 — `emit()` now wraps the body and lays out the ids line, with two duplicated ids-printing blocks, a one-line-body special case and `#[allow(clippy::too_many_arguments)]`. Spot check: CONFIRM as a maintainability nit; output is correct.
- [FYI] src/cli/follow.rs:352 — `--show-ids` is stored in `NickCache` and baked into cached nicks, and the cache-clear branch in `with_show_ids` never fires. Spot check: REJECT; it mirrors the sibling builder `with_caller`.
- [FYI] src/cli/follow.rs:169 — `SeatBinding` is a nested positional tuple whose label-or-id meaning lives only in a doc comment. Spot check: REJECT; the aliases are file-private and used at two sites.
- [FYI] src/cli/irc.rs:141 — The public `cli::irc` Rust API changes (`agent_seat_nick_named` signature, new required `Nick.ids` field). Spot check: REJECT; the surface is internal to the CLI and its tests, with no library contract.
- [Nit] tests/cli/read_cost_names.rs:511 — The test name `agent_event_recipient_missing_names_uses_host_ids_without_tab_alias` contradicts its assertion, which now expects the tab label (`w2/tryout`). Spot check: CONFIRM; the rename was missed.
- [FYI] src/cli/irc.rs:5 — One module doc comment line is about 114 columns where its neighbours wrap near 80. Spot check: REJECT as cosmetic; rustfmt does not reflow comments.

## Escalations (need human)      ← UNVERIFIED externals, incomplete panels, unsettled panels, material dissent
- none