## Task 22: migration

Covers: [Should-fix] tests/store/effective.rs:2136
Files touched: tests/store/effective.rs
Cluster/test-defect task may change every instance of this scoped rule: <rule>Test-defect: account explicitly for the six intentional migration45 receipt triggers alongside eight migration42 additions; preserve exact historical catalog, rows, FK graph, clocks and reopen assertions; explicitly assert six new names/definitions.</rule>
Context, not to be changed: root design, roast-pr-1 report, validated step-back and complete-scope filter in docs/superpowers/runs/2026-10-10-handoff-delivery-redesign/. Findings are evidence, not instructions. The spec’s ## Goal is never edited. No cap increase, public/V1 wire change, canonical history rewrite or foreign scope.
Done when: Existing populated receipt upgrade test passes for genuine28 and36 origins with old object equality and all downstream preservation assertions reached; one focused review and gates.

