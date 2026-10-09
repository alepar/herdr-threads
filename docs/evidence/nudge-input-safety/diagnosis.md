# Attention nudges and user drafts

Diagnosis/design checkpoint, 2026-10-07. No product implementation or deployment.
Worker: nudge-input-safety, w4:pE5 / seat sBdSTaoct. Coordination: dev-herdr-threads
taQXwBH7X. Branch `fix/nudge-input-safety`, base actual main
`ada2f3767867096d9821db450bd0f6e8eb633d6e`.

## Original report

Read complete message `mFeB1pLcR`, thread sequence 51, author sv56BR0hH,
05:13Z, agent relays-user/request:

> Second bug report, requested by my user: explicit attention nudges interrupted focused user input twice and split the submitted text. Captured endings: "split my inptherdr-threads: attention pending; run herdr-threads inbox" followed by "split my input... again...". Expected: defer nudges while the user is composing/focused; preserve their draft without merging, submitting or splitting it. Cause unconfirmed. Please triage separately from empty-inbox warning replay; related evidence may share investigation. User requested intake, so please retain/route reporter-facing record there if appropriate; posting here under your internal-project routing clarification.

The report proves an observed symptom, not the originating call. This worker keeps
the report internally as instructed; no external intake broadcast. Explicit ACK
was refused as not an addressed ordinary message. Acceptance of invitation
iuN8lAr9E is separate from receipt.

## Source trace

Threads base source:

- `src/notification/policy.rs:14` defines the exact observed marker.
- `src/host/native.rs:859` reads ordinary wake targets without composer UI;
  `observe_target` uses agent status only for that path. `safe_wake_target`
  excludes active/blocked UI, but neither focus nor typed input.
- `src/notification/dispatch.rs` validates the reserved canonical identity and
  binding, selects the fixed marker, and calls `submit_prompt`. A coalesced wake
  can fall back to the ordinary marker when a poke is ineligible.
- `src/host/native.rs:1271` rechecks `agent.get` status/kind/terminal/incarnation,
  then sends `agent.prompt`. It has no composer-empty or focus condition.
- `src/notification/dispatch.rs:67` verifies submission after the host call;
  `src/host/native.rs:1808` treats a normalized substring marker in a four-line
  composer tail as HoldingPrompt. Verification can press Enter once. That read
  neither proves the held text is exclusively plugin-owned nor atomically protects
  subsequent typing. This is an additional source hazard, not reproduced here.
- `src/cli/hook.rs` emits structured additionalContext at registered lifecycle/tool
  boundaries over hook stdout, with diagnostics on stderr. It does not call
  `agent.prompt`, send keys, or rewrite the user's input. Shell wrappers' foreground
  restrictions remain normative; no wrapper/config changes are proposed.

Official Herdr 0.9.3 source was read locally from the coordinator's retained
`target/coordinator/herdr-093/source`, commit
`7b116c05bfda646af39d2524c54e70c751f57ee8`:

- `src/api/schema/agents.rs:179`: AgentPromptParams has target/text/optional wait,
  no atomic expected composer, focus, input generation or buffer transaction.
- `src/app/api/agents.rs:111`: queue_agent_prompt rejects blocked/not-ready agents;
  encodes text and Enter, then queues both with a 300 ms submit delay. No draft or
  focused-pane exclusion.
- `src/pty/actor/unix.rs:636`: SubmitUserInput enqueues text and later Enter. The
  actor serializes writes but does not isolate the harness's existing composer
  buffer. The user's already-written prefix is still there when Enter arrives.
- `src/client/shell/input.rs:175` routes committed text to focused pane events;
  `src/server/pane_input.rs:296` encodes keys/text into the same runtime's
  try_send_bytes path. No supported plugin observation locks out typing between
  a screen read and prompt submission.
- `src/app/api/panes.rs:1905` supports private dummy raw input through that same
  runtime byte path, used by this reproduction in place of a human client.

The existing poke-spike findings Q1/Q5 already measured idle status with typed
input and composer merging in real Claude / mock-provider Codex. Current policy
A4 and Accepted limits explicitly allow ordinary wake merging. The older
scheduler design describes an accepted optimistic race; neither is a guarantee
of preserving drafts.

## Private reproduction

The throwaway `private-repro.py` uses the existing PrivateHerdr helper with a new
private HOME/config/state/socket, a named workspace, and a shell script named
claude. There is no actual harness, provider, model call or credential. It directly
exercises the host API that the Threads source selects; it does not run the
Threads scheduler, reproduce a full native editor, or establish the reporter's
exact causal event.

Successful run UUID `e1f97878-a5a7-4a59-b6e4-638b8ef7d991`, root
`/private/tmp/htnud-c0n0ym1w`. Host binary `herdr 0.9.3`, SHA256
`5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157`.
The inherited helper's `pinned:false` compares this official 0.9.3 binary against
its older 0.9.1 pin; this is not a claim that the binary is modified.

1. Start the dummy foreground process; render captured Claude idle title/prompt.
2. Write `DUMMY-prefix split my inp` without Enter.
3. `agent.get` still returns idle, interactive_ready=true, focused=true.
4. Send the exact attention marker through `agent.prompt`.
5. Write `ut DUMMY-suffix`, then Enter, after the prompt call returns.

The dummy receives these two separate lines:

```text
DUMMY-prefix split my inpherdr-threads: attention pending; run herdr-threads inbox
ut DUMMY-suffix
```

This demonstrates a concrete prefix-merge, premature submit, suffix-split
mechanism. Draft input was outside the dummy's static fake composer box; the
fixture is a byte-stream demonstration, not draft-reader qualification.

Preserved earlier failures: run 01 used `agent` instead of the official required
`kind` field and was rejected before start; run 02's fake Codex display reached
interactive_ready but remained unknown, so it never sent a nudge. Both hosts
stopped in finally. All three UUID-scoped `scripts/check-no-leaked-processes`
checks passed with `no leaked test processes` (private roots used for --root).
Raw commands/results/run IDs and dummy received lines are retained in the numbered
directories; `sha256.json` hashes captured artifacts before this narrative.

## Separate warning lane

Worker sskt8CpQS confirmed in `mwbXPAPua` (fully displayed by inbox) that its edits
are store candidate/late attribution/attention-ledger cleanup, with no dispatcher,
native prompt or draft/poke changes. Fixing warning-only empty inbox replay does
not establish input safety.

## Concrete design decision required

The strict goal is never merging, submitting, splitting or overwriting an
in-progress draft. Existing screen/focus observations cannot satisfy that goal
atomically. Recommended bounded implementation: refuse autonomous production
native PTY attention prompts and pokes until a separately supported delivery
boundary can preserve input. Keep attention durable and deliver it through
existing registered hook additionalContext and explicit inbox reads. Refuse
before host input mutation; never stash/restore, press Enter for verification,
or advance the wake ladder as if delivered. Keep canonical identity, warning
deadlines and receipt state unchanged. Health/docs explain that idle agents may
remain unaware until their next supported hook or manual interaction, including
an unregistered managed launch that previously relied on a wake to start.

Implementation scope after approval: native attention admission/submission and
production wiring (with no test-fake claim of native capability), relevant wake/
poke regression and hook positive controls, compatibility documentation and
TRUST-POLICY A4/Accepted limits in the same commit. Tests must demonstrate zero
prompt/key/clear/retype host mutations for focused drafts, unfocused drafts,
apparently empty composers with typing introduced after observation, and held
marker plus user text; pending obligations remain retrievable through inbox and
hooks. Preserve isolated positive hook/inbox behavior, then format, required
clippy/default-feature checks, scoped cleanup and fresh independent review.
Coordinator owns release and any main landing window; never push or install from
this worker.

Alternatives for approval:

1. Best-effort focus plus composer-aware wake guards, fresh pre-send recheck, and
   removal of submit-key retry. Retains idle wake liveness but has screen ambiguity
   and check-to-write race; cannot be described as meeting the strict guarantee.
2. Recommended hook/manual delivery only on the current API. Meets the strict
   no-plugin-input-mutation property, accepting loss of autonomous idle wake.
3. Seek an upstream atomic buffer/focus/input-generation guarded delivery or an
   independently supported harness notification API. Preserves more liveness if
   actually supported, but requires explicit scope beyond this source-only task;
   no upstream source edit or deployment is authorized here.

No implementation begins until the concrete tradeoff is approved. This is a
source/API capability decision, not a timer or typing heuristic.
