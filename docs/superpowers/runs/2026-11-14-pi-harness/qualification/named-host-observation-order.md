# Named host observation ordering (ht-bbe2)

A delayed named-host enumeration assigned its trusted sequence after reading the
name set. An older empty capture could therefore appear newer than an explicit
current-target observation made while that capture was blocked. The effective
observation reader correctly preferred the apparent newer publication, losing the
current mapping and refusing cooperative registration.

The deterministic barrier regression captures an empty set, pauses enumeration,
allocates a named seat and publishes its current-target observation, then releases
and publishes the old capture. Before the correction, effective observation was
`None` and registration returned `CallerUnverified: mapping held or known
invalidated`. This is an effective-observation/admission assertion, not only a
comparison of sequence numbers.

| Neutral core path | Change | Authority |
| --- | --- | --- |
| `src/host/named.rs` | Reserve observation order before the source/focus read; one enumeration sequence for every target and its snapshot | Raw enumeration remains `CompleteUnverified` with `Unknown` incarnation; existing non-closing stamping and admission fences unchanged |
| `tests/store/named_presence.rs` | Barrier regression through actual stage/publication and cooperative mapping reads | Existing mapping verification remains required |
| `tests/host/named_host.rs` | Same reserved order on all targets of a capture | No closure, binding, process or wake authority added |

The regression uses the store fixture's capture-publication helper, which issues
store admission after capture. The production observation lane issues admission
before capture and can also reject older work through its admission fence. This
proof establishes the host's observation-order defect; it does not by itself
establish the cause of every initial named registration refusal.

Focused named library tests after the correction: 164 passed, one ignored. The
ignored case is not credited as executed. Runtime diagnostic extension copies are
exploratory specimens, recorded separately from unmodified native qualification.
