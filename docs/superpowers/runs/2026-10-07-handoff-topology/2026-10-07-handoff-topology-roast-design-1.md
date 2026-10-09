---
super-roast verdict: Blocking (3 confirmed)
mode: design        iteration: 1 of 3
profile (assumed): Internal same-user production durable local tool. Historical data and seat identity continuity are real, and cooperative trust applies. Data-loss and stated identity or handoff purpose violations are Blocking; adversarial caller verification is not required.
inputs: source 18a4c0c1: settled root + canonical-bootstrap + bootstrap-coordinator + existing-delivery specs + tree-settled.json
coverage: scouts 8/8 (premortem, completeness, yagni, failure-mode, feasibility, domain:distributed-systems, domain:auth, domain:durable-workflows) · raw 14 → deduped 4 → panel 4 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI GPT) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 4 · rr 0.50 · rg 0.50 · fg 1.00 · unanimous 0.50 · ground-loo 1.00 (n=2) · reproduce 1/3/0 · refute 3/1/0 · ground 3/1/0
lane-yield (found/confirmed/unique/refuted): premortem 2/2/0/0 · completeness 2/2/0/0 · yagni 1/0/0/1 · failure-mode 3/3/0/0 · feasibility 2/2/0/0 · domain:distributed-systems 2/2/0/0 · domain:auth 0/0/0/0 · domain:durable-workflows 2/2/0/0

## Confirmed findings
- [Blocking] artifact.md:53,79,81,85; root spec CLI and compatibility and Human recovery; recovery task ht-qhz.6; artifact.md:53, 79–81 (recovery CLI and exact-attempt recovery/replay contract); artifact.md:53,63,81 and task ht-qhz.6; artifact.md:53, 63, 79–85 (CLI and compatibility; Human recovery, authority and cleanup); ht-qhz.6 description; artifact.md lines 53, 63, 81, 85: recovery grammar and exact-attempt recovery contract; task ht-qhz.6 — Recovery invocation is not immutably bound to the inspected attempt. [lanes: premortem, completeness, failure-mode, feasibility, domain:distributed-systems]
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats identify the same missing public binding. Line 53 exposes recover REF with a decision but defines no attempt-qualified REF or expected-attempt argument; lines 79 and 81 require exact-attempt recovery and presentation-only replay.
  evidence: The reproduce and refute seats give the concrete response-loss sequence: not-created for A commits, an agent retry makes B uncertain, and the saved recovery command executes again. Selecting current B derives a different deterministic key under line 63 and can authorize another creation using an assertion based only on inspection of A.
  evidence: Ground confirms that ready argv and attempt IDs at line 85, the nested specs, and ht-qhz.6 do not supply this binding. Same-attempt contradiction checks and operation locks do not distinguish the stale invocation once previous invocations have ended.
  severity: The demonstrated replay violates the stated exact-attempt handoff recovery purpose and can permit another topology creation without a noncreation assertion for B. The core-purpose floor makes this Blocking despite the seats' Should-fix ratings.
  fix-shape hint: Freeze the inspected attempt in the operator-visible recovery reference or argument, and define exact-attempt replay and stale-attempt refusal.

- [Should-fix] artifact.md:67,73,75,81,89; root spec Topology state machine and response loss, Human recovery, and Archival; canonical bootstrap task ht-qhz.2.3; artifact.md:67, 73, 75, 81, 89 (live fence, missing-pane refusal, completion, recovery and archival); artifact.md:67,73,75,81 (Topology state machine and response loss; Human recovery, authority and cleanup); artifact.md:67, 73–75, 81, 89 (Topology state machine and response loss; Human recovery; Archival and compatibility); canonical-bootstrap nested spec; artifact.md lines 67, 73–75, 81, 89: Topology state machine and response loss; Human recovery; Archival and compatibility; artifact.md, root spec “Topology state machine and response loss”, “Human recovery, authority and cleanup”, and “Archival and compatibility”; canonical bootstrap nested spec and ht-qhz.6. — Confirmed creation followed by topology loss has no terminal abandonment path, leaving an existing thread permanently protected from archival. [lanes: premortem, completeness, failure-mode, feasibility, domain:distributed-systems, domain:durable-workflows]
  verdict: confirmed (reproduce REJECT / refute ✗-survived / ground ✓)
  evidence: All three seats reproduce the same state: creation is canonically recorded, its exact pane disappears before downstream completion, retry refuses under line 73, and CompleteBootstrap cannot satisfy line 75.
  evidence: Refute and ground establish that line 81's uncertain-attempt recovery cannot truthfully assert noncreation or adopt a missing confirmed result. Lines 67 and 89 keep the archival veto live. The command inventory, nested specs, and task tree provide no abandonment or protection-release transition.
  dissent disposition: Reproduce rejects because the specification does not promise archival liveness for abandoned bootstraps. Refute and ground expressly address the same nonexpiry, exact-topology refusal, and completion rules and show the lifecycle consequence; no separate factual premise remains unanswered. The majority route stands as a material lifecycle gap, without claiming those safety rules themselves are inconsistent.
  fix-shape hint: Define an explicit operator terminal disposition that retains honest creation evidence and releases workflow protection without relocating or closing topology.

- [Should-fix] artifact.md:73,75 and bootstrap-coordinator section Downstream launch and terminal bootstrap replay (lines 177–183); artifact.md, root spec “Topology state machine and response loss”, paragraphs beginning “The result records” and “After attachment”; nested bootstrap coordinator “Downstream launch and terminal bootstrap replay”; ht-qhz.4.3 and ht-qhz.2.3. — A crash after downstream completion can leave the outer bootstrap fence live when retry guards subsequently become invalid. [lanes: failure-mode, domain:durable-workflows]
  verdict: confirmed (reproduce REJECT / refute ✗-survived / ground ✓)
  evidence: All three seats establish separate terminal commits and the intermediate state. Line 75 puts bootstrap completion after canonical downstream completion; no cited contract makes those commits atomic.
  evidence: Refute and ground show that line 73's current topology/incarnation guards can refuse retry before outer completion. The guard-independent retained-report path in lines 75, 142, and 183 applies only to an already completed bootstrap. No early reconciliation of a live parent with its exact terminal child is specified.
  evidence: Lines 67 and 89 retain the outer archival protection despite downstream completion. The nested terminal-replay and canonical completion tasks do not cover this intermediate crash state.
  dissent disposition: Reproduce rejects because no requirement explicitly promises terminal-child reconciliation after live guards fail. Refute and ground address those same guard and completed-only replay clauses and identify the omitted crash transition within the specified replay and archival lifecycle. The dissent adds no unaddressed evidence that would require escalation; the majority route stands.
  fix-shape hint: Define canonical reconciliation from the exact terminal downstream record before guards for further live effects, retaining frozen namespace/origin validation and testing the crash between terminal commits.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] artifact.md: Follow-on and implementation ownership; tree-settled.json tasks ht-qhz.1, ht-qhz.3 and ht-qhz.2.1 — The shared seam task necessarily stalls independent native transport and schema work until peer actor implementation is available.
  verdict: rejected (reproduce REJECT / refute REJECT / ground REJECT)
  reason: All three seats refute the necessary dependency premise. The root at line 57 and ht-qhz.1 at line 1119 explicitly permit additive topology types and inert dispatch/signatures that compile with execution disabled. Classification-before-retry constrains enabled execution; it does not require live peer integration to finish that inert contract.
  evidence: Root line 101 limits unavailable shared boundaries to their integration leaf, and ht-qhz.6 at line 935 explicitly gates human recovery integration. The migration source gate at line 779 is independently limited to ht-qhz.2.1. The claimed global stall is not entailed by the dependency tree.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- none
---
