decision: patch
summary: patch — independent exact-attempt recovery, honest bootstrap-only cancellation/liveness and atomic linked report-backed completion corrections; no shared redesign.
pattern: none — no recurrence; completed child vs still-live legacy child must stay distinct.
clusters:
- recovery-attempt-identity: every recovery/readyargv carries explicit immutable inspected attempt; exact recorded-decision replay is historical even after later attempts, stale undecided mutations refuse.
- honest-bootstrap-abandonment: bootstrap-only administrative Cancelled tombstone releases only its own protection after canonical no-live-legacy-child proof and quiescence claim; preserve old live child indefinitely and document accepted limit, no fabricated completion/work/receipt/topology cleanup.
- linked-terminal-commit: additive atomic wrapper validates/stores exact retained successful report and commits exact linked child+parent terminal transitions together, preserving old child request/digest/report semantics.
