# Parallelism pass — judgement (ht-p03)

Judge: fresh-context reviewer per super-design graph-pass-prompt.md, autonomous run 2026-10-01.
Inputs: graph-pass-shape.txt (verbatim below), graph-pass-tree.json, §Decomposition "Blocking deps
encode genuine blocking" paragraph + five edge rules.

```
shape: leaves 76 · depth 13 · width 5.8 · critical path: ht-p03.2 → ht-p03.3 → ht-p03.41 → ht-p03.9.3 → ht-p03.9.4 → ht-p03.9.6 → ht-p03.27 → ht-p03.23 → ht-p03.32 → ht-p03.19 → ht-p03.37 → ht-p03.51 → ht-p03.20
summary: edges 345 · exempt 103 · candidates 22 (critical 14, epic-level 8)
```

## Output

change: ht-p03.44 <- ht-p03.23 · keep · capability-gated hook parse-failure report in src/cli/hook.rs (.44's old-daemon acceptance criterion exercises it, needs: ht-p03.23)
change: ht-p03.41 <- ht-p03.3 · drop · safe no · HostPort is one of the traits .3 keeps unchanged (its collapse targets StorePort defaults, DeadlinePort's blanket impl and the forwarding traits), so pane_agent_state can be added to today's HostPort; but both beads edit src/ports.rs and src/scheduler/mod.rs, and .41's consumes: line names .3's post-collapse surface. Removing it alone gives depth 13→12 (ht-p03.9.3 still waits on ht-p03.3 directly)
change: ht-p03.9.6 <- ht-p03.9.4 · keep · the loop inventory and the epic-wide rg 'sleep(...(10|20))' check run over the converted deadline/wake lanes; the edge is real for that half of .9.6 only (see the split proposal)
change: ht-p03.37 <- ht-p03.19 · keep · review-debt.md (findings table with fixed/filed ids, P41–P44 dispositions) feeds closure-ledger rows
change: ht-p03.36 <- ht-p03.12 · narrow · ht-p03.36 <- ht-p03.12.1: v10 live/retention index names and SQL; ht-p03.36 <- ht-p03.12.4: live-row observation walk and work discovery (replaces work_jobs_ready); ht-p03.36 <- ht-p03.12.5: wake discovery from pending projections, human seats excluded; ht-p03.36 <- ht-p03.12.9: reserved-seats-only wake recovery walk; ht-p03.36 <- ht-p03.12.6: retention lane keep set, 24 h work-job pruning, warnings retained; ht-p03.36 <- ht-p03.12.8: HistoryQuery.full_bodies capability · safe yes · removed waits are only .12.3, .12.7, .12.11, .12.13 (CLI read-cost leaves and their seam integration; .12.2/.12.10/.12.12 stay transitive via .12.4/.12.8); their files are src/cli/{mod,follow}.rs and tests, .36's are docs only, and neither side references the other; epic-level gate removed (rule 5)
change: ht-p03.36 <- ht-p03.9 · narrow · ht-p03.36 <- ht-p03.9.1: Lane table, lanes_for_table kick map and the '; retrying (attempt N, next ≤ Xs)' Health suffix; ht-p03.36 <- ht-p03.9.4: deadline/wake lane safety ticks, idle bounds and the docs/operations.md lateness paragraph; ht-p03.36 <- ht-p03.9.5: observation lane Herdr-down backoff (≤ 1 commit per step); ht-p03.36 <- ht-p03.9.6: admission-observer lane and the loop-inventory allowlist · safe yes · removes no leaf wait (.9.6 transitively covers .9.1, .9.2, .9.4→.9.3, .9.5), only the epic-level gate (rule 5)
change: ht-p03.9.4 <- ht-p03.9.3 · keep · WakeDriveOutcome::next_due_at and Refused-seat semantics (two acceptance criteria cite needs: ht-p03.9.3)
change: ht-p03.32 <- ht-p03.23 · keep · shipped Optimistic Health/doctor wording the CHANGELOG entry must match (acceptance needs: ht-p03.23)
change: ht-p03.27 <- ht-p03.9.6 · keep · the admission-observer Pacer lane spawn site whose spawn result .27 checks (consumes only .9.6's first half; see the split proposal)
change: ht-p03.23 <- ht-p03.27 · keep · Health line budget and folding rule; both edit src/daemon/health.rs and src/cli/doctor.rs
change: ht-p03.23 <- ht-p03.11 · keep · rate-limited daemon.log logger that records hook parse failures (acceptance needs: ht-p03.11)
change: ht-p03.20 <- ht-p03.9 · keep · native rerun consumes the whole tip; every leaf genuinely consumed and the reason is stated in .20 (spec §Sequencing); R20 criterion cites needs: ht-p03.9; costs no round beside the .51 sweep; ledger r1-47
change: ht-p03.20 <- ht-p03.14 · keep · whole-tip native rerun (same reason as above; r1-47)
change: ht-p03.20 <- ht-p03.51 · keep · integration sweep result (exempt: integration-sweep edge)
change: ht-p03.20 <- ht-p03.12 · keep · whole-tip native rerun incl. v10 schema and retention lane (r1-47)
change: ht-p03.19 <- ht-p03.32 · drop · safe no · .32 is docs-only (release.md, CHANGELOG, README, install.md) while .19's exclusion set is code; but .19's consumes: line names every B7 bead's merged diff and its fixes may touch README/docs. No depth change alone (ht-p03.23 → ht-p03.44 → ht-p03.19 is also 13)
change: ht-p03.19 <- ht-p03.9 · keep · review exclusion set = merged diff of every B2 leaf (whole epic; r1-47)
change: ht-p03.19 <- ht-p03.44 · keep · seam-integration inline fixes are part of the exclusion set (coverage r2)
change: ht-p03.19 <- ht-p03.14 · keep · review exclusion set = merged diff of every B6 canary leaf (r1-47)
change: ht-p03.19 <- ht-p03.12 · keep · review exclusion set = merged diff of every B1 leaf (r1-47)
change: ht-p03.11 <- ht-p03.9.6 · keep · admission-observer lane whose record_failure reaches LaneErrorLog (acceptance needs: ht-p03.9.6); first half of .9.6 only (see the split proposal)
change: ht-p03.3 <- ht-p03.2 · keep · post-deletion ports/store surface; both rewrite src/ports.rs, src/store/mod.rs and src/identity/reconcile.rs
proposal: ht-p03.9.6, ht-p03.27, ht-p03.11, ht-p03.23 · split-bead ht-p03.9.6 (admission-observer lane on the Pacer | loop inventory + epic rg) · the admission-observer half needs only .7, .9.1, .39; repoint .27, .11 and .23 at it and keep .9.2/.9.4/.9.5 on the inventory half — takes .9.6 off the front chain (depth 13→12, path via .11)
proposal: ht-p03.41, ht-p03.9.3, ht-p03.9.4, ht-p03.42 · seam-contract WakeDriveOutcome::next_due_at · add the next_due_at field (stub None) to the existing wake-outcome contract ht-p03.41 so the wake lane builds against it in parallel with .9.3; .9.4's two refused-seat criteria move to seam integration ht-p03.42 — with the split above, depth 13→11
expected: depth 13→13 · width 5.8→5.8
recommendation: The two safe changes only remove epic-level gates on ht-p03.36 and shorten nothing; the real length is the front chain .2→.3→.41→.9.3→.9.4→.9.6→.27→.23. The split of ht-p03.9.6 and the next_due_at seam contract (plus the safe-no drop of .41←.3) are what would cut it, and each needs a human decision.
