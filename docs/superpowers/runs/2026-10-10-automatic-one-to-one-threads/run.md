# super-auto run — 2026-10-10-automatic-one-to-one-threads

flags: planOneShot=true skipPlanRoast=false skipCodeRoast=false autonomous=true
phase: design

idea: Use $super-auto, fully autonomous end to end with both roasts on, to implement this feature in this repo: automatic 1:1 threads. An agent can send a message to another agent by giving only a target seat ID, without specifying a thread. The plugin automatically creates (or reuses an existing) "1:1" thread with exactly two participants, the sender and the recipient, which otherwise behaves like a normal thread. The handoff command gets the same treatment: if no thread is specified, the handoff goes through the 1:1 thread. When the integration branch is ready for merge, run `herdr-threads join dev-herdr-threads` (thread taQXwBH7X) and send a --nudge message there stating that this feature is ready for integration into main.
branch: super-auto/automatic-one-to-one-threads
base: main
spec: 2026-10-10-automatic-one-to-one-threads-design.md
epic: ht-e99

approvals:
- top-split · auto under explicit fully-autonomous delegation · ht-e99.1 PROMOTE, ht-e99.2 LEAF, ht-e99.3 PROMOTE, ht-e99.4 LEAF
