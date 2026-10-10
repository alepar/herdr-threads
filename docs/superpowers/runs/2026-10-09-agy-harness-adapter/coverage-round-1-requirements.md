R1 A bare `agy` started in a Herdr pane enrolls a seat on its first prompt, through globally installed hooks (no launch required), like Claude and Codex.
R2 A subagent never ends, moves or replaces the parent binding; /clear rebinds; the role comes from on-disk witnesses judged by a neutral hook-runner rule failing toward child, and the daemon decides resulting enrollment/rotation/reattachment against A2.
R3 Resume (`agy -c` / `--conversation`) keeps the seat through C1 cooperative continuity.
R4 Pending message bodies (ordinary and lazy) reach the agy model directly between model calls, framed as untrusted peer data, without the agent running `herdr-threads inbox`. Receipts settle only after the session's own transcript shows the delivery.
R5 An idle agy with pending nudge-class attention is woken through Herdr into an empty composer, and only then.
R6 `herdr-threads setup agy` / `unsetup agy` install and remove, reversibly and with recorded ownership:
R7 `launch --kind agy` and `handoff … --kind agy` start agy under the guarded launch, with validated arguments.
R8 Core code (cli, service, daemon, store, host, protocol, app) names no harness. Every harness-specific behaviour sits behind `HarnessAdapter` or its policy traits in `src/harness/<id>/`. `scripts/check-harness-neutral` enforces this in the per-change checks and in CI, with an allowlist whose entries each carry a reason.
R9 Claude's v0.5.0 mod delivery runs through the new `DeliveryPolicy` with byte-identical wire and behaviour.
R10 AGENTS.md documents the harness-neutral rule and the core delivery/wake contract (§Delivery contract). The current code is audited against the contract, and divergences are recorded and fixed.
R11 A seat can opt in at runtime to being woken by any message, lazy included (`wake_on: all`). The herdr-threads SKILL explains how to enable it and when.
R12 Testing, in three tiers:
R13 The final report states how much code outside `src/harness/agy/` the agy adapter needed:
R14 The native gate passes at the final code SHA and the live-smoke record is committed under docs/evidence/ (fallback recorded verbatim if live access is refused).
R15 An enrolled agy session receives its startup check-in context (seat header, rules, non-body attention) as a hook-injected systemMessage.
R16 Golden byte-identity baselines for Claude/Codex/Hermes surfaces exist before the sweep and Claude re-fit move code.
