requirements:
R1 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.13
R2 → ht-kqrt.7
R3 → ht-kqrt.5, ht-kqrt.6, ht-kqrt.8
R4 → ht-kqrt.4, ht-kqrt.44
R5 → ht-kqrt.9
R6 → ht-kqrt.10, ht-kqrt.9
R7 → ht-kqrt.10, ht-kqrt.11
R8 → ht-kqrt.11
R9 → ht-kqrt.2
R10 → ht-kqrt.13 (the gate itself is not wired as a blocker; see finding 6)
R11 → ht-kqrt.16, ht-kqrt.18, ht-kqrt.20
R12 → ht-kqrt.18, ht-kqrt.19
R13 → ht-kqrt.18
R14 → ht-kqrt.23, ht-kqrt.24
R15 → ht-kqrt.24
R16 → ht-kqrt.34, ht-kqrt.30, ht-kqrt.18
R17 → ht-kqrt.27
R18 → ht-kqrt.21, ht-kqrt.15, ht-kqrt.25 (the clause "one unresolved named row does not silence liveness of other named bindings" has no owner; see finding 8)
R19 → ht-kqrt.28
R20 → ht-kqrt.31, ht-kqrt.32, ht-kqrt.25 (the "live-mod Claude on a Named site" clause depends on finding 3)
R21 → ht-kqrt.26
R22 → ht-kqrt.22
R23 → ht-kqrt.33
R24 → ht-kqrt.30, ht-kqrt.20
R25 → ht-kqrt.30
R26 → ht-kqrt.17, ht-kqrt.29, ht-kqrt.30, ht-kqrt.15
R27 → ht-kqrt.25, ht-kqrt.6
R28 → ht-kqrt.17, ht-kqrt.32, ht-kqrt.21, ht-kqrt.22, ht-kqrt.25, ht-kqrt.26, ht-kqrt.27 (the "TRUST-POLICY carries the N rows" clause is partly unowned; see finding 2)
R29 → ht-kqrt.39, ht-kqrt.36, ht-kqrt.38
R30 → ht-kqrt.39, ht-kqrt.37, ht-kqrt.36
R31 → ht-kqrt.38, ht-kqrt.35, ht-kqrt.31
R32 → ht-kqrt.39, ht-kqrt.6
R33 → ht-kqrt.36, ht-kqrt.62
R34 → ht-kqrt.39, ht-kqrt.38, ht-kqrt.62
R35 → ht-kqrt.48, ht-kqrt.46, ht-kqrt.41
R36 → ht-kqrt.47
R37 → ht-kqrt.44
R38 → ht-kqrt.42
R39 → ht-kqrt.46, ht-kqrt.43, ht-kqrt.48
R40 → ht-kqrt.50
R41 → ht-kqrt.45, ht-kqrt.49, ht-kqrt.40, ht-kqrt.51, ht-kqrt.52, ht-kqrt.63
R42 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.57 (the daemon-side authorization handler has no owner; see finding 4)
R43 → ht-kqrt.55, ht-kqrt.57
R44 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.56
R45 → ht-kqrt.58, ht-kqrt.57, ht-kqrt.64
R-new: hc terminal seats are addressable and displayed across nodes (`<session>:tN@node`), the hc half of "cross-node addressing" → ht-kqrt.60
R-new: Claude mod watch/channel delivery works from a Named site (plain and tmux): the register.js spawn env carries HERDR_THREADS_SEAT_NAME/READONLY and does not stop on no_pane → (unmapped)
R-new: the environment-agnostic core is enforced mechanically: no match on HostKind::{Herdr,Named,Hc} in routing, delivery, receipts or binding code (src/service, src/store, src/delivery, src/cli outside caller_location) except at registration, using a source-scan or check script like check-harness-neutral → (unmapped)
R-new: one whole-tree gate on the run branch with every phase merged (F+N+N2+H1+H3): unfiltered suite, check-default-features, leak checks, Herdr regression proof, before merge to main → (unmapped)
R-new: a tmux named seat wakes a real Claude/Codex agent (opt-in wake-e2e case) → ht-kqrt.61

- **type:** ORPHAN
- **subject:** ht-kqrt.53, ht-kqrt.54, ht-kqrt.55, ht-kqrt.56, ht-kqrt.57, ht-kqrt.58, ht-kqrt.64 (ledger: C1-13)
- **evidence:** The changes file says "Amended: ht-kqrt epic goal (C1-13 H3 outcome)", and C1-13 says the "epic goal amended with the H3 handoff/launch outcome". The Goals section in coverage-round-2-inputs.md (lines 3-8) still names only seats, messaging, addressing, the agnostic core and idle wake. It says nothing about handoff, launch or coordinators creating workers. Either the amendment did not land in the goal text or the inputs are stale. Against the stated goal, the H3 leaves and their gate still serve no outcome. R42-R45 exist in the canonical list but trace to no goal sentence.
- **proposed fix:** Write the H3 outcome into the ht-kqrt goal text, for example "handoff and launch from any site create workers in Herdr panes and herdr-core terminals (D7)". Regenerate the inputs and confirm it appears.

- **type:** GAP
- **subject:** R28, clause "TRUST-POLICY/AGENTS.md carry the N rows"
- **evidence:** The C1-10 fix added TRUST-POLICY owns lines only for H1/H3 (.41, .42, .46, .48, .54, .55, .57). On N leaves, the only TRUST-POLICY owner is .32: "rules 5/6 degradation docs (TRUST-POLICY A4, AGENTS.md rules 5/6)". N adds new attribution and continuity paths that AGENTS.md's trust rules require the policy to define. R18 names "cooperative_named_seat" (new C1 path, .21) and R21 names "records cooperative_self_retire" (.26). Read-only bindings that never ACK (D3, .24), the live-holder guard (SA16, .23), inherited-name refusal (D1, .27) and named restore (D12, .22) also land. None of .21, .22, .23, .24, .26 or .27 has a TRUST-POLICY owns line. AGENTS.md: "a new way to attribute an action needs a provenance value defined in the policy"; "A change that weakens an invariant or adds an accepted limit updates TRUST-POLICY.md in the same commit."
- **proposed fix:** Add "TRUST-POLICY N row … (same commit)" owns lines: .21 (named continuity C1 + cooperative_named_seat provenance, cross-kind rebind refusal), .26 (cooperative_self_retire, Operator replication), .24 (readonly binding receipt semantics), .23 (live-holder guard / M7), .27 (D1 inherited-name refusal), .22 (D12 restore placement).

- **type:** GAP
- **subject:** R-new: Claude mod channel delivery from a Named site (R20 "live-mod Claude reports no block" depends on it)
- **evidence:** The goal requires "turn-hook and channel delivery" for "an agent in a plain or tmux process started with HERDR_THREADS_SEAT_NAME". C1-7 fixed this only for Terminal sites. .48 owns "mod watch/channel on a Terminal site (register.js spawn env carries HC_TERMINAL_ID/HC_TERMINAL_SOCKET; never stops on no_pane)". On the Named side, .17 owns only "watch on the Named site". It names no register.js change: spawn env carrying HERDR_THREADS_SEAT_NAME/HERDR_THREADS_SEAT_READONLY, and no stop on no_pane. R20 asserts that "live-mod Claude reports no block" for named seats, which needs the mod running outside a pane.
- **proposed fix:** Extend .17's owns line to the mod on a Named site: register.js spawn env forwards HERDR_THREADS_SEAT_NAME/READONLY, the mod does not stop on no_pane when a name is set, and a test runs the mod watch under a name with no HERDR_PANE_ID. Alternatively, move the register.js site handling to one owner and have .48 extend it.

- **type:** GAP
- **subject:** R42, daemon-side launch-gate authorization handler (ledger: C1-17)
- **evidence:** .53 is a "Seam contract:" leaf and owns "launch-gate authorization command and reply (bootstrap <compound>:<attempt>, (core, tN) witness, bounded timeout, refusal), its local capability, and the rule that the bootstrap Send is staged before the reply". .54 (the shim, the client side) and .57 ("consumes … launch-gate authorization command and reply, bootstrap Send staged before the reply") are consumers only. No leaf owns the daemon handler that receives the shim's request, verifies the terminal against HcHost (terminal.get/root pid), matches it to the pending bootstrap, stages the Send and replies or refuses on timeout. A tree run end to end could leave a typed command with no handler, and the shim would wait forever or time out.
- **proposed fix:** Add the daemon handler (verification against HcHost, staging, bounded-timeout refusal, test against fake_hc) to .57's owns line, or create a leaf between .53/.55 and .57.

- **type:** GAP
- **subject:** R-new: one environment-agnostic core enforced mechanically
- **evidence:** Goal: "All three go through one environment-agnostic core, where routing, delivery, receipts and binding logic never learn which environment a seat lives in." The canonical list checks this only for caller-location env (R6) and dialled-host consumers (R40). Several leaves describe kind-dependent behaviour in core: .32 adds the "offline suffix (M19) on wake=no_wake for named/hc seats"; .21 the "cross-kind rebind refusal"; .25 "named presence semantics"; .31 "WakeBlock::NoWake". Nothing stops these from being written as HostKind::Named/Hc branches in src/store or src/service. There is a check-harness-neutral for harnesses but no host-kind analogue.
- **proposed fix:** Add a requirement plus a leaf (or extend .3/.15's owns line) for a host-kind neutrality scan: HostKind variants are matched only in src/host/** and registration, and core decides through HostPort capability answers. Run it in each phase gate.

- **type:** GAP
- **subject:** R10, F behavioural gate sequencing (ht-kqrt.13)
- **evidence:** .13 says "Before N and H1 build on F, run nice scripts/herdr-release-smoke …". No task in the tree lists ht-kqrt.13 in its deps. The N entry points .15 (deps .4, .5) and .16 (deps .10, .11, .12, .15), and the H1 entry points .40 (deps .1) and .41 (deps .14, .3, .40), all bypass it. The gate can therefore run last, or not at all, without blocking any leaf.
- **proposed fix:** Add ht-kqrt.13 as a blocker of the first N and H1 leaves (.15, .16, .40/.41), or at least of the .34 and .63 phase gates.

- **type:** GAP
- **subject:** R-new: whole-tree gate with every phase merged
- **evidence:** C1-8: "new gates ht-kqrt.62 (N2), ht-kqrt.63 (H1), ht-kqrt.64 (H3); no root sweep". .64 deps only .57 and .58. It does not depend on .63 or on the H1-only leaves .49, .50, .51, .52, .60, nor on .62, .61 or .59. .63 does not depend on .62 or .64. Each gate runs "on the tree with every <phase> leaf merged", so no gate is guaranteed to run on the tree with all phases merged. The user's memory note "Merge needs full suite" and AGENTS.md (check-default-features before a merge) apply to the run branch's merge into main.
- **proposed fix:** Make .64 the last gate (deps on .62, .63, .59) and state that it runs on the fully merged run branch, or add one final "Integration sweep:" leaf that depends on all four phase gates and .59.

- **type:** GAP
- **subject:** R18, clause "one unresolved named row does not silence liveness of other named bindings"
- **evidence:** None of .21's owns items (named continuity C1, cross-kind rebind refusal, Closed guard, wait:host), .25's ("named presence semantics") or .23's ("live-holder guard") names liveness interaction across named rows. The named host is a single builtin host shared by all named seats (.15, "reserved named-host locator"), so one unresolved row inhibiting liveness for the whole host is a natural failure mode with no owner.
- **proposed fix:** Add this clause with a test to .25's (or .21's) owns line and acceptance.

- **type:** UNOWNED-SEAM
- **subject:** ht-kqrt.6 → ht-kqrt.50 (and .34): "consumer filters (F4-audit)"
- **evidence:** .50 has "consumes: … consumer filters (F4-audit)". .6 has no owns line, only "consumes: all_for_reads/attached() (F4)", yet it is the leaf that "Appl[ies] the verified consumer table in 07 §5.3 at each call site". The dialled-only filter semantics that .50's two-host tests assert (idle exit, cap, reattach sweep) are named by no owns line.
- **proposed fix:** Give .6 "owns: dialled-only consumer filters per 07 §5.3 (idle exit, cap, reattach sweep, operator_host text)".

- **type:** UNOWNED-SEAM
- **subject:** ht-kqrt.43 → ht-kqrt.60 / ht-kqrt.52: "hc labels (H1c-labels)"
- **evidence:** .60 has "consumes: … hc labels (H1c-labels)" and .52 has "consumes: … hc labels (H1c-labels)". .43 has no owns line: it implements `HcHost::seat_labels`/`live_topology`, herdr-app doc-name parsing and caching, and the "ambiguous bare tN refusal". .41 owns only the vocabulary and format, not the seat_labels output or the bare-tN refusal that R39 asserts ("a bare t1 is refused listing both candidates").
- **proposed fix:** Give .43 "owns: HcHost seat_labels/live_topology output, herdr-app doc-name parsing/cache, ambiguous bare tN refusal".

- **type:** GAP
- **subject:** Previous-round fix C1-6 / ht-kqrt.60, cross-node hc coverage in the end-to-end smoke
- **evidence:** .60 adds hc cross-node addressing, but the end-to-end configuration smoke .59 deps .16, .34, .39, .48, not .60. C1-12 extends .59's acceptance only to "hc<->named and hc<->pane in one daemon". The goal's "cross-node addressing" for hc is therefore proven only in .60's own tests, and no thin end-to-end path crosses nodes for any non-pane environment besides .33's named fixture.
- **proposed fix:** Add a two-node case to .59 (or to .60's acceptance) covering a named seat and an hc seat addressed as name@B and `<session>:tN@B` from a pane seat on A, and add .60 to .59's deps.

NEEDS-SPEC: ht-kqrt (root) — the Goals text lacks the H3 handoff/launch outcome that C1-13 claims was added. Confirm the amended goal wording.
