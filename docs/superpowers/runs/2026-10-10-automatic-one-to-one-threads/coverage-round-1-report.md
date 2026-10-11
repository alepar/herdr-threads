# Coverage round 1 — automatic one-to-one threads

Scope: ht-e99; immutable bulk dump `coverage-round-1-tree.json`; full root/pair/handoff packets beside this report. No tracker, run-state, integration-sweep or code mutation performed by coverage reviewer orchestrator.

## Review assembly and capacity

Nine independent reviewers completed: root A/B/C, pair A/B/C, handoff A/B/C. Capacity was seven descendants: batch 1 launched root A/B/C, pair A/B/C, handoff A; batch 2 launched handoff B/C as slots freed. Reviewers never read one another’s findings. This was capacity batching, not a claim of nine-way concurrency. Full packets were assembled before fanout. Model adaptation: available inherited harness model used; no opus override is exposed.

Inherited read-only context did not deliver tool outputs: root A/B initial attempts returned INSUFFICIENT-INPUT. Caller authorized exactly one immutable packet acquisition followed by no exploration/tools. A wrapper cap and a source-packet suffix assembly bug then omitted the reviewer checks. Acquisition was repaired by untruncated packet reads and one missing-suffix-only acquisition; specs/tree/canonical requirements never changed during reviews. Final all nine explicitly received complete checks and task inputs. These were assembly repairs inside round 1, not new coverage rounds. No reviewer explored repository or tracker. Final coverage is based only on complete replacements, not tentative reports. Packets now contain the full literal specs/tree/requirements/template; the suffix artifact preserves the bounded assembly repair. Harness adaptation is disclosed rather than describing literal full packets as repeated spawn messages.

## Requirements

Canonical IDs R1–R22 are stored in `coverage-requirements.md` and root packet. No R-new proposed; IDs remain stable.

requirements: 22 · mapped: 21 · unmapped: 1 (R22)

This is the union-unmapped rule: root C returned R22 unmapped although A/B mapped it to the normative root verification obligations and relevant leaves. The disposition below clarifies acceptance instead of concealing that disagreement. Every other requirement mapped in all root reviews.

## Final reviewer results

| Pass | Reviewer | Final findings |
|---|---|---|
| root | A | flag-sweep ht-e99.2; all22 mapped |
| root | B | none; all22 mapped; acknowledges flag override |
| root | C | capability activation UNOWNED-SEAM; R22 GAP/unmapped; acknowledges override |
| pair | A | retention GAP; unbound recipient UNEXERCISED-CONFIGURATION |
| pair | B | unbound recipient UNEXERCISED-CONFIGURATION |
| pair | C | none |
| handoff | A | unsupported-daemon UNEXERCISED-CONFIGURATION |
| handoff | B | none |
| handoff | C | capability activation/negotiation GAP |

All final reviewers audited applicable edges and needs citations. Five genuine edges are .1.2←.1.1, .2←.1.2, .3.1←.1.2, .3.2←.3.1, .3.3←.3.1, each with resolvable blocked-by artifact. No epic blocking edge or cycle. All four handoff destinations have early producer full invocations. Nested canonical lists empty; unrelated .2 flag excluded from nested sweeps. No orphan or unsatisfiable acceptance confirmed.

Tentative assembly-era candidates (parser independence, archival protection repetition, effective/voluntary fixture, resolver-attachment extension, per-destination lost-invite crossing) were explicitly withdrawn once complete checks allowed mapping to normative spec sections. They are not silently counted as final findings.

## Union, verification and dispositions

Six semantic finding identities after dedupe. Type variants GAP/UNOWNED-SEAM at the same capability boundary are one identity. The following are verified against only the exact packets, not source-code investigation. Caller applies proposed amendments; this report does not claim tracker writes occurred.

- COV1-01 — flag-sweep: ht-e99.2 sp:demoted-by-session. ACKNOWLEDGED OVERRIDE. Description supplies exact rationale: parser/journal/coordinator/recovery are inseparable compound CLI behavior; decisions fixed, no unresolved design. Record ledger acknowledgment, retain cohesive leaf; flag alone creates no additional scope.
- COV1-02 — GAP: retained-pair cleanup ownership/acceptance (.1.1/.1.2). REJECTED. Evidence that descriptions do not repeat cleanup is true, but check1 permits mapping to an applicable spec section. Pair `Acceptance and retention` explicitly requires unique identity survive valid retained history and real-store transactions; mechanism owns pair lifecycle and both leaves cite applicable specs. Unmapped requirement is not established. No new retention leaf.
- COV1-03 — UNEXERCISED-CONFIGURATION: resolved-unbound recipient (.1.2). VERIFIED; EXTEND EXISTING LEAF. Root explicitly enumerates human/unbound test configurations; current leaf explicitly exercises human waiver but not successful unbound invocation. Extend .1.2 with real-store Ensure then ordinary SendMessage/direct_recipient for resolved-unbound and Human recipient, asserting ordinary invitation reservation, frozen target audience, no binding requirement or synthetic acceptance, exact same pair reuse. No new blocking edge.
- COV1-04 — GAP: R22 final verification ownership. VERIFIED CLARIFICATION BY CALLER. Add common exact fmt/locked clippy/focused-test acceptance to every leaf; caller’s terminal integration sweep after coverage owns default-feature merge gate, cleanup, conditional scoped leak checks and budgets. No routine full-suite addition. Preserve round1 union-unmapped R22 metric; round2 rechecks actual amended tree.
- COV1-05 — UNOWNED-SEAM/GAP: automatic-handoff capability activation/negotiation (.3.1/.3.2/.3.3). VERIFIED; ADOPT EXISTING CONTRACT/INTEGRATION OWNERS. Existing .3.1 is the compilable shared boundary and canonical attachment contract; .3.2/.3.3 already own full producer integrations. Avoid duplicating seam machinery. Explicit daemon capability ownership resolves the previously ambiguous `complete producer` condition without a cyclic sibling activation dependency: .1.2 advertises resolver+marker support once guarded mechanism usable; .3.1 owns complete canonical automatic-handoff deciding adapters/descriptor and advertises once those effects usable; local CLI producer presence never gates daemon advertisement. Each local producer remains unsupported until implemented and negotiates server capability before its own publication/effects. No new edges/beads.
- COV1-06 — UNEXERCISED-CONFIGURATION: old daemon automatic path refusal (.3.2/.3.3). VERIFIED; EXTEND EXISTING LEAVES, shared with COV1-05. Each independent handoff producer executes an unsupported-capability invocation and asserts no automatic journal publication or effects. Caller also extends .2 direct-send unsupported-capability prepublication refusal; preserve new-tab existing topology sequencing and canonical attachment restrictions. Do not interpret refusal as permission to roll back committed topology or start.

## Proposed exact acceptance additions

.1.2: `Run a real-store EnsureDirectThread followed by ordinary SendMessage with direct_recipient for a resolved unbound target and a Human recipient. Assert invitation reservation, target frozen audience, no binding requirement or synthesized acceptance, human ordinary waiver and exact existing pair reuse. Advertise resolver/direct-marker capability only after canonical pair mechanism and preparation/publication guards are usable.`

.3.1: `owns: automatic-handoff daemon capability descriptor, complete canonical deciding adapters and default-inert activation contract. Acceptance: advertise automatic-handoff support only when canonical Ensure attachment and restricted child deciding effects are usable; CLI producer implementation presence never gates the daemon descriptor; existing V1 bytes/hashes/reports remain unchanged.`

.2/.3.2/.3.3: `owns: local automatic producer capability negotiation before compound journal publication and first effects. Acceptance: execute supported and unsupported-capability invocations; unsupported daemon refuses with no automatic recovery journal/effects or fallback. Preserve original actor/namespace and existing new-tab topology sequencing/recovery restrictions.`

Every leaf: `Acceptance also requires cargo fmt; nice cargo clippy --locked --all-targets --all-features -- -D warnings; relevant nice cargo test --locked --all-features <filter>; stop owned helper processes. Follow repository speed budgets and scoped leak-check requirements.`

Nested handoff spec: replace ambiguous advertisement-until-complete-producer sentence with explicit daemon canonical deciding-path activation; independently implemented CLI producers negotiate that capability before their own effects. Existing full .3.1 contract is adopted, no duplicate contract/integration pair.

## Next round

Caller owns wholesale bead/spec amendments, run.md records and commits. Because tree descriptions change, round2 runs root always plus pair and handoff changed subtrees, with same R1–R22 and changes-since entries COV1-03/.1.2, COV1-04/all leaves, COV1-05/.3.1/.3.2/.3.3, COV1-06/.2/.3.2/.3.3. No third round. Caller creates/adopts terminal integration sweep only after coverage loop ends. No code or ht commands were run here.
