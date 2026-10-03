C1 · r1 · GAP · pre-migration author_kind · applied — ht-1ip.2 backfills from the covering occupant binding (spec §1)
C2 · r1 · GAP · summary caller enforcement · applied — ht-1ip.5 owns entitlement checks and derived_summary stamping (spec §4)
C3 · r1 · GAP · R5 rollup planning · applied — ht-1ip.3 owns rollup planning, with a two-level test
C4 · r1 · GAP · end-to-end summary flow · applied — new leaf ht-1ip.16 Seam integration: summary flow
C5 · r1 · GAP · block version reuse · applied — ht-1ip.5 reuses only the current chunking_version; prompt/model never invalidate (spec §4)
C6 · r1 · GAP · R1 Claude compact fallback · applied — ht-1ip.10 documents the accepted limit when no evidence; ht-1ip.9 keys on EventKind
C7 · r1 · GAP · R8 catch-up entry trigger · applied — ht-1ip.5 computes F and calls enter_or_keep; ht-1ip.6 stores it unchanged
C8 · r1 · UNOWNED-SEAM · is_priority predicate · applied — single predicate moved into the ht-1ip.1 contract
C9 · r1 · UNOWNED-SEAM · composer stash hook · applied — ht-1ip.13 owns the inert stash hook interface
C10 · r1 · UNOWNED-SEAM · summary procedure reference · applied — SUMMARY_PROCEDURE_REF constant in the ht-1ip.1 contract
C11 · r1 · UNOWNED-SEAM · catch-up progress and Ready signal · applied — ht-1ip.5 owns the entry/Ready/progress calls (one progress per new block)
C12 · r1 · UNOWNED-SEAM · soft point vs effective deadline · applied — lazy per-tick evaluation stated in ht-1ip.13 and spec §10
C13 · r1 · UNOWNED-SEAM · job bundle and submission format · applied — edge ht-1ip.11 <- ht-1ip.5
C14 · r1 · UNOWNED-SEAM · Claude compact event · applied — merged with C6 (EventKind routing, no extra edge)
C15 · r2 · UNOWNED-SEAM · progress to extension write · applied — inert extension hooks in ht-1ip.1, called by ht-1ip.6, implemented by ht-1ip.7
C16 · r2 · GAP · relays-user guidance · applied — ht-1ip.11 owns when to pass --relays-user
C17 · r2 · GAP · trust-policy provenance and timing · applied — ht-1ip.1 claims text; poke rule ht-1ip.13; stash note ht-1ip.14; Claude limit ht-1ip.10 (spec §11)
C18 · r2 · GAP · R14 fallback · applied — ht-1ip.2: relays_user 0, NULL reads agent, author_kind_backfilled marker
C19 · r2 · GAP · poke flow integration · applied — new leaf ht-1ip.17 Seam integration: soft-deadline poke
C20 · r2 · UNOWNED-SEAM · poke settings · applied — ht-1ip.1 owns soft_fraction with the summary settings
C21 · r2 · UNOWNED-SEAM · who submits job results · applied — workers are child invocations submitting with the seat's lease (spec §4)
C22 · r2 · GAP · R1 top-level filter · applied — ht-1ip.9 owns top-level discrimination (spec §9)
C23 · r2 · GAP · worker availability · applied — ht-1ip.12 captures worker-spawn evidence; ht-1ip.11 per-harness strategy and sequential fallback
C24 · r2 · GAP · skill delivery · applied — ht-1ip.11 owns the delivery check (existing installer + doctor)
C25 · r2 · UNOWNED-SEAM · ht-1ip.16 needs author_kind · applied — edge ht-1ip.16 <- ht-1ip.2 plus a relays case
C26 · r2 · UNOWNED-SEAM · supersession call site · applied — ht-1ip.6 owns the seats.rs call site
C27 · r2 · GAP · R16 version change · applied — stale-version submit refused; cover/rollup assert one version (spec §4)
C28 · r2 · GAP · p99 cold start · applied — ht-1ip.5 owns the empty-sample behaviour (p99_cold, clamp)
C29 · r2 · GAP · sender-visible deferral · applied — ht-1ip.7 owns the line; ht-1ip.16 case
C30 · r2 · UNOWNED-SEAM · doctor report Claude compact · applied — ht-1ip.10 owns it
C31 · r2 · UNOWNED-SEAM · release on exit · applied — ht-1ip.6 owns release on every exit path (spec §7)
G1 · graph · GRAPH-EDGE · ht-1ip.15 <- ht-1ip.11 · kept — smoke runs the skill procedure
G2 · graph · GRAPH-EDGE · ht-1ip.11 <- ht-1ip.5 · parked — narrow to ht-1ip.4 is safe no (skill body cites ht-1ip.5 behaviour)
G3 · graph · GRAPH-EDGE · ht-1ip.5 <- ht-1ip.4 · kept — submit handler calls validator/fold
G4 · graph · GRAPH-EDGE · ht-1ip.5 <- ht-1ip.3 · kept — planner calls chunker/cover
G5 · graph · GRAPH-PROPOSAL · ht-1ip.3/.4/.5 summary core seam contract · parked
