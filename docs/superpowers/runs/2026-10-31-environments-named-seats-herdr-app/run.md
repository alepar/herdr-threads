# super-auto run — 2026-10-31-environments-named-seats-herdr-app

flags: planOneShot=f skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design
codeMechanism: Workflow
runTag: environments-named-seats-herdr-app-20261031T201115
context: Context: claude · planner=200K/167K · implementer=200K/167K · reviewer=200K/167K · finalReview=200K/167K · assembler=200K/167K · warn: planner, finalReview on a 200K model → set ANTHROPIC_DEFAULT_OPUS_MODEL=<model>[1m] or config.models.<role> to a 1M model · warn: implementer, reviewer, assembler on a 200K model → set ANTHROPIC_DEFAULT_SONNET_MODEL=<model>[1m] or config.models.<role> to a 1M model

idea: lets /super-auto the proposal, fully autonomous, both roasts are on — the proposal: docs/superpowers/specs/2026-10-31-environments-proposal.md (named seats via HERDR_THREADS_SEAT_NAME + herdr-core/herdr-app behind one host seam; phases F, N, N2, H1, H2, H3; run files docs/superpowers/runs/2026-10-31-environments/)
branch: super-auto/environments-named-seats-herdr-app
base: main
spec: 2026-10-31-environments-named-seats-herdr-app-design.md
epic: ht-kqrt

approvals:
- reserved-decisions · none reserved by the user; the proposal's Part 5 decisions D1-D21 are taken at their stated recommendations (autonomous run), each recorded as an assumption and surfaced in the report
- phase-7-merge · user (relayed by herdr-threads-dev coordinator snSGTpBk7xGd, message mnSGzr7MbLbk, 2026-10-31): "merge once ready" · at hand-back report to the coordinator with --nudge (branch, tip SHA, unfiltered full-suite + leak-check evidence, roast results); the coordinator reviews and merges to main; this run neither merges nor pushes
- top-split · auto · ht-kqrt.1 LEAF, ht-kqrt.2 LEAF, ht-kqrt.3 LEAF, ht-kqrt.4 LEAF, ht-kqrt.5 LEAF, ht-kqrt.6 LEAF, ht-kqrt.7 LEAF, ht-kqrt.8 LEAF, ht-kqrt.9 LEAF, ht-kqrt.10 LEAF, ht-kqrt.11 LEAF, ht-kqrt.12 LEAF, ht-kqrt.13 LEAF, ht-kqrt.14 LEAF, ht-kqrt.15 LEAF, ht-kqrt.16 LEAF, ht-kqrt.17 LEAF, ht-kqrt.18 LEAF, ht-kqrt.19 LEAF, ht-kqrt.20 LEAF, ht-kqrt.21 LEAF, ht-kqrt.22 LEAF, ht-kqrt.23 LEAF, ht-kqrt.24 LEAF, ht-kqrt.25 LEAF, ht-kqrt.26 LEAF, ht-kqrt.27 LEAF, ht-kqrt.28 LEAF, ht-kqrt.29 LEAF, ht-kqrt.30 LEAF, ht-kqrt.31 LEAF, ht-kqrt.32 LEAF, ht-kqrt.33 LEAF, ht-kqrt.34 LEAF, ht-kqrt.35 LEAF, ht-kqrt.36 LEAF, ht-kqrt.37 LEAF, ht-kqrt.38 LEAF, ht-kqrt.39 LEAF, ht-kqrt.40 LEAF, ht-kqrt.41 LEAF, ht-kqrt.42 LEAF, ht-kqrt.43 LEAF, ht-kqrt.44 LEAF, ht-kqrt.45 LEAF, ht-kqrt.46 LEAF, ht-kqrt.47 LEAF, ht-kqrt.48 LEAF, ht-kqrt.49 LEAF, ht-kqrt.50 LEAF, ht-kqrt.51 LEAF, ht-kqrt.52 LEAF, ht-kqrt.53 LEAF, ht-kqrt.54 LEAF, ht-kqrt.55 LEAF, ht-kqrt.56 LEAF, ht-kqrt.57 LEAF, ht-kqrt.58 LEAF, ht-kqrt.59 LEAF (promotion review: SPLITs applied to N1-site, N3-alloc, N6, H1c; size-PROMOTE N4 resolved as three leaves N4-inherit/N4-hint/N4-paneform, no nested design; run-final real-agent wake e2e tracked by super-auto phase 6, not a bead)
