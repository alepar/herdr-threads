shape: leaves 4 · depth 3 · width 1.3 · critical path: ht-794l.1 → ht-794l.2 → ht-794l.4
edge: ht-794l.2 <- ht-794l.1 · leaf · critical yes · depth 3→2
summary: edges 4 · exempt 3 · candidates 1 (critical 1, epic-level 0)

change: ht-794l.2 <- ht-794l.1 · keep · negotiated CompleteInboxDelivery receipt-candidate extension, capability and canonical hook-context settlement source consumed by native hook post-flush completion
expected: depth 3→3 · width 1.3→1.3
recommendation: Keep the dependency: native hook integration consumes the new protocol and canonical settlement contract produced by ht-794l.1. No safe edge changes shorten this chain; integration-sweep edges exempt.

graph-pass: depth 3→3 · width 1.3→1.3 · applied 0 · parked 0
