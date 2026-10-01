- [2026-09-27 design/coverage-1] Automatic approval review rejected local Claude Opus batch submission of complete repository design inputs as export to an untrusted destination. No batch ran. Continue with built-in collaboration reviewers; keep exact bounded inputs and model provenance. — Skills assume Opus/Task routing, while available built-in tools expose GPT models and dynamic full-prompt fan-out requires explicit assembly.
- [2026-09-27 design/coverage-1] Built-in collaboration requires literal prompt strings; thirty exact input windows total 2,627,571 bytes. Dedicated assembler could not dispatch all within output limits. Adopt one immutable-file bootstrap read per fresh built-in reviewer, then prohibit further tools; mark coverage advisory and park the qualifier rather than claiming prompt-only compliance. — A tool-supported artifact-to-prompt attachment would preserve the intended no-roaming boundary without retranscribing megabytes.
- [2026-09-27 design/coverage-1] The single-read bootstrap truncated even the smallest52KB input despite raised output limits. Two seats exhausted their one allowed redispatch and remain degraded. Remaining fresh seats load bounded chunks of the same immutable hashed file, with no other evidence access; no retry cap reset. — Tool output caps need explicit chunk-safe input attachment support.
- [2026-09-27 roast-design/1] Manual judge transport saves JSON both in a result file and a final response; two seats paraphrased only their evidence text between those outputs, triggering the documented fail-closed mismatch rule and one fresh retry each. Both versions and invalid attempts were preserved; pending seats now read back and copy their saved JSON verbatim. — This duplication is a local manual-fallback adaptation, not a defect proven in the upstream Workflow engine; record the extra dispatch cost for finish analysis.
- [2026-09-27 roast-design/1] In the four-slot built-in collaboration harness, a live root plus a nested review coordinator leaves two slots for judges. Root resumed direct dispatch at 29/51 completion, allowing three simultaneous judges while the coordinator remains idle until reporting. — Manual fallback orchestration must count coordinator slots explicitly; observed concurrency changed from two to three, without claiming a measured wall-time speedup.

## 2026-09-27 — SDD helper invocation

- Phase: super-code setup.
- Observation: installed scripts/sdd-workspace lacks executable permission; direct invocation returned exit 126.
- Workaround: invoke the unchanged Bash script using bash; canonical workspace creation passed. No permission change or duplicate workspace.

## 2026-09-27 — Worker thread admission

- Phase: super-code first dispatch.
- Observation: contracts and Codex workers started; third Claude spawn twice returned agent thread limit reached despite configured cap3. Live list showed root, two workers and a completed older reporter. Interrupting completed reporter did not free admission.
- Handling: retain task open/queued and use first freed slot; effective concurrency2, no task failure or retry outcome fabricated.

## Task3 report path ambiguity
Root dispatch abbreviated report location as integration/.superpowers instead of absolute path. Worker committed it literally inside task checkout. Root copied exact report into coordinator W and removed duplicate in housekeepingcommit120c97a before review. Use absolute report paths in every dispatch; no product code changed.

- [2026-09-27 roast-design/3] Late-round scout prompt treats zero material findings as valid, while reporter Step4 mandates low coverage on any nontrivial artifact with zero raw findings; engine also reports 0% judge completion when no judges are needed. Preserve literal reporting rules and do not manufacture candidates — these contracts can prevent convergence after an otherwise complete empty late-round review.

## Task48 task-brief default helper permission failure
Default task-brief exit126: nested scripts/sdd-workspace is non-executable. Official helper rerun with explicit OUTFILE succeeded (21 lines), bypassing nested invocation. No skill mutations.

## Startup175 historical prefix metadata unavailable
Initial combined output clipped; exact visibleprefix/hash was not retained. OriginalCLEAN175 and truthful supplement preserved. Bounded freshfullproposalrevalidation176 queued before rootadoption/sourcecorrection; no fabricatedoldreadID.

- 2026-09-30 · super-code/SKILL.md:50-54 ("nothing runs per merge") · per-merge merged-tree `cargo check` caught 3 real cross-branch compile seams (b0d24c5, 80f90cc, bf00d19) that lane tests could not see; recommend keeping a build-only check per merge. Sent to the superpowers session.
- 2026-09-30 · super-auto/SKILL.md:28-61, super-code/SKILL.md:12-32 ("don't end a turn") · conflicts with Claude Code's background Workflow/Agent model, where ending the turn is how the coordinator waits for completion notifications; needs a carve-out. Sent.
- 2026-09-30 · super-auto/SKILL.md:73-96 (pre-flight version check forbids consulting a local checkout) · a mid-run switch to local definitions has no defined procedure; mapped forward in run.md. Sent.
- 2026-09-30 · coordinator error · a previous-round workflow script was relaunched by mistake (wpmvcpf51) and stopped within seconds, with no worktree or report changes; a `git stash push` on a lane worktree was used briefly for a mutation check and restored by SHA, with the entry dropped.
- 2026-09-30 merge-back: a lane agent's identical evidence copy was untracked in the integration worktree and blocked 'git merge' silently; the check loop didn't test the merge exit, so it reported 'check pass' on a no-op. Recovered with diff -r + rm + re-merge. Proposed upstream: require MERGE_HEAD plus a clean porcelain check before mergeCheck.
- 2026-09-30 root cause of the stray integration write: the coordinator's own wave-4 lane prompt gave an absolute integration RUN path for evidence output. Guards are now applied for all merges: clean porcelain first, then merge exit 0 plus MERGE_HEAD, then mergeCheck.
- [2026-10-01 fix-loop/regression-pass] The regression-only pass (no re-roast) merged a fix (ht-4is.37) whose focused tests passed but which broke an existing lib test for a sibling caller of the same helper; only the phase-6 full serial suite caught it (fixed in ad905554 by scoping the allow-list to cooperative intents). — With no re-roast, the phase-6 sweep is the only net for a regression pass; worth stating that a failing sweep re-enters the pass rather than reporting, and that lane prompts should run the tests of every caller of a changed shared helper.
- [2026-10-01 report] report-writing subagent's Write of report.md was refused by the harness ("subagents should return findings as text"); coordinator wrote it from the returned text. — super-auto phase 6 does not say who persists report.md when authorship is delegated.
