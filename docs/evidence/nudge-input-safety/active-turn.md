# Attention during reported active turns

2026-10-08. Follow-up to reviewed input-safety freeze197516c3f044e3b3aa6ee7e50594e26183be692a; same worker/worktree/ownership. Expanded user request relayed by main in m5Vkepycu, reiterated mP20ft8js. Preserve original diagnosis and first implementation evidence.

## Source diagnosis and channel distinction

The literal `herdr-threads: attention pending; run herdr-threads inbox` is notification::policy::MARKER. NativeWakeDispatcher sends it via agent.prompt for ordinary attention; receipt pokes use the separate receipt-due text. Neither hooks nor check-in mutate the pane composer: cli::hook::encode_native sends hookSpecificOutput.additionalContext with trusted commands and peer-derived digest, and check-in emits its CLI output. A quoted marker in peer content would not prove native submission. Existing hook fallback test rejects literal attention-pending boilerplate. The actual user's repeated Codex marker is reported evidence, not a captured correlated RPC; its originating event and host state remain unproved.

Before this follow-up, ordinary native submit_prompt already rechecked recognized bound harness plus idle/done; working/blocked/unknown were refused before composer reads. Ordinary fresh pane observations generally represent UI as Unknown without composer evidence; that does not grant prompt readiness, which final agent.get independently decides. Empty composer alone never establishes idle. Coalesced ordinary authority excludes positive ActiveTurn evidence; PokeOnly used structural identity and a recipe-qualified turn-time entry point. That historical exception admitted receipt pokes during reported working turns and is removed here; it is not claimed as the cause of the specific ordinary Codex marker.

Retained official Herdr0.9.3 source7b116c05bfda646af39d2524c54e70c751f57ee8 was inspected read-only:
- src/app/agents.rs builds AgentInfo using pane.agent_status; src/app/api_helpers.rs maps terminal Idle to idle/done based on seen state, Working to working, Blocked to blocked, Unknown to unknown.
- src/app/api/agents.rs agent.get returns the current cached terminal record. A fresh witnessed response proves identity/incarnation, not physical turn completion. Host agent.prompt itself rejects blocked/launch-pending/wrong foreground process but does not independently forbid Working, so Threads must fence active attention.
- src/detect/manifests/codex.toml recognizes OSC spinner and live screen timer as working. src/pane/agent_detection.rs debounces a working-to-plain-idle transition for three confirmations/up to700ms. src/detect/manifest.rs uses codex_state_ambiguous/Unknown when no rule matches rather than assuming idle. These are screen inference, not an authoritative no-turn-running signal.
- src/integration/assets/codex/herdr-agent-state.sh reports SessionStart session identity only; it does not supply complete busy/idle lifecycle state. No real configuration, telemetry or live host was read/changed for this follow-up.

Consequently a stale/misclassified idle/done host record can still admit a physically active turn under the existing accepted approximation. No evidence here establishes whether that happened in the incident. An optional user question offered keeping best-effort Herdr status or requiring authoritative idle (which would defer otherwise safe Codex wakes); no answer arrived during independent work. This change retains the existing approximate policy and does not invent an authoritative signal.

## Change and regression

Dispatcher now refuses reported ActiveTurn for all unsolicited attention before selecting/submitting, regardless of old recipe capabilities. It never calls submit_prompt_during_turn. Native's compatibility entry point uses the same idle/done gate as submit_prompt. PokeOnly leaves soft_poked_at unset; Ordinary/WithWake return pre-submit RefusedUnsafe, preserving prior ladder and pending obligations. Focused-empty minute and unfocused-empty immediate rules remain intact. No mode downgrade, draft mutation, extra Enter, port/schema or peer source import.

Causal RED: active-capability PokeOnly sent input when expected no input. A broader scheduler filter additionally found three historical tests still asserting active queue/draft stash; these expectations were migrated, preserving receipt and idle positive assertions rather than removing coverage.

GREEN on final source:
- host:: native/transport:82 passed, includes both native entry points rejecting working Codex/Claude before composer I/O, and recognized Codex/Claude idle/done positive controls.
- scheduler::126 passed, includes historical capability negatives, same SQLite receipt staying pending then delivered after an idle transition, draft deferral, refusal ladder seams.
- cli::hook::tests command exit0;76 tests scheduled,75 visible ok lines; process-wide stdout redirection omitted one line/final summary, so no invented complete per-test transcript count. Structured hook channel behavior is also supported by source inspection.
- clippy locked/alltargets/allfeatures -Dwarnings exit0 (12.73s including waiting for cargo lock); default-feature script exit0; fmt/diff checks exit0. Sandbox denied nice priority adjustment but both checks executed successfully.
- UUID1867184b-1672-4898-9f65-677cd50b2d68 leakcheck exit0, no leaked test processes. All test commands finished.
- Fresh independent source review and final test-contract review: no blockers under reported-state contract. No full suite (current repository policy); main owns final composition/integrated release gates.

## Coordination handoff

Main should absorb both worker commits with fresh composition review; initial197516c3 remains preserved. The reviewed lazy zero-producer contract44876c0f/4a581f0e (m37rf2dVA) avoids native attention for Lazy entirely; Ordinary pending/refusal semantics stay unchanged. Existing account-migration pause alone cleared. No global HOLD, main mutation, push, deploy, shared-host, actual config, model call or upstream edit. Main owns release and deciding whether a stricter authoritative signal warrants separately approved scope.
