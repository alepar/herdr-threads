# Final review: cluster 8 (integrations-skill), epic ht-z41e

Source: base 4e4f628a42a03e2b523b1bd1a49f3f7cad8e71e3, head c3e9ab0becd137adb68f23a1c9e1a87669d03168 (both verified with `git rev-parse --verify`). The caller checkout was observed at HEAD c3e9ab0be with a clean status during the review. Cluster input: `integrations/skill/SKILL.md`, slice `slices/000060.diff`. Spec: `docs/superpowers/runs/2026-11-26-per-seat-config-side-wake/2026-11-26-per-seat-config-side-wake-design.md` (it has a Requirement baseline, B1-B10 plus S1).

## Scope of the change

Two sentences, both added to existing lines, so the line count stays under the 220-line bound in `tests/cli/skill.rs`:

1. Commit 1e41efd56, "Priority and waking" bullet: `**Per-seat settings:** herdr-threads config seat list shows them; get [KEY] reads; set KEY VALUE sets an agent-tier key on your own seat; human-tier keys (programmatic_wake) only via herdr-threads human config seat set, which asks the person.`
2. Commit 64d684da7, "Codex /side" bullet: `While a side chat is displayed, a native wake waits until it closes (it is never typed into the side chat); a sender's overdue line shows the refused wake (wake=).`

`SKILL_MD` is compiled in with `include_str!` (`src/cli/skill.rs:9`). The same text goes to Claude and Codex and to the agy, opencode and Pi skill copies (`src/harness/agy/plugin.rs:50`, `src/harness/opencode/setup.rs:236`, `src/harness/pi/setup.rs:96`), so whatever this sentence says applies to every harness.

## How each claim was checked against the code

- CLI spelling: `Top::Config { ConfigSub::Seat { List | Get{key?, --seat} | Set{key, value, --seat} | Unset{key, --seat} } }` in `src/cli/commands.rs` matches `config seat list|get|set`. The agent path refuses `--seat` (`CONFIG_SEAT_FLAG_REFUSAL`) and refuses a human-tier key at parse time with the `ht human` spelling (`refuse_agent_config_write`). That matches "set KEY VALUE sets an agent-tier key on your own seat". `config_seat_need` makes a bare `human config seat set` fall back to the located pane seat, so an agent told to set the key needs no `--seat`.
- Registry: `src/protocol/seat_config.rs` holds exactly `nudge_threshold` (agent tier) and `programmatic_wake` (human tier). The parenthetical `(programmatic_wake)` is accurate.
- "which asks the person": Claude and Codex ask on `human config seat set|unset` (pinned by tests in `src/harness/claude/permission_rules.rs` and `src/harness/codex/permission_rules.rs`). TRUST-POLICY's ask-rule table (head, lines 3339-3346) says agy, opencode, Hermes and Pi have no ask rule on `human`. The commit message of 1e41efd56 records the same and notes that the one `SKILL_MD` source feeds the agy, opencode and Pi copies. So the claim is false on four of the six harnesses that receive the skill.
- /side sentence: in `src/harness/codex/composer.rs`, `Ask a follow-up question` is no longer in `CODEX_PLACEHOLDERS`, so a displayed side composer reads non-empty and native admission refuses before typing. This matches spec §3 and the Codex README paragraph at `integrations/codex/README.md:276-280`. The qualifier "a native wake" is correct: with `programmatic_wake on`, the queue wakes the root while the side chat is displayed (spec §2), and the sentence makes no claim about that case. The `wake=` overdue text is the pre-existing refused-wake reason that `side-draft` already relies on (AGENTS.md).
- Token rule (f): commit 1e41efd56 gives about 253 bytes for its sentence and 64d684da7 gives 165 bytes. Both stay within the once-per-session skill budget. No per-message text is added.

## How the baseline rows map to this cluster

- B5 ("telling an agent in prose to set this setting works"): the skill is where an agent learns the spellings. Both tiers are named, and `list` (needs no daemon) gives the keys. This is delivered for this cluster, apart from the consent wording below.
- B6 / B7 (all keys readable by agents; nudge threshold writable by agents; some keys only through the human path): the sentence states the split correctly. On harnesses with no ask rule, though, the skill is the only cooperative guard that remains, and it tells the agent the harness will ask the person. An agent on agy, opencode, Hermes or Pi then has no reason to wait for its user before running `human config seat set`. The human-only tier in B6 then depends on a guard the skill says exists and that is not there. The fix is wording only and needs no wider grant.
- B9 (no native wake typed into a side chat): the skill sentence matches the implementation.
- The production paths behind these claims (CLI parse, daemon tier enforcement, composer gate) belong to other clusters. They were opened here only to check the skill's wording.

## Findings

- SKILL.md "Priority and waking" says `herdr-threads human config seat set` "asks the person". The one `SKILL_MD` source also goes to agy, opencode and Pi, and TRUST-POLICY says agy, opencode, Hermes and Pi have no ask rule on `human`. On those harnesses the command runs without any prompt, and the skill gives the agent no instruction to wait for its user's request. Suggested wording: "run it only when your user asks for it (Claude and Codex prompt the person)". This keeps the cooperative guard and changes no permissions.
- SKILL.md names `get` and `set` but not `unset` (`config seat unset KEY` / `human config seat unset`). An agent told to put a key back to its default has to discover `unset` from `--help` or from the `list` trailer, and the trailer names only `set`. Minor; a few bytes would cover it.
- No test in `tests/cli/skill.rs` pins either new sentence (that file is unchanged in the range). Every other load-bearing skill rule has a needle test, so a later edit could drop the per-seat settings spelling or the /side deferral rule without any test failing.

## Verdict
not ready (SKILL.md tells agents on every harness that `human config seat set` asks the person; agy, opencode, Hermes and Pi have no ask rule, which weakens the B6 human-tier guard there)

## Must fix before landing
- B6: integrations/skill/SKILL.md "Priority and waking" says `herdr-threads human config seat set` "asks the person", but the same SKILL_MD reaches agy, opencode and Pi (and Hermes has no ask rule either), and TRUST-POLICY lines 3339-3346 plus the commit message of 1e41efd56 record no ask rule on `human` for agy, opencode, Hermes and Pi; reword to tell the agent to run it only when its user asks (Claude and Codex also prompt).

## Untested scope
- tests/cli/skill.rs has no needle for the new per-seat settings sentence (`config seat list`, `human config seat set`) or for the /side native-wake deferral sentence, so either can regress unnoticed.

## Deferred OK
- SKILL.md omits `config seat unset` / `human config seat unset` from the per-seat settings sentence; `--help` and the docs cover it.
