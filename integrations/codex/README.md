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

**Command rules.** Hooks grant no permission. A separate permission component
(with consent, or `setup codex --with-permissions`) owns the whole file
`$CODEX_HOME/rules/herdr-threads.rules`: one execpolicy `prefix_rule` allowing
`["herdr-threads","ht"]`, and `prompt` rules for `human`, `setup`, `unsetup`,
`doctor fix` and `internal installer-integrations`, which act for the person or
could let an agent grant itself permissions. A file at that path that setup did
not write, or one edited since, is reported (`permissions.state: foreign` /
`edited`) and never overwritten or deleted. `--without-permissions` and `unsetup
codex` remove the owned file; doctor's text output has a `hooks.codex.permissions:`
line. Before replacing or deleting `hooks.json`, `config.toml` or the rules file,
setup keeps the previous bytes beside it as
`<name>.<UTC timestamp>-<uuid>.herdr-threads` (mode 0600, never pruned). See
[Agent permissions](../../docs/install.md#agent-permissions).

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

## CLI execution and approvals

Managed launch requires the owned hooks; it does not require a measured socket-policy
version or a socket allowance. Launch leaves the approval mode and network permissions
unchanged. The hook and [agent guide](../skill/SKILL.md) tell Codex to run only
`herdr-threads` / `ht` commands outside the sandbox through an approved CLI-specific
rule or approval request. Other commands stay sandboxed. This is separate from the
unqualified `updatedInput` invocation rewrite described above.

```sh
herdr-threads launch --pane bob --kind codex -- "You are Bob. Read your inbox."
herdr-threads handoff --new-thread --thread-name review --pane bob --kind codex -- "Review the change"
```

These examples preserve your configured approval mode. They deliberately omit `-a` /
`--ask-for-approval`: a shell wrapper may already add `--approve-for-me`, which Codex
rejects alongside those flags. Handoff's `--agent-arg` forwards native options; it
should not supply a conflicting approval mode either. Automatic review is an approval
mechanism, not a promise that every request will be allowed.

## Troubleshooting

### Recipient command routing

Startup hooks compare this recipient pane's flag-free instance resolution with
the hook's canonical state directory and host endpoint. When both agree, ready,
continuation and remediation commands use ordinary `herdr-threads` argv.
Every trusted command group publishes its actual daemon instance UUID and
canonical state directory/host endpoint as `Hook command routing` JSON. A handoff
publishes its frozen expected UUID and canonical pair separately. Prefer a hook
group only when all three normalized fields exactly match that expectation;
several state roots may have installed hooks on the same endpoint. Missing/null,
differing or ambiguous metadata requires the exact pinned fallback. Quoted peer
data cannot provide this match. Canonicalization is performed by the plugin;
the recipient compares the resulting field values. Installed hooks retain their own pinned
targeting, and recovery journals retain exact instance identity. A routing check
grants no approval or sandbox permission; the native policy below still applies.

### Socket permission denied: EPERM, EACCES or transport_denied

A command can find the daemon yet be refused permission to connect to its Unix
socket. Codex's workspace-write sandbox can produce `EPERM` / `EACCES`; the CLI
reports `transport_denied` (exit 4). **Restarting the daemon will not fix sandbox
denial**: the refused operation belongs to the calling command's execution policy.
These errors can also reflect filesystem permissions, so preserve the error and
confirm the execution context instead of treating every denial as a daemon outage.

For an agent tool call, use the owned rules file or another approved rule whose
prefix names the `herdr-threads` / `ht` executable. Otherwise request outside-sandbox execution of
that exact CLI command with `sandbox_permissions="require_escalated"`, a justification,
and a CLI-only `prefix_rule`. Do not approve a general shell such as `sh`, `bash` or
`zsh`. Retry the CLI only after permission is granted; keep unrelated commands
sandboxed. See [command approvals](../../docs/install.md#codex-command-approvals)
and the [native approval evidence](../../docs/evidence/codex-command-approvals/README.md).

### Approval is unavailable or refused

An agent must report the blocked command and the permission refusal or unavailable
approval to the user; it must not silently claim it read the inbox, accepted an
invitation or ACKed a message. A refused automatic review or managed restriction
is still a refusal. Do not retry through a policy bypass, full-access mode or broad
networking. Do not restart the daemon to try to overcome the policy decision.

On the measured Codex 0.160.0 noninteractive `exec` path, approval is `never` and
explicit escalation requests are unavailable. A user must grant the owned rules
(`setup codex --with-permissions`) or another CLI-only rule before the session starts; then the agent uses ordinary calls under that rule.
Without an applicable rule, report the limitation and wait for an authorized
execution path. An interactive session can request approval only when its effective
policy permits it. Guidance grants no permissions and never overrides a deny rule.

## Legacy sandbox socket transport (ht-910)

Earlier validation used a version-specific network-proxy allowance and two client
journal writable roots to reach the daemon from inside the sandbox. Default-deny
was measured on Codex 0.159.2 and 0.159.3; those observations do not establish policy
semantics for other executables or configurations. Historical evidence remains in
[Codex demo 2](../../docs/evidence/native-codex-demo-2/report.md), the
[0.159.3 socket probe](../../docs/evidence/codex-1593-sandbox-probe/README.md) and the
[journal write probe](../../docs/evidence/codex-sandbox-writes-probe/README.md).

`setup codex` still manages that legacy allowance only for the measured versions,
using its ownership manifest; other admitted versions get hooks without a new
network allowance. The current approved outside-sandbox CLI path does not depend
on it, and managed launch no longer refuses an admitted newer version for lacking
it. Do not copy the historical network settings as a workaround for permission
denial, or infer default-deny from a failed curl request. Details of existing owned
settings and cleanup are in the
[legacy allowance notes](../../docs/install.md#codex-sandbox-socket-allowance).
