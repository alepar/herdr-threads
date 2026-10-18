R1 A relay verifies an unknown-layout entry using only the 68-byte prefix invariant, SHA-256 of raw bytes and the author's signature
R2 A verified opaque entry is stored verbatim, joins the hash chain, counts toward have heads
R3 Opaque entries are served only to peers that advertised envelope.opaque.v1; otherwise withheld with have staying behind and no protocol error
R4 Opaque entries are never applied; later entries of the log park and apply after upgrade
R5 Forks between an opaque and a known entry are detected from prefix fields
R6 Re-issue/resync of an opaque entry reports a diagnostic and never silently drops it
R7 Golden 2.1.1 peer-session fixtures (frames, one signed entry per fact kind, fork frames) are committed and decoded by current code in CI, append-only
R8 A frozen 2.1.1 shadow decoder checks that frames current code emits to non-advertising peers decode, and fixtures mutated with unknown fields/frames/reasons decode
R9 scripts/check-wire-compat enforces no deny_unknown_fields, serde(default) on new fields, capability-gated new frame variants, wired into AGENTS.md and CI
R10 Docs and release checklist cover the 2.1.1 relay limit, freeze script and lint
R11 Known-layout traffic with 2.1.1 peers stays byte-identical
R12 HTJRNL01, signed fact shapes and JSON framing are unchanged
