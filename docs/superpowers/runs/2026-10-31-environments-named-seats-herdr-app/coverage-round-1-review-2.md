requirements:
R1 → ht-kqrt.10, ht-kqrt.11, ht-kqrt.13, ht-kqrt.56
R2 → ht-kqrt.7
R3 → ht-kqrt.5, ht-kqrt.6
R4 → ht-kqrt.4, ht-kqrt.44
R5 → ht-kqrt.9
R6 → ht-kqrt.10, ht-kqrt.9, ht-kqrt.56
R7 → ht-kqrt.10, ht-kqrt.11
R8 → ht-kqrt.11
R9 → ht-kqrt.2
R10 → ht-kqrt.13
R11 → ht-kqrt.15, ht-kqrt.16, ht-kqrt.18, ht-kqrt.20, ht-kqrt.21, ht-kqrt.34
R12 → ht-kqrt.19
R13 → ht-kqrt.18
R14 → ht-kqrt.23, ht-kqrt.24
R15 → ht-kqrt.24
R16 → ht-kqrt.34, ht-kqrt.30, ht-kqrt.18
R17 → ht-kqrt.27
R18 → ht-kqrt.21, ht-kqrt.15, ht-kqrt.25
R19 → ht-kqrt.28
R20 → ht-kqrt.31, ht-kqrt.32
R21 → ht-kqrt.26
R22 → ht-kqrt.22
R23 → ht-kqrt.33
R24 → ht-kqrt.30, ht-kqrt.20
R25 → ht-kqrt.30, ht-kqrt.10 (weak: no line names the named-caller recovery hint)
R26 → ht-kqrt.17, ht-kqrt.15, ht-kqrt.29; M11 part (unmapped)
R27 → ht-kqrt.25, ht-kqrt.6
R28 → ht-kqrt.17, ht-kqrt.32, ht-kqrt.21 (S1), ht-kqrt.27 (E1); S2, S4 and the named half of S5 (unmapped)
R29 → ht-kqrt.38, ht-kqrt.39
R30 → ht-kqrt.36, ht-kqrt.37, ht-kqrt.39
R31 → ht-kqrt.38, ht-kqrt.35
R32 → ht-kqrt.39, ht-kqrt.6
R33 → ht-kqrt.36
R34 → ht-kqrt.39 (partial); delivery-contract table, AGENTS.md tmux wake line, T1-T5 record (unmapped)
R35 → ht-kqrt.41, ht-kqrt.46, ht-kqrt.48
R36 → ht-kqrt.47
R37 → ht-kqrt.44
R38 → ht-kqrt.42
R39 → ht-kqrt.46, ht-kqrt.43, ht-kqrt.48, ht-kqrt.26
R40 → ht-kqrt.50
R41 → ht-kqrt.45, ht-kqrt.49, ht-kqrt.40, ht-kqrt.51, ht-kqrt.52
R42 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.55, ht-kqrt.57
R43 → ht-kqrt.55, ht-kqrt.57
R44 → ht-kqrt.53, ht-kqrt.54, ht-kqrt.56
R45 → ht-kqrt.58; L1-L4 answers (unmapped)
R-new-1: Claude mod channel delivery (and its live-channel wake) works for Named and Terminal caller sites, not only Herdr panes → (unmapped)
R-new-2: Cross-node addressing and display of hc-terminal seats (labels like `<session>:tN`, or the name, published and resolved on a remote node) → (unmapped)
R-new-3: Cross-environment messaging in one daemon: send, inbox, ACK and hook delivery between an hc-terminal seat and a named seat or Herdr pane seat → (unmapped; ht-kqrt.34 covers named/pane only, ht-kqrt.48 covers hc only)
R-new-4: TRUST-POLICY carries the H1 rows: Terminal-site attribution, LocateCaller root verification as provenance, and finish-reason closure as binding-end evidence → (unmapped)
R-new-5: The release-gating wake e2e (real agents) covers the new tmux composer wake path, or explicitly records it as out of scope → (unmapped)
R-new-6: User-facing docs (README/install docs) describe HERDR_THREADS_SEAT_NAME, standalone install and hc terminals → (unmapped; only skill K1/K4 and AGENTS.md lines are owned)

- **type:** GAP
  - **subject:** R-new-1 (goal: "turn-hook and channel delivery" for all three environments)
  - **evidence:** The goal requires channel delivery in every environment. R20 and R35 assert "live-mod Claude reports no block / no no_wake", which assumes a live mod channel on Named and Terminal sites. No task changes the Claude mod (register.js) to locate a caller through HERDR_THREADS_SEAT_NAME or HC_TERMINAL_ID. ht-kqrt.17 covers only the Hermes bridge, and ht-kqrt.49 covers only "Hermes PANE_ENV terminal vars".
  - **proposed fix:** New leaf under ht-kqrt: mod channel caller location for Named and Terminal sites (register.js reads the same env family as caller_location.rs, or calls the CLI to resolve the site). Test it on a named and an hc site with live-mod Claude.

- **type:** GAP
  - **subject:** R26 (M11, handoff into a named seat)
  - **evidence:** R26 lists "handoff into a named seat (M11)". No task's title or owns line names M11. ht-kqrt.17 owns M14/M15/M16, ht-kqrt.29 owns M10, and ht-kqrt.30 covers handoff *out of* a named seat.
  - **proposed fix:** Add M11 to ht-kqrt.30's owns, or add a new leaf under ht-kqrt that depends on ht-kqrt.16 and ht-kqrt.20.

- **type:** GAP
  - **subject:** R25 (named-caller handoff recovery hint)
  - **evidence:** R25 says "recovery hint for a named caller contains HERDR_THREADS_SEAT_NAME=<n>". ht-kqrt.10 owns recovery hints, but only for the Pane family. ht-kqrt.30 says only "handoff out of a named seat". No line states the hint change.
  - **proposed fix:** Name the named-caller recovery hint explicitly in ht-kqrt.30 (or ht-kqrt.17), with a test that running the hint resolves to the same seat.

- **type:** GAP
  - **subject:** R28 (spikes S2, S4 and the named half of S5)
  - **evidence:** Some spikes are named in task lines: S1 in ht-kqrt.21, S3 in ht-kqrt.17, S6 in ht-kqrt.32, and E1 in ht-kqrt.27 (the Linux environ reader). No line mentions S2, S4 or S5.
  - **proposed fix:** Assign S2, S4 and S5 (named half) to specific leaves, or add one leaf under ht-kqrt that records the spike answers as tests.

- **type:** GAP
  - **subject:** R34 (delivery-contract table in the commit message, AGENTS.md tmux wake line, test tmux servers killed with a clean leak check, T1-T5 recorded)
  - **evidence:** ht-kqrt.39 owns only "ComposerTraits::TMUX; built-in session wake lane registration". No owns line covers the AGENTS.md tmux line, the T1-T5 record or the delivery-contract audit table. ht-kqrt.38 owns the private tmux test helper, but leak recognition of tmux servers is not stated (contrast ht-kqrt.40, which owns "leak-check recognition of herdr-core serve/hold").
  - **proposed fix:** Extend ht-kqrt.39's owns with the delivery-contract table, the AGENTS.md tmux wake line and T1-T5. Extend ht-kqrt.38 with tmux-server leak-check recognition.

- **type:** GAP
  - **subject:** R45 (L1-L4 answered with live parts listed as remaining evidence)
  - **evidence:** ht-kqrt.58 covers peer launch, the cap and the alias. No task line mentions L1-L4.
  - **proposed fix:** Add L1-L4 to ht-kqrt.57 or ht-kqrt.58, or add a recording leaf under ht-kqrt.

- **type:** GAP
  - **subject:** R-new-2 (cross-node addressing of hc-terminal seats)
  - **evidence:** The goal says "cross-environment and cross-node addressing" for all three environments. ht-kqrt.33 publishes only `display_name` (name@node). ht-kqrt.43 defines `<session>:tN` labels locally. No task publishes or resolves hc labels or hc seats across nodes, and R23 tests only "reviewer@B".
  - **proposed fix:** New leaf under ht-kqrt that depends on ht-kqrt.43 and ht-kqrt.33: a remote node addresses and displays an hc-terminal seat, with an old-node fixture.

- **type:** GAP
  - **subject:** R-new-3 (cross-environment messaging that includes hc)
  - **evidence:** Premortem: every part exists, but no single test sends between an hc seat and a named or pane seat. ht-kqrt.34 is "named seats end to end ... mixed messaging" (named and pane). ht-kqrt.48 is "hc messaging end to end". ht-kqrt.59 "exercises each enumerated configuration once" per caller site, not across pairs of sites.
  - **proposed fix:** Extend ht-kqrt.59 (or add a leaf after ht-kqrt.48 and ht-kqrt.34) with send, ACK and hook delivery across hc↔named and hc↔pane in one daemon.

- **type:** GAP
  - **subject:** R-new-4 (TRUST-POLICY for the H1 Terminal site)
  - **evidence:** AGENTS.md trust rules require a provenance value for every new attribution path and forbid ending a binding on heuristic evidence. ht-kqrt.42 maps hc finish reasons to retire/unresolve, and ht-kqrt.46 adds Terminal attribution through LocateCaller. Only ht-kqrt.32 ("TRUST-POLICY A4") and ht-kqrt.17 (the AGENTS.md named line) touch policy docs, and both are N-side.
  - **proposed fix:** New leaf under ht-kqrt (or added to ht-kqrt.46's or ht-kqrt.42's owns): TRUST-POLICY rows for Terminal-site provenance and finish-reason closure evidence.

- **type:** GAP
  - **subject:** R-new-5 (tmux wake in the real-agent wake e2e)
  - **evidence:** The goal says "Idle wake works as far as each host allows". ht-kqrt.59 uses only "hook stand-ins", and ht-kqrt.39 tests against a private tmux server. No task extends scripts/wake-e2e.py, which gates releases, with a tmux case. As things stand, the real Claude Stop hook and Codex turn edge are never proven to drive the tmux wake.
  - **proposed fix:** New leaf after ht-kqrt.39: an opt-in wake-e2e case for a named seat in tmux, or an explicit record that the case is out of scope.

- **type:** GAP
  - **subject:** R-new-6 (user-facing documentation)
  - **evidence:** Only the skill (K1, K4) and AGENTS.md lines are owned (ht-kqrt.17, ht-kqrt.32, ht-kqrt.48). No task documents HERDR_THREADS_SEAT_NAME, HERDR_THREADS_SEAT_READONLY, standalone install or hc use for users.
  - **proposed fix:** New leaf under ht-kqrt: terse README/install-doc sections.

- **type:** UNOWNED-SEAM
  - **subject:** hc terminal label keys (`herdr.space`, `herdr.tab`, `ht.bootstrap`, `ht.label`)
  - **evidence:** ht-kqrt.57 and R43 write them: "--space sends the herdr.space/tab/ht.bootstrap/ht.label set". ht-kqrt.43 reads them: "take workspace from `herdr.space`, tab from `herdr.tab`". ht-kqrt.43 has no owns line, ht-kqrt.55 and ht-kqrt.57 have none either, and no task names who reads `ht.bootstrap` or `ht.label`.
  - **proposed fix:** Contract the hc label vocabulary, meaning keys, writers and readers. ht-kqrt.43 should own it, and ht-kqrt.55 and ht-kqrt.57 consume it.

- **type:** UNOWNED-SEAM
  - **subject:** launch-gate shim ↔ daemon `(core,tN)` authorization
  - **evidence:** ht-kqrt.54 and R42 describe a shim "waiting for the daemon's (core,tN) authorization before exec, with the bootstrap Send staged". ht-kqrt.54 owns only the shim, and ht-kqrt.53 owns only "V3 bootstrap journal records". No task owns the daemon command or reply the shim waits on, or the staging of the bootstrap Send. ht-kqrt.57 consumes "gate shim (H3b)".
  - **proposed fix:** Contract the authorization protocol (command, reply, timeout, capability) and give it an owner in ht-kqrt.53 or ht-kqrt.54.

- **type:** UNOWNED-SEAM
  - **subject:** tmux hint persistence and verification state
  - **evidence:** ht-kqrt.38 says "`tmux:<socket>:<server_pid>:<pane>`, carried in the resolve command" and owns "per-binding verification". ht-kqrt.4's 0012 schema lists `locator_hint`/`readonly` but no tmux-hint column. ht-kqrt.28 owns `locator_hint` recording, and ht-kqrt.27 owns "locator_hint field on the named resolve command". It is not said whether the tmux hint is a `locator_hint` (which would trigger M9) or separate storage, and ht-kqrt.39 reads it to type into the pane.
  - **proposed fix:** Contract the tmux hint's resolve-command field and its storage, and how it relates to `locator_hint`/M9. ht-kqrt.38 should own it, and ht-kqrt.4 and ht-kqrt.28 should state the decision.

- **type:** UNOWNED-SEAM
  - **subject:** HcDiscovery state paths and discovery API
  - **evidence:** ht-kqrt.49 "consumes: ... HcDiscovery state paths (H1d)" and ht-kqrt.57 "consumes: ... HcDiscovery (H1d)". ht-kqrt.45 (H1d) has no owns line. ht-kqrt.41 owns "HcHost / HcBoot / HcDiscovery", so the owner of the paths and the API is split or ambiguous.
  - **proposed fix:** Give ht-kqrt.45 an owns line for the HcDiscovery API and its state paths, or state that ht-kqrt.41 owns them and ht-kqrt.45 implements them.

- **type:** UNOWNED-SEAM
  - **subject:** HcHost `terminal.run` request/reply
  - **evidence:** ht-kqrt.57 "consumes: HcHost terminal.run (H3c)" and ht-kqrt.58 "consumes: HcHost terminal.run (H3c)". ht-kqrt.55 (H3c) has no owns line. ht-kqrt.40 owns the hc client, but ht-kqrt.55 adds `terminal.run` with params that R43 fixes (`no_notify`, `outlive_parent`, `no_parent`, no caller, no `first_message`).
  - **proposed fix:** Add an owns line to ht-kqrt.55: the terminal.run client method and its parameter set.
