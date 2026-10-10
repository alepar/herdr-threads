requirements:
R1 → ht-poh.4.2, ht-poh.4.3, ht-poh.4.4, ht-poh.2.1 (agent_env_markers), ht-poh.2.2, ht-poh.6.5
R2 → ht-poh.4.1, ht-poh.4.3, ht-poh.6.5
R3 → ht-poh.4.3 (lifecycle reading, weak), ht-poh.6.5
R4 → ht-poh.3.1, ht-poh.3.2, ht-poh.3.4, ht-poh.5, ht-poh.6.6
R5 → ht-poh.3.1 (IdleWake), ht-poh.3.6, ht-poh.4.5, ht-poh.6.7 (no leaf explicitly sets agy's IdleWake policy; see GAP)
R6 → ht-poh.4.4, ht-poh.4.7, ht-poh.6.4
R7 → ht-poh.2.2, ht-poh.4.6, ht-poh.4.8, ht-poh.8, ht-poh.6.4
R8 → ht-poh.1, ht-poh.2.1–ht-poh.2.6 (CI wiring of the check is not named in any line; see GAP)
R9 → ht-poh.3.2, ht-poh.3.3
R10 → ht-poh.1, ht-poh.7
R11 → ht-poh.3.5, ht-poh.3.6, ht-poh.3.7
R12 → ht-poh.6.1–ht-poh.6.8 (tier 1, offline Rust/unit tests of the agy adapter, is implied but not named; only "CI line for offline agy-script tests" in ht-poh.6.1)
R13 → (unmapped)
R-new: The agy session "gets its context" (startup/check-in context injected at session start, as Claude gets it) → ht-poh.4.3 at best ("TRUST-POLICY startup text"); no leaf names agy context injection
R-new: Receipt settlement through transcript confirmation (and witness-based subagent/clear attribution) gets defined provenance values in TRUST-POLICY.md, updated in the same commit → (unmapped)
R-new: An actual passing native-gate run and the live-smoke evidence record at the final SHA are produced and committed → (unmapped; ht-poh.6.8 only builds the runner and README)

findings:

- **type:** GAP
  **subject:** R13 (final report measures every change made outside `src/harness/agy/`)
  **evidence:** The root goal says "the final report measures every change made outside the adapter". No task produces or even scaffolds that measurement: no diff-accounting script, and no report leaf. ht-poh.8 cites "(R7, R13)", but it is the bootstrap V2 change, which is itself an outside-adapter change and not a measurement.
  **proposed fix:** Add a leaf under the root, e.g. "ht-poh.9 · Outside-adapter change accounting". It would run a script, or a documented `git diff --stat` partition, over main..final SHA that splits lines into `src/harness/agy/` (plus the `builtins()` row) and everything else, grouped by subepic. Its output goes into the final report. It depends on every other leaf.

- **type:** GAP
  **subject:** R8: `check-harness-neutral` enforced "in the per-change checks and in CI"
  **evidence:** ht-poh.1's line says only "seam: scripts/check-harness-neutral (shell) failing on harness id literals …". No line mentions adding it to the CI workflow or to the AGENTS.md per-change check command. ht-poh.7 covers only the AGENTS.md rule text. The premortem: the script exists and the allowlist is empty, but nothing runs it, so it regresses.
  **proposed fix:** Make CI and per-change wiring explicit in ht-poh.1, or add a leaf under ht-poh.2 such as "Wire check-harness-neutral into CI + AGENTS.md per-change checks", which asserts zero `to-migrate` entries after ht-poh.2.6.

- **type:** GAP
  **subject:** Root outcome "gets its context" (agy session start context)
  **evidence:** The root goal says "It enrolls on its own, gets its context …", and the ht-poh.4 title lists "context". No ht-poh.4.x leaf names context injection. ht-poh.4.3 has "enrollment, TRUST-POLICY startup text" and ht-poh.4.4 has the skill, but nothing names the hook output that carries the check-in or attention context into the agy model at session start or after `/clear` and resume.
  **proposed fix:** Name it in ht-poh.4.3, or add a leaf under ht-poh.4: "agy session-start context injection (check-in/attention digest via hook output), with a native-gate assertion in ht-poh.6.5".

- **type:** GAP
  **subject:** TRUST-POLICY provenance for transcript-confirmed receipts (R4) and witness-based binding decisions (R2)
  **evidence:** AGENTS.md says "a new way to attribute an action needs a provenance value defined in the policy" and "A change that weakens an invariant … updates TRUST-POLICY.md in the same commit." R4 introduces settlement "only after the session's own transcript shows the delivery", which is a new receipt provenance. R2 introduces daemon decisions "from on-disk witnesses the hook reports". The only TRUST-POLICY mention is ht-poh.4.3 "TRUST-POLICY startup text". ht-poh.3.4 (offered ledger) and ht-poh.5 (transcript confirm) name no policy update.
  **proposed fix:** Add a TRUST-POLICY.md update to ht-poh.3.4/ht-poh.5 (receipt provenance for DeliveryConfirm) and to ht-poh.4.1 (witness evidence class), or add a dedicated leaf "TRUST-POLICY: HookPull receipt provenance + witness evidence".

- **type:** GAP
  **subject:** R5 for agy specifically: agy's `DeliveryPolicy` idle-wake setting and the composer wiring
  **evidence:** ht-poh.3.1 defines `IdleWake` with `delivery_policy()` defaulting to `None`. ht-poh.5's scope is "transcript confirm, pull, systemMessage inject, Stop downgrade", with no idle wake. ht-poh.4.5 produces the composer policy only "from model-free TUI captures". No leaf states that agy's policy returns a wake mode that routes through ht-poh.3.6's core wake decision. ht-poh.6.7 tests it, but nothing implements the agy side.
  **proposed fix:** Extend ht-poh.5 to name agy's `IdleWake` value and its wiring to the composer policy, or add a leaf under ht-poh.4/ht-poh.5.

- **type:** GAP
  **subject:** R12 evidence: the native gate is actually run, and the live smoke runs at the final SHA and its record is committed
  **evidence:** The ht-poh.6 goal says "The orchestrator runs `scripts/agy-live-smoke --live` once at the final SHA … writes a record to `docs/evidence/agy-live-smoke/`". ht-poh.6.8 builds the "runner … and evidence README" only. No leaf runs the gate over all flows on a genuine `HT_AGY_BIN`, or commits the live record. The premortem: every runner exists, but no proof exists at the final SHA.
  **proposed fix:** Add a final leaf under ht-poh.6, "Run agy-native-gate + agy-live-smoke --live at final SHA; commit evidence record", that depends on every gate flow and on all of ht-poh.2/ht-poh.3/ht-poh.4/ht-poh.5.

- **type:** GAP
  **subject:** End-to-end thin path: the live smoke and the agy adapter depend on unfinished core pieces
  **evidence:** ht-poh.6.8's flow is "enroll, then wake, `systemMessage` …", but its deps omit ht-poh.3.4 (DeliveryPull handlers) and ht-poh.3.6 (core wake). ht-poh.5 depends on ht-poh.3.1 (inert handlers) but not ht-poh.3.4, which "Replace[s] L1's inert arms". So the agy delivery can be built against inert arms. ht-poh.4.4 (setup) depends only on ht-poh.4.2, while ht-poh.2.3 "owns: the stable legacy helper signatures above, which L4 consumes unchanged". ht-poh.4.7 consumes the "projection from L3 status" (ht-poh.2.3) and the D7 doctor (ht-poh.2.4) without depending on either. ht-poh.4.6 (launch) does not depend on ht-poh.2.2 (launch/handoff surface). The ht-poh.4 epic depends on ht-poh.1 only, yet the goal text says the agy adapter is written "entirely inside `src/harness/agy/`" after the sweep.
  **proposed fix:** Add the deps ht-poh.5→ht-poh.3.4, ht-poh.6.8→ht-poh.3.4/ht-poh.3.6, ht-poh.4.4→ht-poh.2.3, ht-poh.4.7→ht-poh.2.3/ht-poh.2.4, ht-poh.4.6→ht-poh.2.2, and ht-poh.4→ht-poh.2. The alternative is to state on ht-poh.4 that it lands against inert seams and add a Seam integration leaf.

- **type:** GAP
  **subject:** R7 `handoff … --kind agy` depends on bootstrap V2 (ht-poh.8), which is not wired in
  **evidence:** ht-poh.8 says "the frozen V1 bootstrap grammar (harness/launch.rs compose_bootstrap_v1_argv …; cli/handoff.rs bootstrap_v1::retain …" and has deps: none. ht-poh.4.6 (launch) and ht-poh.4.8 ("handoff check") do not depend on ht-poh.8, and ht-poh.6.4's `launch` flow does not depend on it either. If agy handoff needs adapter-neutral retained composition, the agy handoff path has no ordered prerequisite.
  **proposed fix:** Make ht-poh.4.6, ht-poh.4.8 and ht-poh.6.4 depend on ht-poh.8, or reparent ht-poh.8 under ht-poh.2. Also add a gate assertion for `handoff --kind agy`. The ht-poh.6 goal lists only "launch".

- **type:** ORPHAN
  **subject:** ht-poh.7 (duplicate of ht-poh.1)
  **evidence:** ht-poh.1 is titled "Neutral rule seam: AGENTS.md harness-neutral rule + delivery contract, check-harness-neutral + allowlist, contract audit". ht-poh.7 is titled "AGENTS.md harness-neutral rule + delivery contract, and delivery-contract audit with inline fixes". Both cite "Spec §1 … (R8 …)" and R10. The two scopes overlap on AGENTS.md text and the contract audit. ht-poh.3.2/ht-poh.3.3/ht-poh.3.4/ht-poh.3.6 depend on ht-poh.7, while ht-poh.2.1/ht-poh.3.1/ht-poh.3.5 depend on ht-poh.1.
  **proposed fix:** Narrow ht-poh.1 to the script, the allowlist and the seam, and leave the AGENTS.md text and audit to ht-poh.7, or merge the two. State which one owns the AGENTS.md delivery contract text.

- **type:** UNOWNED-SEAM
  **subject:** `check-harness-neutral` allowlist format and `to-migrate` entries (ht-poh.1 → ht-poh.2.1–ht-poh.2.6, ht-poh.4)
  **evidence:** ht-poh.2.1 says "consumes: `check-harness-neutral` and the allowlist format and `to-migrate` entries (ht-poh.1); the adapter-owned path rule of D14 (ht-poh.1)". ht-poh.1 has no `owns:` line.
  **proposed fix:** Add to ht-poh.1 "owns: allowlist file format (entry kinds permanent/to-migrate, reason field), D14 adapter-owned path rule".

- **type:** UNOWNED-SEAM
  **subject:** `DeliveryPolicy` trait, `DeliveryMechanism`/`IdleWake`, `DeliveryPull`/`DeliveryConfirm` wire (ht-poh.3.1 → ht-poh.3.3, ht-poh.3.4, ht-poh.5)
  **evidence:** ht-poh.3.1 says "**Seam for ht-poh.5.** Add `DeliveryMechanism`, `IdleWake`, the `DeliveryPolicy` trait and `HarnessAdapter::delivery_policy()`". ht-poh.3.4 says "Replace L1's inert arms with DP5/DP6". ht-poh.3.1 has no `owns:` line, and ht-poh.5 and ht-poh.3.4 have no `consumes:` line.
  **proposed fix:** Add an owns line on ht-poh.3.1 for the trait plus the wire request/response shapes, and consumes lines on ht-poh.3.3, ht-poh.3.4 and ht-poh.5.

- **type:** UNOWNED-SEAM
  **subject:** Offered-ledger / confirm semantics (ht-poh.3.4 ↔ ht-poh.5)
  **evidence:** ht-poh.3.4 has "offered ledger, pull-seat routing" (`src/delivery/ledger.rs`). ht-poh.5 has "transcript confirm, pull, systemMessage inject". agy's hook issues `DeliveryPull` and later `DeliveryConfirm`, both against ht-poh.3.4's ledger. The contract (what is offered, the bounds, when it expires, what a confirm carries) is named nowhere.
  **proposed fix:** Add an owns line on ht-poh.3.4 for the ledger offer/confirm contract, with ht-poh.5 consuming it.

- **type:** UNOWNED-SEAM
  **subject:** `local_evidence` + witness rule, `hook_step` (ht-poh.4.1 → ht-poh.4.3, ht-poh.5)
  **evidence:** ht-poh.4.1 says "Neutral hook seams: `local_evidence` + witness rule, `hook_step` … Lands D4 and D6, inert by default". ht-poh.4.3 says "`witness.rs`: D5 `local_evidence` …". ht-poh.5 depends on ht-poh.4.1. No owns line exists.
  **proposed fix:** Add to ht-poh.4.1 "owns: `local_evidence` report shape, daemon witness decision rule (fail toward child), `hook_step` signature".

- **type:** UNOWNED-SEAM
  **subject:** CLI launch/handoff surface and `AdapterText` fields (ht-poh.2.1/ht-poh.2.2 → ht-poh.4.6, ht-poh.4.8)
  **evidence:** ht-poh.2.2 "consumes: … `AdapterText` (`setup_*`, `launch_notes`, `config_root_hint`)". ht-poh.4.6 implements "`AgyLaunch` per D8 … `-c`/`--conversation` conflict". ht-poh.2.2 has no owns line for the launch/handoff surface that agy plugs into. ht-poh.2.1's owns line is truncated and cannot be confirmed to cover `AdapterText`.
  **proposed fix:** Add to ht-poh.2.2 an owns line covering the launch-policy contract consumed by the agy `launch_policy`, and confirm that ht-poh.2.1 owns `AdapterText`.

- **type:** UNOWNED-SEAM
  **subject:** Fake Gemini scenario format `agy-fake-gemini/1` (ht-poh.6.1 → ht-poh.6.4–ht-poh.6.7)
  **evidence:** ht-poh.6.1 has "scenario format `agy-fake-gemini/1` … match routing, `{var.*}`/`{frame.*}`". ht-poh.6.4–ht-poh.6.7 each say "Flow modules and scenarios …". No owns line exists.
  **proposed fix:** Add to ht-poh.6.1 "owns: scenario format agy-fake-gemini/1".

- **type:** UNOWNED-SEAM
  **subject:** Isolated agy world / gate runner API (`agyenv.py`) (ht-poh.6.2 → ht-poh.6.4–ht-poh.6.8)
  **evidence:** ht-poh.6.2 says "Implement `agyenv.py` … `restage_agy`, isolated Herdr …". Every flow leaf and ht-poh.6.8 depend on it. No owns line exists.
  **proposed fix:** Add to ht-poh.6.2 owns covering the flow-module interface and the agyenv API.

- **type:** UNOWNED-SEAM
  **subject:** Relay / OpenAI translation and the budget contract (ht-poh.6.3 → ht-poh.6.8 `RelayBackend`)
  **evidence:** ht-poh.6.3 has "Gemini↔OpenAI relay with enforced budget". ht-poh.6.8 has "`RelayBackend` (in its own module)". No owns line exists for the relay's startup, configuration or budget interface.
  **proposed fix:** Add to ht-poh.6.3 owns covering the relay CLI/config and the budget-cap contract.

- **type:** UNOWNED-SEAM
  **subject:** Composer policy (ht-poh.4.5 → ht-poh.3.6 core wake / ht-poh.6.7)
  **evidence:** ht-poh.4.5 produces the "Composer policy from model-free TUI captures". ht-poh.6.7 tests "wake … into an empty composer". ht-poh.3.6's core wake decision must consult it. No owns line exists.
  **proposed fix:** Add to ht-poh.4.5 owns covering the composer-state classification consumed by core wake.

NEEDS-SPEC: ht-poh.4 — its Goal text is the core-sweep goal ("`scripts/check-harness-neutral` reports zero `to-migrate` entries … so the agy adapter (ht-poh.4) can be written"), not the agy adapter goal. The real ht-poh.4 outcomes (context, enrollment, R2/R3 specifics) cannot be judged.
NEEDS-SPEC: ht-poh.2 / ht-poh.3 — both say "(no spec)", so I could not check the coverage of D1–D16 and DP1–DP8 (for example the D15 L-rows, and whether DP4 or DP7 have a leaf).