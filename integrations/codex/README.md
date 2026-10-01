# Codex hook declaration (recipe registry)

Status: native invocation transport and model-issued receipt are
**UNSUPPORTED** in every recipe. Daemon health reports
`harness.codex = unsupported`, derived from `harness::codex::RECIPES`: Codex
is `supported` only when every recipe proves transport and receipt.

## Supported versions

| Recipe | Installed versions | Evidence | Evidence scope |
| --- | --- | --- | --- |
| `codex-hooks-v1` | exactly 0.157.1 and 0.158.0 | [`codex-probe.md`](../../docs/compatibility/codex-probe.md); [`codex-158-hook-capture`](../../docs/evidence/codex-158-hook-capture/report.md); [`codex-158-live-hook-capture`](../../docs/evidence/codex-158-live-hook-capture/report.md) | 0.157.1: pinned source contract and native root/child PreToolUse probe. 0.158.0: all 23 embedded hook input/output schemas byte-identical to 0.157.1 (also independently re-extracted); live `codex exec` payloads for SessionStart `startup`/`resume`, SubagentStart and root/child Bash PreToolUse (`tests/fixtures/codex-0.158.0-live/`) that parse under `HooksV1`; context-only `additionalContext` delivery to the model observed for SessionStart (startup, and re-injected on resume) and PreToolUse (inserted after the tool call, before its output; the call still ran). SubagentStart output delivery not exercised. Transport and receipt not qualified |

**Schema-fingerprint admission.** An unlisted version is admitted only when
the hook JSON schemas embedded in its binary hash-match a recipe's captured
schemas (`codex-hooks-v1`: `sha256:86858f2456c999030224a92d8dfb535183fe0edf8601690d8941978fadbb066d`,
23 schemas; `harness::codex_schema`). The witness then carries that recipe and
is reported as **schema-matched, live-unverified**: no live hook recipe capture
exists for that version. Codex 0.159.2 is admitted this way (`doctor`:
`hooks.codex.installed: codex 0.159.2: schema-matched, live-unverified: ...`).
The hook keeps a private 0600 fingerprint cache keyed by binary identity under
`STATE_DIR/harness/`, so only the first hook after an install or upgrade scans
the binary. See [install](../../docs/install.md#harness-hooks).

Any other version, meaning one whose schemas do not match or cannot be
extracted within the observation deadline, is refused with a message naming
the supported recipes (0.155.1 embeds the same schemas but has no parse
evidence here; it would be admitted only through this schema-matched path);
unrecognized `--version` output names them too, and so does an `Unavailable`
observation (a missing, relative, failing or timed-out binary). A hook entry
that cannot obtain a witness converts any `VersionError` variant (including
`SchemaUnmatched` and `SchemaUnextractable`)
into `ContextError::UnsupportedVersion(<that message>)`, the same refusal the
Claude entries return.
A new version joins `codex-hooks-v1` only with captured evidence that its hook
payloads match; a version whose payloads differ gets its own recipe and
`InputSchema` variant.

`harness::codex::InstalledVersion::observe` runs an absolute Codex binary with
`--version` and yields a witness only for a canonical `codex-cli X.Y.Z` that
some recipe lists, or that the schema-fingerprint admission above matches; the
witness carries that recipe and how it was admitted. One 5 s
deadline bounds both process exit and reading stdout to EOF; a binary that
exits but leaves a descendant holding stdout is `Unavailable` and its process
group is killed. Every
public parser and setup entry takes that witness; there is no string-based or
unversioned public entry.

`harness::setup::plan_codex_for_version` builds exactly the owned hook groups
listed in `DECLARATION.owned_hooks` from an executable hook command argv:
`SessionStart` (lifecycle), `SubagentStart` (child start) and a Bash
`PreToolUse` group (matcher `^Bash$`) for tool-boundary attention context.
Every group is context-only: its output is `hookSpecificOutput.{hookEventName,
additionalContext}` and never `permissionDecision` or `updatedInput`. The
0.157.1 source, and the identical 0.158.0 output schema, accept that
`PreToolUse` shape without a decision
(`hooks/src/engine/output_parser.rs`, `hooks/src/events/pre_tool_use.rs`). A
live 0.158.0 run (`codex-158-live-hook-capture`) delivered exactly this
envelope to the model for SessionStart and Bash PreToolUse, as `developer`
messages in the persisted rollout, without blocking the tool call; the adapter
tests assert that `encode_event_context` emits the same envelope for those
live payloads. SubagentStart output delivery was not exercised, and 0.157.1
output delivery was not run natively. `plan_codex_for_version` returns
`-c hooks.Event=...` session arguments (a library seam). The public
`herdr-threads setup codex` instead installs the same three groups (each with
timeout 10) at user level, the way Herdr installs its own Codex hook: appended
to `$CODEX_HOME/hooks.json` (default `~/.codex/hooks.json`) through
`install_user_settings(SettingsKind::CodexUser, ..)`, with an ownership marker
in each command and a private manifest, so `unsetup codex` removes only them.
Codex also keeps the hooks of `$CODEX_HOME/config.toml`, `/etc/codex` and
trusted project `.codex/` layers (app-server `hooks/list` with a scratch
`CODEX_HOME` on 0.157.1, 0.158.0 and 0.159.2); setup reads those only to report
them and to refuse when one already holds the exact hook command. Inspection
requires the installed groups, including the Bash matcher, to match the
declaration. The tool-boundary hook entrypoint (non-durable attention read and
coalescing) is owned by the hook layer, not this adapter.

**Hook trust.** Codex runs a user hook only once trusted: `hooks/list` reports a
freshly written `hooks.json` hook as `trustStatus: untrusted` (0.159.2, scratch
`CODEX_HOME`, key `<hooks.json>:session_start:0:0`). Trusting it in Codex's
review (shown at the next interactive start, or `/hooks`) makes Codex write
`[hooks.state."<key>"] trusted_hash = "sha256:..."` into `config.toml`. Herdr's
own `hooks.json` hook is trusted the same way: Herdr writes the hook and
`features.hooks = true`, and the user's review records the hash; Herdr writes
no trust. Setup never writes or relaxes trust either; `setup-status codex`
lists the owned keys and whether a hash is recorded. Setup appends, so the
trust keys of existing hooks keep their positions. Approval policy is
untouched.

`parse_event_for_version` parses with the witness recipe's input schema
(`HooksV1` for both versions), requires a
native session, and requires the complete `SubagentStart` shape
(`session_id`, `turn_id`, `transcript_path`, `cwd`, `model`,
`permission_mode`, `agent_id`, `agent_type`). A child tool call must carry both
`agent_id` and `agent_type` or neither. `encode_context` and
`encode_event_context` return bounded `hookSpecificOutput.additionalContext`
for changed mail and empty stdout when there is no change; they never return a
permission decision, `updatedInput`, or receipt. Children can read and
summarize; the top-level model explicitly accepts invitations and ACKs exact
message IDs. SessionStart `source = fork` is valid native input in both
versions but is a **known-unsupported** input and is refused: no live fork
payload has been captured, so whether a fork is startup-like or resume-like for
check-in is uncharacterized. The live captures record `permission_mode:
"bypassPermissions"` in every payload even under `-s read-only`: under `codex
exec` it reflects the approval policy, never the sandbox. The adapter does not
read `permission_mode` and must never interpret it as a sandbox signal.

`ToolInvocation::scoped_command` is gated: it returns
`TransportError::Unsupported` unless the invocation's recipe is registered in
`DECLARATION.recipes` and records `invocation_transport` supported (none does).
`ToolInvocation` fields are private; its only constructor is
`parse_tool_invocation`, whose recipe comes from the observed version witness,
so a caller cannot supply its own recipe. Codex requires `permissionDecision: "allow"` with `updatedInput`
for a rewrite; no isolated native run has shown that such a rewrite preserves
ordinary deny/approval decisions or reaches a scoped daemon socket under the
intended policy, and the earlier captured sandbox denied the ordinary local
socket. No native receipt or SQLite gate is claimed by this artifact.

## Sandbox socket transport (ht-910)

This is separate from the invocation transport above (an `updatedInput` rewrite, still unsupported). It concerns how
the agent's own `herdr-threads` commands reach the daemon under Codex's sandbox.

The default `-s workspace-write` sandbox refuses `connect()` to the daemon socket (`EPERM`, reproduced without a model
by `codex sandbox -c 'sandbox_mode="workspace-write"' -- herdr-threads --state-dir S daemon health`). The CLI surfaces
that as `transport_denied` (exit 4) with the remedy, not `host_unavailable`.

`setup codex` writes the narrow allowance into `$CODEX_HOME/config.toml` (a structural `toml_edit` edit recorded in
its own manifest; a key holding another value refuses, a key the user already had is never removed):

```
sandbox_workspace_write.network_access = true
features.network_proxy.enabled = true
features.network_proxy.unix_sockets = { "<daemon socket>" = "allow" }
```

- `network_access=true` only starts Codex's network proxy. The proxy settings are inert without it.
- The proxy has no domain allow entries. On Codex 0.159.2 only the named Unix socket becomes reachable; external and
  loopback network stay denied.
- That default-deny was measured on 0.159.2 only, so setup writes the allowance only for 0.159.2. For any other
  admitted version (listed 0.157.1/0.158.0, or schema-matched) it installs the hooks only, with `sandbox.omitted` and a
  warning: on a build that ignored `features.network_proxy`, `network_access=true` would mean unrestricted networking.
  Check a version first with the negative control, which must fail:
  `codex sandbox -c 'sandbox_mode="workspace-write"' <allowance> -- curl https://example.com`.

The same allowance adds the instance's two client-side journal directories to
`sandbox_workspace_write.writable_roots` (`<instance>/intents` and `<instance>/contexts`, as owned array members).
Every mutation and check-in writes a pending-operation intent and the caller context there first. Without the roots,
workspace-write refuses those writes (`Operation not permitted`) whenever the state directory is outside the workspace
and tmp, as it is on a real install. The database and daemon files stay read-only. The
[write probe](../../docs/evidence/codex-sandbox-writes-probe/README.md) on 0.159.3 measured this.

[Codex demo 2](../../docs/evidence/native-codex-demo-2/report.md)
verified this before use. The allowlisted socket connected. Other Unix sockets, the Herdr server socket, loopback and
external TCP were refused (`EPERM`), and proxied HTTPS got 403. Under the allowance the model accepted and ACKed its
exact message itself (manifest PASS, `cooperative`).

The daemon socket path is stable per instance, so the allowlist survives daemon restarts. A boot is identified by
the boot ID in the endpoint descriptor and in every response, not by the filename. The path is specific to one
state directory and one Herdr instance (host endpoint).

Setup writes the allowance only for a measured Codex version (0.159.2), for the detected or given Herdr instance.
`herdr-threads launch --kind codex` adds no hook or sandbox arguments: it requires the user-level installation and,
under a sandbox that needs it, the recorded allowance for this instance's socket; otherwise it refuses unless the
arguments after `--` choose full access explicitly (`-s danger-full-access`). See [install](../../docs/install.md#codex-sandbox-socket-allowance).
