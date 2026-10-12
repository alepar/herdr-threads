# Q3 wire production slice — specification review

Verdict: **REVISE for test/accounting completion; production wire logic passes this static scoped review.** Update the stale capability expectation and adapt the existing lease fixtures before claiming the wire slice verified. Advertisement is explicitly **pending the complete registry/admission/transport composition**; this review does not authorize treating the new capability as implemented by the currently incomplete candidate.

Reviewed current quiescent files in nested `.worktrees/upstream-composition-v090`: `src/protocol/watch.rs`, `src/protocol/capabilities.rs`, `src/cli/watch.rs`, `tests/protocol/capabilities.rs`. Compared with the full revised Q3 design, the production interface plan, existing CLI controls, and the retained corrected baseline log. No production/runtime edit was made. This is a scoped implementation specification review, not a design roast; counters remain unchanged.

## Source findings

- **Absent/false historical bytes: PASS by inspection.** `WatchRequest` retains `claim` first, appends `presentation_v2: bool` with `serde(default, skip_serializing_if = "is_false")`, and keeps `deny_unknown_fields`. Missing defaults to false, explicit false is omitted on serialization, and true adds exactly the specified member. Boolean typing rejects null/numeric/string substitutes. The surrounding envelope/custom validation and optional Accepted lease are unchanged. This is a source conclusion; byte-exact current-build tests have not been run for this review.
- **Conjunction: PASS.** `probe_mod_watch` only returns Supported inside the existing MOD_WATCH branch and sets presentation to `DELIVERY_PRESENTATION_V2 && MOD_WATCH_PRESENTATION_V2 && BINDING_CURSORS`. The independent cursors boolean still determines the historical reader claim. `run_session_marked` passes that conjunction into `open_watch` and retains it for the V2-loop decision. Neither the older generic capability nor an unsolicited valid lease elects V2 alone.
- **Discovery/error boundary: PASS.** Discovery remains before socket connection. Failed discovery returns DaemonUnavailable, missing MOD_WATCH/definitive unsupported returns Unsupported, and these returns precede `open_watch`. No capability error is converted into absent presentation prerequisites followed by registration. Existing retry/refusal classification is preserved.
- **Lease/fallback: PASS in production source.** V2 additionally requires `valid_presentation_token` on Accepted's lease. Missing or syntactically malformed lease selects the historical loop and its conditional binding reader; negotiation false also selects that loop regardless of a returned lease. No version gate, model-visible text, wake or authority behavior is added here.
- **Capability spelling/accounting: correct declaration, incomplete feature composition.** The new constant is exactly `mod.watch_presentation_v2`, separately named from generic presentation. ADVERTISED now includes it, so the real capability response currently exposes it: candidate `src/daemon/control.rs:317` filters only the harness evidence/health capabilities, not this new one. The declaration's “complete registration, registry and lease handlers” prerequisite therefore remains a real integration gate until the other author units are installed and reviewed.

## Exact gaps

### W1 — Update the second capability-list expectation

`tests/protocol/capabilities.rs:659`, `bare_daemon_advertises_legacy_capabilities_without_v2_recorder`, still expects the old complete list without `mod.watch_presentation_v2`. The earlier `capability_constants_are_stable` expectation was updated, but this second assertion was not. Since the daemon copies the new ADVERTISED entry without filtering it, this is a deterministic expected-list mismatch once compiled, not a possible runtime capability ambiguity. Add the new entry in the matching position as part of the completed feature, or explicitly remove/defer advertisement until its handler composition exists. Do not record this mismatch as a behavioral Q3 RED.

### W2 — Existing lease tests need the new prerequisite to keep testing leases

The existing `presentation_watch_requires_both_capability_and_valid_accepted_lease` (`tests/cli/watch.rs:2140`) sets only generic presentation and cursors. `watch_presentation_capability` remains false by default. Consequently its missing/malformed-lease cases now fall back before lease validity can decide anything: the assertions can pass while the lease gate is broken. Enable the new capability for the intended negotiated cases and retain a separate non-negotiated/unsolicited-lease case. The existing successful presentation-loop fixture around line 2253 likewise sets only those two old legs and must elect the new capability to continue exercising the V2 path. These are outside the four-file writer scope, so coordinate the narrow fixture adaptation with their owner rather than editing an active author's file.

The newly authored Q3 conjunction helper does test a valid lease after all four capability legs and unsolicited valid leases when a leg is missing. It does not replace the missing/malformed-lease coverage above because it always returns a valid lease.

### W3 — A missing-seat handler probe is routing evidence only

The new `MOD_WATCH_PRESENTATION_V2 => probe_mod_watch(true)` arm round-trips the flag and then registers an unknown seat expecting Unresolved. That can pass even if the handler ignores the flag, admits all existing channels as V2, or returns the wrong lease. It is useful decode/routing coverage, but cannot certify the advertised feature's mode-bound lease semantics. Before the composed feature is marked complete, retain a real admitted legacy/capable transport control that proves original Accepted bytes omit the lease for absent/false and opted-in Accepted carries a valid matching entry lease, with replacement/fallback behavior. The planned registry and transport work is pending, not falsely cleared by this probe.

## Baseline attribution and next gate

I read the preserved `resume-v090-q3-baseline-9ef83ba7-evidence/ht-C-v090-q3-baseline-compile-corrected/run.log`. On that **earlier** compiled source, explicit false and true failed on unknown `presentation_v2`; all-capabilities request lacked the field; missing binding-cursors and missing new Watch capability incorrectly selected V2. Absent historical bytes, discovery failure/no-socket, missing generic presentation, and missing MOD_WATCH passed. Those are actual prior wire RED/PASS outcomes and align with the changes inspected here. They are not current GREEN evidence.

After W1/W2 and complete registry/transport installation, the parent should run the existing focused Q3 wire controls, the adapted missing/malformed/valid lease cases, both capability-list/handler checks, and the admitted transport lease invariant. The new WatchRequest field also requires caller/fixture construction updates in other owned files; compilation is pending until composition. No whole-Q3 correctness, build success, integration readiness or advertisement readiness is asserted by this slice review.

Reviewed SHA-256 values:

| File | SHA-256 |
|---|---|
| `src/protocol/watch.rs` | `332c5fc1aa5349888b86eccdaeead5e46cfeb3eb9b8f2a7299b4d420b594f8ec` |
| `src/protocol/capabilities.rs` | `02608e1eedcf9ad4bd446e9ccb60ff854e9a74c4d56a0240bd7e86a5de9a1b63` |
| `src/cli/watch.rs` | `992906193f153d8dbfbaf9796a14fed4a51a0836192b7d2582232f7a2cc1bcb2` |
| `tests/protocol/capabilities.rs` | `b92b45babab4766d2cf8ea1147adb44c5609ca38ab681abff8e2bbedfe9be83f` |

No Cargo, runtime, SQL execution, source/index/Git mutation, Herdr command or delegation occurred. Only this review report was written.
