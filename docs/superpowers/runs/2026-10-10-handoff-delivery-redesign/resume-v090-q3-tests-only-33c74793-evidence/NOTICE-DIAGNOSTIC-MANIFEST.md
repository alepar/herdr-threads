# Completed notice-replay diagnostic: preserved failure

This addendum supersedes only the earlier checkpoint observation that the diagnostic had not yet been copied. The completed package is now frozen byte-exact; the earlier observation remains historical. The active Q3 run was not read or copied.

Candidate HEAD `33c74793a369fad377036bf182b832c8cb05bac7`, tree `0c1922d5432308ce41e15d277211892f4d4ded32`, scoped run ID `b36e4e14-d9e6-4144-b6b9-77e58b4ad1e0`. Selection exit 0. Exactly one test ran: `agy::activation::activation_v2_registered_agy_failed_stdout_replays_without_completion`; 0 passed, 1 failed, 82 skipped. Nextest summary duration 1.694 seconds; command duration 1.9052422499516979 seconds, runtime exit 100. Scoped leak check exit 0.

Recorded failure: actual physical replay contains Invitation and Attention, while expected replay additionally contains Notice. The embedded before/after snapshots both retain the published logical prepared warning (`warning_count: 1`) plus digest and recipient, and both have `canonical.physical: null`. These facts preserve the diagnostic distinction without asserting a production root cause or complete repair. Parent reports the execution was sealed at a clean candidate; this task performed no independent Git attestation.

The existing RUN Q3 channel design is also copied unchanged and verified byte-equal with matching SHA-256. All four WS Q3 reports, persistent fixture review and DirectSend review remain frozen in the initial checkpoint. Historical installation-pending reports are untouched.

All five files in the completed diagnostic package are included, including its small 15,211-byte `list.json`; no huge inventory JSON was copied. Originals are preserved. Destination paths below are relative to this checkpoint directory.

| Source | Destination | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| `/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/execution.json` | `ht-C-v090-notice-replay-diagnostic-33c74793/execution.json` | 1432 | `c9bce543be0ba28ff9db2f77c59bac858538067176b409045fbcb52b16981a9d` |
| `/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/leaks.log` | `ht-C-v090-notice-replay-diagnostic-33c74793/leaks.log` | 25 | `87e8218c7e39ad3f7c328894df8a3c19809b0772116e37ea6d9e45195ffd6a4e` |
| `/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/list.json` | `ht-C-v090-notice-replay-diagnostic-33c74793/list.json` | 15211 | `63a3d6a6aa967b637f6cb051f5065764dba1e3d3ebba86cd29e76fd6c670b228` |
| `/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/list.stderr` | `ht-C-v090-notice-replay-diagnostic-33c74793/list.stderr` | 220 | `58af45ef49ac098ae246a84409a7585fea5c0cdc71365782c88ef372b7b30add` |
| `/private/tmp/ht-C-v090-notice-replay-diagnostic-33c74793/run.log` | `ht-C-v090-notice-replay-diagnostic-33c74793/run.log` | 5819 | `de9d298cc74f0193fabcb32be51ea8b8167f4a6a459ac76dbb16e16c634194b6` |
| `/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-delivery-redesign/docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/resume-v090-q3-channel-design.md` | `resume-v090-q3-channel-design.md` | 35289 | `04b09db182355e44a991088b3055073199b797e32d174d808afcf9c11ab9139c` |

Only new evidence paths were written. No source/index/Git mutation, Cargo/runtime test, Herdr command, subordinate agent or process launch occurred. QUIESCENT.
