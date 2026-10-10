# Canonical requirements — ht-e99

R1: Target-seat-only send accepts --seat with body/file/stdin; positional ordinary send stays unchanged and mutual-exclusion/no-target validation is explicit.
R2: Direct recipient qualification rejects self/missing/retired/foreign/unresolved/held recipients while supporting resolved unbound recipients and ordinary human send.
R3: Direct lazy/nudge/ACK/deadline/relay/user-intent behavior freezes exactly the addressed target, rejects extra ACK seats/panes, and retains invitation consent.
R4: Omitted handoff channel selects automatic routing for existing-seat, existing-pane, launched-pane and new-tab destinations; explicit forms and metadata constraints remain.
R5: Indexed canonical unordered seat-pair registry has additive migration, uniqueness/FKs, deterministic defaults, no ordinary-thread adoption and stable retention/identity.
R6: Accountable EnsureDirectThread atomically decides A2/recipient eligibility and create/invite/reuse, converges reversed concurrency and replays historical saved results.
R7: Ordinary create/invite events, membership/attention projections and actor provenance honestly distinguish reservation from joined/accepted state; TRUST-POLICY A5 is updated.
R8: Immutable pair enrollment guards every ordinary/service/operator path and forbids third seats and service ownership while preserving ordinary unmarked channels.
R9: Archive/leave/reject/retirement lifecycle refuses resurrection/remapping, reports explicit recovery and reuses the same channel after ordinary repair.
R10: Send preparation/publication validates pair identity and audience, including corrupt membership/raw protocol/ensure-to-send races.
R11: DirectSend intent freezes original actor/caller/namespace/target/body/options/ensure key/send key before effects, with bounded progress and locked atomic synced publication.
R12: Pending-ops/retry resumes the exact ensure/send and selected thread after loss/restart, retains staged failures, never duplicates or replaces intent and preserves historical completed output/cleanup.
R13: Preflight checks syntax/flags/caller/target/capability before publication/effects where supported; authoritative validation stays transactional and final output names ordinary thread/message IDs.
R14: New automatic handoff variants/versions preserve V1 bytes/hashes/reports/keys and explicit forms, and negotiate additive capability before journal/effects without unsafe fallback.
R15: Existing-recipient automatic handoff freezes recipient/ensure key and canonically registers/attaches selected thread before restricted invite/send, enforcing original caller/payload/namespace/fence equality.
R16: New-tab auto routing occurs after guarded recipient attachment and retains bootstrap/child fence, downstream/outer recovery and at-most-one native submission without new topology/start authority.
R17: Startup remains managed_launch without acceptance/ACK/completion; staged archive protections survive until canonical completion and completed history does not revive them.
R18: Focused store tests exercise migration, identity/index, concurrency, replay, invalid bindings/namespaces, corrupt membership, every enrollment path, lifecycle, human/unbound recipients and ordinary isolation/history.
R19: Focused CLI tests exercise both send forms/body/options and pending retry at every stage with historical journals preserved.
R20: Handoff tests run each of four destination configurations before the terminal gate, retain V1 golden fixtures and exercise child equality, lost replies/restart and topology completion recovery.
R21: README, embedded skill/manual/help document syntax, lazy default, invitations, immutable pair/lifecycle and exact recovery examples without editing real config.
R22: Focused verification follows fmt, locked all-target/all-feature clippy and relevant tests; merge adds default-feature checks, owned helpers stop and scoped leak checks follow full/integration sweeps within stated budgets.
