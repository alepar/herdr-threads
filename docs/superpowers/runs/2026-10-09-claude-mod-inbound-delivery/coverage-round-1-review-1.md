requirements:
R1 → ht-j16.6, ht-j16.7, ht-j16.5
R2 → ht-j16.1, ht-j16.2, ht-j16.5
R3 → ht-j16.2
R4 → ht-j16.6
R5 → ht-j16.6
R6 → ht-j16.6, ht-j16.3
R7 → ht-j16.3, ht-j16.5, ht-j16.6
R8 → ht-j16.4, ht-j16.2
R9 → ht-j16.4, ht-j16.2
R10 → ht-j16.6, ht-j16.2
R11 → ht-j16.7
R12 → ht-j16.1
R13 → ht-j16.6, ht-j16.9
R-new: Non-message attention streamed to the mod is presented to the agent with a defined fallback → (unmapped)
R-new: An automated end-to-end path daemon + watch CLI + mod is proven before the gated live stress → (unmapped)
R-new: A kill switch disables the mod path and forces fallback → (unmapped)

findings:
- GAP · R9 kill switch: no owner for a kill switch definition/reader/reporting. Fix: leaf or contract clause.
- GAP · R1/R9 version gate: no version detection; mod on old Claude. Fix: clause in ht-j16.6/ht-j16.7 + test.
- GAP · R9 in-flight handoff duplicates: item shown in context, crash before ack, native wake redelivers. Fix: handoff rule in contract + daemon tests.
- GAP · R2/R8 non-message attention: ht-j16.4 suppresses digest, ht-j16.6 never presents it. Fix: present in mod or keep hook digest.
- GAP · R13 end-to-end thin slice: only gated live run joins parts; add integration sweep leaf + daemon concurrency tests.
- GAP (minor) · R10 registration before binding session update: retry rule. 
- UNOWNED-SEAM · stall/liveness contract not in ht-j16.1 owns.
- UNOWNED-SEAM · watch CLI invocation contract (args, env, exit codes, restart semantics) between ht-j16.6 and ht-j16.5.
- UNOWNED-SEAM · mod_channel_live check-in flag wire field between hook client and daemon.
- ORPHAN · ht-j16.8 gate.
