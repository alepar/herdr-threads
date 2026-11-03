## Changes since the previous round (redesign design-1 amendment)
Amended spec sections: Caller location, Named host, Wake and delivery contract, hc host (H1), hc launch (H3), TRUST-POLICY and AGENTS.md amendments, Model-visible strings, Testing and evidence, Phase F, Phase N, Phase N2, Phase H1, Phase H3, Key abstractions, Unchanged behavior (new).
New beads (label sp:redesign-design-1), each answering roast-design-1 findings:
- ht-kqrt.67 HcHost D10 finality closure (supersedes ht-kqrt.42) — D10 rewritten on herdr-core Registry::finish (keep, reason)
- ht-kqrt.68 Launched hc worker retires on exit (H3 keep:false finality)
- ht-kqrt.69 tmux composer wake incl. idle-exit keep-alive, pane-mode gate (supersedes ht-kqrt.39)
- ht-kqrt.70 tmux turn state with every turn-ending edge, StopFailure, screen-idle clear (supersedes ht-kqrt.36)
- ht-kqrt.71 restore with nameless hold on name collision (supersedes ht-kqrt.22)
- ht-kqrt.72 inherited-name refusal with same-name limit and revised M6 (supersedes ht-kqrt.27)
- ht-kqrt.73 spawn_detached reparents the daemon (K5 sweep)
- ht-kqrt.74 LocateCaller terminal-root wall and start-stamped cache key
Owner-bead entry text synced: ht-kqrt.15 (NamedHost), ht-kqrt.41 (HcHost), ht-kqrt.11 (LocateCaller). Dependents of superseded beads re-pointed to replacements.
