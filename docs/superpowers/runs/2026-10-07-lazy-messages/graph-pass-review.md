change: ht-big.5.2 <- ht-big.5.1 · keep · occupant-bound fully-displayed candidate journal interface consumed by settlement and explicitly cited by needs
change: ht-big.5.1 <- ht-big.3 · keep · canonical v2 batch handler consumed by chunk rendering and continuation traversal
change: ht-big.2.2 <- ht-big.2.1 · keep · persisted recipient schema and bounded storage accessors consumed by preparation/publication
change: ht-big.2.1 <- ht-big.9 · keep · actual warning migration25 and frozen successor required before lazy migration26
change: ht-big.7 <- ht-big.5.2 · keep · completed default-text settlement/retry workflow exercised by configuration smoke
change: ht-big.5 <- ht-big.3 · narrow · ht-big.5.1 <- ht-big.3: canonical v2 batch handler; ht-big.5.2 <- ht-big.3: canonical completion handler · safe yes · Both leaf edges already exist; narrowing removes no effective wait or consumed interface dependency, and declared CLI files are disjoint from daemon/store files.
change: ht-big.3 <- ht-big.2.2 · keep · published recipient preparation/publication consumed by real lazy inbox traversal and completion
expected: depth 8→8 · width 1.6→1.6
recommendation: Remove redundant epic gate while preserving leaf edges; critical path consumes real artifacts.
