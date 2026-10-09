# Lazy messages bounded post-cap follow-up

Spec: docs/superpowers/specs/2026-10-07-lazy-messages-design.md
Authority: main coordination disposition m50ny7SgE (dev-herdr-threads#708), within existing user-authorized feature; same seat/tab/worktree. This is a separate follow-up, not a replacement super-auto run. Base d6d84a82e1bd34338669e3cbeb8bbbb3f6a375aa. Preserve original epic closure, thrash exit, capped IDs, round counters, historical reports and archive verbatim. Original report remains historical; publish a successor record.

## Global Constraints

- Native send defaults lazy; explicit --nudge selects Ordinary; ACK recipients imply nudge. Lazy arrival creates no wake/model work or receipt obligation and is delivered only at natural explicit inbox. Explicit timelines/summaries may include lazy content.
- Preserve historical wire/journal omissions as Ordinary, v1 actionable-only hooks/check-in, canonical actor/seat/read-only continuation semantics and output bounds.
- Work only on the owned isolated branch; no main merge/push/fullsuite/release, global HOLD, shared server/panes or real user configs. Main owns those landing/integrated gates. Migration26→actual reviewed27→28 unchanged; no migration edit here.
- Fresh regression RED before source edits, GREEN focused controls, cargo fmt, nice cargo clippy --locked --all-targets --all-features -- -D warnings; before reviewed successor also nice scripts/check-default-features. UUID-owned tagged test children/private configs and leak checks; stop every owned helper.
- One implementation chain at a time. Shared guide Task3 requires explicit owner coordination before editing; no unfinished peer branch import.
- Independent task reviews check spec and quality; final whole-branch roast is post-cap audit, never round3. One final review correction wave maximum, then report unresolved honestly. Original3filtered punch-list items stay preserved.

## Task 1: Lazy-only recency must not create recovery obligations

Resolve round2 [Blocking] src/store/queries.rs:5720. HotThreads currently queries ordinary-kind messages including delivery_mode=lazy and creates Recent recovery obligations on Compact/Resume/Clear. Exclude lazy-only arrivals from recency-driven recovery obligations and last_activity while preserving ordinary recency, pending receipts/invitations/warnings and explicitly requested lazy histories/summaries. No new message kind, migrations or policy weakening.
Files: src/store/queries.rs; relevant existing store query/hook/harness tests or one focused stateless combined test module if needed.
Meaningful regression: an otherwise quiet joined thread (prior activity older than the hot window) receives lazy mail, stays absent from recovery hot threads/instructions with zero obligations, yet remains visible at explicit inbox and explicit history/summary source. Ordinary recent send is a positive control; unrelated ordinary attention remains visible. Use real canonical publication; do not merely mutate mode after publication or assert a SQL string. Preserve bounded/time semantics. Read TRUST-POLICY before effects. Record RED/GREEN and focused tests, gates, process cleanup in task report; self-review; commit source/tests only.

## Task 2: Preserve legacy cursor protocol on a v2-capable daemon

Resolve round2 [Should-fix] src/cli/mod.rs:908. Supplied legacy c3 inbox continuation must use v1 despite v2 capability; initial cursor-less inbox selects v2 and ib2 remains v2. Malformed cursors must fail rather than disappear or silently restart. Preserve originating actor/seat/read-only selectors/routing/bounds. Source change belongs in CLI protocol selection, not weakening v2 decoder validation.
Files: src/cli/mod.rs and focused CLI/store executable continuation tests; minimal nearby parser/output changes only if proven necessary.
Meaningful regression: generate a real bounded v1 check-in inbox continuation with more actionable items than its first-page limit, execute printed argv unchanged against v2-capable daemon, recover remaining items without ACK/completion for explicit-seat continuation; include initial v2 and v2 continuation positive controls. Test-first observed RED, GREEN, focused existing continuation controls and all per-change gates, cleanup/self-review/commit/report.

## Task 3: Reconcile actual embedded guide with passive defaults

Resolve round2 [Should-fix] src/cli/commands.rs:2191 / integrations/skill/SKILL.md. Coordinate with active guide owner sskt8CpQS; either consume an exact reviewed permitted frozen successor or perform a specifically authorized bounded guide edit. Match inspected parser: default Lazy, --nudge or ACK recipients Ordinary, no --ordinary. Teach --nudge for coordination replies expected to notify; bare nonurgent announcements delivered next explicit inbox without wake/receipt/adoption promises; ACK means receipt only. Waiting for notified replies finishes turn/hooks; a lazy send promises no wake. Preserve handoff FIRST/new-tab work guidance, intended audience/goal matching, no polling and subagent no-write rules. No native install/config or wider topology work.
Verification: actual compiled skill output/help consistency with shipped parser and semantic passive/ordinary positive controls (avoid implementation-mirroring prose snapshot tests). Record owner permission/frozen source, checks and fresh independent task review.
