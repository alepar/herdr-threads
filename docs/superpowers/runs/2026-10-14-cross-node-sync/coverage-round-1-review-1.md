requirements:
R1 → ht-5fr.1.2, ht-5fr.1.3, ht-5fr.1.7, ht-5fr.1.11, ht-5fr.2.1, ht-5fr.2.7, ht-5fr.2.8, ht-5fr.2.12, ht-5fr.5.9, ht-5fr.5.11
R2 → ht-5fr.1.1, ht-5fr.1.6, ht-5fr.1.10, ht-5fr.3.10, ht-5fr.4.10
R3 → ht-5fr.3.2, ht-5fr.3.5, ht-5fr.3.6, ht-5fr.3.8, ht-5fr.4.3, ht-5fr.4.4, ht-5fr.4.5, ht-5fr.4.7, ht-5fr.4.12, ht-5fr.4.16
R4 → ht-5fr.3.3, ht-5fr.3.7, ht-5fr.3.8, ht-5fr.3.13, ht-5fr.3.14
R5 → ht-5fr.3.11, ht-5fr.4.8, ht-5fr.4.9, ht-5fr.4.10, ht-5fr.4.11, ht-5fr.4.12, ht-5fr.4.16, ht-5fr.4.17, ht-5fr.4.18
R6 → ht-5fr.3.16, ht-5fr.4.6, ht-5fr.4.11, ht-5fr.4.16, ht-5fr.2.2, ht-5fr.4.15, ht-5fr.6.5
R7 → ht-5fr.4.13, ht-5fr.13, ht-5fr.5.6
R8 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.5.3, ht-5fr.5.7, ht-5fr.5.8, ht-5fr.5.9, ht-5fr.5.10, ht-5fr.5.12, ht-5fr.5.13
R9 → ht-5fr.5.12, ht-5fr.5.13, ht-5fr.4.10, ht-5fr.4.18, ht-5fr.11
R10 → ht-5fr.6.1, ht-5fr.6.15, ht-5fr.5.12, ht-5fr.5.13, ht-5fr.8
R11 → ht-5fr.7, ht-5fr.8, ht-5fr.5.4, ht-5fr.1.9
R12 → ht-5fr.1.10, ht-5fr.4.7, ht-5fr.4.14, ht-5fr.13
R13 → ht-5fr.1.4, ht-5fr.1.8, ht-5fr.1.9
R14 → ht-5fr.9, ht-5fr.4.15, ht-5fr.3.16, ht-5fr.6.14, ht-5fr.12
R15 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.3.7, ht-5fr.3.8, ht-5fr.3.13, ht-5fr.10, ht-5fr.11
R16 → ht-5fr.10, ht-5fr.11
R-new: A seat on one node can be found and addressed by an agent on another node before the two have interacted (remote seat directory or listing). → (unmapped)
R-new: Peer authentication binds a node_uuid to its public key across nodes; a link presenting a key that does not match the node_uuid is refused. → (unmapped)
R-new: Every reader and validator of public IDs (CLI parsing, delivery framing, Claude mod register.js, harness adapters) accepts the tagged <prefix><tag><8> form and deterministic d… direct IDs. → (unmapped)

findings:
1 GAP · remote seat discoverability · seats publish only on first reference (ht-5fr.4.5); no listing/lookup of never-referenced remote seats.
2 GAP · node_uuid↔key binding · ht-5fr.5.2 consumes only NodeIdentity::sign/NodeUuid; no check that a peer's key matches its claimed node_uuid.
3 GAP · ID-format consumers · no task updates CLI parsers, delivery frame, mod register.js, adapters for tagged and d… IDs.
4 GAP · discovery reaching the Dialer · ht-5fr.7 deps only 2.7, 5.4; not on 5.7 reload_targets or 5.10 composition.
5 UNOWNED-SEAM · remote seat read rendering + unreachable marker (ht-5fr.4.14 ↔ ht-5fr.13).
6 GAP · docs ordering · ht-5fr.12 deps only ht-5fr.8; misses ht-5fr.13, 4.14, 7.
7 GAP · cross-node cases in pre-release wake e2e.
8 GAP · leaves without decomposition (ht-5fr.7, .8, .9, .10, .12, .13).
NEEDS-SPEC: ht-5fr.8; ht-5fr.13
