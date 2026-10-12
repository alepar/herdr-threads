# Test infrastructure for agy: fake Gemini server, native gate, Gemini↔OpenAI relay and live smoke

Parent: [root spec](2026-10-09-agy-harness-adapter-design.md) · Bead: ht-poh.6

## Goal

A developer with a genuine agy binary (`HT_AGY_BIN`) runs `scripts/agy-native-gate`, a model-free gate. It drives the real agy TUI in an isolated named Herdr session against a scripted fake Gemini server, and proves every agy flow from ht-poh.4 and ht-poh.5 end to end:
- enroll, subagent, `/clear`, resume;
- delivery with transcript-confirmed settlement, and the downgrade;
- the permission entry, setup and unsetup, launch;
- idle wake.

The orchestrator runs `scripts/agy-live-smoke --live` once at the final SHA. It goes through a budget-capped relay to OpenRouter `google/gemini-2.5-flash` and writes a record to `docs/evidence/agy-live-smoke/`. That record shows a real model being woken, receiving a message through a hook-injected `systemMessage`, and ACKing and replying with `herdr-threads`. Neither runner ever runs in CI.

## Problem description

The CI fixture tier (root §5.1) cannot prove the claims that rest on agy's own behaviour:
- agy actually loads the owned plugin;
- agy accepts the `systemMessage` step and writes it to the transcript the confirm scan reads;
- `command(herdr-threads)` really auto-approves;
- a child conversation gets no presence lock;
- `-c` and `--conversation` fire no `SessionStart`;
- an unsupported step aborts the turn and reaches `Stop`;
- Herdr detects agy and types a wake into its composer.

The research left two probe harnesses as throwaway shell scripts with hard-coded scratch paths (`~/Documents/Agy_Integration_Research_20261008/work/B/exp/harness-*`, `…/scratchpad/probe-toolcall/{env.sh,lib.sh,fake_server.py,hooks/inj.py}`). It also left a LiteLLM-plus-shim gateway that drops `model`-role function responses, builds tool ids from function names, keeps only the first system part and strips thought signatures (research report §gateway). This subtree turns them into repeatable, isolated, leak-checked repository tooling. It also turns the live proof into one bounded run that cannot overspend.

## Main challenges

**The downgrade path cannot be forced from the model side.** A fake server only answers model requests. A turn aborts only because of what a *hook* printed, and herdr-threads prints only the verified `systemMessage`. The repo's failpoints cannot help: they cannot be armed by environment by design (`src/test_support/failpoints.rs:1-10`).

**Several things must line up in one place.** agy has to run inside a Herdr pane, because wake, composer reads and agent detection need real Herdr. At the same time it has to be:
- sandboxed to loopback;
- isolated from the real `~/.gemini`;
- reachable by its hooks over the daemon and Herdr sockets;
- visible to the leak checker after `env -i` scrubbing.

**Process hygiene spans many children.** A gate run starts a Herdr server, a daemon, a fake server or relay, agy and its hook processes. All of them must die when the gate dies, including on SIGKILL.

**The relay has to be faithful enough for a live TUI.** It must handle SSE streaming, function-call id round-trips and role repair. It must also make overspend impossible rather than unlikely, and it must keep the key out of every child, log and evidence file.

**Flows depend on adapter leaves that land at different times.** They must not fight over one registry file.

## Key decisions made

**Language and layout.** All tooling is Python 3 standard library, kept 3.9-compatible because macOS `/usr/bin/python3` 3.9.6 runs the in-sandbox hook mutator, while CI uses 3.11. It lives in four directories:
- `scripts/agy-fake-gemini/`
- `scripts/agy-sandbox/`
- `scripts/agy-gate/`
- `scripts/agy-relay/`

plus two sh entry points, `scripts/agy-native-gate` and `scripts/agy-live-smoke`.

**Scenarios.** The fake server is driven by a JSON scenario format, `agy-fake-gemini/1`:
- an ordered step list with match predicates;
- `text`, `call`/`calls` and `error` replies;
- `delay_ms` and chunked streaming for busy states;
- templates bound to `[herdr-threads]` frame headers found in the request.

**Forcing the downgrade.** It is a *hook-output* trigger, not a server feature. The gate wraps the owned plugin's `PreInvocation` command in the isolated HOME with a one-shot step mutator, which rewrites our `systemMessage` step into the probe-verified broken `toolCall` step. The abort is then genuinely agy's and genuinely attributed to `jsonhook__herdr-threads_PreInvocation_0_0`.

**Host.** agy runs in isolated Herdr panes, with no private tmux. This refines root §5.2, which named both (see D5). The `agy` the pane sees is a wrapper on a private PATH that `env -i`-scrubs and `sandbox-exec`s a staged clone of `HT_AGY_BIN`. Re-staging gives a new executable identity for the downgrade's identity test.

**Hygiene.** Every child is owner-tagged and its process group recorded for the `isolated-herdr.sh` reaper. Each world is torn down, and `scripts/check-no-leaked-processes --run-id` gates the exit code.

**Flows.** They are auto-discovered modules (`scripts/agy-gate/flows/*.py`), so flow leaves never share a file.

**Relay.** A pure `translate.py` (Gemini-native ↔ chat completions) plus a server with hard ceilings of 40 requests and $0.50. Flags can only lower them. Enforcement:
- a pre-flight worst-case cost check;
- OpenRouter `max_price`;
- request counting persisted before each upstream call.

The key is read only from the real user's `~/.hermes/.env`, never from the environment.

**Live smoke.** A runner with `--dry-run`, which runs the whole pipeline through the relay against a scripted OpenAI-compatible fake upstream, and `--live`. Only the orchestrator executes `--live`.

**CI.** It gets one line, which runs the *offline* unit tests of these directories. No runner ever runs there, and every runner refuses under `CI`/`GITHUB_ACTIONS`.

## Decision points

### D1. Layout, language, entry points

**Chosen.**

| Path | Contents | Leaf |
|---|---|---|
| `scripts/agy-fake-gemini/server.py` | Scripted Gemini-native HTTP server | L1 |
| `scripts/agy-fake-gemini/scenario.py` | Scenario schema, validation, match and template engine; request-body helpers `system_messages(body)`, `user_requests(body)`, `frames(text)` | L1 |
| `scripts/agy-fake-gemini/test_fake_gemini.py`, `testdata/` | Offline tests | L1 |
| `scripts/agy-sandbox/agyenv.py` | Isolated world: root, env, sandbox profile, binary staging and wrapper, Herdr session, daemon, owned spawning, teardown, leak check; `ModelBackend` protocol and `FakeGeminiBackend` | L2 |
| `scripts/agy-sandbox/loopback.sb.in` | Sandbox profile template | L2 |
| `scripts/agy-sandbox/fetch-agy`, `agy-versions.json` | Optional verified download | L2 |
| `scripts/agy-sandbox/test_agyenv.py` | Offline tests | L2 |
| `scripts/agy-gate/gate.py` | Runner: flow discovery, selection, report, exit codes | L2 |
| `scripts/agy-gate/observe.py` | Read-only observation primitives | L2 |
| `scripts/agy-gate/flows/boot.py` | Infrastructure self-proof flow | L2 |
| `scripts/agy-gate/flows/<flow>.py`, `scripts/agy-gate/scenarios/<flow>-*.json` | One module and its scenarios per flow | L4–L7 |
| `scripts/agy-gate/step_mutator.py` | Downgrade forcing | L6 |
| `scripts/agy-native-gate` | `exec "${HT_AGY_PYTHON:-/usr/bin/python3}" -I -B "$(dirname "$0")/agy-gate/gate.py" "$@"` (the `native-hermes-probe.sh` pattern) | L2 |
| `scripts/agy-relay/{translate.py,budget.py,relay.py,fake_upstream.py,test_*.py,testdata/}` | Relay | L3 |
| `scripts/agy-gate/live_smoke.py`, `scripts/agy-live-smoke` | Live smoke runner | L8 |
| `docs/validation/agy-native.md` | How to run the gate and the smoke, and what each flow proves | L2 |
| `docs/evidence/agy-live-smoke/README.md`, plus one row in `docs/evidence/README.md` | Evidence | L8 |

Further rules:
- Sibling directories are imported by `sys.path` insertion of the absolute sibling directory. No packages and no third-party modules.
- Scenario JSON is validated by `scenario.py` in each flow leaf's own test.
- No Rust, no `Cargo.toml` and no `.config/nextest.toml` changes. The gate is not a nextest target and does not count against the 5-minute suite.

**Discarded.**
- *Rust integration tests behind a feature.* They need a genuine binary and a real Herdr, which the default suite must not depend on. They would also need nextest groups for something that never runs by default.
- *Shell scripts.* The probe harness's shell grew hard-coded paths and could not do JSON assertions cleanly.
- *One `scripts/agy/` directory.* The four concerns are each a reviewable unit, and separate directories keep `unittest discover` runs independent.

### D2. Fake Gemini server and scenario format (owned by L1)

**Protocol surface.** It grows from `harness-fake_server.py`, identical in both probe harnesses.
- `ThreadingHTTPServer`, HTTP/1.1, bound to `127.0.0.1` only (any other `--host` is refused).
- Routes:

| Route | Response |
|---|---|
| `GET /v1beta/models*` | Static model list |
| `POST …:countTokens` | `{"totalTokens": 32}` |
| `POST …/models/<m>:streamGenerateContent?alt=sse` | SSE: `data: <json>\r\n\r\n` per chunk |
| `POST …/models/<m>:generateContent` | JSON |

- A model name containing `flash-lite` is agy's auxiliary title request (research D `native-requests-trimmed.json` n01). It is answered with the scenario's `aux_text` and never consumes a step.
- Every main response carries `candidates[0].content{role:"model",parts}`, `finishReason`, `index:0`, a fixed `usageMetadata` and `modelVersion:"fake"`.

**CLI.** `server.py --ctl DIR [--port 0] [--owner-pid PID]`.
- It writes `DIR/port`, then `DIR/ready`.
- It exits on SIGTERM, or within 1 s of `--owner-pid` dying.

**Control.** The gate atomically replaces `DIR/scenario.json` (write a temp file, then `rename`). The server reloads on a `(st_ino, st_mtime_ns)` change and resets its cursor.

**Scenario format `agy-fake-gemini/1`:**
```json
{
  "format": "agy-fake-gemini/1",
  "name": "delivery-confirm",
  "aux_text": "Gate title",
  "default": {"text": "DONE-default"},
  "vars": {"cwd": "/private/tmp/ih.Ab12Cd/ws"},
  "steps": [
    {"label": "inv0",
     "match": {"contains": ["[herdr-threads] message"], "not_contains": ["CHILD-PROMPT"]},
     "delay_ms": 3000,
     "bind": {"marker": "ack"},
     "reply": {"call": "run_command",
               "args": {"CommandLine": "herdr-threads ack {frame.id}", "Cwd": "{var.cwd}",
                        "WaitMsBeforeAsync": 1000, "toolAction": "gate", "toolSummary": "gate"}}},
    {"label": "inv1", "reply": {"text": "OK", "chunks": 3, "chunk_interval_ms": 200}}
  ]
}
```

**Reply kinds:**

| Kind | Meaning |
|---|---|
| `{"text": s}` | A text part |
| `{"call": name, "args": {...}, "id"?: s}` | One `functionCall`; a missing `id` gets a fresh `uuid4` |
| `{"calls": [ {...}, ... ]}` | Parallel calls |
| `{"error": {"http": 429\|500, "status": "RESOURCE_EXHAUSTED", "message": s}}` | Gemini error JSON with that HTTP status |

Optional reply fields: `finish` (default `STOP`), `chunks` and `chunk_interval_ms`, which split the text over several SSE events. Step-level `delay_ms` sleeps *after* the request is logged and before replying; that is the "busy" state.

**Matching.** Each main request takes the first *unconsumed* step whose `match` holds:
- `contains`: every substring is present;
- `not_contains`: none is present;

both over the request's concatenated part texts. A step without `match` matches anything. No match gives the `default` reply, recorded as `unmatched:true`.

Parent/child ordering is racy: in research `tui-subagent.jsonl` the parent's `PreInvocation` inv1 interleaves with the child's inv0. Subagent scenarios therefore pin child steps with a `contains` on the child prompt marker, and parent steps with `not_contains`.

**Templates.**
- `{var.NAME}` comes from `vars`, which the gate renders.
- `{frame.id}`, `{frame.thread}`, `{frame.kind}` and `{frame.sender}` come from the frame bound by `bind`. Frame headers are matched over the request texts with `^\[herdr-threads\] (\S+) (\S+) in (\S+) from (\S+)(.*):$` (the D4 header line, delivery-policy spec DP2). `bind.marker` picks the first frame whose marker tail contains the substring, `bind.index` picks by position, and the default is the last frame.
- An unresolved placeholder replies `FAKE-GEMINI-TEMPLATE-ERROR <placeholder>` and is recorded as `template_error`.

**Request log.** One line per request in `DIR/requests.jsonl`:
```
{seq, t, path, model, kind: main|aux|count|models, step, unmatched, template_error,
 roles: [...], system_messages: [...], user_requests: [...], function_calls: [{name,id}],
 function_responses: [{name,id}], body: "bodies/000042.json"}
```
- The record is written on arrival, before any delay, so the gate can observe state while a reply is held.
- Full bodies are capped at 1 MiB.
- `system_messages` extracts the inner text of agy's `<SYSTEM_MESSAGE>` wrapper ("The following system message was injected by a PreInvocation hook:…", probe `runs/excerpts.txt`). `user_requests` extracts `<USER_REQUEST>` bodies.
- `DIR/cursor.json` mirrors consumption for diagnosis.

**Discarded.**
- *File-popped `queue.json`, as in the probe.* No match predicates, so it cannot route a parent and a child.
- *An HTTP control endpoint.* It needs authentication and adds attack surface for nothing a file rename doesn't give.
- *An `expect` field that makes the server fail requests.* Assertions belong in the gate, where the failure is reported.

### D3. The downgrade trigger is hook output, not server output (owned by L6)

**Chosen.** `scripts/agy-gate/step_mutator.py --ctl DIR -- <original argv…>`:
1. Reads stdin.
2. Runs the original hook argv with the same stdin, as a child.
3. If `DIR/mutate-armed` exists and the child's stdout parses as `{"injectSteps":[…]}` containing a `systemMessage` step, prints instead the probe-verified unsupported step from `probe-toolcall/runs/19a-abort/hooks.jsonl`: `{"injectSteps":[{"toolCall":{"name":"run_command","args":{"CommandLine":"true"}}}]}`.
4. Renames `mutate-armed` to `mutate-fired`, recording the original stdout and the frame ids in it.
5. Otherwise passes stdout and the exit code through byte-for-byte.

It fires once.

The flow edits only the isolated HOME's owned `config/plugins/herdr-threads/hooks.json`. Before starting agy, it prefixes the `PreInvocation` command with `'/usr/bin/python3' '-I' '<abs>/step_mutator.py' '--ctl' '<ctl>' '--'`; the D3 argv in the agy-adapter spec is already single-quoted shell words. The flow saves the original bytes and restores them byte-identically before any setup-status or unsetup step.

agy then aborts the turn exactly as in probe 19a, and `Stop` reports `terminationReason:"ERROR"` with `error` naming `jsonhook__herdr-threads_PreInvocation_0_0` (probe `tui-04-hooks.jsonl`; plugin hook ids are `jsonhook__<namespace>_<Event>_0_0`). ht-poh.5's genuine `Stop` hook sees it.

**Discarded.**
- *A fake-server reply.* No model reply can make agy reject an injected step.
- *A test-support environment knob in the hook.* It contradicts the failpoint policy (`failpoints.rs:5-6`: no environment variable can arm one), and it would put test code into `src/harness/agy/`.
- *A sibling hook in the global `hooks.json`.* Its error names another hook id, which ht-poh.5 may rightly ignore.
- *An older agy version.* There is no evidence that any version rejects `systemMessage`.

### D4. Obtaining and staging agy (owned by L2)

**Chosen.**
- `HT_AGY_BIN` is required: an absolute path to a genuine agy executable. Without it both runners exit 2 with `set HT_AGY_BIN to a genuine agy binary (scripts/agy-sandbox/fetch-agy can download one)`. An explicitly run opt-in gate must not pass silently.
- Each world stages it at `<root>/agybin/<n>/agy` with `cp -c` (APFS clone: a new inode at near-zero cost), falling back to a plain copy. It records `agy --version`, the sha512 and the size in the report.
- `<root>/bin/agy` is a generated `/bin/sh` wrapper:
  ```sh
  exec /usr/bin/env -i PATH="<root>/bin:/usr/bin:/bin:/usr/sbin:/sbin" HOME="<root>/home" TMPDIR="<root>/tmp" \
    XDG_CONFIG_HOME=… XDG_CACHE_HOME=… XDG_DATA_HOME=… XDG_STATE_HOME=… \
    GEMINI_API_KEY="<backend token>" GOOGLE_GEMINI_BASE_URL="http://127.0.0.1:<port>" \
    TERM="${TERM:-xterm-256color}" LANG=en_US.UTF-8 NO_COLOR=1 \
    HERDR_ENV="$HERDR_ENV" HERDR_PANE_ID="$HERDR_PANE_ID" HERDR_SOCKET_PATH="$HERDR_SOCKET_PATH" \
    HT_TEST_OWNER="<gate pid>" HT_LEAK_RUN_ID="<run id>" ${HERDR_THREADS_AGY_OPTS+HERDR_THREADS_AGY_OPTS="$HERDR_THREADS_AGY_OPTS"} \
    /usr/bin/sandbox-exec -f "<root>/loopback.sb" "<root>/agybin/<n>/agy" "$@"
  ```
  - `sandbox-exec` execs the target, so the process name is `agy` and Herdr 0.9.1 detects it.
  - `herdr-threads` on that PATH is a symlink, `<root>/bin/herdr-threads`, to the gate's build. The model's `run_command` therefore runs a bare `herdr-threads …`, which is what `command(herdr-threads)` matches (research perm matrix p1/p4).
- `world.restage_agy()` makes a new clone directory and atomically replaces the wrapper. Both the binary and the wrapper get a new inode and mtime, so whichever path ht-poh.5 keys the executable identity on changes.
- **`scripts/agy-sandbox/fetch-agy --dest DIR [--version V] [--manifest FILE]`.** It mirrors the official installer (`official-install.sh:68-196`):
  - platform key `${os}_${arch}` (for example `darwin_arm64`);
  - manifest `https://antigravity-cli-auto-updater-974169037036.us-central1.run.app/manifests/<platform>.json`, or a saved `--manifest`;
  - the `url` must start with `https://storage.googleapis.com/antigravity-public/antigravity-cli/`;
  - downloads with no redirects to other hosts;
  - verifies `sha512` against the manifest, and against `agy-versions.json` when the version is pinned there (seeded with 1.3.0 and 1.3.3 from the recorded manifests; 1.3.2's URL is known but its sha512 is added only if recovered from the research evidence);
  - extracts `antigravity` from the tar.gz to `DIR/<version>/agy`, `chmod 0755`, and removes `com.apple.quarantine`;
  - writes `DIR/<version>/manifest.json`;
  - never runs `agy install`.

  `--version` refuses a manifest of another version. It is never invoked by CI or by the runners.

**Discarded.**
- *Running `HT_AGY_BIN` in place.* There would be no identity change for the downgrade test, and a stray write could reach the user's copy.
- *A hard link.* It has the same inode.
- *Auto-downloading in the gate.* That would be a network fetch in a test path.

### D5. Isolation: root, sandbox, Herdr host, daemon (owned by L2)

**Root.** One root per flow world, from `ih_root_new` (`scripts/lib/isolated-herdr.sh:30`), resolved through `realpath` to `/private/tmp/ih.XXXXXX`:

| Path | Purpose |
|---|---|
| `home/` | Herdr's private HOME (`ih_env_args`, `:119`), reused as agy's HOME. agy data is under `home/.gemini` |
| `tmp/` | TMPDIR |
| `ws/` | agy workspace |
| `ctl/` | Server control |
| `ht/` | herdr-threads state dir |
| `bin/`, `agybin/` | Wrapper, symlink and staged binaries |
| `loopback.sb` | Rendered sandbox profile |
| `artifacts/` | Kept evidence |

`ih_teardown` (`:213`) removes the root and kills every process naming it.

**Seed settings.** `home/.gemini/antigravity-cli/settings.json` = `{"altScreenMode":"never","modelProvider":"gemini","trustedWorkspaces":["<root>/ws"]}`:
- the same keys the probe and the ht-poh.4.5 captures used (`probe-toolcall/runs/05-systemMessage/settings.json`), so the composer parser sees the same screen;
- the workspace is pre-trusted, so no trust prompt appears except in the flow that tests agy's rewrite.

**Sandbox profile** (`loopback.sb.in`, rendered with the root and the *real* user home from `pwd.getpwuid(os.getuid()).pw_dir`):
```
(version 1) (allow default) (deny network*)
(allow network-outbound (remote ip "localhost:*")) (allow network-inbound (local ip "localhost:*"))
(allow network* (remote unix-socket))
(deny file-read* (subpath "<REAL_HOME>/.gemini") (subpath "<REAL_HOME>/.claude") (subpath "<REAL_HOME>/.codex")
                 (subpath "<REAL_HOME>/.aisw") (subpath "<REAL_HOME>/.hermes"))
(deny file-write*)
(allow file-write* (subpath "<ROOT>") (literal "/dev/null") (literal "/dev/tty") (literal "/dev/ptmx")
                   (regex #"^/dev/ttys[0-9]+$") (regex #"^/dev/fd/"))
(deny mach-lookup (global-name "com.apple.securityd"))
```
This is the probe profile, with the PTY allowances the research gateway section requires. Hook processes inherit it. They reach the daemon and Herdr over unix sockets and write only under `<ROOT>/ht`.

**Herdr host (refinement of root §5.2).**
- Each world starts its own named Herdr session with `ih_start` (`:133`), with `HT_TEST_OWNER` set to the gate's pid so `ih_reaper_ensure` (`:107`) covers a SIGKILLed gate.
- agy panes are created with Herdr `tab create`/`pane split` and run the wrapper with `pane run`.
- Input uses `agent prompt` (including `/clear`), `agent send-keys` and `pane send_text` (drafts without enter). Output uses `agent read --source detection`, `agent wait --until idle|done|blocked` and `pane read`. These are the verbs `scripts/validate-native-demo.py` already uses (`:1731`, `:1902`, `:1937`, `:2129`).
- **No private tmux.** The research used tmux only for captures. Wake, composer reads and agent detection need Herdr anyway, so a second host would only add processes. The orchestrator should note this as a root-spec refinement.
- `herdr --version` must be at least 0.9.1, the version `tests/native/recovery/private_host.py:19` pins and that detects agy. An older version exits 2.

**Daemon and CLI.**
- The gate builds `cargo build --locked --all-features --bin herdr-threads`, or takes `--herdr-threads-bin`. It starts the daemon explicitly with the root's `--state-dir` and `--host-endpoint <root>/h.sock`, and the environment `HERDR_THREADS_TEST_OWNER_PID=<gate pid>`; the owner watch is test-support only (AGENTS.md).
- Every herdr-threads invocation goes through `world.ht(argv, pane=None)`. It runs with `HOME=<root>/home`, `--state-dir` and `--host-endpoint`, and with `HERDR_PANE_ID=<pane>` when acting as a pane (the `private_host.py:244` pattern).
- It refuses any environment whose HOME is outside the root, so `setup agy` can never touch the real `~/.gemini`.
- A **sender** is a plain shell pane enrolled as a human seat with `me init`. It carries no agent marker, so A4 allows it.

**Discarded.**
- *An `env -i` launched from the gate's own shell, outside Herdr.* No pane, so no enrollment and no wake.
- *A stand-in Herdr endpoint, as in `tests/native/claude_mod/stress.py`.* The wake must actually be typed into agy's composer.

### D6. Gate runner, flow API, observation and report (owned by L2)

**CLI.**
```
scripts/agy-native-gate [--flow NAME]... [--list] [--herdr-threads-bin PATH] [--report PATH]
                        [--keep-on-fail] [--timeout-scale F]
```

| Exit code | Meaning |
|---|---|
| 0 | Every selected flow passed |
| 1 | A flow failed |
| 2 | Environment refusal: no `HT_AGY_BIN`, not darwin, `CI`/`GITHUB_ACTIONS` set, Herdr missing or older than 0.9.1, build failed |
| 3 | The leak check failed, even when every flow passed |

**Flow API** (`gate.py`):
```python
@dataclass(frozen=True)
class Flow:
    name: str
    requires: tuple[str, ...]   # bead ids, printed by --list and in the report
    run: Callable[["World"], None]
```
- Each `flows/*.py` module exports `FLOW`, and `gate.py` discovers them by glob. Adding a flow never edits a shared file.
- Flows run sequentially, each in a **fresh world**, with no fail-fast.

**`World`** (`agyenv.py`):
- `setup_agy()`, `unsetup_agy()`;
- `start_agy(args=(), pane=None) -> AgyPane`, which waits for the Herdr agent `idle`;
- `AgyPane.prompt(text)`, `.keys(*k)`, `.draft(text)`, `.read_detection()`, `.wait(state, timeout)`, `.exit()`;
- `sender()`;
- `backend.load(scenario)` and `backend.requests()`;
- `restage_agy()`;
- `ht(...)`;
- `agy_data` with `presence/`, `brain/`, `conversations/` and the transcripts;
- `check(name, ok, detail)`, which collects assertions; a flow fails when any check fails or it raises.

`ModelBackend` is a protocol with `start(world) -> (base_url, token)`, `stop()`, `requests()` and `load(scenario)`. L2 provides `FakeGeminiBackend`, and L8 adds `RelayBackend` in its own module.

**Observation** (`observe.py`, read-only primitives only; flow-specific queries live in the flow module):
1. `db(world)`: a read-only SQLite connection, `file:<state>/…db?mode=ro`. This is the precedent of `tests/native/claude_mod/stress.py` ("Delivery is asserted from the daemon database (read-only SQLite)"). It is used for bindings (harness, native session, generation, provenance), receipt actions and their provenance, lazy completions and ACKs.
2. `cli_json(world, argv)`: `setup-status agy --json`, `doctor --json`, and seat views.
3. The fake server's `requests.jsonl` and saved bodies.
4. agy's on-disk stores: `presence/<id>.lock`, `brain/<parent>/.system_generated/subagents/<child>.json`, the `transcriptPath` content.
5. Herdr agent state and the detection text.
6. `wait_until(pred, timeout, step=0.1)`, scaled by `--timeout-scale`.

**Report.** JSON at `--report`, and also a summary on stdout. It contains:
- versions (agy and sha512, herdr, herdr-threads git SHA and dirty flag), the run id and the root;
- per flow: name, requires, status, duration, every check and an artifacts path.

`--keep-on-fail` runs `ih_keep` (`:205`) on a failing world; it still stops every process.

### D7. Flow catalogue

Each flow checks *outcomes* at the boundaries named below. The adapter's internals are covered by fixture tests.

| Flow | Script | Assertions | Consumes |
|---|---|---|---|
| `boot` (L2) | No setup. Prompt "hi"; scenario replies `BOOT-OK` | Herdr detects agent `agy`; one main request reached the fake server; `BOOT-OK` appears in the pane. Sandbox self-test: `sandbox-exec -f profile /bin/ls <REAL_HOME>/.gemini` fails with EPERM, and a Python connect to `192.0.2.1:443` fails with EPERM. Teardown, then zero leaks | L1 |
| `setup` (L4) | `setup agy`; start agy; `agy plugin disable herdr-threads`, then `enable` (genuine agy CLI, model-free) | Plugin files exist; `setup-status` reports installed, current, enabled and allow entry `owned`; after disable, `enabled:false` and doctor `agy_plugin_disabled`; re-setup unchanged | ht-poh.4.4, ht-poh.4.7 |
| `permission` (L4) | Scenario `call run_command "herdr-threads --version"`, then `"herdr-threads --version; true"` | First: no `blocked` state, and the next request carries a `functionResponse` containing the version. Second: the agent becomes `blocked` (approval prompt), and the gate denies with keys | ht-poh.4.4 |
| `allow-rewrite` (L4) | Untrusted `ws2`; accept agy's trust prompt | Records whether agy dropped the allow member. If it did, `setup-status` shows `missing` and `setup agy` re-adds it as `owned` | ht-poh.4.4 |
| `unsetup` (L4) | Pre-seed `command(herdr-threads)` in one world (pre-existing case) and not in another; `unsetup agy`; start agy and prompt | Owned files and member are removed; a pre-existing member and the other settings keys are kept; no seat is enrolled afterwards | ht-poh.4.4 |
| `launch` (L4) | `herdr-threads launch --kind agy` into an empty sender-style shell pane; refused argv `--sandbox`, `-p x`, `--dangerously-skip-permissions` | Accepted: agy runs in that pane and enrolls on first prompt. Refused: non-zero exit with the policy text, and no agy process | ht-poh.4.6 |
| `enroll` (L5) | Setup; start; prompt | Seat with a binding: harness `agy`, native session equal to the conversation id (the only `presence/*.lock`), provenance `cooperative_top_level`. The first main request's `system_messages` holds the startup check-in, and its system instruction lists the `herdr-threads` skill path | ht-poh.4.2, ht-poh.4.3, ht-poh.4.4 |
| `subagent` (L5) | Parent step `call invoke_subagent {"Subagents":[{"Model":"inherit","Prompt":"CHILD-PROMPT say hi","Role":"Gate Child","TypeName":"self"}]}` (the research B shape), then child and parent steps routed by match | The child's conversation is listed under the parent's `subagents/` and has no presence lock. The child's request carries the subagent restriction `systemMessage`. Binding generation and native session are unchanged, and no new seat exists | ht-poh.4.3 |
| `clear` (L5) | `agent prompt /clear`, then prompt | Same seat, generation +1, native session equal to the new id; the startup check-in appears again | ht-poh.4.3 |
| `resume` (L5) | A → exit → fresh B → exit → `--conversation A` → prompt → exit → `-c` → prompt | After `--conversation A`: same seat, binding native session A, no new seat. After `-c` (resumes the most recent, A): no rotation, generation unchanged. Neither start fires `SessionStart` (no request carries a second startup check-in) | ht-poh.4.3 |
| `delivery` (L6) | Sender sends lazy L1, ordinary O1, ACK-required A1 and an oversize T1 (over 8 KiB). Prompt. Scenario inv0 holds 3 s and binds `{frame.id}` with marker `ack`, replying `run_command "herdr-threads ack {frame.id}"`; then inv1 | inv0's `system_messages` hold frames for L1, O1, A1 and T1 (T1 truncated). During the hold, nothing is settled. After inv1: O1 settled with provenance `cooperative_hook_delivery`, L1 completed, A1 ACKed by the agy seat with no permission prompt, T1 still pending. Frame header lines appear in agy's transcript | ht-poh.5, ht-poh.3.4 |
| `downgrade` (L6) | Arm the mutator (D3); send M1; prompt. Then send M2; prompt. Then `restage_agy()`, `--conversation <id>`, send M3, prompt. Restore `hooks.json` | 1st: no main request for that turn; the pane shows the error; `doctor --json` reports the downgrade for the identity. 2nd: M2's frame is inside a `user_requests` entry and in no `system_messages`; after the next invocation M2 settles. 3rd: M3 is in `system_messages` again; `setup-status` is `current` after the restore | ht-poh.5, ht-poh.4.7 |
| `wake` (L7) | Idle agy. (a) Send ACK-required N1. (b) Lazy-only Z1. (c) During a `delay_ms` busy step, send N2. (d) Type a draft, send N3. (e) Scenario has the model run `herdr-threads wake-on all`; send lazy Z2 | (a) Within 60 s a `user_requests` entry the gate did not type arrives (the wake), and the same request's `system_messages` hold N1's frame. (b) No new main request within 15 s. (c) No untyped `user_requests` entry; N2 is delivered in-turn at the next `PreInvocation`. (d) No wake; the draft is intact in the detection text. (e) Z2 wakes | ht-poh.4.5, ht-poh.5, ht-poh.3.4, ht-poh.3.6 |

Every flow ends with `backend.requests()` showing every step consumed, no `unmatched` and no `template_error`, unless the flow declares otherwise. (b) and (d) declare that no request is expected.

### D8. Process hygiene (owned by L2, used by everyone)

- **Run start.** The gate sets `HT_TEST_OWNER=<own pid>` and `HT_LEAK_RUN_ID=<uuid4>` in its own environment, and both pass into every child, including through the wrapper's `env -i` allowlist (D4).
- **Spawning.** `agyenv.spawn_owned(argv, env)` uses `Popen(start_new_session=True)` and records `/tmp/ih-reaper.<owner>/g.<pgid>`. This is the convention `ih_recorded_groups` (`:61`) reaps, the same one `test_support::spawn::spawn_owned` writes. Nothing is started any other way.
- **Teardown order**, in `finally` and in SIGINT/SIGTERM/SIGHUP handlers:
  1. agy panes (`/exit` with a 5 s bound, then their process groups);
  2. backend server;
  3. daemon stop (bounded), then kill;
  4. `ih_teardown(root)`.
- **Leak check.** After all flows, `scripts/check-no-leaked-processes --run-id "$HT_LEAK_RUN_ID" --grace 15`. A non-zero result gives exit 3, with the listed pids in the report.
- The shared Herdr server and the real daemon are never contacted, because `ih_env_args` unsets inherited `HERDR_*`.
- The live smoke follows the same rules.

### D9. Relay translation (owned by L3; `translate.py` is pure)

**Gemini to OpenAI** (`gemini_to_openai(body, ids) -> dict`):

| Gemini input | OpenAI output |
|---|---|
| `systemInstruction.parts[*].text` | Joined with `"\n\n"` into one `system` message (fixes LiteLLM's first-part-only behaviour) |
| `contents[]`, any role, part `text` (non-thought) | Text of a `user` (role `user`) or `assistant` (role `model`) message |
| `functionCall` | `assistant.tool_calls[{id, type:"function", function:{name, arguments: json}}]` |
| `functionResponse`, whatever its content's role (agy sends it in `model`-role turns) | One `{"role":"tool","tool_call_id":…,"content": json(response)}` per response, placed after the assistant message that holds its call. This is the root's "user role for function responses", expressed in chat completions |
| Parts with `thought:true`, and every `thoughtSignature` field | Dropped |
| `tools[].functionDeclarations[]` | `tools[{type:"function",function:{name,description,parameters}}]`. Schema `type` values are lower-cased; `parametersJsonSchema` is used as-is when present |
| `toolConfig.functionCallingConfig.mode` | `AUTO→auto`, `ANY→required`, `NONE→none` |
| `generationConfig` | `temperature`, `topP` and `stopSequences` mapped. `maxOutputTokens` gives `max_tokens = min(v, 1024)`. `thinkingConfig` ignored |

- Mixed parts in one content are split in order.
- Consecutive same-role messages are merged.

**Tool ids.**
- A Gemini `functionCall.id` is kept verbatim as the OpenAI id when it matches `[A-Za-z0-9_-]{1,64}` and is unique in the request. Otherwise it gets a request-scoped bijective alias, `ht_<sha256(id)[:24]>`.
- A call with no id gets `ht_<sha256(position,name)[:24]>`.
- A `functionResponse` pairs by id, or, with no id, with the earliest unanswered call of the same name.
- An unpaired response becomes a `user` text note, never a dangling `tool` message.

**OpenAI to Gemini.**
- Streamed chunks are translated incrementally:
  - text deltas become `candidates[0].content{role:"model",parts:[{text}]}` events, so the TUI streams;
  - tool-call deltas are accumulated per `index` and emitted once, at `finish_reason`, as `functionCall{name, args: json.loads(arguments) or {}, id}`. The id is a fresh `ht-<uuid4>` for each call, never the upstream id; the LiteLLM name-derived ids collided.
- `finish_reason` maps: `stop`→`STOP`, `tool_calls`→`STOP`, `length`→`MAX_TOKENS`, `content_filter`→`SAFETY`.
- The final event carries `usageMetadata` from `usage`.
- Non-streaming `:generateContent` is aggregated.
- OpenRouter SSE comment lines are ignored.
- An upstream error becomes Gemini `{"error":{"code","message","status"}}` with the mapped HTTP status.

**No thought signatures** are ever sent to agy. Upstream reasoning is disabled. Research D's live loop through LiteLLM, which strips signatures, completed, so agy does not require them.

**Local routes.**
- `GET models` and `:countTokens` (estimated as bytes / 4) are local.
- `flash-lite` title requests are answered locally with `agy smoke` unless `--forward-aux` is given. This saves budget, and the title is no part of the proof.

### D10. Relay budget, key and guards (owned by L3)

**CLI.**
```
relay.py --run-dir DIR (--live | --fake-upstream URL) [--max-requests N] [--max-cost-usd X]
         [--port 0] [--owner-pid PID] [--forward-aux]
```
- Writes `DIR/port` and `DIR/local-token` (0600, random 32 bytes hex).
- Every request must present the token as `x-goog-api-key` or `?key=`, or gets 403. agy's `GEMINI_API_KEY` is that token.
- Binds `127.0.0.1` only.

**Hard ceilings.**
- `MAX_REQUESTS = 40` and `MAX_COST_USD = 0.50` are constants. Flags may only lower them, and a higher value exits 2.
- `MAX_INPUT_BYTES = 400_000` per translated request.
- `max_tokens` at most 1024.

**Per-request enforcement, under one lock:**
1. `worst = input_bytes/3 × 0.30e-6 + max_tokens × 2.50e-6`. Refuse with Gemini `429 RESOURCE_EXHAUSTED "agy-relay budget exhausted"` if `requests ≥ max` or `cost + worst > max`. A refusal is recorded and sets `exhausted:true`.
2. Otherwise increment `requests` and persist `DIR/budget.json` with fsync *before* the upstream call. A sent request always counts.
3. After the response, add `usage.cost` from OpenRouter usage accounting. If it is absent, add `worst` and flag `cost_estimated`. Persist.

**Upstream body.**
```
{model:"google/gemini-2.5-flash", stream:true, stream_options:{include_usage:true}, usage:{include:true},
 reasoning:{enabled:false}, max_tokens, provider:{max_price:{prompt:0.3, completion:2.5}, require_parameters:true}}
```
This follows research D's `prepare_upstream` (`tool_probe.py:41-54`). The upstream is fixed at `https://openrouter.ai/api/v1/chat/completions`, with no redirect following, no retries and a 120 s timeout.

**Key.**
- `--live` only.
- Read only from `<pw_dir>/.hermes/.env`, where `pw_dir` comes from `pwd`, not `$HOME`: the runner's children have an isolated HOME.
- The `OPENROUTER_API_KEY` line is parsed as in `tool_probe.py:57-65` and must start with `sk-or-`.
- It is held in memory only, never in any child environment, and never written. Every log record is redacted: no `Authorization`, and anything shaped like `sk-or-…` is replaced.
- An environment-variable key is ignored, by design.

**Guards.**
- `--live` exits 2 when `CI` or `GITHUB_ACTIONS` is non-empty, or the platform is not darwin.
- `--fake-upstream` must be a loopback URL.

**Log.** `DIR/relay.jsonl`, per request:
```
{seq, t, path, kind: main|aux_local|count|models, status, upstream_status, model,
 input_bytes, roles, tool_names_count, tool_calls: [{name, args_trimmed}], text_trimmed,
 usage, cost_usd, cost_estimated, refused}
```
Text is trimmed to 2 KiB, and the root path is replaced with `<root>`.

**`fake_upstream.py`.** A loopback OpenAI-compatible SSE server driven by an ordered list of replies (`text` / `tool_calls`) with the same `{frame.*}` templating over message texts. It has its own copy of the D4 header regex, so L3 stays independent of L1. It is used by the relay tests and by the smoke's `--dry-run`.

**Discarded.**
- *A cross-run persistent ledger.* The root fixes caps per run, and the orchestrator runs once.
- *Answering title requests upstream by default.* It spends budget on nothing the smoke proves.

### D11. Live smoke runner and evidence (owned by L8; executed only by the orchestrator)

**CLI.**
```
scripts/agy-live-smoke (--live | --dry-run) [--evidence-dir DIR] [--allow-dirty] [--herdr-threads-bin PATH]
```
- `--live` requires a clean worktree unless `--allow-dirty` is given; the record names the SHA.
- `--live` defaults to `docs/evidence/agy-live-smoke/<UTC date>-<sha12>/`. `--dry-run` defaults to the scratch root.
- It refuses under CI, as D10 does.

**Flow.** One `World` with `RelayBackend` (relay outside the sandbox, agy inside it):
1. `setup agy`, start agy, prompt `This is an automated herdr-threads smoke test. Reply with the single word READY.` Check: the seat is enrolled.
2. Wait for `idle`. The sender sends an ACK-required message in a new thread: `Smoke check: please acknowledge this message, then reply in this thread with the text SMOKE-OK.`
3. **Wake.** Within 90 s the relay sees a main request whose `<USER_REQUEST>` the runner did not type, and whose `<SYSTEM_MESSAGE>` holds the frame header with the message id.
4. **Act.** Wait up to 180 s for `idle`. Then check:
   - the agy seat ACKed the message, through `run_command` with no `blocked` state;
   - a reply from the agy seat in that thread contains `SMOKE-OK` (case-insensitive);
   - the delivery receipt was settled under `cooperative_hook_delivery`, or superseded by the ACK.
5. **Fallback.** If there is no ACK after the woken turn, one explicit prompt: `Check your herdr-threads messages, acknowledge any that require it, and reply SMOKE-OK in that thread.` Record `ack_phase: "prompt"` instead of `"wake"`.
6. **Pass** requires wake, injection, ACK, reply, no permission prompt, no relay refusal and zero leaks. `ack_phase:"prompt"` passes, flagged as model behaviour.
7. A wall-clock cap of 10 minutes; a relay `exhausted` aborts with `budget`.

The expected spend is about 6–12 main requests at about $0.003 each.

`--dry-run` runs the identical flow with `fake_upstream` scripted as READY, then `tool_call herdr-threads ack {frame.id}`, then `tool_call` the thread reply command (exact argv per `src/cli/commands.rs`), then `done`. It needs genuine agy but spends nothing. This is the leaf's own acceptance.

**Evidence** (`--live`):

| File | Contents |
|---|---|
| `summary.json` | SHA, dirty flag, agy version and sha512, herdr version, model, requests, `cost_usd`, phases, `ack_phase`, each check, pass |
| `report.md` | A short narrative |
| `relay.jsonl` | Redacted (D10) |
| `daemon-excerpt.json` | The seat, binding, receipt, ACK and reply rows involved |
| `pane-final.txt` | |
| `requests-excerpt.json` | The injected `<SYSTEM_MESSAGE>` text and the model's tool calls |

There are no full bodies, and the runner refuses to finish if `sk-or-` appears in any file it wrote.

`docs/evidence/agy-live-smoke/README.md` documents the format, the pass rule, how to rerun, and that only the orchestrator runs `--live`, once per final SHA. Committing the record is the orchestrator's step.

### D12. CI and the "never in CI" rule

- The gate, the smoke and `fetch-agy` never run in CI. Each refuses under `CI`/`GITHUB_ACTIONS` (exit 2), and a unit test pins that refusal.
- **One** CI edit (L1), in the "Python demo, validator and canary suites" step (`.github/workflows/ci.yml:230-249`). It adds a loop that runs the *offline* unit tests:
  ```
  for d in scripts/agy-fake-gemini scripts/agy-sandbox scripts/agy-relay scripts/agy-gate; do [ -d "$d" ] && python3 -m unittest discover -s "$d" -p 'test_*.py'; done
  ```
  Those tests need no agy, no Herdr, no network and no key. They cover:
  - scenario validation and matching;
  - translation, the budget arithmetic and the key-file parsing, against a temp file;
  - profile and wrapper rendering;
  - manifest sha512 verification, against a local file;
  - the mutator's pure function and flow discovery.

  Leaves that land later need no further CI edits.

**Discarded.**
- *No CI coverage at all.* The translation and scenario logic would rot silently.
- *Running the gate in CI.* It needs a genuine binary and a GUI-free macOS sandbox, and the root forbids it.

### D13. Accounting (R13)

Every commit carries `Outside-adapter: <list>`, naming its `scripts/agy-*`, `docs/…` and (L1 only) `.github/workflows/ci.yml` paths. The run report should classify these as **R12 test infrastructure**, not as adapter leakage: none of them is production code, and none is needed for the adapter to work. Any production `src/` edit a flow leaf finds necessary is a finding. It goes to the owning adapter leaf, not into this subtree.

## Acceptance

- **Offline tests.** `python3 -m unittest discover -s <dir> -p 'test_*.py'` passes for every `scripts/agy-*` directory under both `/usr/bin/python3` (3.9) and 3.11, and the CI step runs them.
- **Fake server.** Every reply kind, `delay_ms`, chunked SSE, match routing, templates, `unmatched`/`template_error`, aux and countTokens routes, scenario hot-swap and the owner-pid exit are tested. The log record is written before the delay.
- **Boot.** `HT_AGY_BIN=… scripts/agy-native-gate --flow boot` passes on macOS with Herdr 0.9.1:
  - Herdr detects agent `agy`;
  - the sandbox denies the real `~/.gemini` and non-loopback connects;
  - the leak check is zero, and no process tagged with the run id remains.
- **Refusals.** No `HT_AGY_BIN`, `CI=1`, non-darwin and Herdr older than 0.9.1 each exit 2 with the stated text.
- **Flows.** With their bead dependencies landed, `scripts/agy-native-gate` passes every D7 flow, and `--list` shows each flow's `requires`.
- **Downgrade.** The flow passes with only the genuine agy, our genuine hooks and the gate-only mutator; no fake-server involvement.
- **Relay.** Against `fake_upstream`, offline tests show:
  - joined system parts;
  - `functionResponse` in a `model`-role turn arriving as a `tool` message with the matching id;
  - unique emitted function-call ids;
  - streaming text events in order;
  - thought parts and signatures dropped.
  
  Request 41, or a request whose worst case would pass $0.50, is refused with 429 *before* any upstream call. `budget.json` is persisted before each forward. Flags cannot raise the ceilings. `--live` refuses under CI. The key is read only from the passwd-home `.hermes/.env`, and the `sk-or-` redaction holds.
- **Smoke dry run.** `scripts/agy-live-smoke --dry-run` passes end to end with genuine agy: wake, `systemMessage` injection, ACK and reply, zero relay refusals, zero leaks.
- **Smoke live.** `--live` is executed only by the orchestrator, once at the final SHA. It produces `docs/evidence/agy-live-smoke/<date>-<sha12>/summary.json` with `pass:true`, `requests ≤ 40` and `cost_usd ≤ 0.50`.
- **Hygiene.** No runner writes outside its root, `--evidence-dir` and the build `target/`.
- **Accounting.** Every commit carries its `Outside-adapter:` trailer.

## Post-Implementation Notes

> *As this design is implemented and iterated on — bug fixes, adjustments, anything that diverged from the assumptions above — append a dated note here, whether or not a formal debugging skill was used.*
