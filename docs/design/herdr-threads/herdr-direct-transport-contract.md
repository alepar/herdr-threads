# Herdr direct transport contract

Report-only source evidence; no review counter change. Root adoption required before implementation. Cutoff: 2026-09-28T05:07:43.757776+00:00.

## Decision and provenance

The preferred direct Unix socket route in adopted assessment 7eeddd9 has a concrete pinned source contract. Local official-origin checkout `/private/tmp/herdr-plugin-v091` is clean, HEAD and exact tag `v0.9.1` resolve to `065ef9d6a531c49fb8bee7e818ef837065b21ee9`; origin is `https://github.com/herdrdev/herdr.git`. Installed `~/.local/bin/herdr --version` reports 0.9.1. This establishes release-source correspondence by version, not a reproducible binary/source build attestation or live server version. No network retrieval was necessary. Immutable source links use [this commit](https://github.com/herdrdev/herdr/tree/065ef9d6a531c49fb8bee7e818ef837065b21ee9).

Applied `~/.codex/skills/herdr/SKILL.md`, checked HERDR_ENV=1 in a non-login shell, read installed help. Read-only `herdr status` was denied by the sandbox (Operation not permitted), so no current server compatibility claim is made. No native mutation, launch, restart, prompt, configuration edit, tests, or source implementation occurred.

## Proven automation socket exchange

All source references below are relative to the pinned checkout.

- `src/api/client.rs:30–98,157–189`: automation uses a new local connection per request, compact serialized JSON followed by LF, then one JSON response line. No length prefix, bincode, binary hello, or authentication token appears in this exchange. `src/ipc.rs:36–43` maps the path to a Unix filesystem local socket. The separate `herdr-client.sock` terminal protocol (`src/protocol/wire.rs:20–35,1604–1640`) uses binary framing and is not this API.
- `src/api/server.rs:157–199,270–302`: server reads one initial line, parses Request, dispatches, writes one response line, and returns for ordinary requests. Do not pipeline ping plus operation on a single connection. Subscription and graphics methods have separate streaming branches; they are outside this package.
- `src/api/server.rs:27–32,514–575`: initial request nominal cap 1 MiB; initial timeout five seconds; polling interval100ms; stream write timeout five seconds. Exact implementation checks LF before its size check, and tests deadline only on Pending reads. Therefore these server limits are not substitutes for the adapter's strict inclusive byte cap and absolute deadline under continuously available input. Client read_line has no response-size bound. No general response maximum was found in the inspected request client/server path; the adapter must impose its own configured response cap and fail on overflow without truncating into a successful snapshot.
- `src/api/schema.rs:35–47`: request `{ "id": "unique-call-id", "method": "...", "params": {...} }`. ID is a string, flattened method enum. `src/api/schema/response.rs:24–48`: success `{ "id": "...", "result": { "type": "...", ... } }`; failure `{ "id": "...", "error": { "code": "...", "message": "..." } }`. Require exact matching ID, exactly one success/error branch, expected result type and bounded field decoding. The upstream client parser does not itself compare IDs; adding correlation is adapter validation, not a claimed server feature. Malformed requests can yield invalid_request with an empty ID (`server.rs:177–194`); treat that as failed exchange, never successful correlation.

## Version and discovery

`src/cli.rs:769–802` performs a separate ping/status exchange before the actual request. `src/api/client.rs:77–94` sends `{ "id":"api-client:status", "method":"ping", "params":{} }`; server `src/api/server.rs:340–354` returns pong with version, protocol and capabilities. `src/cli/protocol_guard.rs:16–43` requires exact protocol equality; this source protocol is22. There is no automatic JSON-connection handshake or version field in the ordinary Request. Match this compatibility check without adopting restart advice as permission to restart anything. A successful ping on a different connection is not proof of unchanged server incarnation at subsequent submission; keep that evidence gap explicit and retain existing epoch/identity fences.

`src/session.rs:157–180`: explicit session selection uses its session path; otherwise HERDR_SOCKET_PATH wins; otherwise active HERDR_SESSION/default path. Default API path is config_dir/herdr.sock; named session is config_dir/sessions/NAME/herdr.sock. `src/config/io.rs:30–35` honors XDG_CONFIG_HOME. The terminal client override is not the API path. Resolve and validate the authorized target once at the process boundary and pass the explicit path into the adapter. Do not inherit changing ambient selection during a call, select a focused pane implicitly, create/remove the server socket, or start a missing server. A socket pathname alone is not server boot evidence.

## Exact request mapping

| Operation | Method and params | Expected result |
|---|---|---|
| Status | `ping`, `{}` | `pong`, version/protocol/capabilities |
| Full snapshot | `session.snapshot`, `{}` | `session_snapshot`, snapshot |
| Explicit pane | `pane.get`, `{ "pane_id":"w4:p1" }` | `pane_info`, pane |
| Prompt | `agent.prompt`, `{ "target":"explicit-target", "text":"..." }` | `agent_prompted`, agent |
| Guarded start | `agent.start`, `{ "name":"...", "kind":"...", "pane_id":"...", "args":[], "timeout_ms":30000 }` | `agent_started`, agent and argv |

References: `src/api/schema.rs:74–75,134–137,180–181`; `src/api/schema/common.rs:34`; `src/api/schema/agents.rs:167–184`; `src/api/schema/response.rs:51–53,99–119`; CLI pane normalization at `src/cli/pane.rs:82–98`. Require already normalized explicit pane IDs rather than duplicating implicit context resolution. Start timeout schema documents greater than3000 and at most300000ms. Prompt wait is optional; omitted wait dispatches directly (`src/api/wait.rs:177–194`); wait semantics are not native ACK or invocation attribution. These are wire mappings, not permission to enable launch/wake despite missing native proof. Existing agent_pane_busy-only prestart-refusal and unknown-after-possible-start rules remain in force; this package does not broaden the set of safe-retry errors.

## Minimal implementation contract for adoption

Task11 owns a single request exchange and the owned socket. Use nonblocking connect and bounded encode/write/read/decode governed by one absolute operation deadline and cancellation. The preliminary ping plus operation must share the overall budget, with separate connections as the source requires. Bound JSON input/output before allocation growth; reject incomplete/oversized/malformed/unexpected/correlation-failing responses. Never retry or reconnect an operation implicitly. No subprocess, shell, generic process-group cleanup, abandoned thread, or detached drain is needed. If a worker is used, close/drop all its owned socket handles and join before reporting local completion; keep Task17's physical slot through that boundary.

Submission classification must remain conservative: once any operation request bytes may have been sent, a cancellation, deadline, disconnect, malformed response or missing reply cannot prove non-submission. Close the local transport and return an unknown outcome for possible prompt/start submission. This does not cancel server-side work, revoke a prompt, kill agents, or prove explicit ACK. Before any operation bytes are sent, a local compatibility/connection/encoding failure may be classified separately. A correlated error requires existing semantic classification, not blanket retry.

Future evidence after adoption: deterministic private socket fixtures for connect/write/read stalls, continuous bytes across deadline, partial LF frame, over-cap reply, malformed/foreign-ID reply, cancellation at each phase and late reply; observe actual task exit/socket closure and Task17 capacity crossing. Separately authorize an isolated pinned Herdr read-only ping/pane/snapshot compatibility check. This report has not executed that check. Prompt/start native validation, subscriptions, response-connected host incarnation, coherent enumeration authority, current execution attribution, BOTH-harness transport and native ACK remain distinct gates. ht-910, F6, Task31/34 and all original review counters are unchanged.

## Source hash manifest

These files were inspected at relevant implementation sections; this is a source-contract investigation, not a fresh cumulative roast audit.

- `src/api/client.rs` — SHA-256 `b5e023619d20e36df21d969ea973b80665d05deae8069cfb0375e8897dd83e91`.
- `src/api/server.rs` — SHA-256 `b49bac502083765f8958ec31ee90bf29e6b93a5434f4e96e196e2b156d94efea`.
- `src/api/schema.rs` — SHA-256 `80afed689fbba2bc4adf8e11c0549e6d09a6c24e0cab3494bb88cd37e4027f78`.
- `src/api/schema/response.rs` — SHA-256 `d25bfce1e06f018371b261167ed1f1cd9853798f0c1052eec323b5e9d3d554f2`.
- `src/api/schema/agents.rs` — SHA-256 `50cc48302f1cccdddd532afa3335c5f68e7cebb2e9d10c43cf4cfa4aa55fe07b`.
- `src/api/schema/common.rs` — SHA-256 `aacc11e403f5f73fe61260c97231aeffa635f65c64c3f31cc12ef1bb0f330984`.
- `src/api/wait.rs` — SHA-256 `9110fa66622b3ccdb98c206204db3509364092135dab0ff903c96b8f8acf93c7`.
- `src/session.rs` — SHA-256 `6efe1629aefe0a9d6d15d5c7014e8d80e302fc682143627c7f43981c3c679dc6`.
- `src/config/io.rs` — SHA-256 `a4b7447c874f9f7d10d93485dd6f1e8fd8d44d1933a0492f1e5208d6d06b099b`.
- `src/ipc.rs` — SHA-256 `dd6ac4ae5f0658c350cb8fa0c73dd74ca91b6e06da4c2bb8dd78c8b5d0bbb85d`.
- `src/cli.rs` — SHA-256 `f9a2659b548125ae17333d12c7a920c38a33160088abc0df31d3c1ce19148158`.
- `src/cli/protocol_guard.rs` — SHA-256 `8472348778dbd63abc83501af6cf41e7713b284fc56935ca10d95173a4b458eb`.
- `src/cli/pane.rs` — SHA-256 `726a96461ca94f05461aa77778546a106dcd26dd827c1c55c3c0190197a83116`.
- `src/protocol/wire.rs` — SHA-256 `8f816b7791865ddc48628055045f1678898e4a8b30cafdfa83ef380955e566f8`.

Installed binary SHA-256 `5fc7a7e7adfaca56fa80aa89dcb025693357268dab8285b9ce2d08a2313c89de`.
