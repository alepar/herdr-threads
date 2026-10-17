R1 Complete/empty hook/channel injection requires no explicit inbox or ACK call.
R2 Undisplayed overflow, truncation, failed/unavailable native delivery emits exact bounded inbox continuation.
R3 Fully delivered eligible native hook messages automatically settle existing receipt obligations after successful context write+flush with truthful provenance.
R4 Text inbox, channel and hook-pull preserve complete-display auto ACK, idempotence, read-only/partial/source safety and no-receipt defaults.
R5 Bounded channel batches preserve order, retained tail and recovery at 32 items/64KiB, with no partial/tail ACK.
R6 Canonical source validation and receipt authoring preserve cross-node convergence and truthful source; combined final candidate verification is caller-owned.
R7 Shared guidance and model-visible recipes explain automatic receipt ACK once and reserve inbox/manual ACK for actual fallback or exceptional reads.
R8 Preserve binding-specific streams, invitation consent, wake predicates, output limits and old-daemon compatibility.
