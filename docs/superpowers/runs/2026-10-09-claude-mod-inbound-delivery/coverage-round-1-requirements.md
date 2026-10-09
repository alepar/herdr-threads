R1 A bundled Claude Code mod runs a session-long herdr-threads watch child in every Claude session in a Herdr pane (Claude >= 2.1.287).
R2 The watch child streams the seat's pending ordinary messages, lazy messages and non-message attention as bounded JSON lines, pushed by the daemon when attention changes.
R3 The watch channel registers only against the seat's current open cooperative_top_level Claude binding with matching native session (A2); it never enrolls, allocates or rebinds.
R4 While a main turn runs, messages are delivered between tool calls as tool-result context.
R5 While idle, messages are delivered as a new turn via $.prompt.submit; never while a main turn is open; held after an interrupted turn.
R6 Lazy messages are delivered passively via $.session.append without starting a turn.
R7 Each fully delivered ordinary message settles its receipt with provenance cooperative_mod_delivery; lazy rows complete as displayed; truncated or unknown items stay pending.
R8 While the mod channel is live the daemon sends no native send-keys wake, poke or hook attention digest for that seat; a stalled channel lets the native ladder run.
R9 When the mod is not connected (disconnect, crash, kill switch, old Claude, reload gap) delivery falls back to hooks plus native wake, losing and duplicating nothing.
R10 The mod survives /clear, /resume and reload: it rebinds to the new session id and restarts the watch child.
R11 setup claude installs the mod reversibly via env.CLAUDE_CODE_PLUGIN_DIRS under the owned manifest; unsetup removes it; setup-status reports it.
R12 TRUST-POLICY.md defines the new provenances, the routing change and accepted limits in the same change that introduces them.
R13 Repeated race stress tests (unit randomized and live TUI) show no submit while busy, no duplicate delivery or ack, and no lost message.
