requirements:
R1 → ht-poh.4.1, ht-poh.4.2, ht-poh.4.3, ht-poh.4.4 (gate evidence: ht-poh.6.5)
R2 → ht-poh.4.1, ht-poh.4.3 (gate: ht-poh.6.5). Partial: the design decides the role in the hook runner, not in the daemon as R2's text says. See finding 1.
R3 → ht-poh.4.3 (gate: ht-poh.6.5)
R4 → ht-poh.5, ht-poh.3.1, ht-poh.3.2, ht-poh.3.4, ht-poh.4.1 (hook_step) (gate: ht-poh.6.6)
R5 → ht-poh.4.5, ht-poh.5, ht-poh.3.3 (wake_suppressed) (gate: ht-poh.6.7)
R6 → ht-poh.4.4, ht-poh.2.1, ht-poh.2.3 (gate: ht-poh.6.4)
R7 → ht-poh.4.6, ht-poh.8, ht-poh.2.2, ht-poh.4.8 (gate: ht-poh.6.4)
R8 → ht-poh.1, ht-poh.2.1–2.6, ht-poh.7
R9 → ht-poh.3.2, ht-poh.3.3, ht-poh.2.7
R10 → ht-poh.7
R11 → ht-poh.3.5, ht-poh.3.6, ht-poh.3.7
R12 → ht-poh.6.1–6.8
R13 → ht-poh.9, ht-poh.1 / ht-poh.7 (trailer convention)
R14 → ht-poh.6.9, ht-poh.10
R15 → ht-poh.4.3, ht-poh.4.2 (encode)
R16 → ht-poh.2.7
R-new: Root-spec text is amended, or the orchestrator acknowledgement recorded, wherever a subepic deliberately departs from it: R2's "decided in the daemon", and the root's `internal hook agy <event>` argv. → (unmapped)
R-new: Herdr itself identifies an agy pane as host kind `agy`, which launch's `expected_host_kinds` and idle wake rely on. → (unmapped; ht-poh.4.5 assumes it)

Findings

1. **type:** GAP
   **subject:** R2 / the root-spec refinement that needs orchestrator acknowledgement
   **evidence:**
   - R2 says the role "is decided in the daemon from on-disk witnesses the hook reports".
   - ht-poh.4 spec §Key decisions: "Root-spec refinement: where the role is decided. This needs orchestrator acknowledgement." The hook runner applies `RoleWitness::decide`, and the result reaches the daemon only as the ordinary cooperative `claim.role` ("No wire change").
   - D3 also departs from the root text ("Discarded. The root text's `internal hook agy <event>`").
   - No task records the acknowledgement or amends R2 and the root spec. ht-poh.6.5 and ht-poh.6.9 would then check against an R2 that the code does not meet as written.
   **proposed fix:** Add a leaf under ht-poh (or fold it into ht-poh.7) that records the orchestrator decision on D4 and amends the R2 wording and the root §3 hook argv text. If it is overruled, ht-poh.4.1 changes to the daemon-read variant (D4's last paragraph).

2. **type:** GAP
   **subject:** TRUST provenance honesty for the agy role (AGENTS.md: "a new way to attribute an action needs a provenance value defined in the policy")
   **evidence:**
   - D9's text names a provenance: "The resulting role is a cooperative top-level claim (`cooperative_top_level`). It is not payload-declared and not daemon-verified."
   - D4 says the role reaches the daemon "as the ordinary cooperative `claim.role`… No wire change". The daemon therefore cannot tell a witness-derived role from a payload-declared one, and cannot record `cooperative_top_level` on any seat or receipt.
   - No task defines or records that provenance value in code. ht-poh.4.3 owns only the "TRUST startup text".
   - The ledger's c10 rejection covered `cooperative_hook_delivery` (3.4), not this one.
   **proposed fix:** In ht-poh.4.3, either carry and record a `cooperative_top_level` provenance (which means a wire/claim field and a store column), or reword D9 so it names no provenance value the code cannot record. Have the TRUST-POLICY change state which.

3. **type:** GAP (ledger: c13)
   **subject:** R4 end to end. ht-poh.5 consumes 3.4 and 3.2 outputs with no dependency edge.
   **evidence:**
   - ht-poh.5's deps are `ht-poh.3.1, ht-poh.4.1, ht-poh.4.2, ht-poh.4.3`.
   - Yet its line says it "consumes: offered-ledger confirm semantics (DeliveryConfirm outcomes, re-offer bounds) owned by ht-poh.3.4; the D4 frame text via DeliveryPage owned by ht-poh.3[.2]".
   - 3.1 lands only "inert handlers", so ht-poh.5 can be built and closed against inert `DeliveryPull`/`DeliveryConfirm`. Its transcript-confirm settlement could then not be tested against the real ledger.
   - The c13 fix added a consumes line, not the edge.
   **proposed fix:** Add the edges ht-poh.5 ← ht-poh.3.4 and ht-poh.5 ← ht-poh.3.2.

4. **type:** GAP (ledger: c7)
   **subject:** R5. agy idle wake needs the composer policy, but ht-poh.5 has no edge to it.
   **evidence:**
   - The changes list says "ht-poh.5 (… IdleWake::Native via 4.5 composer)". ht-poh.5's deps do not include ht-poh.4.5.
   - If ht-poh.5 declares `IdleWake::Native` before `AgyComposer` exists, the stub composer policy from 4.2 governs wake. Nothing guarantees that the stub refuses to wake into a non-empty composer ("only into an empty composer").
   - Only the gate flow ht-poh.6.7 joins 4.5 and 5.
   **proposed fix:** Add ht-poh.5 ← ht-poh.4.5, or require the 4.2 composer stub to return `Unreadable` (no wake) until 4.5 lands, and state that in 4.2.

5. **type:** UNOWNED-SEAM
   **subject:** The `SKILL_MD` content that ht-poh.3.6 edits and ht-poh.4.4 embeds into the byte-locked plugin SKILL golden
   **evidence:**
   - ht-poh.3.6: "Core wake decision honours `wake_on: all`, plus SKILL section".
   - ht-poh.4.4: "render … SKILL.md (`SKILL_MD` + `AGY_SKILL_GUIDANCE`)". The spec's D7 adds "Golden files are byte-locked".
   - The two tasks have no ordering edge and no owns line names `SKILL_MD` as a boundary. Whichever lands second breaks the other's golden, or the agy plugin ships without the wake-on section.
   **proposed fix:** Name an owner for `cli::skill::SKILL_MD` content (3.6). Either have 4.4's golden template the core part (placeholder), or add the edge 4.4 ← 3.6.

6. **type:** UNOWNED-SEAM
   **subject:** Allow-member-only settings engine with `native_rewrites`. 2.1 or 2.3 defines it; 4.4 consumes it.
   **evidence:**
   - The spec's D7: "This is ht-poh.2.1's allow-member-only mode … `OwnedAllowSpec{rule: "command(herdr-threads)", retired: &[], native_rewrites: true}`", with `hook_groups: |_| vec![]`. A member that agy rewrote away must be "reported missing and re-added".
   - ht-poh.4.4 consumes "ht-poh.2.1 (`SettingsKindSpec` allow-member-only engine)" but depends on 2.3.
   - 2.3 is a byte-identical de-harnessing sweep. 2.1's visible owns line lists `settings_kind` as a hook with a neutral default, not engine behaviour that no current harness exercises.
   - No owns line names `OwnedAllowSpec` or the native-rewrite semantics.
   **proposed fix:** Make 2.1 (or 2.3) explicitly own `SettingsKindSpec`/`OwnedAllowSpec` and the allow-member-only and native-rewrites engine path, with tests independent of agy.

7. **type:** UNOWNED-SEAM
   **subject:** Lifecycle-digest policy (ht-poh.3 DP7) between ht-poh.3.3 and ht-poh.4.3 / ht-poh.5
   **evidence:**
   - The spec's D5: "The context is today's `compose_context`, with the lifecycle digest kept for HookPull per ht-poh.3 DP7."
   - ht-poh.3.3 owns `ack_policy_delivered`, `delivery::settle` and `wake_suppressed`, not the digest rule. ht-poh.4.3 delivers R15's context and has no edge to 3.3.
   - No owns line names who decides whether the startup digest is kept or suppressed per delivery mechanism.
   **proposed fix:** Add the digest-per-policy rule to 3.3's owns line, and add the edge 4.3 ← 3.3 (or state that agy's default `None` policy keeps today's digest).

8. **type:** UNOWNED-SEAM
   **subject:** The fake Gemini server that ht-poh.4.5 uses to capture composer fixtures, which ht-poh.6.1 rewrites
   **evidence:**
   - The spec's D8 says the composer captures are taken with "genuine agy against the probe fake Gemini server (busy = scripted delay) in an isolated named Herdr session".
   - ht-poh.6.1: "Grow `harness-fake_server.py` into `server.py` plus `scenario.py`… `delay_ms`".
   - 4.5 depends only on 4.2, and 6.1 owns the scenario format. Two tasks drive the same server with no contract or ordering, and the capture recipe may not be reproducible after 6.1.
   **proposed fix:** Have 4.5 consume 6.1's server (edge 4.5 ← 6.1), or state that it uses the frozen probe copy and record the capture recipe in the fixtures.

9. **type:** UNOWNED-SEAM
   **subject:** The `Outside-adapter:` commit-trailer format, which ht-poh.1 and ht-poh.7 define and ht-poh.9 parses
   **evidence:**
   - ht-poh.1: "Accounting (R13): every commit carries an 'Outside-adapter: <none | list>' trailer".
   - ht-poh.7: "Accounting (R13): every commit carries an 'Outside-adapter: <none | list>' trailer naming each change outside src/h…".
   - ht-poh.9 owns only "outside-adapter report format".
   - Two tasks state the trailer and none owns its grammar, which the report tool must parse.
   **proposed fix:** Assign the trailer grammar to one owner (ht-poh.9 or ht-poh.1), and have the others consume it.

10. **type:** GAP
    **subject:** R13 (ledger: c1). The design-recorded R13 notes do not reach the final report.
    **evidence:**
    - The ht-poh.4 spec records R13 items that are not commit-local: D2's "Recorded as an R13 note" (the core contract-mode alternative) and D6's "This is an R13 finding" (`hook_step`).
    - ht-poh.9 builds a tool from trailers. The ledger says it is "run by orchestrator at final SHA", but no leaf (6.9 or 10) depends on 9 or produces the final measurement.
    **proposed fix:** Add an edge 6.9 ← 9 (or a final-report leaf after ht-poh.10) that runs `scripts/outside-adapter-report` and merges the spec-recorded R13 notes.

11. **type:** GAP
    **subject:** R-new: Herdr reports agy panes as host kind `agy`
    **evidence:**
    - The spec's D2 has `host_kinds: &["agy"]`, and D8 has `expected_host_kinds is ["agy"]`. Idle wake and the composer read through `agent read --source detection`.
    - No task verifies that Herdr's agent detection classifies an agy TUI as `agy`, or handles the case where it does not. ht-poh.4.5 captures detection text without asserting the kind, and launch's host-kind verification would refuse every agy launch.
    **proposed fix:** Extend the ht-poh.4.5 or ht-poh.4.6 acceptance to capture and assert Herdr's reported kind for an agy pane, and document a fallback if Herdr does not know agy.

12. **type:** GAP
    **subject:** The DP10 frozen-suite gate that ht-poh.4.1 relies on
    **evidence:**
    - ht-poh.4 spec: "suites that ht-poh.3's DP10 gate freezes", and in Acceptance: "the DP10 suites have no diff".
    - No ht-poh.3 leaf line names DP10 (3.1 covers DP1, 3.2 DP2, 3.3 DP3/DP7, 3.4 DP5/DP6, 3.5–3.7 DP8). It is unclear which task defines the frozen set that 4.1 must keep diff-free.
    **proposed fix:** Name DP10's owner (likely 3.3 or 2.7) in an owns line, or add a leaf for it.

13. **type:** GAP (minor)
    **subject:** Recorded agy payload fixtures
    **evidence:**
    - The spec's Acceptance mentions "Decode on recorded payloads (`tests/fixtures/agy/payloads/`)". The fixtures for "fresh conversation 1, resumed 12 and 14" come from probe runs.
    - No task line owns the creation of `tests/fixtures/agy/payloads/` or the `agy --help` fixture (`tests/fixtures/agy/help/agy-1.3.2.txt`, which ht-poh.4.6 names as an input). ht-poh.5 and 6.x consume `tests/fixtures/agy/`.
    **proposed fix:** Add the fixture import from the probe runs to ht-poh.4.2's owns line (payloads and help text).

Previous-round fix check:
- c13 (ht-poh.5 consumes 3.4 and 3.2): the consumes line was added but there is no dependency edge. Re-reported as finding 3.
- c7 (R5 IdleWake via the 4.5 composer): there is no edge 5 ← 4.5. Re-reported as finding 4.
- c1 (ht-poh.9): no downstream task runs it. Re-reported as finding 10.
- c11 (ht-poh.1 narrowed, 7 owns the AGENTS text): both still state the trailer rule, which opens finding 9.
- c2 and c8 (agy-after-sweep edges, and 4.8 ← 8, 6.4 ← 8): closed. ht-poh.4.6 itself has no edge to ht-poh.8, which is acceptable because the spec says "ht-poh.4 adds no grammar".
- c4, c3, c6, c9, c14: closed as stated.

NEEDS-SPEC: ht-poh.3 — I could not judge who owns DP10 (the frozen suites) or the DP7 lifecycle-digest rule that ht-poh.4 cites (findings 7 and 12).
NEEDS-SPEC: ht-poh.5 — I could not judge whether it declares `IdleWake::Native` before the agy composer exists, or needs the live 3.4 handlers to complete (findings 3 and 4).