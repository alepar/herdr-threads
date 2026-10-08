## Graph numbers
shape: leaves 18 · depth 10 · width 1.8 · critical path: ht-uwd.1 → ht-uwd.4 → ht-uwd.5.2 → ht-uwd.5.3.1 → ht-uwd.5.3.2 → ht-uwd.7.1 → ht-uwd.7.2 → ht-uwd.8.1 → ht-uwd.8.2 → ht-uwd.12
edge: ht-uwd.8.2 <- ht-uwd.8.1 · leaf · critical yes · depth 10→9
edge: ht-uwd.8.2 <- ht-uwd.7 · epic · critical no · depth 10→10
edge: ht-uwd.8.1 <- ht-uwd.7 · epic · critical yes · depth 10→10
edge: ht-uwd.7.2 <- ht-uwd.7.1 · leaf · critical yes · depth 10→9
edge: ht-uwd.7.1 <- ht-uwd.5 · epic · critical yes · depth 10→10
edge: ht-uwd.5.3.2 <- ht-uwd.5.3.1 · leaf · critical yes · depth 10→9
edge: ht-uwd.5.3.1 <- ht-uwd.5.1 · leaf · critical yes · depth 10→10
edge: ht-uwd.5.3.1 <- ht-uwd.5.2 · leaf · critical yes · depth 10→10
edge: ht-uwd.5.3 <- ht-uwd.5.2 · epic · critical yes · depth 10→10
edge: ht-uwd.5.3 <- ht-uwd.5.1 · epic · critical yes · depth 10→10
edge: ht-uwd.9 <- ht-uwd.3 · epic · critical no · depth 10→10
edge: ht-uwd.8 <- ht-uwd.7 · epic · critical yes · depth 10→10
edge: ht-uwd.7 <- ht-uwd.6 · epic · critical no · depth 10→10
edge: ht-uwd.7 <- ht-uwd.5 · epic · critical yes · depth 10→10
edge: ht-uwd.4 <- ht-uwd.1 · leaf · critical yes · depth 10→9
edge: ht-uwd.3 <- ht-uwd.1 · epic · critical no · depth 10→10
summary: edges 52 · exempt 25 · candidates 16 (critical 12, epic-level 10)

## Task tree
/Users/alepar/AleCode/herdr-threads/.worktrees/installer-permissions/docs/superpowers/runs/2026-10-07-installer-human-permissions/tree.json
## Edge rules
**Blocking deps encode genuine blocking, not narrative order.** Add an edge only when the dependent
task literally cannot start until the other finishes — its interface, schema, or file must exist
first. Do not encode the order you happened to describe things in: a decomposer narrating a build
order writes a linear chain by reflex, and a chain of N tasks is N sequential execution rounds no
matter how disjoint their files are. Execution dispatches every ready task at once, so each
unnecessary edge is a round of parallelism deleted at design time, invisibly — `super-code` cannot
tell a decorative edge from a real one. When in doubt, leave it out: execution's serial merge gate
and post-rebase test run catch a genuine conflict at the cost of one rework cycle, while a
decorative edge costs a full round on every run.

Five edge rules, each the generalization of a measured live failure (the parallelism pass
audits them — §Parallelism Pass):

- **Name the consumed artifact or drop the edge.** Every edge is justified by a specific artifact
  — an interface, schema, file, or recorded decision — that the dependent consumes and the blocker
  lands. "Depends on that area being done" is narrative. If you cannot name the artifact AND the
  bead that produces it, there is no edge.
- **Point the edge at the artifact's earliest producer** — never at the other sub-epic's most
  prominent bead. A cross-sub-epic edge aimed at the headline bead when the needed artifact lands
  three beads earlier gates everything behind work the dependent never consumes.
- **One sibling owns each cross-sub-epic integration** (the same ownership principle as seam
  contracts). If a sibling already carries the edge into that sub-epic and owns integrating with
  it, a second edge from another sibling is usually duplicated scaffolding — consume the owning
  sibling's artifact instead.
- **An edge you justify with a question instead of an artifact is a seam, not a dependency.**
  Chaining two tasks because nobody has decided which of them owns a piece of data does not answer
  the question — it hands the decision, implicitly, to whichever implementer runs first. Decide it
  in the spec, or extract it as a seam contract (§Coverage's `UNOWNED-SEAM` machinery) and let
  both tasks run in parallel against the decided boundary.
- **Prefer leaf-level edges over epic-level edges.** An epic→epic edge claims every leaf under
  the dependent needs ALL of the blocking epic — and it is invisible to `bd ready` readers of the
  leaf graph, so nothing downstream re-audits it (measured live: one such edge idled a leaf whose
  own deps were satisfied for 11.4 days, in an epic holding two leaves with no deps at all).
  Write the specific leaf→leaf edges the artifacts justify; reserve an epic-level edge for the
  rare case where every leaf genuinely consumes the whole predecessor epic, and say why in the
  dependent epic's description.

