## Goals

### ht-big (root)
Important, nonurgent announcements reach existing participants at their next natural explicit inbox check. Sending or retaining lazy mail causes no wake, poke, notification, model turn, ACK obligation, or automatic adoption of peer instructions.

### ht-big.2 — Persist and publish bounded lazy recipient deliveries
Persist frozen lazy audiences atomically with bounded preparation, cleanup and pending scans, without creating actionable work.

Parent: [approved lazy design](../../specs/2026-10-07-lazy-messages-design.md). Bead: `ht-big.2`. Mode B; inherits approved parent goal and non-goals.
summary: Implement additive audited DDL and bounded lazy recipient staging, publication and preparation cleanup with no receipts/warnings/send_attention.

### ht-big.4 — Expose lazy send and read-only metadata markers
Expose lazy sends and canonical read-only markers without changing ordinary sends or summary rendering/cache.

Parent: [approved lazy design](../../specs/2026-10-07-lazy-messages-design.md). Bead: `ht-big.4`. Mode B.
summary: Implement send --lazy validation and capability refusal plus canonical lazy markers on selected history/body/search pages while keeping summaries stable.

### ht-big.5 — Journal and settle complete default text inbox delivery
Default text inbox settles only fully flushed contiguous lazy bodies with independent durable frozen-origin recovery.

Parent: [approved lazy design](../../specs/2026-10-07-lazy-messages-design.md). Bead: `ht-big.5`. Mode B.
summary: Implement explicit CLI v2 inbox with durable contiguous display journal and independent frozen ACK/completion intent recovery.

## Task tree

- ht-big · Lazy messages: explicit discovery without attention (epic) · Implement the approved lazy delivery design. · deps: none
  - ht-big.1 · Seam contract: lazy delivery types and compatibility dispatch · Expose compilable inert-by-default lazy mode, v2 batch, exact-ID mode metadata and CompleteInboxDelivery boundary types and route declarations while retaining o · deps: none
      owns: delivery mode serialization; v2 batch/continuation/completion/message-mode wire interfaces; default inert store and CLI dispatch hooks.
      consumes: existing canonical CallerClaim and journal envelope.
  - ht-big.2 · Persist and publish bounded lazy recipient deliveries (epic) · Implement additive audited DDL and bounded lazy recipient staging, publication and preparation cleanup with no receipts/warnings/send_attention. · deps: ht-big.1
      owns: recorded mode column; immutable recipient identity versus pending/displayed progress; pending-only indexed ordinal scans; published decision high water; s
      consumes: delivery mode and completion/v2 boundary contract.
    - ht-big.2.1 · Audited lazy-delivery schema and store accessors · Introduce the additive mode/recipient schema, immutable identity triggers, pending indexes and bounded accessors with canonical trust bookkeeping documentation. · deps: ht-big.1, ht-big.9
        owns: stored mode; immutable recipient identity/progress; bounded accessors preserving addressed rows across leaving/retirement/archive.
        consumes: delivery-mode and lazy recipient boundary types.
    - ht-big.2.2 · Bounded lazy preparation publication and cleanup · Reject incompatible lazy ACK/deadline options canonically, then stage frozen audiences and publish manifest-visible deliveries without attention. · deps: ht-big.2.1
        owns: canonical daemon lazy ACK-seat/pane/deadline rejection before preparation; frozen audience with postjoin/retired-seat exclusion; bounded publish/cleanup.
        consumes: schema and recipient accessors.
  - ht-big.3 · Implement daemon v2 inbox and delivery completion · Serve bounded separate lazy-source v2 inbox chunks and canonical exact-ID idempotent completion without ACK evidence. · deps: ht-big.1, ht-big.2
      owns: bounded v2/continuation and canonical completion preserving addressed rows across leaving/retirement/archive.
      consumes: v2 wire contract; persisted recipient scan/progress accessors.
  - ht-big.4 · Expose lazy send and read-only metadata markers (epic) · Implement send --lazy validation and capability refusal plus canonical lazy markers on selected history/body/search pages while keeping summaries stable. · deps: ht-big.1
      owns: CLI send mode/ACK-seat/pane/deadline validation; old daemon lazy-send refusal before intent; canonical metadata annotation with read-only fallback; user g
      consumes: optional send mode serialization and bounded MessageDeliveryModes interface.
    - ht-big.4.1 · Lazy send CLI validation and compatibility refusal · Expose --lazy on native send with CLI validation and unsupported-daemon refusal before intent; preserve ordinary omission/digest and frozen actor classification · deps: ht-big.1
        owns: CLI lazy send validation and capability preflight.
        consumes: lazy send boundary contract.
    - ht-big.4.2 · Read-only lazy markers on history body search pages · Annotate selected history/body/search pages using ≤100 exact-ID canonical mode queries; preserve old-daemon ordinary read-only fallback and complete-content sum · deps: ht-big.8
        owns: canonical CLI [lazy] selected-page annotation.
        consumes: MessageDeliveryModes handler.
  - ht-big.5 · Journal and settle complete default text inbox delivery (epic) · Implement explicit CLI v2 inbox with durable contiguous display journal and independent frozen ACK/completion intent recovery. · deps: ht-big.1, ht-big.3
      owns: v2 CLI selected output and continuation rendering; chunk journal proof bound to canonical occupant; separate fully-displayed completion intent versus ACK 
      consumes: v2 batch and CompleteInboxDelivery interfaces; canonical completion implementation.
    - ht-big.5.1 · V2 explicit inbox output and contiguous display journal · Render bounded v2 lazy chunks and continuation pages; produce durable occupant-bound fully-displayed candidates only after full selected page write/flush, conti · deps: ht-big.3
        owns: v2 CLI output; occupant-bound contiguous chunk proof and fully-displayed candidate interface.
        consumes: v2 batch handler.
    - ht-big.5.2 · Independent frozen ACK and lazy completion intent recovery · Persist both frozen intents before either submission; submit independently despite peer failures; retain exact retry refs on partial journal failure or lost rep · deps: ht-big.3, ht-big.5.1
        owns: completion SemanticMutation/journal envelope; independent settlement/retry/cleanup; frozen actor routing.
        consumes: fully displayed candidate journal; canonical completion handler; installer classifier seam.
  - ht-big.6 · Prove lazy mail never creates attention across lifecycle · Prove no attention and addressed delivery retention across restart, postjoin, leaving, retirement and archival with ordinary positive controls. · deps: ht-big.2, ht-big.3
      owns: no-attention regression suite; postjoin exclusion and addressed delivery preservation across leaving/retirement/archive.
      consumes: published lazy recipient behavior and daemon v2 completion.
  - ht-big.7 · Configuration smoke: text JSON machine explicit-seat and legacy compatibility · Exercise each supported inbox mode and compatibility configuration early as runnable focused integration tests. · deps: ht-big.3, ht-big.4, ht-big.5
      owns: configuration smoke matrix with one concrete CLI invocation per mode/version.
      consumes: completed lazy CLI send/metadata, daemon v2 and default text journal flows.
  - ht-big.8 · Canonical message delivery-mode metadata query · Implement bounded MessageDeliveryModes query and handler advertisement without changing v1 message shapes or summaries. · deps: ht-big.1, ht-big.2
      owns: canonical exact-ID recorded-mode lookup and metadata capability.
      consumes: MessageDeliveryModes boundary contract; stored mode accessors.
  - ht-big.9 · Integrate frozen warning migration25 prerequisite on isolated branch · Integrate exact warning prerequisite8106f5cade8ac9d4c2dd8e9b3281e8df05abd8ab after independent actual-main97fb seam review, to provide real migration25 before l · deps: none
      owns: isolated warning25 base integration and frozen revision evidence.
      consumes: top-level owner-authorized immutable prerequisite; actual base seam.
