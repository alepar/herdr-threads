requirements:
R1 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.13
R2 → ht-kqrt.7
R3 → ht-kqrt.3, ht-kqrt.5, ht-kqrt.6
R4 → ht-kqrt.4
R5 → ht-kqrt.9
R6 → ht-kqrt.10, ht-kqrt.9
R7 → ht-kqrt.10, ht-kqrt.11
R8 → ht-kqrt.11
R9 → ht-kqrt.2
R10 → ht-kqrt.13
R11 → ht-kqrt.15, ht-kqrt.16, ht-kqrt.18, ht-kqrt.20, ht-kqrt.21, ht-kqrt.23
R12 → ht-kqrt.19
R13 → ht-kqrt.18
R14 → ht-kqrt.23, ht-kqrt.24
R15 → ht-kqrt.24
R16 → ht-kqrt.34, ht-kqrt.30, ht-kqrt.18
R17 → ht-kqrt.27
R18 → ht-kqrt.21, ht-kqrt.25, ht-kqrt.15
R19 → ht-kqrt.28
R20 → ht-kqrt.31, ht-kqrt.32
R21 → ht-kqrt.26
R22 → ht-kqrt.22
R23 → ht-kqrt.33
R24 → ht-kqrt.30, ht-kqrt.20
R25 → (unmapped; at most implied by ht-kqrt.30 "handoff out of a named seat")
R26 → ht-kqrt.17, ht-kqrt.29, ht-kqrt.15 (M11 unmapped)
R27 → ht-kqrt.25, ht-kqrt.6, ht-kqrt.15
R28 → ht-kqrt.17, ht-kqrt.32, ht-kqrt.21 (S1), ht-kqrt.17 (S3), ht-kqrt.32 (S6), ht-kqrt.27 (E1); S2, S4, named half of S5 (unmapped)
R29 → ht-kqrt.38, ht-kqrt.39
R30 → ht-kqrt.36, ht-kqrt.37, ht-kqrt.39
R31 → ht-kqrt.35, ht-kqrt.38
R32 → ht-kqrt.39
R33 → ht-kqrt.36
R34 → ht-kqrt.39 (partial: T1-T5 and the AGENTS.md tmux wake line are named by no task)
R35 → ht-kqrt.41, ht-kqrt.46, ht-kqrt.48
R36 → ht-kqrt.47
R37 → ht-kqrt.44
R38 → ht-kqrt.42
R39 → ht-kqrt.43, ht-kqrt.46, ht-kqrt.48, ht-kqrt.26
R40 → ht-kqrt.50
R41 → ht-kqrt.40, ht-kqrt.45, ht-kqrt.49, ht-kqrt.51, ht-kqrt.52
R42 → ht-kqrt.54, ht-kqrt.55, ht-kqrt.57
R43 → ht-kqrt.55, ht-kqrt.57
R44 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.56
R45 → ht-kqrt.58 (L1-L4 named by no task)
R-new-1: Cross-node addressing and display of hc terminal seats (an hc seat on node A is addressable and labelled from node B) → (unmapped)
R-new-2: Claude mod channel delivery and channel idle wake work from Named and Terminal sites, not only Herdr panes → (unmapped)
R-new-3: Herdr-pane behaviour stays unchanged (golden surfaces, herdr-release-smoke) after the N, N2, H1 and H3 phases, not only after F → (unmapped; ht-kqrt.13 gates F only, R33 covers TUI wake for N2 only)
R-new-4: TRUST-POLICY covers Terminal-site attribution and hc finish-reason binding closure (a new attribution path, and ending bindings on host evidence) → (unmapped; ht-kqrt.32 and R28 cover only the N rows)
R-new-5: Idle wake on hc terminals reaches what the host allows (wake ruled out with evidence, or implemented) → ht-kqrt.48 (only the no_wake reporting)
R-new-6: tmux idle wake for each builtin harness that has a turn-state signal (Codex, Hermes, agy), not only Claude → (unmapped; ht-kqrt.36 names Claude only)
R-new-7: Join/invite/accept work from Named and Terminal seats → ht-kqrt.34, ht-kqrt.48, ht-kqrt.59 (implicit only)

- **type:** GAP
- **subject:** R25 (handoff recovery hint for a named caller)
- **evidence:** No task line mentions a recovery hint carrying `HERDR_THREADS_SEAT_NAME=<n>`. ht-kqrt.10 owns "recovery hints" for the Pane family only ("three Pane-site handoff recovery hints", R1). ht-kqrt.30 says only "handoff out of a named seat".
- **proposed fix:** New leaf under ht-kqrt (N phase, after ht-kqrt.16 and ht-kqrt.30): add a Named-site recovery hint to caller_location.rs, with a test that running it resolves to the same seat.

- **type:** GAP
- **subject:** R26 M11 (handoff into a named seat refused)
- **evidence:** R26 lists M11. M14/M15/M16 are owned by ht-kqrt.17, M10 by ht-kqrt.29, M6 by ht-kqrt.27, M7 by ht-kqrt.23, M9 by ht-kqrt.28, M12/M19 by ht-kqrt.32, M17/M4 by ht-kqrt.46, M18/M20 by ht-kqrt.26 via R21, M13 by ht-kqrt.16. No owns: line names M11.
- **proposed fix:** Add M11 to ht-kqrt.30's owns (it already handles handoff around named seats), or add a new leaf under ht-kqrt.

- **type:** GAP
- **subject:** R28 spikes S2, S4 and the named half of S5
- **evidence:** S1 appears in ht-kqrt.21, S3 in ht-kqrt.17 ("watch S3"), S6 in ht-kqrt.32 ("S6 doctor") and E1 in ht-kqrt.27. No task line names S2, S4 or S5.
- **proposed fix:** Name the leaves that answer S2, S4 and named-S5, or add one spike-answer test leaf under ht-kqrt (N phase).

- **type:** GAP
- **subject:** R34 T1-T5 and the AGENTS.md tmux wake line
- **evidence:** R34 requires "AGENTS.md gains the tmux wake line ... and T1-T5 are recorded". ht-kqrt.39's owns are limited to "ComposerTraits::TMUX; built-in session wake lane registration". No task names T1-T5 or the AGENTS.md tmux line. ht-kqrt.32 owns AGENTS.md rules 5/6 for no-wake only.
- **proposed fix:** Extend ht-kqrt.39 (or a new N2 docs/evidence leaf) to own the AGENTS.md tmux wake line, the delivery-contract table and the T1-T5 records.

- **type:** GAP
- **subject:** R45 L1-L4
- **evidence:** R45 says "L1-L4 answered with live parts listed as remaining evidence". No H3 task (ht-kqrt.53 to ht-kqrt.58) names L1-L4.
- **proposed fix:** Add L1-L4 to ht-kqrt.57 or ht-kqrt.58, or add a new H3 evidence leaf.

- **type:** GAP
- **subject:** R-new-1, cross-node addressing of hc terminal seats
- **evidence:** The goal requires "cross-environment and cross-node addressing" for all three environments. Cross-node work is only ht-kqrt.33 ("display_name publish ... name@node"), which covers named seats. R23 covers "reviewer@B" only. hc labels (ht-kqrt.43, `<session>:tN`) are local HostTopologies, with nothing about remote publication or remote selectors. Premortem: an agent on node B cannot address or recognise an hc seat on node A except by raw seat id.
- **proposed fix:** New leaf under ht-kqrt (H1): publish and render hc seat labels across nodes (or record explicitly that cross-node addressing is by seat id), with a two-node test.

- **type:** GAP
- **subject:** R-new-2, Claude mod channel for Named and Terminal sites
- **evidence:** The goal names "turn-hook and channel delivery" for every environment. R20 ("hooks-only Claude classifies None and live-mod Claude reports no block", "mod-channel flap") and R35 ("live-mod Claude reports no no_wake") assume the mod channel binds from named and hc sites. ht-kqrt.7 takes `channel_live`, but no task changes integrations/claude/mod (register.js) to locate a Named or Terminal caller. The only bridge task is Hermes (ht-kqrt.17, ht-kqrt.49). Premortem: the mod channel never attaches outside a Herdr pane, so channel delivery and channel wake are missing for named, tmux and hc seats.
- **proposed fix:** New leaf under ht-kqrt (N, then H1 extension): Claude mod caller location for `HERDR_THREADS_SEAT_NAME` and `HC_TERMINAL_*`, with tests for live channel delivery and Channel wake on a named and an hc seat.

- **type:** GAP
- **subject:** R-new-3, end-of-tree integration sweep and Herdr regression gate
- **evidence:** The only behavioural gate is ht-kqrt.13 ("F behavioural gate: herdr-release-smoke and wake-e2e self-test evidence"), which depends only on F leaves. No `Integration sweep:` task exists for N, N2, H1 or H3. No task re-runs golden surfaces, herdr-release-smoke, check-wire-compat or check-harness-neutral after ht-kqrt.15 to ht-kqrt.58. ht-kqrt.59 is a configuration smoke with stand-ins, not a regression gate. Premortem: N2's Stop hook registration or H3's generic CreateTargetPort refactor (ht-kqrt.56) silently changes Herdr-pane behaviour.
- **proposed fix:** New leaf `Integration sweep: ht-kqrt` depending on every phase's last leaves: full suite, leak checks, wire/neutral checks, herdr-release-smoke on the newest Herdr, golden-surface diff.

- **type:** GAP
- **subject:** Real-agent and real-host end-to-end for the new environments (premortem on the thin end-to-end path)
- **evidence:** Every end-to-end task uses fakes or stand-ins: ht-kqrt.34 (in-process daemon), ht-kqrt.48 ("against fake_hc", R35), ht-kqrt.59 ("hook stand-ins"). The real-core smoke ht-kqrt.52 is "opt-in, only with a verified pin". No task extends scripts/wake-e2e.py, the release gate, with named, tmux or hc cases. Premortem: tmux wake passes against stand-ins but fails with real Claude or Codex composers, or the real hc core differs from fake_hc beyond schema conformance.
- **proposed fix:** New leaf under ht-kqrt: opt-in wake-e2e cases for named+tmux (real Claude/Codex) and an hc terminal case, with evidence recorded. Also record that hc-smoke stays a matrix, not a version gate (AGENTS.md "never gate ... on an exact version").

- **type:** GAP
- **subject:** R-new-4, TRUST-POLICY for Terminal-site attribution and hc closure
- **evidence:** AGENTS.md trust policy requires "a new way to attribute an action needs a provenance value defined in the policy" and that the policy is updated when bindings end on host evidence. ht-kqrt.42 makes "`killed`/`removed` retire, exit/signal/... unresolve" (binding closure on hc finish reasons), and ht-kqrt.46 adds the Terminal caller site. Only ht-kqrt.32 ("TRUST-POLICY A4") and R28 ("TRUST-POLICY/AGENTS.md carry the N rows") touch TRUST-POLICY, and both cover N only.
- **proposed fix:** Add TRUST-POLICY H1 rows (Terminal-site provenance, D10 finish-reason closure, C2 core-id rotation) to ht-kqrt.42 and ht-kqrt.46, or add a docs leaf under ht-kqrt.

- **type:** GAP
- **subject:** R-new-5, hc idle wake "as far as each host allows"
- **evidence:** hc gets only no_wake reporting (ht-kqrt.48 "hc messaging end to end with wake=no_wake"). ht-kqrt.40's client says "never c[...]" (truncated), so the tree avoids some hc call, but no task records whether hc offers an input or typing API that would allow a composer wake. If it does, the goal's "Idle wake works as far as each host allows" is unmet for hc and Herdr desktop-app panes.
- **proposed fix:** Add an evidence item to ht-kqrt.48 or a new leaf: either justify `WakeCapability::None` for hc against the vendored schema, or implement an hc composer wake.

- **type:** GAP
- **subject:** R-new-6, tmux wake for harnesses other than Claude
- **evidence:** ht-kqrt.36: "`HarnessAdapter::turn_state_edge(event)` (default `None` = no tmux wake for that harness)", and only "Claude report-only Stop" is named. ht-kqrt.37 recognizers exist only for "harnesses whose captured fixtures support" them. Codex, Hermes and agy have turn hooks, but no task implements turn_state_edge for them. Premortem: a Codex agent in tmux never wakes.
- **proposed fix:** New N2 leaf: implement turn_state_edge and screen_blocked for Codex (and Hermes/agy where hooks allow), or record per-harness no-wake as an accepted limit.

- **type:** ORPHAN
- **subject:** ht-kqrt.53 to ht-kqrt.58 (H3: CreateTargetPort, launch-gate shim, hc terminal.run, Herdr launch refactor, hc handoff/launch CLI, peer launch NewTerminal)
- **evidence:** The root goal covers seats, messaging, addressing and idle wake only, and says nothing about launching into or handing off to hc terminals. Example: "ht-kqrt.58 · Peer launch: NewTab onto hc spaces, cap-gated LaunchTarget::NewTerminal, peer-target-create alias". R42 to R45 exist in the canonical list, but no goal outcome backs them. ht-kqrt.56 also refactors existing Herdr launch with no goal outcome needing it.
- **proposed fix:** Either add a goal outcome ("handoff/launch can create hc terminals and Herdr desktop-app tabs") to ht-kqrt, or move H3 to a follow-up epic. ht-kqrt.53 is titled Seam contract: and is not itself an orphan.

- **type:** ORPHAN
- **subject:** ht-kqrt.52 (opt-in real-core hc-smoke)
- **evidence:** "Ship `scripts/hc-smoke` ... only if the implementer can fetch and verify the herdr-core 0.1.15 manifest". It is conditional and opt-in, so it may deliver nothing. R41 only asks that "hc-smoke status is recorded".
- **proposed fix:** Keep it tied to R41 and state the deliverable when the pin cannot be verified (a recorded status), or fold it into the real-host end-to-end gap above.

- **type:** UNOWNED-SEAM
- **subject:** `seat.created.display_name` wire field (publisher vs restore reader)
- **evidence:** ht-kqrt.33: "`ensure_published` sets `seat.created.display_name` from `seats.name`; on receive it is validated". ht-kqrt.22: "Change the local-origin restore in `materialize_seat_created`: a row with `display_name` goes onto the named host". ht-kqrt.22 reads display_name, which ht-kqrt.33 writes, yet ht-kqrt.22 does not depend on ht-kqrt.33 (it runs earlier), and neither owns: line names the display_name field or its wire shape. ht-kqrt.33 has no owns: line at all.
- **proposed fix:** Contract the `seat.created.display_name` field (shape, `#[serde(default)]`, NULL on invalid) in one owner, ideally ht-kqrt.20 or a seam-contract leaf, before ht-kqrt.22 and ht-kqrt.33.

- **type:** UNOWNED-SEAM
- **subject:** hc terminal label keys (`herdr.space`, `herdr.tab`, `ht.bootstrap`, `ht.label`) written by terminal.run and parsed for labels
- **evidence:** ht-kqrt.43: "take workspace from `herdr.space`, tab from `herdr.tab`". R43 (ht-kqrt.55/ht-kqrt.57): "--space sends the herdr.space/tab/ht.bootstrap/ht.label set". ht-kqrt.43 and ht-kqrt.55 have no owns: lines, and ht-kqrt.57 has none either.
- **proposed fix:** Contract the hc label key set and the `<session>:tN` label/selector format in one owner (ht-kqrt.41 or ht-kqrt.43).

- **type:** UNOWNED-SEAM
- **subject:** Default standalone state dir path (instance discovery vs Herdr-less setup)
- **evidence:** ht-kqrt.18 owns "named-site instance discovery and default state dir creation". ht-kqrt.19 bakes "--state-dir" into hooks ("Herdr-less setup baking --state-dir"), and ht-kqrt.49 also "bakes `--state-dir`". ht-kqrt.19 depends only on ht-kqrt.1 and does not consume ht-kqrt.18's default, so the path setup bakes and the path discovery creates are agreed by no line.
- **proposed fix:** Make ht-kqrt.19 consume ht-kqrt.18's default-state-dir definition (add a consumes: line and a dep), or contract the path in ht-kqrt.2 or ht-kqrt.10.

- **type:** UNOWNED-SEAM
- **subject:** Launch-gate shim authorization protocol (shim vs daemon)
- **evidence:** ht-kqrt.54: "`herdr-threads internal launch-gate --bootstrap <compound>:<attempt> -- <argv>`". R42: "waiting for the daemon's (core,tN) authorization before exec, with the bootstrap Send staged". ht-kqrt.54 owns the shim. ht-kqrt.53 owns "V3 bootstrap journal records". No owns: line names the daemon-side authorization command or reply the shim waits on. ht-kqrt.57 only "consumes ... gate shim (H3b)".
- **proposed fix:** Contract the shim-to-daemon authorization command (wire shape and capability) in ht-kqrt.53 or ht-kqrt.54.

- **type:** UNOWNED-SEAM
- **subject:** Resolve-command extensions (readonly flag, locator_hint, tmux hint) layered on the named resolve command shape
- **evidence:** ht-kqrt.16 owns the "named resolve command shape". ht-kqrt.24: "`HERDR_THREADS_SEAT_READONLY=1` (read in caller_location.rs, carried in the resolve command)" with owns: "occupant_bindings.readonly semantics" only, so no task owns the readonly field on the resolve command. ht-kqrt.27 and ht-kqrt.38 each own their own field.
- **proposed fix:** Add "readonly field on the resolve command" to ht-kqrt.24's owns, or list all resolve-command fields in ht-kqrt.16's contract.

- **type:** UNOWNED-SEAM
- **subject:** HcDiscovery owned by ht-kqrt.41 but implemented by ht-kqrt.45
- **evidence:** ht-kqrt.41 owns: "HcHost / HcBoot / HcDiscovery". ht-kqrt.45 has no owns: line ("consumes: HcHost (H1c); implements the discovery part of the HcHost abstraction"). ht-kqrt.49 and ht-kqrt.57 consume "HcDiscovery state paths (H1d)" and "HcDiscovery (H1d)" from ht-kqrt.45. Ownership of the state-path contract is split from the implementing task.
- **proposed fix:** Move HcDiscovery and the core state-path contract to ht-kqrt.45's owns.

- **type:** UNOWNED-SEAM
- **subject:** Site-capability gate mechanism used by seat.terminal_sites_v1
- **evidence:** ht-kqrt.12 owns "CLI site-capability gate mechanism". ht-kqrt.48 does "gate Terminal sites on it in the CLI (M13 against an N-only daemon)", but its consumes: line does not list F9 and ht-kqrt.48 does not depend on ht-kqrt.12 directly.
- **proposed fix:** Add ht-kqrt.12 (F9) to ht-kqrt.48's consumes:.

NEEDS-SPEC: ht-kqrt.40 / ht-kqrt.48 — the truncated "never c[...]" in ht-kqrt.40 and the hc wake decision; I need the H1 spec to judge whether hc offers an input API that the goal's "as far as each host allows" would require using.
