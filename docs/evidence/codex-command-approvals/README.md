# Codex command approval transport

Measured 2026-10-03 with Codex 0.160.0, macOS arm64, private HOME/CODEX_HOME,
workspace-write, no network proxy/socket allowance or added writable roots.
A local mock Responses provider issued the shell call; no external model was used.

The interactive TUI executed the real herdr-threads binary's JSON daemon health
command with `sandbox_permissions="require_escalated"`. A private user-layer
`.rules` file allowed only the selected CLI executable prefix. The response
contained the real daemon health envelope and ready database/schema. The private
daemon was intentionally degraded because its isolated Herdr endpoint was absent.
An owned SessionStart hook independently called the same health command and
persisted a valid health envelope before the model's tool call.

Controls: replacing that command rule with `forbidden` produced a tool rejection;
removing the rule and selecting approval `never` rejected the explicit escalation.
Each control retained successful hook-side health. Private hooks used the explicit
hook-trust bypass solely for this vetted fixture; production setup never trusts
hooks automatically. Other commands were not granted outside-sandbox execution.

Codex `exec` was also checked: it forces approval `never` even when `on-request`
is supplied through config or the top-level flag, and rejects explicit escalation.
A second exec probe issued an ordinary shell call without `sandbox_permissions`
or `justification`: the CLI-only allow rule ran it outside the sandbox and returned
the real health envelope; a forbidden rule rejected it; no rule kept it sandboxed
and returned exit 4 / transport_denied. An exec agent must use a preapproved rule,
not request escalation. This measures execution, not automatic model compliance
with guidance. Launch preserves native policy;
forbidden/unavailable approvals must be reported rather than bypassed.

Every private Codex process group and daemon was terminated/reaped, and private
HOME/config/rules/hooks/workspace paths were removed. Raw logs are retained outside
the public source tree; they include transient local paths.

Official behavior reference: [Codex command rules](https://learn.chatgpt.com/docs/agent-configuration/rules).
