requirements:
R1 → ht-5fr.1.2, ht-5fr.1.3, ht-5fr.1.5, ht-5fr.1.7, ht-5fr.1.11, ht-5fr.2.7, ht-5fr.2.8, ht-5fr.2.12, ht-5fr.5.9, ht-5fr.5.11, ht-5fr.11
R2 → ht-5fr.1.1, ht-5fr.1.6, ht-5fr.1.10, ht-5fr.3.10
R3 → ht-5fr.3.2, ht-5fr.3.5, ht-5fr.3.6, ht-5fr.3.8, ht-5fr.4.3, ht-5fr.4.4, ht-5fr.4.5, ht-5fr.4.7, ht-5fr.4.12, ht-5fr.4.16
R4 → ht-5fr.3.3, ht-5fr.3.7, ht-5fr.3.8, ht-5fr.3.13, ht-5fr.3.14, ht-5fr.5.13
R5 → ht-5fr.3.11, ht-5fr.4.8, ht-5fr.4.9, ht-5fr.4.10, ht-5fr.4.11, ht-5fr.4.12, ht-5fr.4.16, ht-5fr.4.18
R6 → ht-5fr.3.16, ht-5fr.4.6, ht-5fr.4.16, ht-5fr.2.2, ht-5fr.6.5, ht-5fr.6.11
R7 → ht-5fr.4.13, ht-5fr.13
R8 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.5.7, ht-5fr.5.8, ht-5fr.5.9, ht-5fr.5.10, ht-5fr.5.12, ht-5fr.5.13
R9 → ht-5fr.5.12, ht-5fr.5.13, ht-5fr.4.10, ht-5fr.6.11, ht-5fr.11
R10 → ht-5fr.6.1, ht-5fr.6.15, ht-5fr.5.12, ht-5fr.8
R11 → ht-5fr.7, ht-5fr.8, ht-5fr.5.4, ht-5fr.1.9
R12 → ht-5fr.1.10, ht-5fr.4.7, ht-5fr.4.14, ht-5fr.13
R13 → ht-5fr.1.4, ht-5fr.1.8, ht-5fr.1.9
R14 → ht-5fr.9, ht-5fr.12
R15 → ht-5fr.5.1, ht-5fr.5.2, ht-5fr.6.2, ht-5fr.3.7, ht-5fr.3.8, ht-5fr.3.13, ht-5fr.10, ht-5fr.11
R16 → ht-5fr.10, ht-5fr.11
R-new: A seat on one node can be found and addressed (invite, direct pair) from another node before it has authored or been named in any replicated fact. → (unmapped)
R-new: Clients that parse or validate public IDs keep working with the tagged ID form and the d… direct-thread form (harness integrations, mod, hooks). → (unmapped)
R-new: The node store can still change its schema after the clean baseline (forward upgrade path). → (unmapped)

findings:
1 GAP · remote seat discoverability (same as review 1 #1).
2 GAP · ID-format consumers (same as review 1 #3).
3 GAP · real ssh transport never exercised end to end (default ssh argv, remote PATH).
4 GAP · discovery not composed/exercised: ht-5fr.7 lacks deps on 5.7, 5.10; ht-5fr.11 has no discovery case.
5 GAP · operator CLI and unreachable marker not exercised across daemons: ht-5fr.11 lacks deps on ht-5fr.8, ht-5fr.13.
6 GAP · docs ordered before features: ht-5fr.12 lacks deps on ht-5fr.13, 4.14.
7 GAP · post-baseline schema evolution path absent (ht-5fr.1.4, 1.8 delete the chain; no forward node migrations).
8 UNOWNED-SEAM · sync status/unapplied read API between ApplyEngine/coordinator and ht-5fr.8.
9 UNOWNED-SEAM · node naming + reachability join and threshold for the marker (ht-5fr.13 ↔ 1.10/5.4/5.6; 4.14 shares output).
NEEDS-SPEC: ht-5fr.7; ht-5fr.8 / ht-5fr.13
