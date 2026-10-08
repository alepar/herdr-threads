# Independent review

Reviewer: /root/input_safety_review, read-only source/design review; no tests executed by reviewer.

Initial actionable finding: an intervening nonempty/unreadable composer observation did not reset the ordinary empty window. Corrected in read_composer and observe_target, with causal RED then GREEN. A status-only idle observation returns Unknown and must not reset the window; the causal regression and correction preserve accumulation on the actual ordinary path.

Policy clarified: PokeOnly keeps unconditional focus exclusion; ordinary attention alone uses the focused one-minute rule. Removed obsolete extra-Enter/stash claims. Final production/policy rereview: no remaining blocker.

Fixture follow-up: no blockers. Sweep exposes actual scripted pane focus and captured recognized-empty Claude/Codex composers; unknown harness remains unreadable. SlowHost is Claude-only and delays agent.read through its existing host-delay function. No policy weakening or production change from fixture updates.
