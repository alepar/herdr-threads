requirements:
R1 → ht-54r.1, ht-54r.3, ht-54r.17, ht-54r.21
R2 → ht-54r.1, ht-54r.2, ht-54r.3, ht-54r.5, ht-54r.6, ht-54r.4
R3 → ht-54r.1, ht-54r.7, ht-54r.22
R4 → ht-54r.20
R5 → ht-54r.7, ht-54r.8, ht-54r.22
R6 → ht-54r.9, ht-54r.12, ht-54r.13
R7 → ht-54r.10, ht-54r.11, ht-54r.23, ht-54r.16
R8 → ht-54r.14
R9 → ht-54r.15
R10 → ht-54r.17, ht-54r.11, ht-54r.20
R11 → ht-54r.4
R12 → ht-54r.1, ht-54r.6, ht-54r.11
R13 → ht-54r.18
R14 → ht-54r.19
R-new: Mixed-version upgrade window: an old client or old installed hook (no session/env claim) talking to the new daemon gets a clear refusal or re-setup prompt, not silent misattribution → (unmapped)
R-new: Herdr restart under multi-binding still goes through restore holds, and that path is exercised → (unmapped)

findings:
- GAP · R13 TRUST-POLICY.md — no owns line names the normative policy text (provenance values, refusals, shared-seat consequence, retirement).
- GAP · R12 upgrade: stale installed hooks/plugins and re-enrolling mid-session agents have no owner.
- GAP · R12 V29 data backfill without duplicate delivery — contract owns 'schema/stubs', backfill of cursors and lazy/catch_up rows unowned.
- GAP · R8 restore holds under multi-binding — .14 scoped to live incarnation; no restart scenario in .19.
- GAP · R-new mixed-version callers — refusal/guidance for claim-less old clients/hooks unowned.
- UNOWNED-SEAM · process-info port / fingerprint format shared by client (.2) and daemon (.3, .23).
- UNOWNED-SEAM · Hermes lineage detection (.17) → lineage cursor copy (.20).
- UNOWNED-SEAM · subagent binding creation / parent resolution split between .3 and .11.
- UNOWNED-SEAM · pane foreground-agent predicate for composer wake (.13).
- UNOWNED-SEAM · obligations query / catch-up vs fast-forward boundary (.8, .20, .22).
- UNOWNED-SEAM · managed-launch placeholder fingerprint (.4 ↔ .2/.3).
- UNOWNED-SEAM · availability recompute (.23) → presence display (.16).
