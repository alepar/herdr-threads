requirements:
R1 → ht-poh.4.2, ht-poh.4.3, ht-poh.4.4, ht-poh.6.5
R2 → ht-poh.4.1, ht-poh.4.3, ht-poh.6.5
R3 → ht-poh.4.3, ht-poh.6.5
R4 → ht-poh.3.1, ht-poh.3.2, ht-poh.3.4, ht-poh.5, ht-poh.6.6
R5 → ht-poh.4.5, ht-poh.5, ht-poh.3.3 (wake_suppressed), ht-poh.6.7
R6 → ht-poh.2.3, ht-poh.4.4, ht-poh.6.4
R7 → ht-poh.2.2, ht-poh.4.6, ht-poh.4.8, ht-poh.8, ht-poh.6.4
R8 → ht-poh.1, ht-poh.2.1–2.6, ht-poh.7 (text)
R9 → ht-poh.2.7, ht-poh.3.2, ht-poh.3.3
R10 → ht-poh.7
R11 → ht-poh.3.5, ht-poh.3.6, ht-poh.3.7
R12 → ht-poh.6.1–6.8 (third tier is assumed to be the offline agy-script tests plus the Rust tests in each leaf. The requirement text is truncated after "in three tiers:", so this can't be confirmed)
R13 → ht-poh.9, ht-poh.1 / ht-poh.7 (trailer). No task runs the tool at the final SHA (see GAP)
R14 → ht-poh.10, ht-poh.6.9
R15 → ht-poh.4.3, ht-poh.6.5
R16 → ht-poh.2.7 (partial: it does not gate every task that moves code; see GAP)
R-new: When the sweep ends, the check fails on any remaining or new `to-migrate` entry, so the rule is enforced from then on and not just reported → (unmapped)
R-new: agy delivery degradations raised by ht-poh.5 (Stop downgrade) appear in `doctor` → partly ht-poh.4.7 (unwired)

---

- **type:** GAP
- **subject:** R16 / R9: golden baselines do not come before every task that moves code (previous-round fix c4 falls short)
- **evidence:** R16 says baselines must exist "before the sweep and Claude re-fit move code". ht-poh.2.7 gates 2.2–2.6 and 3.3 only. These tasks also move or rewrite per-harness code and have no edge onto 2.7:
  - ht-poh.3.2 (deps: ht-poh.7). Its line says "Move-only extraction (DP2)" of `src/delivery/` from `watch`. That is Claude's v0.5.0 delivery code.
  - ht-poh.2.1 (deps: ht-poh.1). It adds "the `agent_env_markers` metadata field (all 48 literals in src and tests)", which rewrites literal sites.
  - ht-poh.8 (deps: none). It changes "the frozen V1 bootstrap grammar (harness/launch.rs compose_bootstrap_v1_argv …; cli/handoff.rs …)", which Claude, Codex and Hermes handoff use.
  - ht-poh.3.7 (deps: 3.2, 3.5). It changes the Claude push channel.
  - ht-poh.7 (deps: none). It is an "audit with inline fixes" to delivery code.
- **proposed fix:** Add edges from ht-poh.3.2, 2.1, 8, 3.7 and 7 onto ht-poh.2.7. Alternatively, have 2.7 snapshot only after ht-poh.7's intended fixes and say so explicitly. In either case, state whether ht-poh.7's fixes may change the golden output.
- ledger: c4

- **type:** GAP
- **subject:** ht-poh.7's inline fixes against byte-identity (R9 and R10 pull in opposite directions)
- **evidence:** ht-poh.7 is an "audit with inline fixes" and has no deps. ht-poh.2.7 also has no deps. R10 calls for divergences to be "fixed", and R9 requires Claude's wire and behaviour to stay "byte-identical". Nothing orders the two tasks, and nothing says whether a fix may change a golden surface. Either the snapshots go stale, or a fix gets blocked as a regression.
- **proposed fix:** Add the edge ht-poh.2.7 ← ht-poh.7, or the reverse with a re-baseline step. Record in ht-poh.7 that each fix which changes a surface updates its golden and names this in the commit.

- **type:** GAP
- **subject:** ht-poh.5 is missing dependency edges on what it consumes (previous-round fixes c7 and c13 close the seam only on paper)
- **evidence:** ht-poh.5's line says it "consumes: offered-ledger confirm semantics (DeliveryConfirm outcomes, re-offer bounds) owned by ht-poh.3.4; the D4 frame text via DeliveryPage owned by ht-poh.3…". The change log adds "IdleWake::Native via 4.5 composer". But ht-poh.5's deps are only 3.1, 4.1, 4.2 and 4.3. There is no edge to 3.4, 3.2 or 4.5. Edges to 4.4 (`hooks.json` must register the PreInvocation/Stop delivery hooks) and 4.7 are also missing. Against 3.1's inert handlers, ht-poh.5 cannot prove transcript-confirmed settlement.
- **proposed fix:** Add the edges ht-poh.5 ← 3.4, 3.2, 4.5 and 4.4.
- ledger: c7, c13

- **type:** GAP
- **subject:** ht-poh.3.3 is missing an edge to ht-poh.3.2, which creates `src/delivery`
- **evidence:** ht-poh.3.3 "owns: `ack_policy_delivered` …, `delivery::settle`, `wake_suppressed`". `delivery::settle` lives in the `src/delivery/` module that ht-poh.3.2 creates ("owns: the src/delivery API"). ht-poh.3.3's deps are 1, 2.7, 3.1 and 7, with no 3.2. ht-poh.3.4 then consumes "L2 src/delivery API … the L3 `ack_policy_delivered` and `delivery::settle`".
- **proposed fix:** Add the edge ht-poh.3.3 ← ht-poh.3.2.

- **type:** GAP
- **subject:** ht-poh.3.6 and ht-poh.3.7 against ht-poh.3.3: the wake decision and the push path are edited unordered
- **evidence:**
  - ht-poh.3.3 owns `wake_suppressed` (the "DP7 idle-wake part"). ht-poh.3.6, "Core wake decision honours `wake_on: all`", depends only on 3.5 and 7.
  - ht-poh.3.7, "Push-channel promotion of `wake_on: all` lazy rows", changes Claude's PushChannel. It depends on 3.2 and 3.5 but not on the 3.3 re-fit, which turns that same path into a policy lookup.
  - Both tasks edit the code that 3.3 re-fits, so they can race with it or skip the policy lookup.
- **proposed fix:** Add the edges 3.6 ← 3.3 and 3.7 ← 3.3. Name `wake_suppressed` as a consumes line on 3.6.

- **type:** GAP
- **subject:** R13: the outside-adapter report is never produced at the final SHA (previous-round fix c1 is partial)
- **evidence:** ht-poh.9 builds `scripts/outside-adapter-report` and has no deps. No task depends on ht-poh.9. ht-poh.6.9 ("Final-SHA evidence: full native gate + live smoke, commit records", deps ht-poh.10 …) does not run it. R13 says "The final report states how much code outside `src/harness/agy/` the agy adapter needed". The ledger says "run by orchestrator", but nothing in the tree records the result.
- **proposed fix:** Add a final-SHA leaf, or extend ht-poh.6.9, that runs `scripts/outside-adapter-report` after ht-poh.10 and commits its output. It should depend on ht-poh.9 and ht-poh.10.
- ledger: c1

- **type:** GAP
- **subject:** R8: the rule stays enforced after the sweep (`to-migrate` drops to zero and stays there)
- **evidence:** ht-poh.2's goal says "`scripts/check-harness-neutral` reports zero `to-migrate` entries". ht-poh.1 owns the "to-migrate entries" format. No leaf makes the check fail once `to-migrate` entries are gone or new ones are added. Without that, ht-poh.4 or ht-poh.5 could add a `to-migrate` entry and still pass. That breaks the root goal's "a check enforces the rule" and ht-poh.4's goal that "all agy code lives in `src/harness/agy/`".
- **proposed fix:** Add a closing leaf under ht-poh.2, depending on 2.2–2.6, that removes the `to-migrate` category (or makes it a hard failure) and asserts the count is zero in CI.

- **type:** UNOWNED-SEAM
- **subject:** agy delivery downgrade state, passed from ht-poh.5 to ht-poh.4.7 doctor
- **evidence:** ht-poh.4's goal says "`doctor` reports every agy degradation". ht-poh.6.6 "consumes … ht-poh.5 (pull/confirm/inject, Stop downgrade keyed on executable identity, doctor downg…". So ht-poh.5 produces a downgrade record that doctor must show. But ht-poh.4.7 (doctor) consumes only "L3 status … L2b witness helpers", and ht-poh.5 does not depend on 4.7 or the other way round. No owns line names the downgrade-record format.
- **proposed fix:** ht-poh.5 should own the downgrade-state record (location and format). Either add a ht-poh.4.7 consumes line and an edge 4.7 ← 5 (if that would create a cycle, add a follow-up doctor leaf ← 5), or have ht-poh.5 extend `doctor.rs` itself and say so.

- **type:** UNOWNED-SEAM
- **subject:** `delivery::page`, from ht-poh.3.2 to ht-poh.3.7
- **evidence:** ht-poh.3.7 "consumes: L5's `wake_on` accessor; L2's `delivery::page`". ht-poh.3.2's owns line lists "`Assembler`, `DeliveryItem`, `PageSource`, `drain_pages`, `DrainEvent`, `frame`, `FrameItem`, `HEADER`, page constants". There is no `page` function or item.
- **proposed fix:** Add `delivery::page` to ht-poh.3.2's owns line, or point 3.7 at the item 3.2 actually owns.

- **type:** UNOWNED-SEAM
- **subject:** the `Outside-adapter:` commit trailer, written by ht-poh.1 and ht-poh.7 and parsed by ht-poh.9
- **evidence:** Two tasks own the trailer:
  - ht-poh.1 "owns: … Accounting (R13): every commit carries an 'Outside-adapter: <none | list>' trailer".
  - ht-poh.7 "owns: … Accounting (R13): every commit carries an 'Outside-adapter: <none | list>' trailer naming each change outside src/h…".

  ht-poh.9 "owns: outside-adapter report format". It has no consumes line naming the trailer, and no edge to either owner. Nothing checks that commits carry the trailer. Commits from tasks with no dep on ht-poh.1 (2.7, 6.1, 6.3, 8, 9, 7) may land before the convention is in place.
- **proposed fix:** Give the trailer one owner (ht-poh.1 or ht-poh.7). Add a ht-poh.9 consumes line and an edge. State whether ht-poh.9 measures from trailers or from the diff, so a missing trailer cannot hide cost.

- **type:** GAP
- **subject:** ht-poh.8 is not ordered against the neutral check, the sweep or the agy launch policy
- **evidence:** ht-poh.8 has no deps. It edits `src/harness/launch.rs`, `src/cli/handoff.rs` and `src/store/…`. ht-poh.2.2 sweeps the same "launch/handoff surface", ht-poh.1 owns the check that this core change must pass, and 2.7 owns the goldens. ht-poh.4.6 (agy `LaunchPolicy`) does not depend on ht-poh.8. Only ht-poh.4.8 ("handoff check") does, so `handoff --kind agy` (R7) can't be built into the policy until docs time.
- **proposed fix:** Add the edges ht-poh.8 ← ht-poh.1 and ht-poh.2.7. Order 8 against 2.2 in one direction. Add the edge ht-poh.4.6 ← ht-poh.8, or name which task makes `--kind agy` handoff produce a V2 plan.
- ledger: c8

- **type:** GAP
- **subject:** R11: the SKILL section reaches agy's installed skill
- **evidence:** ht-poh.3.6 adds the `wake_on` "SKILL section". ht-poh.4.4 renders agy's SKILL.md from "`SKILL_MD` + `AGY_SKILL_GUIDANCE`" with an owned-asset manifest. Neither task depends on the other. If 3.6 lands after 4.4, the installed agy skill and its manifest hashes may change unseen, and `setup-status` could report drift. The 6.4 setup flow does not depend on 3.6.
- **proposed fix:** Add the edge 4.4 ← 3.6, or state that setup-status tolerates SKILL_MD content changes, and cover it in 6.4.

- **type:** GAP
- **subject:** R12 tier count is unverifiable
- **evidence:** R12 reads "Testing, in three tiers:" and the list is cut off. The ht-poh.6 goal names only two runners (native gate and live smoke), plus ht-poh.6.1's "CI line for offline agy-script tests". It can't be confirmed that the third tier (most likely CI-run Rust or offline tests for the agy adapter and HookPull) has an owner.
- **proposed fix:** Spell out the third tier in R12 and name its owning task(s). If it is CI-run agy adapter tests, add an acceptance line to 4.x and 5.

- **type:** GAP
- **subject:** End-to-end thin path for R4 inside CI (no gate binary)
- **evidence:** The only end-to-end proof of pull, inject, transcript confirm and settle is ht-poh.6.6, which needs `HT_AGY_BIN` and "Neither runner ever runs in CI". No leaf states a CI-run test that drives `DeliveryPull`, a fake agy transcript, `DeliveryConfirm` and then settled receipts through the daemon. Without one, a regression in 3.4 or 5 is caught only by a manual gate.
- **proposed fix:** Give ht-poh.5 (or a new leaf under ht-poh.3) an acceptance criterion for a CI integration test that uses the fixtures in `tests/fixtures/agy/`.

No ORPHAN findings: every task traces to a goal outcome or requirement. ht-poh.3.7 → R11 (Claude push seats), ht-poh.8 → R7, ht-poh.10 → R14 gate, ht-poh.9 → R13.

Previous-round fixes:
- ht-poh.9: partial, as above (never run, no edge to 10).
- ht-poh.2.7: partial (3.2, 2.1, 8, 3.7 and 7 are not gated).
- ht-poh.10 / ht-poh.6.9: closes R14. It does not cover R13's run.
- 4.x edges: closed, except 4.6 ← 8.
- ht-poh.6.4 ← 8 / 4.8: closed.
- ht-poh.6.8 ← 3.4 / 3.6: closed.
- ht-poh.1 / ht-poh.7 split: it created a trailer with two owners.
- ht-poh.4.3 titles: closed.
- ht-poh.5 consumes lines: no matching dependency edges.