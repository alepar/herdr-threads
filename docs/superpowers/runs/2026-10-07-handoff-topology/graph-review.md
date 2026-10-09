change: ht-qhz.19 <- ht-qhz.4.3 · keep · Downstream completion and terminal replay implementation consumed by cross-effect authority-guard integration tests.
change: ht-qhz.4.3 <- ht-qhz.4.2 · keep · BootstrapPlan live resume and native submission progress consumed by downstream launch/completion; both also declare src/cli/topology_handoff.rs and src/cli/retry.rs.
change: ht-qhz.4.2 <- ht-qhz.2.3 · keep · Canonical recipient/thread attachment and guarded resolution implementation consumed by the live coordinator.
change: ht-qhz.4.2 <- ht-qhz.2.2 · keep · Canonical reserve/evidence/recovery transitions and one-use submission authorization consumed before native creation.
change: ht-qhz.2.3 <- ht-qhz.2.1 · keep · Canonical bootstrap persistence, schema and exact identity lookup consumed by attachment and atomic completion.
change: ht-qhz.2.2 <- ht-qhz.2.1 · keep · Canonical bootstrap persistence, schema and exact identity lookup consumed by attempt transitions and recovery decisions.
change: ht-qhz.9 <- ht-qhz.4.3 · keep · Executable downstream launch, bootstrap completion and terminal replay consumed by the actual CLI smoke and retry assertions.
expected: depth 7→7 · width 2.3→2.3
recommendation: Keep all seven edges: each connects an actual consumed implementation to its producing leaf. The sole individually depth-reducing candidate, ht-qhz.4.3 <- ht-qhz.4.2, carries an explicit live-resume dependency and shared files, so removing it is not safe.
