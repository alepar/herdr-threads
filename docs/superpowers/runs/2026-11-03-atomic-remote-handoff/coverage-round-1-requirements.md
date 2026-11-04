R1 A remote handoff (handoff --pane P@M, --new-tab --space S@M, --new-terminal --space @M) is one signed request carrying the prompt, the task message and the profile dir.
R2 The target allocates the seat and writes thread.direct and the message in one transaction before launching; the agent starts with the prompt and the message present.
R3 --summary and --profile-dir work for remote handoffs; the profile dir is projected by the adapter's LaunchPolicy, and an invalid path is an explicit Refused error.
R4 A refused remote handoff authors nothing on any node.
R5 A sender killed right after the request still leaves a tasked agent.
R6 Every node verifies the embedded sender signature from the fact alone (never peer_requests); a tampered sender block is rejected by every node.
R7 The initiator's node owns the ACK deadline and overdue warning, which fires with the target offline.
R8 One prompt renderer (summary, then from: <name> (<id>), no thread id) serves local and remote handoffs as a new prompt version; summary_prompt_v1 stays frozen.
R9 A peer without the capability gets an upgrade error with no two-step fallback; an older third node marks the fact failed and can replay after upgrading (documented best effort).
R10 Bare launch --pane P@M and plain launch keep their gate-only behaviour.
R11 TRUST-POLICY N4, N5 and N15 are amended with the behaviour.
R12 hc targets use the staged Send gate; Herdr targets need no gate.
R13 The wire stays version-tolerant (check-wire-compat), the core stays harness-neutral, and every new model-visible string states its size.
