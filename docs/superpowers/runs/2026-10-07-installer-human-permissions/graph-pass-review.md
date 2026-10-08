change: ht-uwd.8.2 <- ht-uwd.8.1 · keep · installer inventory validation/transport consumer used by actual shell-to-Rust gateway regressions
change: ht-uwd.8.2 <- ht-uwd.7 · repoint · ht-uwd.8.2 <- ht-uwd.7.2: permission reconciliation and consent · safe no · Acceptance explicitly consumes ht-uwd.7 lifecycle and consent; its lifecycle requirement must be verified through ht-uwd.7.2.
change: ht-uwd.8.1 <- ht-uwd.7 · repoint · ht-uwd.8.1 <- ht-uwd.7.1: explicit permission lifecycle API · safe no · Removed wait on ht-uwd.7.2 involves shared src/cli/installer.rs and an explicit consent reference.
change: ht-uwd.7.2 <- ht-uwd.7.1 · keep · explicit permission lifecycle API consumed by permission reconciliation
change: ht-uwd.7.1 <- ht-uwd.5 · repoint · ht-uwd.7.1 <- ht-uwd.5.3.2: independent Claude lifecycle incorporating renderer and historical transfer · safe no · The dependent explicitly consumes ht-uwd.5 backend and historical transfer outputs.
change: ht-uwd.5.3.2 <- ht-uwd.5.3.1 · keep · prepared ownership transfer engine used by independent Claude lifecycle
change: ht-uwd.5.3.1 <- ht-uwd.5.1 · keep · Claude rule renderer and representability contract
change: ht-uwd.5.3.1 <- ht-uwd.5.2 · keep · independent hook completeness and legacy ownership evidence
change: ht-uwd.5.3 <- ht-uwd.5.2 · narrow · ht-uwd.5.3.1 <- ht-uwd.5.2: legacy ownership evidence · safe no · Existing producer edge suffices structurally, but the epic declares shared src/harness/setup.rs and historical ownership consumption.
change: ht-uwd.5.3 <- ht-uwd.5.1 · narrow · ht-uwd.5.3.1 <- ht-uwd.5.1: Claude renderer and representability contract · safe no · The lifecycle child and renderer share src/harness/permissions/claude.rs; renderer consumption is explicit.
change: ht-uwd.9 <- ht-uwd.3 · keep · runtime actor boundary and human lifecycle/communication integration used by continuation guidance and human me help
change: ht-uwd.8 <- ht-uwd.7 · narrow · ht-uwd.8.1 <- ht-uwd.7.1: permission lifecycle API; ht-uwd.8.2 <- ht-uwd.7.2: permission reconciliation and consent · safe no · Removed waits include shared src/cli/installer.rs and declared lifecycle/consent consumption.
change: ht-uwd.7 <- ht-uwd.6 · narrow · ht-uwd.7.1 <- ht-uwd.6: Codex permission backend · safe yes · This leaf edge already exists; the omitted ht-uwd.7.2 wait has disjoint declared files and consumes ht-uwd.7.1 lifecycle API rather than the Codex backend.
change: ht-uwd.7 <- ht-uwd.5 · narrow · ht-uwd.7.1 <- ht-uwd.5.3.2: independent Claude lifecycle · safe no · ht-uwd.7.2 and ht-uwd.5.3.2 share tests/installer_integrations.rs, and the epic explicitly consumes Claude backend outputs.
change: ht-uwd.4 <- ht-uwd.1 · keep · ordinary command catalog and namespace grammar consumed by permission boundary contract
change: ht-uwd.3 <- ht-uwd.1 · narrow · ht-uwd.3.1 <- ht-uwd.1: InvocationActor route; ht-uwd.3.2 <- ht-uwd.1: InvocationActor route and command catalog · safe no · Both leaf edges already exist, but declared src/cli/mod.rs overlap and explicit route consumption prevent the strict safe classification.
expected: depth 10→10 · width 1.8→1.8
recommendation: Safely narrow ht-uwd.7’s Codex gate to its existing lifecycle leaf edge; this improves graph precision without changing execution depth. The critical chain carries real interfaces and ownership artifacts, so no verified safe change shortens it.
