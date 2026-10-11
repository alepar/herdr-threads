# Finding dispositions — session-keyed seat bindings

Every must-fix and confirmed roast finding raised against this branch, with its disposition and evidence.
Sources: code-final-review-1..6.md, roast-pr-1/2, roast-pr-post-cap-audit (1, 2), run.md. Bead ids are
under epic ht-54r; commits are on `super-auto/session-keyed-seat-bindings`.

## Correctness and attribution

| Finding (source) | Disposition | Evidence |
|---|---|---|
| Lazy inbox source per seat, not per binding (F1; roast-pr-1 queries.rs:6514) | fixed | ht-54r.57; inbox tests with two top-level + one subagent binding |
| Subagent cannot read/advance its own stream (F2, ht-6v2; roast-pr-1 cli/mod.rs:2797) | fixed | ht-54r.58, ht-54r.73; `config1_subagent_display_advances_its_own_cursor` un-ignored, passing |
| Display-ACK refused for plain-call bindings (F3; roast-pr-1 receipts.rs:315) | fixed | ht-54r.59 (role+liveness, not provenance); `plain_call_enrolled_agent_display_acks` passing |
| Hermes child check-in accepted (roast-pr-1 seats.rs:4480) | fixed | ht-54r.58; Hermes-child CheckIn refusal test |
| Lazy unread ignores membership (roast-pr-1 lazy_delivery.rs:287) | fixed | ht-54r.57 |
| Superseded verdict race (roast-pr-1 bindings.rs:665) | fixed | ht-54r.60 |
| Malformed CODEX_THREAD_ID silently top-level | fixed | ht-54r.58 (fails closed) |
| Fingerprint-less connected bindings never disconnected | fixed | ht-54r.56 |
| 12 deterministic failures on the run tip | fixed | ht-54r.52/.53/.54/.55, ht-54r.39 |
| Continuity successor inserted without fingerprint (roast-pr-2 liveness.rs:72) | fixed | ht-54r.70, ht-54r.72 |
| Continuity reuse submits stale/absent fingerprint (audit-1 hook.rs:2009, :2008) | fixed | ht-54r.78 a56ed12a (`submit_continuity` always sends the current fingerprint), tests/cli/hook.rs |
| Cursor stuck after mod/hook-pull ACK of an ordinary message (roast-pr-2 lazy_delivery.rs:507; FR4 #2) | fixed | ht-54r.76; tests proven red on 212dbd0c by ht-54r.90, exact-equality assertions |
| agy hook delivery lost after upgrade (roast-pr-2 delivery_pull.rs:249; FR4 #3) | fixed | ht-54r.77 (re-adopt, no session_mismatch for a disconnected binding) |
| Continuity reattach without cursors → history unread (FR4 #1) | fixed | ht-54r.78 (general resolver, `seed(Readopt)`), ht-54r.91 (fast-forward) |
| continuity_candidates newest row only (S3) | fixed | ht-54r.78 (any matching row), ht-54r.91 (prefers seat_unresolved) |
| Hermes child before subagent_start supersedes parent / stray row (S4; FR5 #1) | fixed | ht-54r.79 (plain calls never supersede), ht-54r.87 (`supersede_plain_child_row`) |
| Mod channel/hook pull provenance-gated + provenance overwrite (S5) | fixed | ht-54r.79 |
| Single-target composer wake ignores foreground PID (FR4 #7; deviation ht-54r.13) | fixed | ht-54r.82; ht-54r.89 matches the fingerprint against the foreground process set (live probes: Claude MCP/watch children and npm Codex share the group) |
| Plain reconnect makes superseded bindings flap (FR5 #2) | fixed | ht-54r.87 (`plain_reconnect_allowed`; plain-only ranks below lifecycle-claimed); spec §1/§3 reconciled |
| ACK refused for a disconnected binding (deviation ht-54r.20) | fixed per spec §3 | ht-54r.81 + ht-54r.87: process-exit/migrated bindings reconnect on a call; superseded/ended stay refused |
| Compact participants omits presence (deviation ht-54r.16) | fixed | ht-54r.83 |
| Poke path no liveness re-check (S6) | fixed | ht-54r.91 |
| `foreground_binding` test-only predicate (S6) | fixed | ht-54r.75 (removed; config9 asserts the production predicate) |
| Placeholder outranks a plain-call session (audit-2 bindings.rs:148) | fixed | 99bc9a63 (`may_win_same_process`; red→green test in tests/service/liveness_worker.rs; TRUST-POLICY A4) |
| Untested `binding.connected` gate in ack_mod_delivered (audit-2 dispatch.rs:829) | fixed | 25893108 (test fails with the gate mutated away) |
| delivery_pull duplicate reconnect policy (audit-2 delivery_pull.rs:226) | fixed | 14b4a4df (`claim_reconnects_on_plain_call` + `hook_plain_reconnect`; existing tests unmodified) |

## Base absorption

| Finding | Disposition | Evidence |
|---|---|---|
| Migration collision with main 29–31 (S1) | fixed | renumbered 0032–0035 (8d5120ed); V28→latest upgrade tests pass |
| agy unknown to caller_env (S1) | fixed | `HarnessAdapter::caller_env()` / `CallerEnvShape` (8d5120ed); ht-54r.71 moved all harnesses behind it |
| seat_wake_on overlap (S1) | fixed | per-binding composition in 8d5120ed |
| check-harness-neutral 68 violations (roast-pr-2 ×4) | fixed | ht-54r.71; 0 violations |
| main v0.8.0 / v0.8.1 absorbed | done | 8d5120ed, 12300b4c, 2ef00231 (+ eb28b769 InboxQuery.claim) |

## Tests and pipeline

| Finding | Disposition | Evidence |
|---|---|---|
| Sweep failures (continuity_successor, lazy_display selector, config9, watch_admission hang) | fixed | ht-54r.72–.75 |
| config9 race with the liveness worker (FR6 #2) | fixed | 0a2a220c (one-shot in a stand-in the test ends); smoke configs 10/10 ×3 |
| agy native gate 5/15 failing (FR5 #3) | fixed | ht-54r.88: 15/15; all five were pre-binding gate flows (evidence/ht-54r.88/triage.md) |
| Fix-round tests never shown red (FR5 #5) | fixed | ht-54r.90 red-on-212dbd0c evidence; loose `>=` assertions tightened |
| Stale skill test needle (sweep 3) | fixed | 6abfd4f9 |

## Docs and policy

| Finding | Disposition | Evidence |
|---|---|---|
| TRUST-POLICY agy children / lifecycle replaces / ht-4ne retire wording; SKILL.md; CHILD_RESTRICTION (roast-pr-2 TRUST-POLICY.md:804, FR4 #8) | fixed | ht-54r.80, ht-54r.91 |
| TRUST-POLICY/agent-usage reconnect wording vs ht-54r.87 (FR6 #1) | fixed | 0a2a220c |
| operations.md:224 reconnect wording (audit-2) | fixed | 0160c11e |

## Decisions (settled from the spec, per main mljuxaWLC)

| Item | Disposition | Basis |
|---|---|---|
| `/clear` wake model (ht-ozo) | stands | spec §1.3/§1.5/§3: new binding at seat-max cursors, composer wake only for its unread, `/resume` keeps the binding |
| agy run_command attribution (roast-pr-2 escalation) | verified | ht-54r.84 live probe: agy 1.3.3 run_command shells carry `ANTIGRAVITY_CONVERSATION_ID`, agy parent (evidence/ht-54r.84/) |

## Remaining (not fixed; accepted or follow-up)

| Item | Disposition | Reason |
|---|---|---|
| Managed-launch placeholder uses the strict single-foreground read (FR6 #4) | follow-up | launch proceeds with a note; only the lost-first-prompt recovery wake is affected; review rated deferrable |
| Lost lifecycle check-in never replayed → plain-only session loses to predecessor (FR6 #5) | accepted limit | journal replay covers timeout / daemon down; no test |
| read_cursors full-scan probes; lazy probe index; per-seat hook-pull offer ledger; first-contact per-thread counts | punch list (performance) | no correctness effect (roast-pr-1/2 scope filter) |
| Misleading error on a fail-closed rewrite refusal; forward-only migrations note | punch list | UX / existing practice |
| Real-harness payload shapes (Codex SubagentStop/SessionEnd, Hermes subagent_*/resume/compression), sandboxes hiding ancestor PID, sibling PreToolUse hook dropping `--agent-id`, Linux /proc paths | untested scope | need live credentials / Linux CI; parked escalations from the design roast |
| `Outside-adapter:` commit trailer (AGENTS.md rule merged from main mid-run) | gap, main's call | 9 of 156 branch commits since 43cc19cd carry it (all new commits do); retrofitting the rest needs a history rewrite |
| Placeholder + plain-call session of one process both stay connected → no composer wake until the session's first lifecycle check-in (independent review of 99bc9a63, Minor) | accepted (fail-safe) | no wrong-target wake; window ends when `launch_placeholder_replace` runs at the first lifecycle check-in; TRUST-POLICY A4 allows a group with no lifecycle claimer to stay as is |
| operations.md omitted "replacement" from non-reconnecting reasons (independent review, Minor) | fixed | docs/operations.md:224 |

Independent review of the audit-2 fixes (99bc9a63, 25893108, 0160c11e, 14b4a4df): all four resolved, no Critical/Important, security angle: no bypass (no plain call or hook pull revives a superseded, ended, retired or replaced binding; subagent claims refused before the store; mod-ACK gate pinned by test).
