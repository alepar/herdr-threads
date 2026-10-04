# Hermes integration audit

<!-- facet: hermes -->

The shell-hook serializer excludes parent_session_id from extra and does not emit it as a separate payload key. [6]. <!-- claim: 62b5604964fb77d2; evidence: c3334ac950b21e2e; source: 94c13c730f6c770f -->

The pre_llm_call callback passes parent_session_id. [7]. <!-- claim: d31028046701d928; evidence: f7140841e89d062d; source: 644e513b6c23e74d -->

This defeats a shell-only claim of reliable child suppression. Recommended design inference: an owned, dependency-free Python plugin bridge filters callback data before invoking Rust. It forwards session, parent role information, event/reason, platform and runtime identity; it must not forward the conversation history or user message. Missing role evidence is explicit, not assumed top-level. The bridge does not replace Hermes, monkeypatch internals, auto-approve tools or run a daemon.

on_session_start is an observer whose return is ignored; pre_llm_call supports context injection. [8]. <!-- claim: a6d4a1c10114b24c; evidence: 5c408b75eb7bd7d4; source: b9b921665c208b2d -->

Use a context-capable turn callback for check-in offers. Lifecycle observers can record pending reset/recovery metadata and make no delivery claim. Hermes on_session_end is per conversation call, so it cannot end a durable occupant binding. Gateway hook directories are gateway-only; do not use them for a CLI integration. Proposed initial support is interactive CLI in Herdr, default or explicitly selected profile. Gateway, Desktop and TUI are separate admission surfaces.

Hermes runtime identity resolves from an install stamp, live git, or unknown provenance. [9]. <!-- claim: 5cd785c17eaa3d62; evidence: 5122efb0992416c9; source: cc35996af809b8fc -->

Preserve derived identity, commit and dirty/unknown status alongside an optional release version. Never credit a development commit's success to the bare release, or use a PATH probe as the running process's version. Capture the runtime identity once with a bounded bridge startup strategy; the official resolver itself can call git, so hot callbacks cannot repeatedly call it. If bounded attribution is unavailable, record unavailable evidence and continue only capabilities admitted independently.

General Hermes plugins are disabled by default and use explicit plugins.enabled selection. [10]. <!-- claim: be4df049c599c695; evidence: e0c5982da89e1d68; source: c21c9f0fbfdd3ca4 -->

Setup stages owned plugin files and reports the native enable step. It never writes plugin consent grants or adds --accept-hooks, HERMES_ACCEPT_HOOKS, hooks_auto_accept or --yolo. Status distinguishes installed, enabled and observed. Removal verifies the files it owns; foreign plugin/configuration remains intact. Profiles need explicit selection because the active profile's config/home governs discovery. Whether setup should optionally perform the native interactive enable flow is a user design choice, not implied by installed files.

Proposed lifecycle state machine: role-incomplete observers never register; the first role-qualified pre_llm_call registers a Startup/attach execution even when history exists. Later callbacks are Current. A documented reset observer marks pending recovery for the new session; the next role-qualified callback performs Clear and emits recovery. Unknown session rotation causes a new attachment, never inferred continuity. A resumed process without a native resume discriminator cannot use cooperative resume repair.

Only context-capable turn callbacks deliver offers. Hermes pre-tool callbacks may supply payload/version evidence without consuming attention or injecting a context response; long running turns have no promised mid-turn delivery. Check-ins keep bounded, fresh external event IDs. Rust stdout flush and Python return are offered context, not proof Hermes admitted it or the model consumed it. Use sub-timeouts inside the native callback deadline, and test callback discard, replay and lost context. Keep existing explicit inbox/accept/ACK provenance.

Other constraints: native snake_case event names need validation support. Opaque session IDs must not be coerced into UUIDs. Reset/new callbacks are evidence only for the declared transition; a first callback with existing history cannot alone justify cooperative resume continuity. CLI compression has no dedicated general shell-hook event in the audited catalog: per-turn reinjection can restore instructions, but exact compression-triggered hot-thread recovery remains a separately measured limitation. No composer parser or poke-during-turn capability is declared for Hermes without a capture.

Upstream pin: `ea81748579ee1732d214ccb75f91d22208ed623d`; installed source pin: `37daf85b2ad0ee50ed45d7234dc47b7fa24cec09`. Herdr's installed kind list includes Hermes. This proves availability for a later native probe, not compatibility. Raw upstream fetch hit a rate limit; GitHub's contents API supplied the pinned code successfully. Read-only source inspection never touched user config.

Open gaps: recipe floor/build identity, bounded Python version capture, profile discovery, native enable/refusal flow, child paths, resume/clear/compaction mapping, launch argv and actual cooperative accept/read/ACK. These require the post-approval feasibility and implementation checks.


## Bibliography

[6] [Hermes shell hook serializer](https://github.com/NousResearch/hermes-agent/blob/ea81748579ee1732d214ccb75f91d22208ed623d/agent/shell_hooks.py)

[7] [Hermes turn preparation](https://github.com/NousResearch/hermes-agent/blob/ea81748579ee1732d214ccb75f91d22208ed623d/agent/turn_context.py)

[8] [Hermes event hooks documentation](https://github.com/NousResearch/hermes-agent/blob/ea81748579ee1732d214ccb75f91d22208ed623d/website/docs/user-guide/features/hooks.md)

[9] [Hermes runtime version identity](https://github.com/NousResearch/hermes-agent/blob/ea81748579ee1732d214ccb75f91d22208ed623d/hermes_cli/version_info.py)

[10] [Hermes plugin opt-in](https://github.com/NousResearch/hermes-agent/blob/ea81748579ee1732d214ccb75f91d22208ed623d/website/docs/user-guide/features/plugins.md)
