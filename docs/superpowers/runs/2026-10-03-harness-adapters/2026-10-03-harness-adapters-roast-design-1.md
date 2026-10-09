---
super-roast verdict: Blocking (2 confirmed)
mode: design        iteration: 1 of 3
profile (assumed): Production local cooperative developer tooling with durable real-user SQLite state and owned native config. Data-loss/migration or authority/provenance drift is material; native evidence labels and the one-interface author criterion are core purposes. Same-user cooperative trust does not require adversarial process attestation, and unsupported native modes may remain explicit.
inputs: harness-adapters settled root and nested specs, ht-3bi
coverage: scouts 8/8 (premortem, completeness, yagni, failure-mode, feasibility, domain:plugin-systems, domain:auth, domain:distributed-systems) · raw 16 → deduped 9 → panel 9 · spot 0 · promoted 0 · judge completion 100% · remainder-capped: 0
independence: same-family (OpenAI) — seat-differentiated panel · rung: manual fan-out
seat-agreement: panels 9 · rr 0.89 · rg 0.44 · fg 0.56 · unanimous 0.44 · ground-loo 0.50 (n=8) · reproduce 1/8/0 · refute 2/7/0 · ground 6/3/0

## Confirmed findings
- [Blocking] Architecture and boundaries; One author interface; Identity and serialization; Architecture and boundaries (line 17), One author interface (line 55), Identity and serialization (line 71); 2026-10-03-harness-adapters-design.md:17,58,72 (Architecture and boundaries; One author interface; Identity and serialization); Architecture and boundaries (line 17), One author interface (line 58), Identity and serialization (lines 70–72); Architecture and boundaries line 17; One author interface line 58; Identity and serialization line 72; Architecture and boundaries (line 17), One author interface (line 58), Identity and serialization (line 72) — Registry validation does not reject colliding adapter context serialization spellings or reserve Human, leaving persisted context and journal identity ambiguous for otherwise valid registrations.
  verdict: confirmed (reproduce ✓ / refute ✗-survived / ground ✓)
  evidence: All three seats confirm that 2026-10-03-harness-adapters-design.md:17,58,72,157 checks duplicate IDs/kinds but independently permits an adapter-declared context spelling.
  evidence: Reproduce shows a distinct reviewbot registration declaring Claude passes those checks while sharing the preserved Claude journal token; refute shows the equivalent Human collision. Neither registry order nor late read refusal preserves both identities.
  evidence: The shipped spellings are distinct, but the specified one-module/one-registration author interface accepts this counterexample. Its persistent identity round trip fails, violating the stated core author criterion; the core-purpose severity floor applies. This does not establish a canonical-authority bypass.
  fix-shape hint: Validate uniqueness of every accepted context spelling and reserve Human during registration; exercise collisions in the injected author proof.
- [Should-fix] Setup and ownership (lines 100–106) — Hermes asset ownership does not define serialization of concurrent setup and unsetup transactions targeting the same resolved profile home.
  verdict: confirmed (reproduce ✗ (REJECT) / refute ✗-survived / ground ✓)
  evidence: Refute and ground cite Setup and ownership, 2026-10-03-harness-adapters-design.md:100–106: multi-asset installation and exact-owned removal use atomic file helpers but have no shared resolved-profile operation ordering or overlap refusal.
  evidence: Refute describes unsetup checking an old asset, setup publishing its replacement and manifest, then unsetup deleting the replacement using its stale decision. Ground independently identifies mixed asset/manifest generations.
  evidence: Reproduce cites the general ownership/refusal/recovery outcomes and possible existing helpers, but identifies no specified Hermes ordering or conditional-publication rule that closes the majority counterexample. The default confirmation stands. Foreign-file loss or an irreversible real-data migration is not established by these seats.
  fix-shape hint: Specify shared exclusion at the resolved profile home or bounded refusal of concurrent setup/unsetup, with an ownership-generation interleaving fixture.

## Not verified (beyond panel cap)
- none

## Not verified (dedupe failed or judge lost)
- none

## Beyond remainder cap (count only)
- none

## Rejected (with reason)
- [FYI] Identity and serialization, lines 70–72; Storage migration, lines 142–144 — The design lacks a persisted-identity representation that can retain an unknown harness while its only agent identity type requires registry membership.
  reason: All seats reject the mandatory-decoder premise. Identity and serialization:70–72 and Storage migration:142–144 distinguish stored strings and lexical validity from registered operational IDs; acceptance:157 already requires an unsupported diagnostic with no authority, history deletion, or binding termination. No requirement forces every persisted row into AgentHarnessId.
- [FYI] 2026-10-03-harness-adapters-design.md:114-118,160 (Hermes Python bridge; Focused acceptance and validation item 4); Hermes Python bridge, lines 114–118; Focused acceptance and validation, item 7 — Successful Hermes context delivery assumes the complete synchronous callback pipeline fits the native deadline without a measured healthy-path end-to-end latency acceptance margin.
  reason: All seats cite the actual qualified callback with delivered turn context required by acceptance:162. Bridge:118–120 explicitly permits bounded timeout/no-context outcomes and reports undersized deadlines; acceptance:165 forbids promoting failed or inconclusive native results. A separate latency margin or cold/contended-host SLA is not promised.
- [FYI] Architecture and boundaries; One author interface (observe_install/admit/status); Health, doctor and compatibility — A stalled or panicking adapter observation has no specified per-adapter failure boundary to prevent it from stopping registry-wide daemon observation.
  reason: All seats cite the same-binary static registry and CallBudget-bearing author interface. Independent status dimensions do not promise continued observation after a compiled adapter violates its budget or panics. The proposed failure needs an additional fault-containment requirement absent from this contract.
- [FYI] Hermes Python bridge (lines 114, 118, 120) — The bridge has no specified limit on simultaneously running owned Rust subprocesses, so bounded cache sizes and per-callback deadlines do not establish bounded resource use for overlapping callbacks.
  reason: Reproduce and refute distinguish bounded session/turn state and individual I/O/deadlines at Bridge:114,118 from a workload-independent aggregate child ceiling. Bridge:120 explicitly accommodates abandonment and late completion. Ground reiterates the missing aggregate cap but provides no unmet stated aggregate-resource guarantee; the majority addresses that premise.
- [FYI] One author interface (line 32), Architecture and boundaries (line 17), Event metadata, contracts and evidence (lines 78–82) — The design does not define registry rejection of conflicting contract descriptors with the same harness/domain/origin/contract selector, leaving generic evidence classification and verification policy ambiguous.
  reason: Reproduce and refute cite adapter-owned classify returning one ContractObservation and the registration-bound admission handle. They expressly address descriptor hashes and operational fields at One author interface:32 and Event metadata:78–82: unchanged hashes do not establish the hypothesized generic selector algorithm or conflicting supported descriptors. Ground’s descriptor-consistency concern does not resolve that missing execution-path premise.
- [FYI] Hermes Python bridge (line 122), Focused acceptance and validation item 8 (line 163); Hermes Python bridge (line 122); Focused acceptance and validation item 8 — The runtime-identity resolver timeout contract does not specify ownership and bounded cleanup of git descendants spawned by the resolver helper.
  reason: Reproduce and refute explicitly acknowledge that Popen.kill only kills the direct child, as documented at https://docs.python.org/3/library/subprocess.html#subprocess.Popen.kill. They cite Bridge:118,122 and acceptance:160,163 requiring bounded owned work, no leaked children, and stopping every owned helper. Ground’s same direct-child distinction is therefore addressed; a leaking resolver violates an existing outcome requirement rather than demonstrating an absent cleanup contract.

## Unverified nits (spot-checked)
- none

## Escalations (need human)
- Event metadata, contracts and evidence, line 86; Hermes Python bridge, line 122 — Caching Hermes runtime identity once per bridge load assumes the resolver snapshot remains a truthful identity for all callbacks despite possible source-install changes, without defining invalidation or an accepted immutability limit.
  reason: Material dissent — Event metadata, contracts and evidence, line 86; Hermes Python bridge, line 122: ground cites the upstream resolver’s live git identity and refresh for checkout swaps, plus subsequent Python imports; reproduce/refute rely on captured once-per-load semantics but do not settle whether successful capture remains an honest exact-build key after those source changes.
  evidence: Ground cites https://raw.githubusercontent.com/NousResearch/hermes-agent/main/hermes_cli/version_info.py:187–210,247–273,284–286, including refresh to re-read identity after swapping a checkout beneath a running process. Ground also cites https://docs.python.org/3/reference/import.html#the-module-cache for subsequent uncached module loading.
  evidence: Reproduce cites module caching and requires concrete later reload/lazy-import evidence; refute cites Bridge:122’s once-per-load observation lifetime and the unavailable-identity feasibility gate. Those answers do not resolve the ground seat’s concrete supported-mutation circumstance against exact-build attribution at Event metadata:86–88.
  evidence: Rejected default overruled to Escalations for material dissent; no confirmation, native PASS, or runtime implementation defect is asserted.
---
