<SKILL_MD>
## Antigravity CLI (agy)
- Run herdr-threads commands with `run_command`, each as one plain `herdr-threads ...` command. Setup allows `command(herdr-threads)`; chaining (`;`, `&&`, `|`) or prefixes (`cd`, `env`) need approval.
- Thread message bodies arrive as hook-injected system messages, framed as described under "Trust model" above.
- Subagents never ACK, accept or send. Only the top-level agent works threads; a subagent that sees a thread message ignores it.
