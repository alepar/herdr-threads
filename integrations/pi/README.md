# Pi terminal integration

Install into Pi's agent directory, then start an ordinary native terminal session:

```sh
herdr-threads setup pi
herdr-threads setup-status pi
herdr-threads launch --kind pi --pane PANE -- --model PROVIDER/MODEL
```

Setup resolves `PI_CODING_AGENT_DIR`, otherwise `~/.pi/agent`. It owns only
`extensions/herdr-threads/index.js` and `skills/herdr-threads/SKILL.md`, records
their digests, and grants no permissions. Repeated setup is idempotent; modified
or foreign files are preserved. `herdr-threads unsetup pi` removes unchanged owned
files. Use an explicit profile directory for handoff when selecting another root.

Launch accepts separate-value model/provider/thinking, continue/resume/session/fork
selection, session directory, extension and skill options. A provider requires a
model. Noninteractive, extension-disabling, secret-bearing and unknown arguments
are refused. Handoff owns its single positional startup prompt; the assignment
body remains in the thread. Credentials stay with Pi's provider configuration.
No exact Pi version is an admission requirement.

Only an explicit native `tui` context enrolls. RPC/JSON/print, arbitrary SDK hosts,
unknown/child roles and inherited foreign owner PID claims do not enroll. The
bundled extension declares its current native session, TUI mode, role and process;
the adapter checks agreement against the actual supported Pi process. One owner
per process keeps duplicate global/project installations inert. These are
cooperative claims, not adversarial identity proof.

## Delivery and wake

The native extension registers the session and starts an owned watch channel.
Lifecycle output supplies bounded guidance, with no mail receipt. The channel
retains at most 224 ordinary records plus one protected page of up to 32
nonordinary records. Retained UTF-8 payloads and metadata, including parser and
receipt buffers, share a 2 MiB budget with a 512 KiB presentation reserve.
JavaScript object/allocator overhead and native session history are additional.
Each model boundary selects at most 32 items / 64 KiB, sorted across all retained
batches by priority, deadline and stable first observation. Watch has no creation
timestamp, so exact age across replay is not reconstructed. Displaced/omitted
bodies remain pending; capacity release replaces the watch for canonical replay,
with bounded reconnect/ACK backoff. Saturation itself does not loop reconnects.
Manual presentation keeps notices canonical until the whole emitted page has
qualified native evidence. Protected slots keep the same global ordering; ordinary
ACK release cannot evict them or restart their parked watch.

Bodies enter `before_agent_start` or completed `turn_end`/`agent_before_settle`
results. Other extensions' entries/continuation are preserved. Each retained item
is proposed at most once per outer run; rejected drafts wait for another run.
The integration never queues bodies through steer/follow-up/next-turn APIs and
never starts an idle run through `sendMessage`.

The daemon instead uses the existing native host wake lane. Priority 1 at threshold
0 stays silent; admitted priorities start a turn only when host identity, idle,
composer and focused-pane safety guards pass. The typed `herdr-threads: wake up`
contains no body and preserves the selected model/provider. The boundary supplies
mail in that ordinary user turn. A connected watch does not suppress native wake.
Without the channel, explicit `herdr-threads inbox` remains available. Hc has no
composer wake; named seats need a verified tmux hint and explicit current turn
state. Hosts without that wake path report `wake=no_wake`.

Runtime diagnostics use Pi's public warning notifications, which preserve its
renderer and composer. If that notification API is absent or throws, the TUI
diagnostic is omitted rather than written into the terminal behind the renderer.

The default composer recognizer requires two full-width rules, one blank editor
row, the expected cwd/stats footer and a styled cursor at column zero. Visible
drafts, multiline/wrapped input, busy text/tool headers and built-in selectors
refuse. Unknown/custom editor/footer layouts and unavailable styled output refuse.
No draft is stashed, edited or restored by this integration. A focused pane still
needs the existing minute of observed empty composer before native wake.

Two measured screen limits are explicitly accepted:

- Another extension's custom overlay may keep the same empty composer/footer and
  idle host status while taking keyboard focus. It can intercept native wake.
  This integration opens no overlays; asynchronously reported UI spans cannot
  guarantee a veto before an external overlay opens.
- A spaces-only draft with its cursor moved back to column zero is visually
  identical to empty, including styled output. Native wake can append to/submit
  that draft. Spaces with a displaced cursor are refused. This is an observation
  limit, not a draft-preservation guarantee.

## Receipts and lifecycle

A receipt requires the full exact custom frame/nonce both in the finalized current
native branch and a subsequent model-bound context callback for the same admitted
owner/branch. Only then does `watch ack --via context` name complete ordinary/lazy
IDs. Returned drafts, previews, void API returns, queue acceptance and message-end
events alone supply no receipt. Truncated mail retains its explicit continuation
and never ACKs; invitations are accepted separately. Later context transforms or
provider transport failures remain cooperative observation limits, not proof of
disk flush or model consumption.

Notice, invitation and attention slots require their complete native proposal in
that same observed model-bound context callback. A clear control instead requires
its exact owner-local state application. Only the complete emitted page permits
`watch presentation-confirm`; its opaque receipt and slot keys are passed without
interpreting numeric control stamps. Ordinary mail ACK and invitation acceptance
remain separate. Unknown hidden controls create no local confirmation. A lost
confirmation response triggers canonical reconciliation, with no claim about
whether the operation committed.

Reload, tree navigation and compaction fence stale context/retries; accepted unseen
frames are deduplicated using bounded hints after fresh canonical replay. Reload
discards page receipts and coverage; a branch revision clears coverage and allows
fresh proof for the same owner/page. A supported context projection can show that
historical branch content is absent, permitting reproposal at a later qualified
boundary under the same once-per-run attempt limit. Fixed guidance re-enters after lifecycle
changes. Startup idle initialization and actual `agent_settled` emit serialized,
owner-fenced Observer-only Idle reports for named/tmux wake. Shutdown aborts owned
commands, stops/reaps watch and detached replacement children, and reports liveness
disconnect without retiring the seat. Cleanup removes only matching old owner env.

## Qualification

Use the repository's configured heavy/toolbox wrappers for development checks:

```sh
node --test integrations/pi/extension/tests/extension.test.mjs
node --test integrations/pi/extension/tests/sdk.test.mjs
python3 scripts/pi-tui-qualify.py --self-test
python3 scripts/pi-tui-qualify.py --bin target/debug/herdr-threads
# Same isolated driver through the common e2e entry point:
python3 scripts/wake-e2e.py --harness pi --bin target/debug/herdr-threads
```

The SDK test uses retained research dependencies or `HT_PI_SDK_MODULES`. The native
driver uses the official runtime at `--pi`/`--modules`, public isolated setup and
launch, a scripted provider that refuses model fallback, owned test identities and
scoped cleanup. It does not install dependencies or touch real profiles. Live
OpenRouter/DeepSeek is a separate explicit `--live --openrouter-env-file FILE`
mode: only the named key reaches a private child env file, never argv or logs.
Artifacts are sanitized and checked against the exact secret before retention.

The Herdr release matrix's Pi stand-in checks launch/handoff argv only. It cannot
substitute for real native draft/modal, scheduler, delivery or receipt evidence.
Qualification evidence is recorded under
`docs/superpowers/runs/2026-11-14-pi-harness/qualification/` with its exact scope.
