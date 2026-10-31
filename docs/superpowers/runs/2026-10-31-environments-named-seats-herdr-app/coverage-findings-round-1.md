GAP · R25 named-caller handoff recovery hint · no leaf builds the Named-site env_assignments hint with HERDR_THREADS_SEAT_NAME=<n> (ht-kqrt.10 is Pane-only, ht-kqrt.30 says only "handoff out of a named seat")
GAP · R26 M11 handoff into a named seat · no owns line names M11; ht-kqrt.30 covers handoff out of a named seat only
GAP · R28 spikes S2, S4, named half of S5 · S1/S3/S6/E1 are named by leaves, S2/S4/S5 by none
GAP · R34 tmux docs and evidence · no leaf owns the delivery-contract table, AGENTS.md tmux wake line, TRUST-POLICY N2 row, T1-T5 records or tmux -L ht-test-* leak recognition
GAP · R45 L1-L4 · no H3 leaf answers L1-L4 or lists their live parts as remaining evidence
GAP · cross-node addressing of hc terminal seats · only named seats (ht-kqrt.33, reviewer@B) are published and resolved across nodes; hc seats have no cross-node leaf or test
GAP · Claude mod channel from Terminal sites · ht-kqrt.17 (watch S3) covers the Named site, but no leaf makes register.js locate an HC_TERMINAL_* caller, which R35's live-mod check assumes
GAP · per-phase integration sweep after F · only ht-kqrt.13 gates F; no leaf runs the spec's per-phase gate (full suite, leak checks, wire/neutral checks, herdr-release-smoke, golden surfaces) for N, N2, H1, H3
GAP · real-agent tmux wake e2e · no leaf adds an opt-in wake-e2e case for a named seat in tmux with real Claude/Codex, or records it out of scope (hc half rejected: spec keeps hc wake e2e opt-in until H2)
GAP · TRUST-POLICY and AGENTS.md H1/H3 rows · the spec's H1 rows (hc vocabulary, D10 closure, A1 addendum, A4 no wake) and H3 rows (managed_launch via gate shim, C4 staging, ht.bootstrap hint-only) plus the H1 AGENTS.md lines are owned by no leaf
GAP · tmux turn_state_edge for Codex and agy · spec N2 says "Codex and agy map their existing events", but ht-kqrt.36 names only Claude's report-only Stop
GAP · cross-environment messaging that includes hc · no test sends, ACKs and hook-delivers across hc<->named and hc<->pane in one daemon
ORPHAN · H3 leaves ht-kqrt.54 to ht-kqrt.58 · the root goal text has no launch/handoff-into-hc outcome, although the spec scopes H3 in; amend the ht-kqrt goal
UNOWNED-SEAM · seat.created.display_name semantics · ht-kqrt.33 (publisher, no owns line) and ht-kqrt.22 (restore reader) share the set-from-seats.name / SeatName-validated / NULL-on-invalid rule with no owner
UNOWNED-SEAM · hc label vocabulary herdr.space/herdr.tab/ht.bootstrap/ht.label and <session>:tN · written by ht-kqrt.55/ht-kqrt.57, read by ht-kqrt.43, owned by none
UNOWNED-SEAM · default standalone state dir · ht-kqrt.18 creates it, ht-kqrt.19 and ht-kqrt.49 bake --state-dir, with no consumes/dep between ht-kqrt.19 and ht-kqrt.18
UNOWNED-SEAM · launch-gate shim <-> daemon (core,tN) authorization · no leaf owns the daemon command/reply the shim waits on or the bootstrap Send staging
UNOWNED-SEAM · readonly field on the resolve command · ht-kqrt.24 carries it but owns only occupant_bindings.readonly semantics
UNOWNED-SEAM · tmux hint resolve-command field and storage vs locator_hint/M9 · ht-kqrt.38 owns the format but not whether it is a separate field, daemon-memory only, and never a locator_hint
UNOWNED-SEAM · HcDiscovery API and core state paths · ht-kqrt.41 owns HcDiscovery, ht-kqrt.45 implements it with no owns line, and ht-kqrt.49/ht-kqrt.57 consume it from ht-kqrt.45
UNOWNED-SEAM · site-capability gate mechanism for seat.terminal_sites_v1 · ht-kqrt.48 uses ht-kqrt.12's F9 mechanism without consuming it
UNOWNED-SEAM · HcHost terminal.run request/reply · ht-kqrt.55 adds it with the R43 parameter set but has no owns line; ht-kqrt.57/ht-kqrt.58 consume it
GAP · REJECTED hc idle wake "as far as host allows" · spec run constraint 3 puts H2 (hc composer wake) out of scope; no_wake for hc is the intended outcome
GAP · REJECTED user-facing README/install docs · neither the goal nor the spec asks for them; the skill (K1/K4) is the spec's agent-facing doc surface
ORPHAN · REJECTED ht-kqrt.52 hc-smoke · it backs R41 ("hc-smoke status is recorded") and the spec defines its fallback (closes as follow-on, no TSV row)
NEEDS-SPEC · REJECTED ht-kqrt.40/ht-kqrt.48 hc wake decision · answered by the spec: hc composer wake is H2, out of scope
