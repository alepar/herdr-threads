Read-only fresh graph reviewer; inspect only named input files. Review every candidate edge using the graph-pass-prompt.md template at /Users/alepar/.codex/plugins/cache/superpowers-alepar/superpowers/6.4.2-alepar6.2/skills/super-design/graph-pass-prompt.md. No writes, no tracker mutation. Output one change per candidate plus expected and recommendation.
## Graph numbers
shape: leaves 10 · depth 5 · width 2.0 · critical path: ht-1urq.1 → ht-1urq.3 → ht-1urq.6 → ht-1urq.9 → ht-1urq.10
edge: ht-1urq.9 <- ht-1urq.6 · leaf · critical yes · depth 5→4
edge: ht-1urq.6 <- ht-1urq.3 · leaf · critical yes · depth 5→5
edge: ht-1urq.6 <- ht-1urq.5 · leaf · critical yes · depth 5→5
edge: ht-1urq.6 <- ht-1urq.4 · leaf · critical yes · depth 5→5
edge: ht-1urq.6 <- ht-1urq.2 · leaf · critical yes · depth 5→5
summary: edges 19 · exempt 13 · candidates 5 (critical 5, epic-level 0)

## Task tree
/Users/alepar/AleCode/herdr-threads/.worktrees/handoff-profile-and-skill-guidance/docs/superpowers/runs/2026-10-17-handoff-profile-and-skill-guidance/design-tree.json
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

