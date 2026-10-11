
    0. **Requirement mapping (root pass only).** For each canonical requirement, list the
       task ids that deliver it. You may propose a new requirement the canonical list
       missed as `R-new: <text>` — the orchestrator appends it to next round's list; never
       renumber or reuse an existing id yourself. Every canonical requirement with no
       mapped task is **also** reported as a `GAP` in the findings list below (check 1
       still applies as before; this makes the omission explicit rather than folding it
       silently into the forward trace). Emit this mapping as the `requirements` output
       block, **before** the findings list.

    1. **Forward trace → GAP.** Decompose the goal into its necessary elements. Every
       element must map to at least one task or spec section. An unmapped element is a
       `GAP`.
    2. **Backward trace → ORPHAN.** Every task must serve some goal element. A task that
       serves none is an `ORPHAN`. Tasks titled `Seam contract:`, `Seam integration:`, or
       `Integration sweep:` are seam machinery created by coverage disposition, not decomposition — they
       serve the boundary or the tree they name and are never `ORPHAN`s.
    3. **Walking skeleton.** Does some subset of tasks form a thin end-to-end slice of the
       goal — literally "playable," not "all parts exist but nothing connects them"? Use
       a premortem framing to test it: the tree was fully executed and the goal still
       wasn't met — why?
    4. **Seam trace → UNOWNED-SEAM.** For every pair of tasks that exchange a named piece
       of data or an interface — one defines a config value the other reads, one exposes a
       function the other calls, one writes a format the other parses — check that exactly
       one task's description **owns** the boundary (its "owns:" line names it; the
       counterpart "consumes:" it). If no task owns the wiring between them, that is an
       `UNOWNED-SEAM`: name both task ids and the exchanged thing concretely, and quote
       the description text that implies the exchange. The canonical miss this check
       exists for: a config parameter implemented in a schema task and honored in a
       consumer task, with the pass-through wiring owned by neither — both tasks satisfy
       their local descriptions and the parameter ships inert. Disjoint files are not
       evidence of independence; the exchange is dataflow, not file overlap. Do not
       report a seam whose boundary a task plainly owns, and do not invent exchanges the
       descriptions don't imply.
    5. **Edge audit → NARRATIVE-EDGE.** For every blocking dep in the task tree, first read the
       dependent's description for its `blocked-by <blocker-id>: consumes <artifact>` line
       matching this edge's blocker id. No such line — or a line whose `<artifact>` cannot be
       resolved to something the blocker actually delivers (its own description, acceptance, or
       fixed token: `all leaves (integration sweep)` for a sweep edge, `boundary contract` for a
       seam-contract edge) — is a finding on its own: report it with `unstated` in the evidence,
       naming the edge (dependent ← blocker) and the missing or unresolvable line. When the line
       is present and resolves, use it (plus the two descriptions) to name the specific artifact
       the dependent consumes and the blocker produces, and check for three further failures, one
       kind: (a) UNNAMEABLE — no artifact connects them; the edge encodes narration or "that area
       first". (b) MISDIRECTED — the artifact is real but a DIFFERENT task (often earlier in
       the same sub-epic) produces it; the edge gates the dependent behind work it never
       consumes; propose repointing at the actual producer. (c) DUPLICATED — a sibling
       already owns the integration with that sub-epic and carries the same edge; propose
       consuming the owning sibling's artifact instead. If the edge's only justification is
       an UNANSWERED design question (neither description decides who owns the exchanged
       data), report it as `UNOWNED-SEAM` instead — the boundary needs a contract, not an
       ordering. Do not flag edges onto `Seam contract:` beads (that is the seam mechanism
       working), and do not flag an edge whose artifact you can name and locate at its
       producer — a real edge is not a finding.
       Audit edges on `(epic)` nodes with EXTRA scrutiny: an epic→epic edge claims every
       leaf under the dependent epic needs all of the blocking epic finished. That claim is
       rarely true and its cost is the whole subtree's idle time — if the dependents' real
       needs are specific leaves of the blocking epic, the finding is MISDIRECTED with the
       proposed fix "narrow to leaf-level edges: <dependent leaf> → <specific producer
       leaf>, and drop the epic-level edge". An epic edge you can fully justify (every leaf
       genuinely consumes the whole epic's output) is not a finding, but say so explicitly
       in one line rather than skipping it.
    6. **Acceptance satisfiability → UNSATISFIABLE-ACCEPTANCE.** Two passes, the
       mechanical one FIRST. (a) MECHANICAL — for every `(needs: <id>)` citation in a
       task's acceptance text, look the cited id up in the task tree and walk the blocking
       edges: the cited bead must be a transitive BLOCKER of the task (it completes first).
       If it is a transitive DEPENDENT of the task, that is a finding, whatever the prose
       says — the criterion cannot be met at completion time. If no edge path connects the
       two in either direction, that is a finding of the `unwired` variant — propose the
       edge. This pass is a graph walk, not a judgment: do it for every citation, and report
       each result. (b) PROSE — where an acceptance criterion names another deliverable
       WITHOUT a citation ("the X validator accepts...", "passes the Y suite", "conforms to
       §N's contract"), resolve that deliverable to the task that produces it yourself and
       apply the same direction test; a producing task that is a transitive dependent is
       unsatisfiable — the cycle lives across the acceptance-text/dependency-graph boundary,
       which no single-artifact check spans. Propose either restating the criterion in
       terms of an artifact that exists when the task completes (adding the `(needs: <id>)`
       citation so the next round checks it mechanically), or re-pointing the edge so the
       validator genuinely precedes it. A criterion referencing a blocker is correct and is
       not a finding. (Measured: six prose-only review passes missed one instance of this
       class; the citation convention exists so that pass (a) cannot.)
    7. **Configuration coverage → UNEXERCISED-CONFIGURATION.** Where the spec (root or
       subepic) ENUMERATES runtime configurations — modes, player counts, platforms,
       feature-flag combinations, a test matrix — check that some task in the tree
       *exercises* each enumerated configuration (runs it end to end: one decision, one
       request, one invocation — not merely reads or unit-tests the code behind it), and
       that this task is not solely the terminal readiness/integration gate. An enumerated
       configuration that nothing runs before the terminal gate is a finding: evidence is
       the quoted enumeration and the configurations with no exercising task; the proposed
       fix is a `Configuration smoke:` leaf (name the entry point or command per
       configuration if the spec gives one) depending on the beads that make those
       configurations runnable, or naming the one existing bead that should absorb the
       exercise. A spec that enumerates nothing yields no finding of this kind — do not
       invent a configuration space.
    8. **Flag sweep.** Every id in "Flagged tasks" is an automatic finding — known-
       underdesigned work must not sail through silently — *unless* it already has an
       entry in the rejected-findings ledger, which takes precedence over the sweep.
    9. **Previous round's fixes.** Every task named in "Changes since the previous round"
       was created or amended by this run's own coverage loop and has been read by no one.
       Run checks 1-7 over them as over any other task, and answer the two questions only
       these raise: does the fix actually close the finding it claims to (if not, re-report
       the original finding, with the shortfall as evidence), and did the fix introduce a
       seam, edge, or acceptance criterion of its own that checks 4-6 would flag? An empty
       section means this is round 1 — skip this check; its emptiness is NOT an
       INSUFFICIENT-INPUT.

    ## Required structured output (do NOT write a prose essay)

    **Root pass only:** first, a `requirements` block mapping every canonical id to the
    task ids that deliver it (`R-new: <text>` entries appended for any you propose) —
    this comes **before** the findings list, one line per requirement:

    ```
    requirements:
    R1 → bd-101, bd-104
    R2 → (unmapped)
    R-new: <text> → bd-107
    ```

    Then, one entry per finding:
    - **type:** `GAP` | `ORPHAN` | `UNOWNED-SEAM` | `NARRATIVE-EDGE` | `UNSATISFIABLE-ACCEPTANCE` (with `dependent` or `unwired` in the description) | `UNEXERCISED-CONFIGURATION` | flag-sweep
    - **description:** the problem, one sentence
    - **evidence:** the unmapped goal element, the orphaned task id, the seam's two
      participant task ids + the exchanged data/interface + the quoted description text
      implying the exchange, the edge (dependent ← blocker) + which of unstated/unnameable/
      misdirected/duplicated it is + the quoted description text (or the absence of a
      `blocked-by` line, for `unstated`), the quoted acceptance
      criterion + the producing (or cited) task id + the dependency path proving it is a
      transitive dependent (or the absence of any path, for `unwired`), the quoted
      configuration enumeration + the configurations with no exercising task, or the flag
      and its task id
    - **proposed fix:** a new leaf task under a named epic, or a new subepic needing
      design; for `UNOWNED-SEAM`, name the boundary to be contracted (the caller turns a
      verified seam into a contract bead + integration bead — see SKILL.md §Coverage); for
      `NARRATIVE-EDGE`, drop the edge, or name the task it should repoint to; for
      `UNSATISFIABLE-ACCEPTANCE`, the restated criterion (with its `(needs: <id>)`
      citation), the edge to re-point, or the edge to add; for `UNEXERCISED-CONFIGURATION`,
      the `Configuration smoke:` leaf (per-configuration entry point, and the beads it
      depends on) or the existing bead to extend

    Report only defensible findings. Quality over quantity — but do not soften.
```
