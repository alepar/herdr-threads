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
c13 · r2 · GAP · R9/R17 handoff · applied — 30 s reconnect grace before native kick (ht-j16.1 contract, ht-j16.2); unrecovered handoff at-least-once accepted limit
c14 · r2 · GAP · R8 hook digests · applied — ht-j16.4 omits digest on SessionStart and PreToolUse(Bash), the only installed Claude hooks
c15 · r2 · GAP · truncated items · applied — excluded from stall; settle through agent inbox/ACK via 'run herdr-threads body'
c16 · r2 · GAP · stall dual delivery · applied — Close{stalled} plus 10-minute re-registration cooldown
c17 · r2 · GAP · R14 attention route/fallback · applied — route like messages, re-sent per version and per watch start, native ladder fallback when not live
c18 · r2 · GAP · sweep cases · applied — ht-j16.10 case list extended
c19 · r2 · UNOWNED-SEAM · exit codes · applied — contract maps 0/1/2/3 and mod reactions
c20 · r2 · UNOWNED-SEAM · generation · applied — binding generation; /clear and resume rotate; reload does not
c21 · r2 · GAP · ledger durability · applied — $.store keyed by session id; failed acks retried
c22 · r2 · UNOWNED-SEAM · ModChannelStatus · applied — contract owns shape
c23 · r2 · GAP · operator switch scope · applied — daemon setting mod_delivery closes live channels; env override kept
c24 · r2 · GAP · post-abort hold vs stall · applied — hold ends after 120 s idle with empty box; longer holds stall into native ladder with its own composer guards (accepted)
c25 · r2 · GAP · ack failure retry · applied (with c21)
c26 · r2 · GAP · orphan watch · applied — parent-death detection in ht-j16.5
c27 · r2 · GAP · API presence · applied — mod checks APIs before spawning watch
c28 · r2 · GAP · subagent tool calls · applied — main-agent only, stress includes subagents
c29 · r2 · GAP · draft vs idle submit · applied — hold up to 120 s while box non-empty, then submit (draft preserved per spike)
c30 · r2 · UNOWNED-SEAM · stall accessor · applied — ht-j16.4 consumes is_live and stalled
c31 · r2 · GAP · R12 registration policy duty · applied — ht-j16.2 TRUST clause
c32 · r2 · NEEDS-SPEC · turns without tool calls / append mid-turn · not honoured (round 2) — spec D5 delivers after turn end; spike showed append mid-turn is read in-turn
g1 · graph · GRAPH-EDGE · ht-j16.9 <- ht-j16.10 · applied — drop: live stress consumes no artifact of the sweep; gate ht-j16.8 already orders it after the fix loop
