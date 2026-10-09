requirements:
R1 → ht-j16.6, ht-j16.7
R2 → ht-j16.1, ht-j16.2, ht-j16.5
R3 → ht-j16.2
R4 → ht-j16.6
R5 → ht-j16.6
R6 → ht-j16.6
R7 → ht-j16.3, ht-j16.5, ht-j16.6
R8 → ht-j16.4, ht-j16.2
R9 → ht-j16.4, ht-j16.2
R10 → ht-j16.6, ht-j16.2
R11 → ht-j16.7
R12 → ht-j16.1
R13 → ht-j16.6, ht-j16.9
R-new: A non-live end-to-end integration test runs the real watch CLI against a test daemon and the mod harness, including disconnect fallback → (unmapped)
R-new: An operator kill switch disables mod delivery so delivery reverts to hooks plus wake → (unmapped)
R-new: Handoff from mod to native neither loses nor duplicates in-flight unacked items → (unmapped)

findings:
- GAP · end-to-end thin path: add Seam integration leaf depending on .2-.7; .9 depends on it.
- GAP · R9 kill switch: no task mentions one.
- GAP · R1/R9 version gate: unowned.
- GAP · R10 rebind ordering race: registration refused for stale session must be retryable.
- GAP · R9 in-flight handoff duplicates: late ack or accepted limit.
- GAP · R12 policy drift from later beads.
- GAP · R2 notify call sites not enumerated.
- UNOWNED-SEAM · stall/liveness criterion.
- UNOWNED-SEAM · watch/watch ack CLI invocation contract.
- UNOWNED-SEAM · mod_channel_live flag consumer.
- ORPHAN · ht-j16.8 gate.
