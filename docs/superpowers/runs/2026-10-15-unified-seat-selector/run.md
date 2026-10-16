# super-auto run — 2026-10-15-unified-seat-selector

flags: planOneShot=t skipPlanRoast=f skipCodeRoast=f autonomous=t
phase: roast-design
codeMechanism: Workflow

idea: Unified seat selector across the herdr-threads CLI. Every place that accepts a seat (send --seat for 1:1 direct send, send --require-ack, invite --seat, inbox/pending-receipts/diagnostics/warnings/thread list --seat, handoff --existing --seat, seat inspect/retire/rebind SEAT and rebind --replace) accepts either an exact seat ID or a name selector. Parse order: exact existing seat ID wins; live pane ID; whole string as one label (labels may contain '/'); then split on '/' into segments assigned to space/tab/pane in order with skipping (1 segment: pane | sole-pane tab | sole-tab-sole-pane space; 2 segments: space/tab, tab/pane, space/pane; 3: space/tab/pane); also accept the read-output agent nickname form SPACE/PANE/SEAT_ID where the trailing seat ID wins and earlier segments must agree with its live location else Conflict. Pane names match pane labels and live agent names. Pool matches from all interpretations, dedupe by pane, then tier by proximity to caller: same tab, same space, anywhere; first nonempty tier wins; multiple distinct panes in that tier -> Conflict listing each candidate with matched interpretation, space/tab/pane path, pane ID, seat ID (bounded, escaped). Missing -> NotFound; host unavailable -> HostUnavailable, never pass names through. --space/--tab may narrow --seat. Read filters only look up existing canonical mappings, never allocate; invite/send/handoff keep ordinary guarded resolution that can allocate for an empty pane. Freeze resolved seat IDs before journal/intent; retries never re-resolve. When a name (not an exact ID) was resolved, print one compact line e.g. `resolved: project/tryout/alice w4:p7 seat=sAb12Cd34 (tier: space)` (~60 bytes); exact IDs print nothing extra. Keep --pane / --require-ack-pane working. Update docs/agent-usage.md (supersedes the no-implicit-cross-space/tab rule), skill text, TRUST-POLICY.md if invariants are touched. Finish: when the integration branch is ready, message the main tab's agent via herdr-threads to merge into main and cut a minor-version release; do not merge or release ourselves.
branch: super-auto/unified-seat-selector
base: main
spec: 2026-10-15-unified-seat-selector-design.md
epic: ht-u3ym
roastDesignRound: 1
approvals:
- top-split · auto · ht-u3ym.1 LEAF, ht-u3ym.2 LEAF, ht-u3ym.3 PROMOTE, ht-u3ym.4 LEAF, ht-u3ym.5 LEAF
- coverage-round-1 · c1 applied auto, c2 applied auto, c3 applied auto, c4 applied auto, c5 applied auto, c6 applied auto, c7 applied auto, c8 applied auto, c9 applied auto, c10 applied auto, c11 rejected auto · R-list: coverage-round-1-requirements.md (R1–R15; R12–R15 from reviewer R-new) · requirements: 11 · mapped: 11 · unmapped: 0 · NEEDS-SPEC ht-u3ym.3 from review-1 not re-dispatched: its two questions were spec gaps, decided directly (c6, c8)
- coverage-round-2 · c12–c20 applied auto, c21 rejected auto, c22–c23 applied auto · divergence: findings 11 → 12 · novel 12/12 (100%) · widening: yes (cap holds at two rounds; design roast reviews the settled tree)
