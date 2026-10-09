requirements:
R1 → ht-j16.6, ht-j16.7
R2 → ht-j16.1, ht-j16.2, ht-j16.5
R3 → ht-j16.2
R4 → ht-j16.6
R5 → ht-j16.6
R6 → ht-j16.6, ht-j16.3
R7 → ht-j16.3, ht-j16.6, ht-j16.5
R8 → ht-j16.4, ht-j16.2, ht-j16.1
R9 → ht-j16.2, ht-j16.4, ht-j16.10
R10 → ht-j16.6, ht-j16.2
R11 → ht-j16.7
R12 → ht-j16.1, ht-j16.3, ht-j16.4
R13 → ht-j16.6, ht-j16.9
R14 → ht-j16.6
R15 → ht-j16.10
R16 → ht-j16.1, ht-j16.5, ht-j16.7
R17 → ht-j16.1, ht-j16.3, ht-j16.6
R-new: Oversize messages can still be fully delivered while the channel is live → (unmapped)
R-new: During a stall only one path delivers an item → (unmapped)
R-new: A mod loaded where it cannot work neither registers nor retries forever → ht-j16.5, ht-j16.6

findings:
F1 GAP · R9/R17 handoff duplicates vs at-least-once accepted limit (ledger c4)
F2 GAP · digest suppression only on PreToolUse; other hook events may render digest
F3 GAP · truncated items never complete while channel live
F4 GAP · during stall both paths may deliver
F5 GAP · attention marker route and fallback undefined (ledger c5)
F6 GAP · sweep misses /clear-resume, attention marker, operator switch, non-Herdr/old Claude (ledger c1)
F7 UNOWNED-SEAM · exit code for missing HERDR_PANE_ID; mod reaction per exit code; API presence check
F8 UNOWNED-SEAM · meaning of generation across re-registration and /clear
F9 GAP · durability of delivered-unacked ledger across reload/crash (ledger c4)
F10 UNOWNED-SEAM · setup-status live channel read shape (ht-j16.7 vs ht-j16.2)
F11 GAP · operator switch scope for live sessions and setup-status env (ledger c2)
F12 GAP minor · post-abort hold counts toward stall -> native wake into interrupted pane
F13 GAP minor · watch ack failure retry
NEEDS-SPEC: turns without tool calls; $.session.append mid-turn
