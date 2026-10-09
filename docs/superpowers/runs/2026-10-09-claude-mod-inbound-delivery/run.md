# super-auto run — 2026-10-09-claude-mod-inbound-delivery

flags: planOneShot=t skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: design
codeMechanism: Workflow
resumeChange: 2026-10-09 · "Goal set: finish super-auto work, prepare integration branch ready for merge into main, notify /herdr tab 'main' for merging and release cutting" · phase 7 prepares the branch (sweep at tip, base absorbed) and hands the merge and release to the Herdr tab 'main' instead of merging locally

idea: build herdr-threads native Claude Code inbound delivery via a herdr-threads Claude Code mod (per spike on branch spike/claude-mod-delivery, docs/research/claude-mod-delivery-spike/README.md, and research ~/Documents/Claude_Code_Mods_Delivery_Research_20261008/report.md): a `herdr-threads watch` streaming subcommand, a bundled mod that checks in (pane env + session id), delivers mid-turn via tool.call context, idle via $.prompt.submit (engine decides idleness; hold after aborted turns; never queue while busy), lazy via $.session.append, acks receipts; daemon routes to the mod while its watch stream is connected and falls back to hooks+send-keys otherwise, resuming from unacked cursor; installer support (CLAUDE_CODE_PLUGIN_DIRS or plugin install); TRUST-POLICY provenance for mod check-in; repeated race stress tests.
branch: super-auto/claude-mod-inbound-delivery
base: main
spec: 2026-10-09-claude-mod-inbound-delivery-design.md
epic: ht-j16
approvals:
- top-split · auto · ht-j16.1 LEAF, ht-j16.2 LEAF, ht-j16.3 LEAF, ht-j16.4 LEAF, ht-j16.5 LEAF, ht-j16.6 LEAF, ht-j16.7 LEAF, ht-j16.8 LEAF (gate), ht-j16.9 LEAF
