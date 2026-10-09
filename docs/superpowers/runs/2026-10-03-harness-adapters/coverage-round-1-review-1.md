requirements:
R1 → ht-3bi.1, ht-3bi.2.1, ht-3bi.3.3, ht-3bi.5.1, ht-3bi.7, ht-3bi.9
R2 → ht-3bi.2.1, ht-3bi.2.3, ht-3bi.5.2, ht-3bi.5.3, ht-3bi.5.4, ht-3bi.7
R3 → ht-3bi.1, ht-3bi.4, ht-3bi.9
R4 → ht-3bi.4, ht-3bi.2.4.1, ht-3bi.7
R5 → ht-3bi.2.1, ht-3bi.2.2, ht-3bi.6.3, ht-3bi.6.4, ht-3bi.7
R6 → ht-3bi.2.2, ht-3bi.6.3, ht-3bi.6.4, ht-3bi.7, ht-3bi.8
R7 → ht-3bi.2.3, ht-3bi.2.4.1, ht-3bi.2.4.2, ht-3bi.3.4, ht-3bi.3.6, ht-3bi.6.1, ht-3bi.6.4, ht-3bi.6.6
R8 → ht-3bi.2.4.1, ht-3bi.2.4.2, ht-3bi.2.4.3, ht-3bi.7
R9 → ht-3bi.3.1, ht-3bi.3.4, ht-3bi.7
R10 → ht-3bi.3.2, ht-3bi.5.1, ht-3bi.7
R11 → ht-3bi.5.1, ht-3bi.5.2, ht-3bi.6.1, ht-3bi.6.2, ht-3bi.7
R12 → ht-3bi.5.3, ht-3bi.5.4, ht-3bi.6.5, ht-3bi.7
R13 → ht-3bi.6.1, ht-3bi.6.3, ht-3bi.6.4, ht-3bi.6.6, ht-3bi.8
R14 → ht-3bi.3.3, ht-3bi.3.4, ht-3bi.3.5, ht-3bi.3.6, ht-3bi.6.6, ht-3bi.7
R15 → ht-3bi.7, ht-3bi.6.6, ht-3bi.8
R16 → ht-3bi.8, ht-3bi (root-owned final-SHA measurement gate)
R17 → ht-3bi.8
R18 → ht-3bi (coordinator handoff gate; batching/settings preservation coverage missing)

necessary outcomes derived from Goals:
- A single adapter implementation and registration reach generic selection, hooks, setup/status, discovery, health, doctor and host lookup.
- Claude/Codex retain their existing behavior throughout the migration.
- Runtime and domain evidence remain bounded, transactional and honestly attributed.
- Hermes uses qualified native runtime/profile contracts and owned plugin assets to deliver bounded context at top-level turn boundaries.
- Hermes preserves native approvals and supports isolated cooperative invitation acceptance, inbox reads and ACKs.
- Discovery, canary and release tooling distinguish exact development-source/model-free evidence from release/live evidence.

R-new: none.

findings:
- type: GAP
  subject: R18 — explicit batching/settings preservation at coordinator handoff
  evidence: R18 requires wake_batch_delay_ms default0, explicit batching/retry overrides and legacy HealthSettings fallback. Tree only names generic wake fences and strict legacy health compatibility; root handoff does not specify concrete conservation checks. The whole adapter tree could pass while main absorption silently changes a default or loses an override.
  proposed fix: Amend root handoff to require concrete checks after main absorption; relevant existing tests or focused regressions under ht-3bi.7. Coordinator retains full sweep/merge/cleanup/release; no immediate rebase.

orphans: none.
unowned seams: none established by supplied ownership lines.
previous-round fixes: none.
