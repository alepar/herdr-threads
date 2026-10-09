c1 · r1 · GAP · end-to-end path · applied — new leaf ht-j16.10 Integration sweep (daemon, watch CLI, mod, ack, fallback); ht-j16.9 depends on it
c2 · r1 · GAP · R9 kill switch · applied — operator switch HERDR_THREADS_MOD_DELIVERY=off in ht-j16.1 contract, ht-j16.5, ht-j16.7; remote Anthropic switch already falls back by disconnection
c3 · r1 · GAP · R1/R9 version gate · applied — ht-j16.7 setup-status reports unsupported version; ht-j16.10 tests no-mod session keeps native wake
c4 · r1 · GAP · R9 handoff · applied — ht-j16.6 re-acks delivered-unacked ids after re-registration; accepted limit in ht-j16.1 TRUST text; late ack settles (ht-j16.3); tested in ht-j16.10
c5 · r1 · GAP · R2/R8 non-message attention · applied — ht-j16.6 delivers attention items as marker once per version, never acked
c6 · r1 · GAP · R10 rebind race · applied — ht-j16.2 refused registration retryable, never live; ht-j16.6 retries on exit 2
c7 · r1 · GAP · R12 policy drift · applied — ht-j16.3 and ht-j16.4 acceptance update TRUST-POLICY on divergence
c8 · r1 · GAP · R2 notify call sites · applied — ht-j16.2 acceptance enumerates call sites
c9 · r1 · UNOWNED-SEAM · stall criterion · applied — ht-j16.1 owns the criterion; ht-j16.2 implements
c10 · r1 · UNOWNED-SEAM · watch CLI invocation contract · applied — ht-j16.1 owns; ht-j16.5 implements; ht-j16.6 consumes
c11 · r1 · UNOWNED-SEAM · mod_channel_live flag · applied — ht-j16.4 owns producer and sole consumer
c12 · r1 · ORPHAN · ht-j16.8 · rejected — gate bead mandated by super-auto for final-SHA evidence (ht-j16.9); serves R13 ordering
