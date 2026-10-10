R1 Several agents (foreground, subagents, background/one-shot) occupy one pane's seat concurrently, each as its own binding keyed by harness session; a new session never ends another session's binding.
R2 Every CLI and hook call is attributed to the exact calling session (and subagent) from the harness environment; agent calls without a session id or a liveness fingerprint are refused; human seats keep pane attribution.
R3 Each binding has its own per-thread read cursor; every binding receives every new message exactly once in its own stream; one binding's display never hides mail from another.
R4 ACK obligations stay per seat and are settled once by a top-level binding; subagent bindings are read-only.
R5 A new binding starts at the seat's maximum cursor and gets first-contact per-thread history stats once; a resumed binding catches up or fast-forwards beyond one inbox page without skipping obligations.
R6 Hook notifications, the mod delivery channel and wake operate per binding (composer wake to the pane's foreground agent once).
R7 Bindings become disconnected on process exit, same-process session switch, or harness end events; never deleted; a seat is active iff any binding is connected; presence is visible.
R8 A pane closing within a live Herdr retires its seat (bindings disconnect, memberships retire, record persists); a Herdr restart still goes through restore holds.
R9 Threads whose joined members are all inactive archive after the quiet window; the archival scan completes on busy instances.
R10 Hermes enrolls globally via its hooks (ht-4ne) on bindings; a one-shot hermes -z in a Claude pane adds a binding instead of displacing; Hermes lineage carries cursors across compression/branch.
R11 Human seats (me init) remain exclusive; managed launch records a fingerprinted placeholder binding replaced by the launched agent's check-in.
R12 Existing installs migrate to the new schema without duplicate delivery; protocol/wire and installed hooks are updated; agents mid-session re-enroll.
R13 TRUST-POLICY.md, the herdr-threads skill and docs describe the new model (shared-seat consequence, refusals, provenance values).
R14 Every configuration the spec enumerates (Claude+subagents, Codex embedded/daemon, Hermes + one-shot, human exclusivity, resume/fast-forward, pane close→retire→archive) is exercised end to end.
