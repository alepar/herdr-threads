requirements:
R1 → ht-poh.4.2, ht-poh.4.3, ht-poh.4.4, ht-poh.6.5
R2 → ht-poh.4.1, ht-poh.4.3, ht-poh.6.5
R3 → ht-poh.4.3, ht-poh.6.5 (no leaf names resume/C1 explicitly; 4.3 says "lifecycle reading, enrollment")
R4 → ht-poh.3.1, ht-poh.3.2, ht-poh.3.4, ht-poh.5, ht-poh.6.6
R5 → ht-poh.3.6, ht-poh.4.5, ht-poh.5 (implied), ht-poh.6.7
R6 → ht-poh.4.4, ht-poh.6.4
R7 → ht-poh.2.2, ht-poh.4.6, ht-poh.4.8, ht-poh.8, ht-poh.6.4
R8 → ht-poh.1, ht-poh.2.1–ht-poh.2.6
R9 → ht-poh.3.2, ht-poh.3.3
R10 → ht-poh.7 (and ht-poh.1, which carries the same title text)
R11 → ht-poh.3.5, ht-poh.3.6, ht-poh.3.7
R12 → ht-poh.6.1–ht-poh.6.8 (the third tier is not identifiable from the task lines)
R13 → (unmapped)
R-new: The ht-poh.4 agy adapter is written only after the core sweep has emptied every to-migrate entry, so that the R13 measurement is meaningful. → (unmapped as an ordering; ht-poh.4 depends only on ht-poh.1, and its leaves only on ht-poh.2.1)
R-new: Golden or byte-identical baselines exist for Claude, Codex and Hermes wire, JSON, hook output and setup/doctor reports before the sweep and the PushChannel re-fit. → (unmapped; the ht-poh.2 goal relies on "the existing tests guard them")
R-new: The native gate and the live smoke actually run at the final SHA, and the evidence record is committed to docs/evidence/agy-live-smoke/. → (unmapped; ht-poh.6.8 only builds the runner and the README)
R-new: The agy seat "gets its context" (the startup/context injection on enroll). → ht-poh.4.3 (only "TRUST-POLICY startup text"; the epic title says "context", but no leaf owns it explicitly)

Findings:

- **type:** GAP
  - **subject:** R13, the final report that measures code outside src/harness/agy/
  - **evidence:** Root goal: "the final report measures every change made outside the adapter". R13: "The final report states how much code outside `src/harness/agy/` the agy adapter needed". No task in the tree produces this report or the measurement (diffstat by path, allowlist delta).
  - **proposed fix:** Add a new leaf under the ht-poh root: "Final report: measure the non-agy diff of the agy work (per path and per allowlist entry)". It should depend on ht-poh.4.*, ht-poh.5 and ht-poh.6.8.

- **type:** GAP
  - **subject:** The ht-poh.4 Goal section is wrong (it is ht-poh.2's goal)
  - **evidence:** Under "ht-poh.4" the Goal reads "After this subtree, the core … names no harness … `scripts/check-harness-neutral` reports zero `to-migrate` entries … That way the agy adapter (ht-poh.4) can be written entirely inside `src/harness/agy/`". This is the core-sweep goal. ht-poh.2 shows "(no spec)". So ht-poh.4 has no goal describing the agy adapter outcomes (R1, R2, R3, R5, R6, R7), and ht-poh.2 has no stated goal. Coverage of ht-poh.4 cannot be checked against its own goal.
  - **proposed fix:** Move that goal text to ht-poh.2, and write ht-poh.4's real spec/Goal (agy metadata, plugin, skill, allow entry, witness, context, composer, launch, doctor).

- **type:** GAP
  - **subject:** Ordering: the agy adapter is built before the core sweep finishes (root goal "on a core that names no harness"; R13)
  - **evidence:** ht-poh.4 has "deps: ht-poh.1". ht-poh.4.2 has "deps: ht-poh.2.1" only. The (misplaced) goal says "That way the agy adapter (ht-poh.4) can be written entirely inside `src/harness/agy/`" only after zero to-migrate entries. Today nothing makes ht-poh.4.4 wait for ht-poh.2.3 (setup engine), ht-poh.4.7 for ht-poh.2.4 (doctor), ht-poh.4.6 for ht-poh.2.2 (launch surface), or ht-poh.4.3/ht-poh.5 for ht-poh.2.6 (hook runner/attribution). The agy leaves would edit core, or race the sweep.
  - **proposed fix:** Make ht-poh.4 depend on ht-poh.2, or add leaf-level deps: 4.4→2.3, 4.7→2.4, 4.6→2.2, 4.3→2.6, 4.3→2.5.

- **type:** GAP
  - **subject:** End-to-end execution of the gates and the committed live-smoke evidence (ht-poh.6 goal, R12)
  - **evidence:** ht-poh.6 goal: "The orchestrator runs `scripts/agy-live-smoke --live` once at the final SHA … writes a record to `docs/evidence/agy-live-smoke/`". ht-poh.6.8 is "Live smoke runner (`--dry-run` / `--live`) and evidence README". It builds the tool, but no task runs the gate or the smoke at the final SHA, or commits the record. Premortem: every flow module exists, but no task ever runs the whole native gate green on one SHA.
  - **proposed fix:** Add a leaf under ht-poh.6: "Final-SHA run: full `scripts/agy-native-gate`, then `agy-live-smoke --live`, then commit the evidence record". It should depend on every ht-poh.6.* leaf and on the last code leaf.

- **type:** GAP
  - **subject:** Byte-identical baselines for Claude, Codex and Hermes (ht-poh.2 goal, R9)
  - **evidence:** "wire, JSON, hook output and setup/doctor reports stay byte-identical, and the existing tests guard them". R9 asks for "byte-identical wire and behaviour". No task checks that the existing tests cover every surface the sweep touches (setup report lines, doctor text, help text, env markers, foreground advice), or adds golden snapshots before ht-poh.2.2–2.6 and ht-poh.3.3 move code.
  - **proposed fix:** Add a leaf under ht-poh.2 that ht-poh.2.2–2.6 and ht-poh.3.3 depend on: "Golden snapshots of per-harness setup/doctor/help/hook output and wire before the sweep".

- **type:** GAP
  - **subject:** R8, `check-harness-neutral` wired into the per-change checks and CI
  - **evidence:** R8: "`scripts/check-harness-neutral` enforces this in the per-change checks and in CI". The ht-poh.1 line names the script and the allowlist, but no task line mentions adding it to the CI workflow or to the AGENTS.md per-change check command.
  - **proposed fix:** Spell out the CI job and the per-change-check wiring in ht-poh.1, or add a leaf for it.

- **type:** GAP
  - **subject:** R3, resume (`agy -c` / `--conversation`) through C1 continuity
  - **evidence:** Only ht-poh.6.5 (gate flow "resume") and ht-poh.4.6 ("`-c`/`--conversation` conflict" in the launch flags) mention resume. No implementation leaf names C1 cooperative continuity for agy (the resume witness or hook claim). ht-poh.4.3 says only "lifecycle reading, enrollment".
  - **proposed fix:** Name C1 resume continuity in ht-poh.4.3's scope explicitly, or add a leaf under ht-poh.4.

- **type:** GAP
  - **subject:** The agy idle-wake policy declaration (R5)
  - **evidence:** The core wake is in ht-poh.3.6, the `IdleWake` type is in ht-poh.3.1, and the composer is in ht-poh.4.5. ht-poh.5 covers "transcript confirm, pull, systemMessage inject, Stop downgrade". No task says that agy's `delivery_policy()` returns an IdleWake via Herdr composer that uses the 4.5 composer policy. 6.7 tests it.
  - **proposed fix:** Add "agy IdleWake (composer, empty-only)" to the ht-poh.5 scope, and make ht-poh.5 depend on ht-poh.4.5.

- **type:** GAP
  - **subject:** The handoff `--kind agy` path depends on bootstrap V2 (R7)
  - **evidence:** ht-poh.8: "the frozen V1 bootstrap grammar … cli/handoff.rs bootstrap_v1::retain". It has "deps: none", and no task depends on it. ht-poh.4.8 has a "handoff check" and ht-poh.6.4 tests "launch", but neither depends on ht-poh.8. agy handoff could ship on the V1 grammar or break on it.
  - **proposed fix:** Add deps 4.6/4.8→ht-poh.8 and 6.4→ht-poh.8, and add a handoff row to the gate's flows (6.4 lists launch only).

- **type:** ORPHAN
  - **subject:** ht-poh.7, which duplicates ht-poh.1
  - **evidence:** ht-poh.1 is "Neutral rule seam: AGENTS.md harness-neutral rule + delivery contract, check-harness-neutral + allowlist, contract audit". ht-poh.7 is "AGENTS.md harness-neutral rule + delivery contract, and delivery-contract audit with inline fixes". Both cover R8 text and R10. Either one is redundant, or the scope split is unstated.
  - **proposed fix:** Narrow ht-poh.1 to the script and the allowlist (R8 enforcement) and keep ht-poh.7 for the AGENTS.md text and the audit (R10), or delete one of them.

- **type:** UNOWNED-SEAM
  - **subject:** The `DeliveryPolicy` trait, `DeliveryPull`/`DeliveryConfirm` wire and `HarnessAdapter::delivery_policy()`
  - **evidence:** ht-poh.3.1: "**Seam for ht-poh.5.** Add `DeliveryMechanism`, `IdleWake`, the `DeliveryPolicy` trait and `HarnessAdapter::delivery_policy()`". ht-poh.5 (deps ht-poh.3.1), ht-poh.3.3 and ht-poh.3.4 consume it. There is no `owns:` line.
  - **proposed fix:** Add an owns line to ht-poh.3.1 for the trait and the wire messages, with consumes lines on ht-poh.3.3, ht-poh.3.4 and ht-poh.5.

- **type:** UNOWNED-SEAM
  - **subject:** The `local_evidence` + witness rule and the `hook_step` hooks
  - **evidence:** ht-poh.4.1: "Neutral hook seams: `local_evidence` + witness rule, `hook_step` … Lands D4 and D6, inert by default". ht-poh.4.3 ("D5 `local_evidence`") and ht-poh.5 (deps ht-poh.4.1) consume them. There is no owns line.
  - **proposed fix:** Add an owns line to ht-poh.4.1, with consumes lines on ht-poh.4.3 and ht-poh.5.

- **type:** UNOWNED-SEAM
  - **subject:** The agy hook argv and hook-event registration in hooks.json, between ht-poh.4.2, ht-poh.4.4 and ht-poh.5
  - **evidence:** ht-poh.4.2 has "decode/encode, hook argv". ht-poh.4.4 has "render plugin.json, hooks.json". ht-poh.5 needs the pre-model-call and Stop hooks ("systemMessage inject, Stop downgrade") registered in hooks.json, but ht-poh.5 does not depend on ht-poh.4.4, and 4.4 does not depend on 5.
  - **proposed fix:** Contract the hook-event list and the argv format (owned by 4.2), consumed by 4.4 and 5, and add the 5↔4.4 dependency.

- **type:** UNOWNED-SEAM
  - **subject:** The scenario format `agy-fake-gemini/1`
  - **evidence:** ht-poh.6.1: "scenario format `agy-fake-gemini/1` … match routing, `{var.*}`/`{frame.*}`". The scenarios in ht-poh.6.4–6.7 are written in this format. There is no owns line.
  - **proposed fix:** Add an owns line on ht-poh.6.1, with consumes lines on 6.4–6.8.

- **type:** UNOWNED-SEAM
  - **subject:** The isolated agy world and gate-runner API (`agyenv.py`, the flow-module interface)
  - **evidence:** ht-poh.6.2 has "Implement `agyenv.py` … native gate runner". ht-poh.6.4–6.7 add "Flow modules", and ht-poh.6.8 depends on 6.2. There is no owns line for the flow interface or the agyenv API.
  - **proposed fix:** Add an owns line on ht-poh.6.2 for the flow-module contract and the agyenv API.

- **type:** UNOWNED-SEAM
  - **subject:** The relay and `RelayBackend` interface, between ht-poh.6.3 and ht-poh.6.8
  - **evidence:** ht-poh.6.3 has "Gemini↔OpenAI relay with enforced budget". ht-poh.6.8 has "`RelayBackend` (in its own module)" and depends on ht-poh.6.3. There is no owns line for the relay's CLI, config and budget interface.
  - **proposed fix:** Add an owns line on ht-poh.6.3.

- **type:** UNOWNED-SEAM
  - **subject:** The `wake_on` storage, wire and store API
  - **evidence:** ht-poh.3.5 has "DP8 storage, wire, store and CLI". ht-poh.3.6 and ht-poh.3.7 depend on it ("Core wake decision honours `wake_on: all`", "promotion of `wake_on: all` lazy rows"). There is no owns line.
  - **proposed fix:** Add an owns line on ht-poh.3.5, with consumes lines on 3.6 and 3.7.

- **type:** UNOWNED-SEAM
  - **subject:** The `src/delivery/` module surface (D4 frame, page source) after extraction
  - **evidence:** ht-poh.3.2 has "Extract `src/delivery/` from `watch` and port the D4 frame to Rust". ht-poh.3.4 ("`src/delivery/ledger.rs` … store-backed `PageSource`") and ht-poh.3.7 depend on it, and ht-poh.5 presumably frames its systemMessage with the D4 frame. There is no owns line.
  - **proposed fix:** Add an owns line on ht-poh.3.2 for the frame and the module API, with consumes lines on 3.4, 3.7 and 5.

- **type:** UNOWNED-SEAM
  - **subject:** The delivery-contract text (AGENTS.md §Delivery contract), which the re-fit consumes
  - **evidence:** ht-poh.3.2, ht-poh.3.3, ht-poh.3.4 and ht-poh.3.6 all have "deps: … ht-poh.7". ht-poh.7 has no owns line for the contract they implement against. ht-poh.2.1's "consumes: … the adapter-owned path rule of D14 (ht-poh.1)" names ht-poh.1 as the producer, and ht-poh.1 also has no owns line.
  - **proposed fix:** Add owns lines on ht-poh.1 (allowlist format, the D14 path rule) and on ht-poh.7 (the delivery contract).

- **type:** UNOWNED-SEAM
  - **subject:** The L3 setup status, consumed by the agy doctor projection
  - **evidence:** ht-poh.4.7: "`doctor.rs` per D10: projection from L3 status". L3 is ht-poh.2.3, whose owns line names only "the stable legacy helper signatures above, which L4 consumes". ht-poh.4.7 depends only on ht-poh.4.4.
  - **proposed fix:** Extend ht-poh.2.3's owns line to the status projection type, and add 4.7→2.3 and 4.7→2.4.

- **type:** UNOWNED-SEAM
  - **subject:** The composer policy, consumed by the idle wake
  - **evidence:** ht-poh.4.5 has "Composer policy from model-free TUI captures". ht-poh.6.7 has "wake … into an empty composer" (deps ht-poh.4.5), and the core wake (ht-poh.3.6) has to call into it. There is no owns line.
  - **proposed fix:** Add an owns line on ht-poh.4.5.

NEEDS-SPEC: ht-poh.4 — its Goal section holds ht-poh.2's text, so the agy adapter's intended outcomes (context injection, resume/C1, idle-wake declaration) cannot be checked against the leaves.
NEEDS-SPEC: ht-poh.6 — R12's "three tiers" are truncated, so I could not judge whether the unit and offline-CI tier is covered beyond ht-poh.6.1's "CI line".