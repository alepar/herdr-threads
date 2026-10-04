# Combined CLI preview — awaiting design approval

Baseline: v0.2.1, main `4b026b382b063e0795bc27a4befca94c988eeb80`.
This is an audit and proposed interface, not implemented behavior.

## Audit

- `src/cli/panes.rs` already resolves pane labels and single-pane tab labels for
  `seat resolve`, `seat rebind` (including replacement), and `launch`. It searches
  the entire host snapshot, reports ambiguity as InvalidRequest, and silently
  passes unresolved names through if the host read fails. There are no parent
  selectors. `--cooperative-target` is another pane locator, currently unprocessed.
- `invite` accepts only `--seat`; `send --require-ack` takes durable seat IDs.
  Recipient selection must stay separate from caller selection.
- Ordinary resolution can allocate an unclaimed seat, but the daemon enforces
  restoration holds and seat ownership. Labels do not provide continuity evidence.
- Threads have topic and goal, but no separate name. The directory walks creation
  ordinals, not recent activity. Existing HotThreads is a bounded attention view,
  not an inventory suitable for the requested picker.
- Thread selectors occur in thread show/topic/participants, the participants alias,
  invite, accept/accept-required, leave, send, archive/reopen, read/follow,
  pending-receipts --thread, diagnostics --thread, warnings --active,
  search --thread, and summary THREAD. All need the same name rule.
- `launch -- ...` currently preserves native argv. It gates versions/hooks, resolves
  the target seat, checks the bound-agent guard, and uses Herdr's guarded start.
  Correlated startup can record managed_launch; it never accepts or ACKs. Possible
  startup is reported as outcome_unknown, exit 5. Invitations/messages survive it.
- Create joins the sender; invite requires a joined sender except explicit orphan
  recovery. Send can address an invited seat explicitly through require-ack.
  Each mutation has durable exact-key recovery, but no compound handoff journal.
- Public usage and embedded skill describe the manual prelaunch flow. Some prose
  predates human receipt waivers/display ACKs; touched guidance must agree with
  TRUST-POLICY.md and current behavior.

## Proposed target selection

Use command-local `--space SPACE --tab TAB --pane PANE`. Selectors accept IDs or
exact, case-sensitive labels; do not trim, case-fold, or fuzzy-match execution
targets. Space means Herdr workspace. No implicit search across spaces or tabs.

1. An explicit pane ID keeps its direct meaning. Explicit parent qualifiers must
   agree with its live location; a mismatch fails instead of retargeting it.
2. For a name, omitted space/tab come from the caller's live pane in the selected
   Herdr instance. Inherited IDs are hints: resolve the live caller location, not
   UI focus. Outside Herdr, pass parents or a pane ID.
3. An explicit tab ID identifies its space. An explicit space changes the scope
   for tab lookup. Do not carry the caller's child into a different parent: an
   omitted child under a different parent selects its sole child, otherwise
   Conflict with candidates. For tabs, sole child means one live pane; for spaces,
   it means one live tab, then the same pane rule.
4. With all selectors omitted, ordinary resolution and recipient/read selection
   use the caller's own pane. Omitted read selectors use `HERDR_PANE_ID` and the
   daemon's canonical mapping without requiring a live Herdr read (TRUST-POLICY A1/C4).
   Explicit names or parents still use live locator scope. `launch`, `handoff`,
   `seat rebind`, and fresh-seat repair require an explicit pane selector: these actions need target intent.
   `me init` remains exclusively the caller's own identity declaration.
5. Pane names match the union of exact pane labels and exact live agent names
   inside the selected tab, deduplicated by pane ID. If they name different panes,
   Conflict; neither silently shadows the other. Retain the single-pane tab-label
   alias only within the selected scope; recommend explicit `--tab LABEL` instead.
6. Missing name is NotFound. Multiple matches are Conflict (existing exit 1),
   showing escaped space/tab/pane labels, exact IDs, match kind and qualification
   examples. A failed Herdr read stays HostUnavailable; never pass a name as an ID.
7. Resolve once, freeze canonical IDs before journal submission, and never resolve
   names again on retry. Rename does not retarget that operation. Move/disappearance
   or incarnation/seat change is handled by existing canonical daemon/launch fences;
   no automatic chase, seat transfer, heuristic binding change or fabricated claim.

```sh
# Before
ht seat resolve --pane w4:p7               # obtain SEAT
ht invite tZVblxlC8 --seat SEAT
# After (caller space; explicit tab)
ht invite tZVblxlC8 --tab tryout --pane alice
# Caller space and tab
ht invite tZVblxlC8 --pane alice
ht seat resolve                           # caller's own pane
ht seat resolve --space project --tab tryout --pane alice
ht launch --tab tryout --pane bob --kind codex -- -p work
ht seat rebind OLD --tab tryout --pane bob --replace NEW --operator
```

Keep `--seat SEAT` as an exact durable seat selector. Add the same pane selector
alternative to seat-filtered reads (thread list, inbox, pending-receipts, warnings,
diagnostics); foreign-pane inbox is read-only like explicit --seat. Read selectors
only look up existing canonical mappings and never allocate. Invite's pane form
uses ordinary guarded resolution so an empty future agent pane can receive work.
Add repeatable `send --require-ack-pane PANE`, scoped by that command's space/tab;
it can coexist with exact --require-ack SEAT values, deduplicated by canonical seat.
Parent-only read filters must not accidentally become instance-wide reads.
`--cooperative-target` accepts the same unique live locator resolution, while its
seat/harness/role remain explicit and the daemon still validates the claim. New
recipient flags never modify caller attribution.

## Thread names

An optional name is separate from descriptive topic and goal; duplicate names are
allowed. Recommend a bounded UTF-8 single-line name (128 bytes, no control bytes,
not empty); quoted spaces are allowed. Names use exact case-sensitive matching.

```sh
ht thread create --name psa-global --topic "Announcements"
ht thread name t12345                     # show current name, or unnamed
ht thread name t12345 --set team-feature-abc
ht thread name t12345 --clear
ht thread rename psa-global team-feature-abc  # alias for name --set
ht read psa-global
ht send team-feature-abc --body "Ready for review"
ht search "release" --thread psa-global
ht warnings --active team-feature-abc
```

All thread-selector positions audited above accept ID or name, including name
management and handoff --thread. Exact existing ID wins even if another thread
has that name; otherwise resolve the exact name. Scope is the explicitly selected
herdr-threads instance, across all memberships and including archived threads,
because history/discovery are already available to nonmembers. Membership does
not silently hide duplicate names or authorize writes. Conflict lists canonical
IDs, topic, archived state and caller membership, then suggests the exact ID.
Do not add pane/workspace ownership to threads: a thread can span spaces.

Store nullable name with a nonunique (instance_id, name) index; existing rows
migrate to unnamed without changing IDs. Create/name mutations are journaled and
name changes require the same joined caller authority as topic changes. Resolve
names through bounded indexed daemon queries, not a client scan of directory
pages. Resolution includes the second match before declaring uniqueness; conflicts
are bounded and report omitted candidates. Freeze the chosen ID for each operation
and emit IDs in hook commands, continuations and retry records. Show names beside
IDs in thread directory/details/picker without changing compact row framing.

## Bare read and picker

```sh
ht read                         # interactive channel picker, then recent 20
ht read --follow                # picker, then existing read-follow behavior
ht read --recent 50              # picker, then selected history
ht thread list --recent --all    # paged activity list for scripts/agents
ht --json thread list --recent --all
ht read t12345                  # existing explicit read behavior
```

Recommend a small builtin picker, avoiding an external fzf requirement or an
environment-dependent fallback. Type to fuzzy-filter name/topic; arrows or Ctrl-N/P
move; Enter reads the highlighted canonical ID. Display name (or unnamed), topic,
exact ID, last activity and archived marker. Picker UI goes to the terminal error
stream; the selected transcript uses normal stdout. Restore terminal state on all
exits; Esc/Ctrl-C cancel with exit 0, no transcript and no mutation. Empty list
prints a brief no-channels message and exits 0. Empty filtered results cannot select.

Require stdin/stdout/stderr TTYs, TERM other than dumb, normal human presentation,
and no agent-harness marker. Bare read with --json, --machine, redirected streams
or in an agent context returns InvalidRequest immediately with the recent-list
command and an explicit read example; --human alone cannot enable an agent picker.
Bare read --cursor is invalid: a history cursor belongs to a selected thread.
All selected reads, including follow, remain read-only and never ACK or accept.

Default picker inventory is all threads in the selected instance, newest activity
first, including archived rows marked as such. No arbitrary time cutoff: older
channels remain reachable. Activity is the latest committed timeline event, or
creation time when empty; ties use stable creation ordinal. Load bounded pages
from an indexed recent-directory query, progressively, indicating loading/more.
Filtering must continue loading pages even when the first page has no matches;
never present a prefix as the entire inventory. Freeze an activity watermark/order
for the session; new activity appears on refresh, and relevant concurrent changes
that invalidate traversal produce a restart hint rather than skipped rows. Add an
activity-order index/materialized activity field maintained with timeline publication;
backfill once in migration. Preserve existing directory order by default.

## Durable handoff

```sh
ht handoff --new-thread --pane bob --kind codex -- \
  "You are Bob. Answer herdr-threads messages addressed to you, briefly."
ht handoff --new-thread --thread-name team-feature-abc \
  --topic "Feature implementation" --pane bob --kind codex -- "Implement the feature"
ht handoff --thread team-feature-abc --tab tryout --pane bob --kind codex -- "Review it"
```

Require exactly one of --new-thread/--thread. --thread-name, --topic and --goal
apply only to new threads. Topic defaults to `Handoff to bob` using the resolved
display label (or pane ID); goal defaults to topic, consistent with thread create.
`--name` retains launch's agent-name meaning; --thread-name names the channel.
Text after -- is exactly one quoted durable message body, not native argv. Preserve
launch's existing argv semantics; handoff uses repeatable --agent-arg for native
options if needed, and supplies a fixed initial bootstrap telling the agent to
open its inbox/read the canonical thread ID. The task body is stored only once,
not duplicated in native startup text. The bootstrap triggers Codex's first-turn
hook; durable inbox/wake remains recovery if the bootstrap is lost.

Before thread work: validate text, caller claim, selected thread/membership,
target scope, supported harness/hooks and available shell as far as current
preflight allows. Then persist a compound private plan with frozen pane/seat,
caller, thread selector/result and exact sub-operation keys. Execute create (if
new), invite (already joined is a no-op), send with explicit target receipt, then
guarded launch. Existing-thread sender must already be joined; do not auto-accept
or invent operator permission. Invite acceptance is the recipient's choice. A
required invitation still follows its explicit accept-required revision procedure.

No rollback of committed threads/invites/messages after later failures. Output
reports completed steps, exact IDs, the failed/uncertain phase and one recovery
command, without echoing body. `pending-ops` includes the compound operation;
`retry REF` resumes exact keyed durable steps without duplicate create/send.
Definite pre-start refusal can be retried after repair; recheck launch guards.
Persist the possible-start boundary before submission: unknown startup or a crash
across it never automatically launches a second agent. Print inspect commands and
an explicit ordinary launch command to use only after confirming no agent started.
Successful start is not receipt, registration, acceptance or work completion.

## Choices for discussion

Recommended package: scoped exact target resolution; all-instance exact thread
names; all-instance recent picker; builtin fuzzy UI; body-only durable handoff
with a fixed bootstrap. Alternatives are membership-scoped names/picker (less
noise, but hidden duplicate names and inconsistent nonmember discovery), or an
optional external fzf picker (less UI code, additional dependency/fallback policy).

Useful related conveniences within this task: foreign-pane read selectors,
require-ack-pane, recent directory output, thread rename alias, and native
--agent-arg for handoff profiles. Defer fuzzy execution names, automatic pane
creation, automatic acceptance, global cross-space lookup and thread topics as
implicit aliases.

After approval: write the accepted spec and plan in this worktree; implement with
focused parser/resolver/store/recovery/terminal tests, migration and hostile-label
fixtures, private native scenarios where needed; run required targeted lint/default
feature/format checks, independent review, scoped process leak check, and hand off
a frozen clean SHA to threads-main. Coordinator owns final sweep/merge/release.
