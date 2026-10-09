### Per-task verdicts

ht-3bi.3.1 / Registry observation and negotiated daemon health — LEAF — One daemon observation/report deliverable: atomic cached registry observations feed the negotiated report and frozen projections; vocabulary, scope, compatibility and failure policy are already specified, and supporting protocol/dispatch edits do not independently constitute another subsystem.

ht-3bi.3.2 / Generic doctor and selected-profile diagnostics — LEAF — One CLI diagnostic consumer with resolved negotiation, fallback, rendering and consent policy; it consumes the native status producer instead of implementing setup/profile resolution. The pending `.5` producer edge is explicitly caller-owned and is not treated as an unexplained omission.

ht-3bi.3.3 / Binary discovery and strategy-driven canary — PROMOTE — The size test fires on distinct local CLI discovery and external canary execution/reporting subsystems: concrete work is (1) adapters discovery command/schema, (2) adapter strategy metadata and Claude/Codex probe companions, (3) shared isolated bounded orchestration/selection, (4) report/version utilities preserving semver versus exact-build stages, and (5) workflow dispatch/scheduling integration. These are implementation deliverables, not five tests or a file-count limit; the discovery/artifact contract can unblock the manifest consumer independently of runner/workflow completion.

ht-3bi.3.4 / Domain runtime manifest and release compatibility — PROMOTE — The size test fires on the runtime manifest lookup/recorder-consumer subsystem and the external manifest-generation/release-replay subsystem: separate work includes validated Rust runtime collection/index, bounded Python writer/retention, release-binary interrogation/fallback, exact indexed artifact/attempt joins, and narrow integration into the completed rich recorder/fetch. Their identity/stage policies are specified, but they remain separable deliverables rather than one leaf solely because their schemas agree.

### Decomposition verdict

ISSUES

- Every supplied child description exchanges producer/consumer boundaries with siblings but lacks the required literal `owns:` and `consumes:` declarations. For example, `.3.3` publishes discovery/probe artifacts consumed by `.3.4`, `.3.4` publishes lookup consumed by `.3.1`, and `.3.1` publishes health wire consumed by `.3.2`. The spec's expanded numbered-leaf prose describes these boundaries, but does not supply the required tokens in the task descriptions. This is a decomposition bookkeeping defect under the skill's explicit boundary rule; record those declarations before treating the split as settled.

### Criteria and scope notes

- No SPLIT verdict fires. Counting blocking dependencies between the four supplied children gives `.3.1 → .3.2`, `.3.3 → .3.4`, and `.3.4 → .3.1`: each producer has exactly one child dependent. The narrower SPLIT criterion requires at least two. A producer-heavy task may still merit revised decomposition following a PROMOTE verdict, but that does not make the current task a SPLIT by this test.
- Completeness/correctness/nonduplication otherwise pass: the four children represent observation/health, doctor, discovery/canary and manifest/release work; frozen strict messages, honest unsupported/inconclusive outcomes and exact-build isolation agree with the ancestor goal. The supplied hooks/evidence and recording sibling specs own the rich evidence model/store/handler/client gates, while this subtree owns health and the later manifest lookup integration, so that integration is not duplicate recording ownership.
- The explicit later attachment of the `.5` status producer edge is accepted as caller-managed sequencing. Health and manifest consume the model/store transitively through the named recording/discovery producers. The recorder-first, manifest-integration-second direction is preserved; there is no requirement to create a reverse edge from recording to manifest.
- This review read only the named root/health specs, supplied child dump, promotion criteria/template, and the two caller-authorized already-designed hooks/evidence sibling specs. It performed no repository/tracker exploration or project edits.
