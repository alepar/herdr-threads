# Independent review

Reviewer: native subagent `/root/join_review`, read-only, precise feature brief and
actual-main baseline supplied, without parent conversation history.

Initial finding: P2 — `src/cli/irc.rs` only recognized accept/accept_required/leave,
so public join would disappear from human read/follow. Reproduced with a failing
consumer-output test; the whitelist and joined-text match now include join.

Advisory gaps were covered: historical receipts across before/joined/left/rejoined,
sealed-send snapshot invalidation, rejected and cancelled required episodes,
required accepted state and leave guard, human claim provenance, CLI capability
refusal without new intent (and retained retry intent), real-store response loss
through the journal, historical replay after leave and restart.

Final verdict: Ready from independent code review; no remaining important
correctness issues. Canonical authority, required consent, fresh membership
intervals and immutable replay are consistent with the existing safeguards.

Limits: reviewer inspected code and tests, without independently running tests,
clippy/default checks or live integration. Response-loss testing uses journal/retry
against the real store, not the complete socket transport.
