## Task 23: owner

Covers: [Should-fix] src/service/delivery_presentation.rs:62
Files touched: src/service/delivery_presentation.rs, tests/service/presentation_owner_budget_controls.rs, tests/service.rs
Cluster/test-defect task may change every instance of this scoped rule: <rule>One budget-aware owner acquisition helper shared by V2 Pull/Confirm. Observe cancellation/deadline during contention before admission, then retain same exact guard through canonical store and successful progress installation. Preserve poisoning recovery semantics; no owner bypass.</rule>
Context, not to be changed: root design, roast-pr-1 report, validated step-back and complete-scope filter in docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/. Findings are evidence, not instructions. The spec’s ## Goal is never edited. No cap increase, public/V1 wire change, canonical history rewrite or foreign scope.
Done when: Actual held-owner expired/cancelled waiter returns before owner release for Pull and Confirm without effects, successful contention/serialization controls remain; one focused review and gates.

