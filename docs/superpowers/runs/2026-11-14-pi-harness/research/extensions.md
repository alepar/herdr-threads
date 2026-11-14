# Extension delivery and wake

These findings describe retrieved upstream source and documentation. Proposed integration decisions are distinguished below from upstream guarantees.

<!-- facet: F1 -->
Extensions are discovered globally under the pi agent extensions directory and in trusted project-local .pi/extensions directories. [1] <!-- claim: 4fa5988ac40166ea; evidence: 75f1329a58b7abc7; source: 458882967aaf8636 -->

<!-- facet: F1 -->
Session replacement shuts down the old extension instance and starts a new one; start reasons distinguish startup, reload, new, resume and fork. [1] <!-- claim: 5c6aa01d5870ed7c; evidence: a2e50e9bae70efff; source: 458882967aaf8636 --> [3] <!-- claim: 5c6aa01d5870ed7c; evidence: 42bb598f3c044834; source: c155a99b7cd580d6 -->

<!-- facet: F3 -->
The extension sendMessage API returns void, with steer, followUp and nextTurn delivery options. [3] <!-- claim: 07f8977db1b6819b; evidence: 86ad88fb37b6d3d4; source: c155a99b7cd580d6 -->

<!-- facet: F3 -->
The implementation handles nextTurn first. Otherwise, when streaming and triggerTurn is not false, it selects followUp if requested or steer otherwise. With no streaming run, triggerTurn true can start one. [2] <!-- claim: 9ff9624a4d92c22d; evidence: 316b625ab33adf12; source: acc3443733afabb8 -->

<!-- facet: F3 -->
Outside nextTurn delivery, triggerTurn false queues a streaming message for a later append and uses the immediate append path when not streaming. [2] <!-- claim: 95f2623ef2624057; evidence: 316b625ab33adf12; source: acc3443733afabb8 -->

<!-- facet: F3 -->
agent_end is not final idle; agent_settled is the documented final-idle signal. [1] <!-- claim: 64aa49262ee29d4c; evidence: 2227af8cd1e2361b; source: 458882967aaf8636 -->

<!-- facet: F3 -->
The implementation distinguishes active-run state from idle: idle also excludes compaction. [2] <!-- claim: 97765e86902d54fd; evidence: 1ac99f294e6317a4; source: acc3443733afabb8 -->

<!-- facet: F3 -->
Extension message callbacks precede custom-message entry insertion; the agent context transformation precedes conversion to provider messages. [2] <!-- claim: f77822bbad775d44; evidence: 823d604fbe3e18ed; source: acc3443733afabb8 --> [8] <!-- claim: f77822bbad775d44; evidence: 0a7d35769748f403; source: c8186231332ca614 -->

<!-- facet: F3 -->
Session entry persistence can be deferred until conversation content exists. [7] <!-- claim: f0eb1c5f9fd0a72a; evidence: e855ebcc5f1bfba1; source: 81c8aeb92fa5effb -->

## Design implications and counterevidence

Use an in-process channel, with a content-free idle wake and in-turn steering. This is a design inference. A void return, a queued body or a message-end callback cannot establish the proposed receipt predicate. Require an exact current-session/current-branch frame in observed model-bound context. Later transformations and provider failure remain outside that observation.

The [credential-free real-SDK probe](candidates/probe/results.md) reproduces busy append missing the current run, a stale busy snapshot starting a new run, and message-end preceding entry insertion. Its scripted provider is not live-model or TUI qualification. A source-level isIdle check alone is insufficient during manual compaction; the implementation needs positive active-run evidence and a synchronous final check.

Historical upstream wake-preparation bugs motivated the counterevidence search. Current source prepares custom runs, but this research does not promise that every installed older release has the same behavior. Capability failures must degrade the affected feature without an exact-version gate.

## Limits

Global modal-blocked state and actual TUI draft/modal behavior are not empirically qualified. Session-tree presence is not disk flush or model consumption. Observe and test shutdown/reload fencing and downstream context transformation before declaring support complete.

## Boundary-based revision

<!-- facet: F5 -->
The documented turn_end boundary runs after assistant and tool-result persistence. [1] <!-- claim: 4a26ad9d199c339d; evidence: ff9eee237a7fa5a2; source: 458882967aaf8636 -->

The documented boundary continue option ensures one next provider request. [1] <!-- claim: fe7ba0854c428efe; evidence: a5b045a2cd64e867; source: 458882967aaf8636 -->

The first design review found queue survival across reload, cross-batch FIFO ordering, and capacity gaps. Revised recommendation: keep bodies in a bounded extension buffer and return a sorted custom-message draft at before_agent_start, turn_end or agent_before_settle; use sendMessage only for the content-free idle wake. This is a design inference, not a universal Pi guarantee.

The extended [real-SDK probe](candidates/probe/results.md) asserts nine boundary cases. Completed-run draft continuation reaches the next provider request within one outer run. The tested specimen unexpectedly permits a pre-settle proposal to continue after synthetic error/aborted responses, despite the documentation’s hard-exit wording. Guard explicitly on outcome equal to completed. Actual cancellation can still commit a draft without continuation; no ACK follows without later exact context evidence. TUI, actual provider and lifecycle race qualification remain open.

## Bibliography

[1] [extensions.md — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/extensions.md)
[3] [types.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/extensions/types.ts)
[2] [agent-session.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/agent-session.ts)
[8] [agent-loop.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/agent/src/agent-loop.ts)
[7] [session-manager.ts — upstream pi](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/src/core/session-manager.ts)
