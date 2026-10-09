# Parallelism Pass Prompt Template

Use this template for §Parallelism Pass: once per run, on the settled tree, after the roast loop
and before the design-ready point. It looks at the whole bead graph for cheap ways to shorten
the longest chain: edges that encode narrative order, epic-level gates whose dependents need only
specific leaves, and chains serialized on one shared file. **Model: opus.** Fresh context.

The numbers come from `scripts/graph-shape`; the reviewer judges only the edges. The caller
pastes §Decomposition's "Blocking deps encode genuine blocking" paragraph and its five edge rules
verbatim into `## Edge rules`.

```
Task tool (general-purpose), model: opus:
  description: "parallelism pass: [ROOT_TOPIC]"
  prompt: |
    You are looking at a settled design tree just before it goes to execution. Execution runs
    every ready bead at once and spends one round per bead along a chain of blocking edges, so
    the longest chain sets the run's length. Your job: find the low-hanging ways to shorten it.
    Change edges, not scope: no new work, no re-decomposition.

    You are read-only. Read the dump below and use `bd show` / `bd list` as needed. Edit
    nothing, add or remove no edges, create no beads. Your output is a list of proposed changes
    the caller verifies and applies.

    ## Graph numbers (from scripts/graph-shape)
    [GRAPH_SHAPE_OUTPUT — verbatim: the `shape:` line, one `edge:` line per candidate with the
    depth if only that edge were removed, and the `summary:` line. Seam-contract and
    integration-sweep edges are already excluded; leave them alone.]

    ## Task tree
    [DUMP_PATH — `bd list --label sp:<root-epic-id> --all --json --limit 0`. Each bead's
    description carries its files-touched hint, its `owns:` / `consumes:` boundaries, and a
    `blocked-by <id>: consumes <artifact>` line per blocking edge.]

    ## Edge rules
    [Paste §Decomposition's "Blocking deps encode genuine blocking" paragraph and its five edge
    rules here, verbatim.]

    ## What to look at
    - Every `edge:` line. Is the named artifact real, and does the dependent's work consume it?
      The `blocked-by` line alone does not make an edge real.
    - An epic-level edge: which leaves under the dependent actually consume which leaves of the
      blocker? Narrow the edge to those leaf→leaf edges.
    - An edge aimed at a sub-epic's headline bead when the consumed artifact lands earlier:
      repoint it at the earliest producer.
    - The critical-path beads: consecutive beads that are chained only because they declare the
      same file. That is a proposal (split the file, or a seam contract), not an edge change.

    **Safe** means the caller may apply the change with no human in the loop. Mark a change
    `safe yes` only when, for every wait the change removes, the two beads declare no file in
    common and nothing else in either bead (`owns:` / `consumes:`, acceptance criteria,
    `(needs: <id>)` citations, the description body) references the other's output or
    interface. Otherwise `safe no`. Proposals are never safe.

    `keep` is a normal answer for an edge that carries a real artifact. Say so in one line and
    move on. Report only changes that shorten the chain or remove an epic-level gate.

    ## Output (exactly these lines)

    change: <dependent> <- <blocker> · drop · safe yes|no · <reason>
    change: <dependent> <- <blocker> · narrow · <new-dependent> <- <producer>: <artifact>; … · safe yes|no · <reason>
    change: <dependent> <- <blocker> · repoint · <dependent> <- <producer>: <artifact> · safe yes|no · <reason>
    change: <dependent> <- <blocker> · keep · <the artifact that makes it real>
    proposal: <bead ids> · split-file <path> | seam-contract <boundary> · <one line>
    expected: depth <D>→<D'> · width <W>→<W'>   (if every safe change is applied)
    recommendation: <one or two sentences: which changes matter most, and why>

    One `change:` line per `edge:` line in the numbers above, in the same order; `proposal:`
    lines only when you found one.
```


## Graph numbers
shape: leaves 16 · depth 7 · width 2.3 · critical path: ht-qhz.1 → ht-qhz.2.1 → ht-qhz.2.3 → ht-qhz.4.2 → ht-qhz.4.3 → ht-qhz.9 → ht-qhz.20
edge: ht-qhz.19 <- ht-qhz.4.3 · leaf · critical yes · depth 7→7
edge: ht-qhz.4.3 <- ht-qhz.4.2 · leaf · critical yes · depth 7→6
edge: ht-qhz.4.2 <- ht-qhz.2.3 · leaf · critical yes · depth 7→7
edge: ht-qhz.4.2 <- ht-qhz.2.2 · leaf · critical yes · depth 7→7
edge: ht-qhz.2.3 <- ht-qhz.2.1 · leaf · critical yes · depth 7→7
edge: ht-qhz.2.2 <- ht-qhz.2.1 · leaf · critical yes · depth 7→7
edge: ht-qhz.9 <- ht-qhz.4.3 · leaf · critical yes · depth 7→7
summary: edges 45 · exempt 25 · candidates 7 (critical 7, epic-level 0)

## Task tree
docs/superpowers/runs/2026-10-07-handoff-topology/tree-final.json

## Edge rules (verbatim)
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
