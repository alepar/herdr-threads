# Native Codex demo 2 (ht-4is.11.3, ht-910 Codex half)

Date: 2026-09-30. The report was written by the demo agent and saved by the coordinator. This was evidence gathering only: no product source was changed, the agent wrote nothing to `~/.codex` or any aisw profile, and the shared Herdr server was not restarted.

## Verdict

- **Default `-s workspace-write`: UNSUPPORTED (transport).** The Codex seatbelt sandbox refuses the CLI's `connect()` to the daemon Unix socket with `EPERM`. The model got `daemon connection unavailable (host_unavailable)` and stopped without accepting or ACKing anything. The manifest says FAIL at S18, but the cause is transport (driver D2).
- **With one narrow documented config: PASS (cooperative).**
  - The config is `sandbox_workspace_write.network_access=true`, plus `features.network_proxy.enabled=true` with `features.network_proxy.unix_sockets={"<exact daemon socket>"="allow"}`.
  - It was verified to default-deny before use.
  - Under it, the model accepted the invitation and ACKed its exact message itself, on both initial and resume.
  - Manifest PASS 26/26, `manifest_source=cooperative`.

| # | Item | Run 1 (default sandbox) | Run 2 (narrow unix-socket proxy) |
|---|---|---|---|
| A1 | Seat invited before launch; require-ACK pending; nothing ACKed | PASS | PASS |
| A2 | Hooks run under live Codex 0.159.2 (`setup codex`, `setup_mode=cli`) | PASS | PASS |
| A2b | Hook additionalContext reached the model (rollout, read-only) | PASS: SessionStart 1×, PreToolUse 0× | PASS for initial and resume: SessionStart 1× each |
| A3 | The model accepts and ACKs its exact message itself | FAIL: transport | **PASS**. `accept thread-…`, `ack msg-7dbbc2b2-…` (initial), `ack msg-f3c9bdff-…` (resume). These were root calls, and SQLite matches. |
| A4 | Provenance | hook check-in only | accept and both ACKs are `cooperative_top_level`. The bindings are gen 2 then gen 3 on the same `native_session`. |
| A5 | Socket reachability from the Codex sandbox | **DENIED** (EPERM, reproduced without a model) | **REACHABLE**, but only the exact allowlisted path |
| A6 | Resume | BLOCKED | PASS. Same thread_id; the earlier receipt is unchanged. |
| A7 | Subagent activity | none | none (S18C/S21C PASS) |
| A8 | Tokens | 124,697 in (114,688 cached) / 726 out | 153,753 / 973 (initial); 244,486 / 1,344 (resume) |

## Setup

- Integration `57081bb`, built with `cargo build --locked`. Binary sha256 `d3fddd4d1e3d6eab34da909dd803685df518f5ae0296c7091979879d2ea7c5a9`.
- Codex 0.159.2, reported as "schema-matched, live-unverified" (recipe `codex-hooks-v1`). Herdr 0.9.1.
- Launch: `codex --no-daemon exec -s workspace-write --ignore-user-config --dangerously-bypass-hook-trust --json -m gpt-6-luna -c model_reasoning_effort="low"`, with stdin from `/dev/null`, the default prompt and no hint. The hook-trust bypass was user-approved for the scratch project only; there was no sandbox bypass.
- `--codex-profile auto` selected **codex-1** on the first attempt, with no retries.
- 3 model launches.
- Run 2 used an externally started isolated daemon, so that its exact socket path was known at launch time (driver D3).

## Socket reachability probe (`sandbox-socket-probe/`)

Under default `workspace-write`, the probe got EPERM on:
- the daemon socket;
- a negative-control Unix socket;
- loopback TCP.

DNS also failed.

Unsandboxed, every target connected.

With the narrow config:
- the allowlisted daemon socket connected, and `daemon health` returned rc 0;
- everything else was still denied: other Unix sockets, the Herdr server socket, loopback TCP and external TCP all got EPERM;
- proxied HTTPS got a 403.

## Defects

**Product**

- **P1 (High, ht-910).** No transport works under Codex's default sandbox, and `setup codex` installs none. The socket path embeds the daemon boot ID (`src/daemon/paths.rs:138-144`, `<stem>-<boot_id>.sock`), so a static allowlist goes stale after every daemon restart. The fix needs a stable socket path, with `setup codex` then emitting the narrow unix-socket allow config. Repro: `codex sandbox -c 'sandbox_mode="workspace-write"' -- herdr-threads --state-dir S daemon health`.
- **P2 (Low).** `src/client/local.rs:166-168` and `src/client/service.rs:558` map every connect error, including a sandbox EPERM, to `host_unavailable`. A permission denial should say so.
- **P3 (Low).** The ready commands embed `--state-dir`, and clap rejects an identical repeat. This is the same issue as Claude demo 3's P7.

**Driver**

- **D1 (Medium).** The PATH wrapper duplicates `--state-dir`. This is the same as Claude demo 3's D8.
- **D2 (Medium).** A transport denial is reported as a product FAIL. It should be UNSUPPORTED `transport_denied`.
- **D3 (Low).** There is no way to inject the exact socket allowlist for the driver's own daemon; this needs a `--codex-allow-daemon-socket`.
- **D4 (Low).** The S17H title says "Claude" on Codex runs.
- **D5 (Low).** `setup codex` ran under the ambient `CODEX_HOME` rather than the launch profile.

**Environment**

- **E1.** Codex wrote `[projects."<scratch>"] trust_level = "trusted"` entries for both scratch projects into codex-1's `config.toml` (206 → 438 bytes) despite `--ignore-user-config`. The agent did not edit it; the user may remove the entries. `auth.json` is unchanged. Codex also wrote new rollouts and sqlite updates.

## Hygiene

The owned tabs are closed, all daemons are stopped (including the probe daemon), and no sockets remain. The Herdr server was not restarted.

## Evidence

- `dry-run-baseline/`
- `run1-default-sandbox/`
- `run2-narrow-unix-socket-proxy/`
- `sandbox-socket-probe/`
- `codex-home-diff.txt`

Redaction: `~`, `<user>`, `$SCRATCH`, `$TARGET` and `$DRIVER_REPO` stand in for local paths.
