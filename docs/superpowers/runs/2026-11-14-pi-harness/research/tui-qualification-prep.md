# Pi real-TUI qualification preparation

This is an execution plan, not qualification evidence. No provider credentials, real profiles, setup, hosts or child processes were used during preparation. Native `pi` was not found on the preparation environment's PATH. The retained local npm lockfile resolves the official `@earendil-works/pi-coding-agent` package (historical specimen 1.5.2); its executable is `node_modules/.bin/pi`, targeting `dist/bundle/cli.js` with a Node shebang. Dependencies were removed after prior probes. Restore them only under the worktree, never globally or under the temp filesystem. Do not gate support on this version.

## Runtime and scripted provider

Use `research/candidates/probe/package-lock.json` as the reproducible starting point. Install with `npm ci --prefix <probe-dir> --no-audit --no-fund`, then use the absolute resolved local `pi` executable in a private PATH shim directory so public launch resolves the real runtime. Record package integrity, runtime version as diagnostics, candidate commit and actual executable basename. Check free space first. All package, build and test execution goes through the coordinator's `heavy tb` wrappers; commands below denote that as `HEAVY TB`.

The existing `probe.mjs` overrides an SDK agent's stream function and therefore cannot qualify native TUI. Instead create a **qualification-only extension**, outside `integrations/pi/`, which registers a fake provider with `pi.registerProvider`. Supply a single model, dummy non-secret API key/preflight, and its documented `streamSimple` callback returning `createAssistantMessageEventStream`. Recheck installed declarations before writing this extension: do not assume SDK stream override is a CLI extension option. Native Pi loads both the public setup-installed production extension and this fake-provider extension from the scratch agent extension directory. Start the TUI with `--provider <fake-provider> --model <fake-model>`; no print/JSON/RPC flags and no extension-disabling flags. No real model fallback is permitted: assert every model request reaches the fake callback and fail closed if it does not.

The fake provider writes append-only JSONL observations and accepts a bounded scripted response sequence via private fixture files. Record full model-bound message content using synthetic test text only. Responses should support: immediate text; a held response released by a fixture signal; a tool-call response to a qualification-only held tool; error; and cancellation. Signal cancellation must terminate the fake stream promptly. Register a qualification tool which blocks on a fixture signal, checks its abort signal and returns a synthetic result. This proves tool-running delivery separately from text streaming. No shell tool is needed. Never log ambient environment or credential values.

Do not replace production hooks or call private delivery methods in this test. An observer extension may record boundary/context/session events and exact active-branch custom entries, but should return no proposal or message transform. Load a separate transformer observer only in the explicit negative-receipt test. Confirm extension discovery/load order in the trace.

## Private host and public entry points

Use the helper contract in `scripts/lib/isolated-herdr.sh`: source it, create `ih_root_new`, call `ih_start <root> pi-tui`, and install a trap that calls `ih_stop` plus ownership-checked helper cleanup. It seeds sync discovery off. Construct every Herdr invocation with `ih_env_args <root>`; the resulting private endpoint, HOME and XDG values must reach panes and the candidate CLI. Never invoke an unscoped Herdr command from the coordinator's inherited environment. Reuse patterns from `scripts/wake-e2e.py` for owned workspace creation, `pane read --source visible`, literal composer typing, and only-owned workspace cleanup; do not assume Herdr agent detection recognizes Pi yet.

Inside the private fixture create separate HOME, PI_CODING_AGENT_DIR, project cwd and candidate daemon state. Run the **public** commands `herdr-threads setup pi`, `setup-status pi` and `unsetup pi` with that isolated environment (confirm integrated help grammar first). No setup permission grant should be introduced. Test idempotence, modified-owned-file preservation and foreign-file preservation. Reinstall for delivery tests. Run public `herdr-threads launch --kind pi` with the fake provider/model through the accepted adapter option grammar, supplying the private Herdr endpoint explicitly where required. Capture the exact parsed launch result and actual Pi process argv; never substitute a direct SDK host. A second direct native TUI run may isolate an upstream behavior, but cannot replace public-launch qualification.

Use a private person sender created via the public human identity path; do not impersonate the coordinator's agent seat. Send and query through the private candidate daemon only. Avoid copying the current shell environment wholesale: pass the minimal helper routing, PATH, owner/leak tags and isolated profile variables. Do not pass provider keys or inherited HERDR_THREADS_PI ownership markers. Private launch must scrub markers itself; a separate deliberate inherited-marker case checks refusal.

Core host coverage is separate. Source `scripts/lib/isolated-core.sh`, use `ic_start pi-tui`, its generated private `HC_SESSION`/socket, and `ic_stop`. Never use `hc server status`. Core startup preserves HOME by default, so explicitly override HOME to the private fixture before calling it. Core messaging/channel qualification is possible once terminal admission works; public launch composition may remain blocked by the shared-core work identified in design D1. Record that as blocked, not as a passing substitute from the Herdr path.

## Required cases and acceptance evidence

| Case | Procedure and acceptance |
|---|---|
| Startup and handoff | Public setup/launch; first fake request contains fixed guidance and one startup/handoff body, with current session and own PID claims. No duplicated positional input. |
| Passive idle | At threshold zero send p1; no fake request and no composer mutation for a bounded observation window. Next explicit user turn contains its exact frame. |
| Priority wake | At thresholds zero, three and minus one send priorities around each boundary, plus invitation/ACK request. Exactly admitted idle items start a request. Content-free wake is separate from the body. |
| Busy text and tool | Hold fake response, then hold fake tool. Send p1/p0 during each; no new outer run/wake is submitted, exact frames appear at the next eligible boundary request, receipts follow exact finalized branch plus model-bound context. |
| Sorting | Accumulate reverse-priority/deadline/age arrivals across at least two watch batches before releasing a boundary. Assert one sorted proposed/model batch, not FIFO by batch. |
| Draft and modal | Type an unsent literal draft using pane keys, then admit p0; capture before/after composer and outgoing requests. Open a real Pi picker and an extension blocking prompt separately. No silent draft submission/loss or picker selection; wake waits safely or the affected path is disabled. A context API without global picker state does not prove safety. |
| Hard exits | Error, user cancellation and cancellation during pre-settle: no automatic revival. Persisted-only drafts never receive context receipts. Prior SDK probe requires explicit completed-outcome guard. |
| Lifecycle fencing | Resume, new session, fork, reload, reload failure, compact and shutdown while deliveries await. Old callbacks never ACK a successor frame; watch child ends/reaps; channel loss yields no idle wake and reconnect restores it. |
| Negative receipts | Transform/remove exact custom body after proposal, omit current-branch match, and inject identical type with different nonce/body. Receipt remains pending. Include truncation/replay/late callback. |
| Identity exclusions | Non-TUI mode, inherited foreign owner PID, explicit child and malformed role never become top-level. Observe subprocess markers without reading arbitrary process environment. |

The existing model-free SDK traces already establish startup-message persistence, boundary preview ordering, same-outer-run completed continuation and cancellation limitations. Repeat relevant assertions in real TUI rather than count those SDK traces as TUI results. Real model quality, provider authentication and quota are outside this credential-free qualification; a live-model matrix remains separately authorized work.

## Commands and output layout

Suggested staged commands (placeholders are intentionally unresolved until integrated setup/launch grammar exists):

```sh
df -h .
HEAVY TB npm ci --prefix <probe-dir> --no-audit --no-fund
HEAVY TB node <probe-dir>/boundary.mjs
HEAVY TB nice scripts/build.sh
# Proposed driver to implement only after integration; this file does not create it:
HEAVY TB python3 <qualification-driver> --candidate bin/herdr-threads \
  --pi <probe-dir>/node_modules/.bin/pi --provider scripted \
  --evidence-dir docs/superpowers/runs/2026-11-14-pi-harness/qualification/tui
HEAVY TB scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID"
```

The driver must export a fresh `HT_LEAK_RUN_ID` before creating anything. Store `manifest.json`, case-level `results.json`, provider/context/boundary traces, sanitized pane captures, setup/launch outcomes, private process ownership and cleanup verification under the evidence directory. Runtime scratch, sockets and heavily rewritten signal files may live under helper temp roots; npm dependencies and build output stay in the worktree. Public evidence contains no absolute machine paths, hostnames, ambient environment, auth data or private identifiers. Refer to fixture paths symbolically.

Cleanup is part of each case: cancel/finish held fake streams and tools, exit owned Pi TUI, wait for extension watch children, close only the owned workspace, stop the private candidate daemon, call private host cleanup, await every owned process, then run the tagged leak check (including temp-file check). Preserve sanitized evidence before removing helper roots; remove restored dependencies and qualification scratch once the investigation finishes. Do not kill unrelated processes. Any remaining process, omitted case, absent model request evidence or unsafe modal/draft behavior is an explicit failure/blocker.
