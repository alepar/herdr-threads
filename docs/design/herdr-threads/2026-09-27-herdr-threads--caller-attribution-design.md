# Native caller attribution

## Goal

Establish reproducible, versioned evidence that a mailbox operation came from the current native top-level Codex or Claude session, and distinguish its subagents and stale occupants before accountable operations are enabled.

Parent: [root design](2026-09-27-herdr-threads-design.md). Bead: ht-4is.2. Autonomous nested design under the user's explicit direction.

## Scope and evidence

This subtree produces a bounded native feasibility probe, sanitized fixtures and an adapter recipe. It does not implement the production thread store or enable ACKs. The recipe is a prerequisite consumed by seat validation and harness adapters.

Identity verification (`identity-verification.md`, archived in git tag `archive/herdr-threads-run-2026-09-26` under `docs/superpowers/runs/2026-09-26-herdr-native-mailbox-thread-plugin/`) establishes that Codex hook session_id may be the shared parent ID; existing transcript session_meta headers distinguish source=cli from source.subagent. Inspection of this root's native transcript also found turn_context.turn_id and root_turn_id fields. These observations are candidate correlation inputs, not proof of per-call identity. Claude documents agent_id on subagent tool hooks.

## Probe method

Use newly created isolated native sessions and a hook that records an allowlisted metadata envelope only: harness/version, hook event, session_id, transcript path normalized to a fixture label, tool_use_id, turn_id, agent_id/agent_type when present, native thread/session environment values and a unique harmless probe nonce. Do not log credentials or arbitrary tool contents. Preserve the original full evidence in a scoped probe directory if needed; committed fixtures contain only synthetic IDs and relevant metadata.

For each harness, have the parent issue a harmless metadata command, delegate that same command to a native child, and issue it again after the child exits. Exercise concurrent child activity, a resumed top-level conversation, a cleared/new conversation in the same pane, and stale/absent metadata. Codex must also exercise calls through code mode if the installed session exposes that path. Identify the actual child native thread separately from inherited pane context.

Do not change global hooks/configs or participation in existing hcom sessions. Use the supported per-session isolation established in prior PoCs; no blanket permission disabling. Every Codex worker uses --no-daemon and validates its own non-login HERDR_ENV/workspace/tab/pane values. Use only the experiment's panes, config and state. Do not restart the shared Herdr server.

## Attribution decision

The observer compares per-call hook metadata with the native reference that Herdr reports for the top-level occupant. First test whether the hook's transcript identity reliably identifies the actual calling thread. For Codex, independently test turn_id correlation against native turn metadata to handle any parent-shared transcript path. For Claude, verify documented agent_id behavior on the installed version. A path name, inherited pane variable or shared session_id alone is never accepted.

The selected recipe must distinguish parent/child calls even when both share a process and pane, survive supported resume/clear transitions, and reject stale generation and missing/unknown formats. Correlation must be available when the supported hook executes; a field observed only after the operation cannot authorize it synchronously. Document any flush/retry requirement and keep it bounded.

The production design will obtain a short-lived operation context from the validated hook and check its seat/current occupant generation at the daemon. Prefer an invocation-scoped context over a global per-pane credential that a child inherits. Whether the context is passed by supported command rewriting or another harness mechanism must be established by this probe's actual metadata/transport evidence. Do not add a model-visible attestation ritual or trust a client-provided top-level boolean. This local-account trust boundary prevents accidental child attribution; it does not promise resistance to a malicious same-account process rewriting the database/transcript.

The per-harness capture tasks may complete with a reproducible unsupported result: their deliverable is evidence. The shared recipe/support-gate task completes successfully only if BOTH harnesses have native root/child evidence and a verified invocation-context transport. If either lacks a tested mechanism, that task returns BLOCKED with precise evidence and dependent implementation remains blocked for the autonomous redesign workflow. It must not mark the task successfully complete with only the parent path passing. In autonomous mode investigate a supported alternative or record an external blocker; do not silently weaken the root requirement.

## Deliverables and acceptance

- Separate Codex and Claude capture/probe results, reproducible commands, versions and sanitized root/child/resume/clear/stale/missing fixtures.
- One shared recipe document naming the accepted fields, correlation timing, rejection rules and invocation-context transport for each harness.
- A small deterministic fixture classifier proving the documented parent/child distinction, alongside native capture evidence proving those fixtures came from the harness path. The classifier is a probe artifact; production adapters still need integration tests.

The two harness captures are independent. Recipe consolidation consumes both and owns any compatibility decision. No attribution result is inferred from a terminal prompt success or an agent's prose description.

## Fresh execution evidence and authority boundary

The recipe must establish what Herdr's native reference actually measures on the pinned host version. A newly issued RPC is not itself evidence of a current execution: cached metadata that still names a predecessor fails the freshness requirement. Capture controlled replacement before observation and stale host metadata alongside per-invocation root/child evidence. If the available native sources cannot establish the required relation synchronously, the shared positive support gate remains blocked; do not substitute inherited pane identity or a transcript filename.

Production identity preparation performs a new bounded target observation after request arrival and combines it with the tested per-call recipe. It issues an internal, payload-bound single-use mutation permit with a 250 ms lifetime; the store rechecks boot, observation epoch, occupant generation, expiry and known invalidation at its transaction decision. Host calls and native evidence waits happen outside the SQLite writer. A proof prepared too early must be discarded and reacquired rather than extended by the client.

The authority point is the validated observation plus the transaction decision checks, not an atomic native-process/SQLite commit operation. Replacement before that observation or learned before decision rejects the predecessor. An otherwise invisible replacement after observation can fall in the documented residual gap; any committed action retains its original proven actor and observation/decision provenance, never the successor's identity. The probe records this limit honestly and does not infer an impossible atomic guarantee from a passing fixture.

Add sanitized fixtures for request-before-observation ordering, cached predecessor metadata, stale/expired permits and evidence unavailable until after the hook returns. Both harnesses still require positive invocation-context transport evidence before downstream implementation can pass its prerequisite gate.

## Post-Implementation Notes

*As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
