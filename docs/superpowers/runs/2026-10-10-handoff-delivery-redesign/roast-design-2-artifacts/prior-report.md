---
super-roast verdict: Blocking (2 confirmed)
mode: design        iteration: 1 of 3
profile (assumed): Internal, cooperative same-user coordination CLI with durable message data and real native configuration ownership. No adversarial caller-authentication requirement is implied. Receipt and invitation confirmation deadlines are core behavior; data/configuration safety and explicitly stated core-purpose floors remain binding.
inputs: settled 6c74fa0a: all five handoff-delivery-redesign specs, exact full task-tree.json, coverage-ledger.md, author-refinements.md, coverage-changes.md and coverage-round-2 summaries; baseline 7c40c83c028d4a1a7650d6a58e92cd0bdedd3cdb
coverage: premortem, completeness, yagni, failure-mode, feasibility, domain:durable-messaging, domain:protocol-evolution-and-data-migrations, domain:native-hook-configuration-ownership · 11 raw → 5 deduped → 5 panel / 0 spot-checked · judge completion 100% (15/15) · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel
seat-agreement: panels 5 · rr 0.80 · rg 0.60 · fg 0.80 · unanimous 0.60 · ground-loo 0.75 (n=4) · reproduce 1/4/0 · refute 2/3/0 · ground 3/2/0

## Confirmed findings

- [Blocking] R1-1 — Root design (settled-input.md): Goal; Message deadline contract; Risks and mitigations; task-tree ht-0lo.1.2 and ht-0lo.1.3 — The bounded-time warning promise relies on fair, sufficiently frequent progress through overdue work, but the design specifies neither a detection-lag bound nor a backlog-progress test that establishes that assumption.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm the design assumption. Ground cites settled-input.md:41's “bounded-time warning”, :62's retained “bounded deadline detection”, and :60/:112's normal bounded scan and bounded scan work. The complete artifact supplies no scan cadence, fair multi-batch continuation rule or per-obligation detection-lag bound. ht-0lo.1.2's indexed discovery and ht-0lo.1.3's bounded warning burst do not establish progress through several batches while native delivery remains blocked. Reproduce identifies the concrete dependency: an obligation beyond the first upgrade batch needs later scanner progress, which bounded work per invocation alone does not establish.
  severity: The confirming seats suggest Should-fix because an actual scanner defect is not demonstrated. This is a confirmed design-level dependency rather than a demonstrated implementation defect; nevertheless, the missing progress/latency contract leaves the artifact's stated core bounded-time warning purpose unsupported. Apply the explicit core-purpose Blocking floor before profile demotion. No unlimited-load or bounded native-delivery guarantee is required.
  fix-shape hint: State a finite supported workload, fair scanner continuation and detection-lag bound; require multi-batch receipt/invitation progress with blocked native wakes, restart and continuing due arrivals, or narrow the bounded-time promise.

- [Blocking] R1-4 — Root design: Message deadline contract; Invitation contract capability negotiation; Risks and mitigations; receipt task ht-0lo.1.1; invitation task ht-0lo.2.1 — The design does not specify capability negotiation for the changed ordinary-send receipt semantics, so a modern client cannot establish that an older daemon will start an unavailable recipient’s ACK deadline at send time.
  verdict: confirmed (reproduce REJECT / refute ✗-survived / ground ✓)
  evidence: Refute and ground confirm that “every new durable ACK-required ordinary send” must use canonical committed send time, including unavailable/unbound recipients. The only explicit negotiation contract is for mutations requiring the new invitation representation or new incompatible fields. ht-0lo.2.1 owns that refusal; ht-0lo.1.1 assigns receipt projection without an ordinary-send semantic capability contract. The local schema/semantics marker does not establish the connected daemon's semantics.
  dissent resolution: Reproduce rejects because mixed-version ordinary sends are not expressly guaranteed to be supported. Refute and ground directly address that scope premise: the ordinary-send guarantee is universal as written, the artifact explicitly considers old peers, and it supplies neither a global minimum-daemon-version requirement nor an accepted receipt exception. The objection does not identify an existing mechanism excluding the proposed wire-compatible send. Retain the arithmetic confirmation as a missing compatibility/scope contract; this does not assert that an actual implementation has been shown to admit the combination. An explicit upgraded-daemon limitation can resolve it without adding a new capability.
  severity: Confirming seats suggest Should-fix, but the omitted compatibility boundary leaves the core send-anchored deadline guarantee unenforceable in the described old-peer case. Apply the stated core-purpose Blocking floor; the internal cooperative profile does not remove that requirement.
  fix-shape hint: Specify receipt-semantics negotiation/refusal and mixed-version coverage, or explicitly limit the receipt guarantee to upgraded daemons and define how that boundary is enforced.

## Not verified (beyond panel cap)

- none

## Beyond remainder cap (count only)

- none

## Rejected (with reason)

- [FYI] R1-2 — Root design: Message deadline contract and Invitation contract; acceptance and validation — Late confirmation before the first overdue scan need not create a historical missed-deadline warning under the stated pending-confirmation contract. Reproduce and refute cite the goal's “unconfirmed” condition, the receipt subtree's “Every pending ACK-required message”, preserved terminal states and invitations that “settle at acceptance”. Ground confirms that scanner ordering can change whether a warning existed, but cites no requirement for a historical deadline-audit record after genuine confirmation. Dissent resolved in favor of rejection: warning wording constrains existing warning meaning; it does not establish that stronger audit guarantee. The pending obligation has settled in the proposed ordering, so the finding does not demonstrate a material core-purpose violation.
- [FYI] R1-3 — Stable hook entrypoints and ownership; Risks and mitigations — All three seats reject. The selected ownership contract is manifest-owned exact groups/fingerprints plus installation ID (ground cites settled-input.md:51/:86). The example retains that management record and every exact recorded observation; the foreign-group rule concerns “Existing unmanaged identical groups”. No seat establishes a requirement that an unobserved identical deletion/recreation revoke retained manifest ownership. Requiring that historical distinction adds a continuity guarantee absent from the chosen contract.
- [FYI] R1-5 — Options and decisions item 3; Stable hook entrypoints and ownership; hooks subtree — All three seats reject. The alternative existing syntax could also be deterministic, but dedicated hook-event entrypoints and legacy compatibility are explicit design choices (ground cites settled-input.md:51/:84/:86). Parser/dispatch, registry/help/canary and migration consequences are assigned to ht-0lo.4.1/.4.2. The missing comparison demonstrates neither a violated stability/ownership requirement nor significant implementation risk or rework; it remains a rationale suggestion.

## Unverified nits (spot-checked)

- none

## Escalations (need human)

- none — all panels are complete, no external premise is unverified, and both split-panel disagreements are explicitly resolved above.
---
