# Review intent recovery diagnosis

Recommendation: preserve the original failed invocation and its immutable source, then use the documented bounded captured-source / settled-null recovery path. A metadata-only correction with the normal commit-before-launch ordering cannot establish a complete whole-range review with the installed assembler. A documented orchestration ordering exception can avoid the self-reference after original preservation succeeds; see the final section. Do not delete the original launch record, relabel the unavailable result clean, or retarget its captured head.

## Observed facts

- Workspace: `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-profile-and-skill-guidance`.
- Original invocation: `inv-8759cc82-264d-4506-beaa-307c9600bb8d`.
- Immutable source recorded in progress.md: base `b4d5f9628d17f869ab5b139b7f4ae3c933d02841`, head `02a7cb85237aba386eaa9d4d9b1d96357e0a9ad7`.
- run.md currently retains one prelaunch review-collection with reviewPath null, review pending, required [], settled false.
- Existing assembled/final-review-ID-final/review-inputs.json has sourceAvailable true, status unavailable, wholeRangeReady false, no omitted files, no slices, full changed-path inventory, and both incomplete claimed collection and invalid null-review bundle gaps. Its companion diffstat and generated-input-provenance.md exist. Preserve all these exact bytes.
- The ledger reaches active0 / implementation-drained and has a persisted reviewSource. This supports implementation drainage, not by itself a claim that the coordinator has returned or invocation settlement was confirmed. The parent must confirm actual settlement/result before collecting.

## Installed contract and implementation

Read from `/Users/alepar/.codex/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar6.2/skills`:

- super-auto/run-state.md, Review collection records, lines 224–253: expected non-null path before launch; settlement confirmation; append-only records; explicit settled-null recovery; bounded reports preserve failed selection; exit 3 blocks re-entry/cleanup until preservation succeeds.
- super-auto/scripts/report-status, lines 398–421: supplying a non-null path directly over this existing unsettled null path fails with invocation identity or required bundle changed. `--recover-null` requires a preceding settled-null record, same workspace and ordered required list, exact expected report path, strict extraction, matching actual verdict.
- super-auto/scripts/collection-records.cjs: a retained valid settled-null transition needs the hash-bound recovery marker on every subsequent snapshot. The helper supplies it; do not synthesize it.
- super-code/scripts/dispatch-text.mjs, lines 155–235: review-inputs parses collection history from captured Git trees, considers latest snapshots, and flags any latest !complete or !settled collection. It has no exemption for the current invocation's valid pending launch intent.
- dispatch-text.mjs, lines 316–345 and 358–405: source binding is immutable for an invocation; bounded-review validates old captured source, inventory and real hunk headers, writes only the expected final report exclusively, and forces not-ready.

The null launch path is a bookkeeping mistake, but replacing it with the expected path leaves incomplete claimed collection. Even a fully correct fresh invocation whose pending prelaunch intent is committed in its captured head encounters that same gap. Therefore no supported invocation/collection command sequence here promises a fresh whole-range assembly with zero provenance gaps. Resolving that general self-reference requires a separately authorized skill/tool fix or supported upstream workflow change. Do not work around it by hiding/removing the current intent, using an earlier head that omits real changes, omitting archives, or bypassing provenance checks.

## Supported bounded preservation sequence

These are commands for the parent, not commands run by this diagnostic agent. Shell variables below are task-local. Run in the integration worktree. Check each exit; do not chain through failures.

```sh
skills=/Users/alepar/.codex/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar6.2/skills
worktree=/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-profile-and-skill-guidance
run="$worktree/docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/run.md"
workspace="$worktree/.superpowers/sdd/ht-1urq-plan"
ledger="$workspace/progress.md"
invocation=inv-8759cc82-264d-4506-beaa-307c9600bb8d
assembled="$workspace/assembled/final-review-$invocation-final"
base=b4d5f9628d17f869ab5b139b7f4ae3c933d02841
head=02a7cb85237aba386eaa9d4d9b1d96357e0a9ad7
bash "$skills/super-code/scripts/dispatch-text" --ledger "$ledger" bind-review-source --read
```

1. Confirm the invocation has actually settled and retain its exact returned review result. If it returns null with an unavailable/not-ready verdict, settle that actual result first. Substitute its exact returned verdict below; this placeholder is deliberately not an invented result. Keep required [] unchanged (no --required flags).

```sh
bash "$skills/super-auto/scripts/report-status" --collect "$run" --invocation "$invocation" --workspace "$workspace" --review none --verdict 'ACTUAL RETURNED VERDICT'
```

Expected exit 3 while the null report has recoverable artifacts. It appends the honest settled-null record, retains the original launch record, and archives available evidence with explicit gaps. Never supply no work landed: implementation did land.

2. Obtain a fresh bounded reviewer of the original captured Git source. Have that reviewer create only `$assembled/bounded-draft.md` and `$assembled/bounded-coverage.json`; these are not yet present and this diagnosis does not create them. Use strict final-review headings, preserve all actual findings, list full changed-path inventory plus all original selection gaps and remaining paths/hunks. coverage JSON shape is `{"inspected":[{"path":"actual changed path","hunks":["@@ exact captured hunk header @@"]}]}`. Each claimed header must have actually been inspected and match `git diff --no-renames "$base..$head" -- PATH`. A report that merely repeats this diagnosis is not a review.

```sh
bash "$skills/super-code/scripts/dispatch-text" --ledger "$ledger" bounded-review --base "$base" --head "$head" --selection "$assembled/review-inputs.json" --draft "$assembled/bounded-draft.md" --coverage "$assembled/bounded-coverage.json" --out "$workspace/final-review-$invocation.md"
```

This writes a new bounded report at the expected path without overwriting original failed assembled evidence. Its forced verdict is `not ready (unreviewed: whole-epic coverage and omission provenance remain incomplete)`. It does not change source capture or establish whole-range acceptance.

3. Reconcile the retained settled-null event with the newly created honest bounded report using the documented explicit recovery, then replay collections:

```sh
bash "$skills/super-auto/scripts/report-status" --collect "$run" --invocation "$invocation" --workspace "$workspace" --review "$workspace/final-review-$invocation.md" --verdict 'not ready (unreviewed: whole-epic coverage and omission provenance remain incomplete)' --recover-null
bash "$skills/super-auto/scripts/report-status" --collect "$run"
```

Exit 0 means evidence preservation, not review acceptance. Exit 3 means preserve sources and archives and reconcile the named gap before re-entry. Exit 2/4 establishes no collection. Do not edit the hash-bound recovery metadata manually. Collection discovers bounded draft/coverage in the existing assembled directory without altering required []. Commit the appended history, archived report/input tree, original gaps and profile/friction records together at the existing settlement boundary.

## Subsequent full fresh invocation

Only after preservation succeeds, and only if the parent elects a further review, mint a new invocation ID and commit the correct expected non-null prelaunch path with settled false. Pass the unchanged runTag. Bind a fresh full captured source for that new ID after the preservation commit; do not reuse or rebind the original ID. Keep the old ledger/Launch/source evidence retained when adding the new Launch. The original source needs no recapture for bounded recovery; a changed head necessarily needs a fresh invocation.

To diagnose fresh selection independently, use an empty/nonexistent directory, retaining the old assembly untouched:

```sh
bash "$skills/super-code/scripts/dispatch-text" --ledger "$ledger" review-inputs --base "$new_base" --head "$new_head" --out-dir "$new_selection_dir"
```

The parent supplies actual full committed SHA values and a new destination. With an active committed pending launch record, this installed implementation still returns unavailable/incomplete claimed collection. Exit 3 and all evidence must be retained; no command flag in review-inputs suppresses that condition. A fresh bounded direct review can still inspect all product changes honestly, but remains forced not-ready and cannot discharge the whole-range acceptance obligation. Report this limitation to the run owner rather than claiming a clean full review. A supported tool correction must be reviewed separately if whole-range review is mandatory.

No project source, tests, git state, run records, collection records, artifacts or threads were mutated by this diagnosis. Only this report was written.


## Parent-proposed explicit ordering exception: semantically viable after preservation

The parent proposes persisting the new invocation intent in the working run.md before launch, but committing it only after settlement/collection. This is an explicit exception to the skill's literal commit-before-launch ordering, not a native helper option. If the parent selects and documents that routine workflow exception under its existing authorization, the helper semantics support a fresh full review without suppressing evidence:

1. Confirm the original invocation actually returned null/unavailable. `--review none --verdict unavailable` is a valid actual result (unlike invented no work landed). Its collection exits 3 until a report is recovered; raw assembly archival alone does not pass the preservation gate.
2. Complete the bounded report and settled-null recovery commands above. Require replay exit 0. Commit old original/recovery history, complete archive and profile/friction records. Its forced not-ready report and original gaps remain ordinary review inputs, with finding-source occurrences retained; preservation is not acceptance.
3. Before capturing a new source, ensure every existing review input / actual change that needs review is committed. Capture full immutable base/head after that old-preservation commit. The new captured head therefore contains a completed prior collection and no new active pending collection. Do not choose an old head that excludes actual changes.
4. Mint a fresh invocation. Append its correct expected-path pending intent to the working run.md before launch, retain a durable local record of the explicit commit-order exception and exact source, and pass the new ID/unchanged runTag. Retain original ledger/source by appending a new Launch rather than rewriting the old one. Keep tracked run files unchanged while the invocation is running. Do not commit the new pending record into its own source head.
5. Bind the new ID to the newly captured base/head before output artifacts. Run the normal full assembler/review with a new assembled destination. `review-inputs` enumerates only captured Git run.md records, so it will see the old completed recovery and will not see the new working-tree-only pending intent. Current fresh outputs did not exist at capture; their absence is not an omission of an existing source input. It should not emit the current-intent gap for this reason; inspect every returned gap rather than assuming success.
6. On actual settlement, collect using the fresh ID, exact workspace/review path and actual returned verdict. Replay all collections and require exit 0 before cleanup. Commit the fresh original intent plus helper-appended settlement/completion history, findings, archive, profile/friction and the ordering exception together. Old gaps remain preserved in old report/sidecar/history and should be explicitly evaluated by the fresh review; a fresh complete selection does not retrospectively erase them.

This exception removes the circular metadata dependency; it does not grant a clean verdict or skip original collection. No re-capture of the original invocation is permitted or needed; the second source belongs only to the fresh invocation. Without electing this documented ordering exception (or a supported tool correction), the standard committed-current-intent sequence remains self-referential as described above.


## Bounded reviewer output and raw continuation naming

Use the original `$assembled` directory and source bindings from the command block above. New files may be added there for collection; do not rewrite its existing review-inputs.json, generated-input-provenance.md, diffstat.txt or slices directory. The reviewer should save the full immutable raw diff and inventory once with exclusive creation, then read bounded chunks and record every truncation/continuation:

```sh
(set -C; git -C "$worktree" diff --no-renames "$base..$head" > "$assembled/bounded-raw-source.diff")
(set -C; git -C "$worktree" diff --no-renames --name-only "$base..$head" > "$assembled/bounded-raw-inventory.txt")
sed -n '1,180p' "$assembled/bounded-raw-source.diff"
sed -n '181,360p' "$assembled/bounded-raw-source.diff"
```

Continue only ranges actually needed; a saved raw file does not mean the reviewer read it. If a tool truncates a page, read the missing continuation before claiming that coverage. Save the actual inspected ranges / missing continuations in `$assembled/bounded-continuations.md`. New names belong to this original bounded recovery; fresh full invocation uses its own final-review-NEW_ID-final directory and does not reuse these destinations.

Required actual reviewer outputs:

- `$assembled/bounded-draft.md`: strict final-review report, including `## Verdict` and `## Untested scope`; retain every actual finding, original two selection gaps, full changed-path inventory, actual inspected hunks and unreviewed remainder. Extract with report-verdict to confirm strict format before bounded-review.
- `$assembled/bounded-coverage.json`: exactly an object with an `inspected` array. Each item is `{"path":"repo/relative/changed-file","hunks":["@@ -OLD_START,OLD_COUNT +NEW_START,NEW_COUNT @@ optional actual trailing context"]}`. At least one inspected item and one real exact hunk header per item; unique changed paths; all headers must be copied verbatim from the original captured Git diff and actually inspected. Include all inspected hunk headers, not invented summarized locations. JSON may be pretty printed. The helper validates changed-path membership and exact header presence but the reviewer owns the honesty of coverage.

Exact final extraction / production / recovery commands, in order after a real bounded reviewer has written those files:

```sh
bash "$skills/super-code/scripts/report-verdict" --complete --source "$invocation/bounded-draft" "$assembled/bounded-draft.md"
bash "$skills/super-code/scripts/dispatch-text" --ledger "$ledger" bounded-review --base "$base" --head "$head" --selection "$assembled/review-inputs.json" --draft "$assembled/bounded-draft.md" --coverage "$assembled/bounded-coverage.json" --out "$workspace/final-review-$invocation.md"
bash "$skills/super-auto/scripts/report-status" --collect "$run" --invocation "$invocation" --workspace "$workspace" --review "$workspace/final-review-$invocation.md" --verdict 'not ready (unreviewed: whole-epic coverage and omission provenance remain incomplete)' --recover-null
bash "$skills/super-auto/scripts/report-status" --collect "$run"
```

Before these commands, the actual original null/unavailable settlement command is:

```sh
bash "$skills/super-auto/scripts/report-status" --collect "$run" --invocation "$invocation" --workspace "$workspace" --review none --verdict unavailable
```

Use that literal unavailable only if it is the actual confirmed returned result; expect exit 3, retain it, and continue the original recovery rather than starting a fresh invocation at that point. The recovery command's actual bounded verdict is fixed by the writer above. Do not add --required flags: the original ordered list was []. Normal discovered assembled-tree collection preserves every raw source/continuation/draft/coverage file added above, plus the original failure evidence.
