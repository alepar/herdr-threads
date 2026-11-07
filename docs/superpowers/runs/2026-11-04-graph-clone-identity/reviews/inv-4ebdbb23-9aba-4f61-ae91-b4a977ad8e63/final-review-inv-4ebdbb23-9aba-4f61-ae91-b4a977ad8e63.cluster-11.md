# Final review, cluster 11 (src-host), epic ht-ma6i

Source: base cf13ed76c1c2871b321ad290d03229cb785c400e, head 0755e609de0328e6d73584dd65eb2854e1fd7bf7. Both resolve as commits. The caller checkout was at the head with a clean status when read; it was not switched or modified apart from this file and the authorized ledger binding.

Scope: `src/host/mod.rs` (slices/000038.diff), 5 added lines from one commit (7a87f5209): the `CONTINUITY_KEY_ENV` constant and its entry in `child_env_scrub_list`.

## What the change must do

Spec section 4.2: the variable is `HERDR_THREADS_CONTINUITY_KEY`, and threads' own `launch` and `handoff` scrub it from what they start. TRUST-POLICY *Key inheritance outside threads* (line 2186) states the claim precisely: Herdr `agent.start` carries argv only, and the launch-gate shim, the detached daemon and a peer launch child drop the key.

## Trace of the scrub claim

- The name matches the spec. It has one Rust definition; `cli::caller_location` re-exports it and reads it through `CallerEnv::from_lookup`. The Claude mod and Hermes bridge spell the literal because they cannot import it.
- Detached daemon: `daemon/lifecycle.rs:458` applies `child_env_scrub_list`, so the key is removed. `internal daemon-detach` inherits that scrubbed environment. `detached_daemon_environ_drops_child_env_scrub_list` sets the key through `SCRUB_NAMES_PROBE` and reads `/proc/<pid>/environ` of the real child.
- Launch-gate shim: `gate_shim_scrub_list` filters only `hc::ENV_VARS`, so the key stays on it. `scrub_names` also removes any ambient `HERDR_*`, so the shim has two independent removals. `handoff_scrubs_the_continuity_key` asserts the removal on the prepared command.
- Herdr `agent.start`: a socket request from `host/native.rs` with no environment field in the host requests inspected, so the client's environment does not travel with it.
- herdr-core `terminal.run`: `RunRequest.env` comes from `directory_context.env`, the adapter's config roots, or is empty. The key is not among them, and the terminal runs the shim in any case.
- Peer launch child: `service/peer_launcher::child_env` is an allowlist that does not include the key; the same test asserts it.
- Other production spawn sites read: `harness/hermes/runtime.rs:62` clears its environment; `host/named/tmux.rs`, `daemon/control.rs` (`ps`) and the version and shell probes are short-lived helpers that present nothing to a daemon. `cli/update.rs:784` runs the installer in the caller's environment; a key there reaches only that installer, which matches the accepted inheritance limit.
- Test helpers: `SCRUBBED_PREFIXES` includes `HERDR_`, so `test_support::spawn` already removes the key from test children, as AGENTS.md requires.
- Harness neutrality and wire compatibility: the file names no harness and adds no wire type.
- Compile shape: `.chain([&CONTINUITY_KEY_ENV])` promotes the const reference to `'static`. `cli/caller_location.rs:180` uses the same form.

## Requirement baseline, for this cluster

- B1: the cluster supplies the variable name and the scrub. Neither blocks presentation: the agent's own environment is untouched, only children threads starts. Presentation, reservation and reattachment leave this cluster and are the seam reviewer's to trace.
- B2, B3: this file has no retirement or membership behaviour. Those rows leave this cluster and are the seam reviewer's to trace.
- B4: the change adds no Herdr dependency or request; it is an environment scrub on the threads side only.

## Verdict
ready

## Must fix before landing
- none

## Untested scope
- The three focused tests named above were read at the head, not re-run by this review, which avoided a separate build; their pass on this tree rests on the caller's full-suite run.
- That a real Herdr server starts an agent without the caller's key rests on the code read of the `agent.start` request; spec 4.2 places live harness environment filtering in the real-Herdr qualification, outside this run.

## Deferred OK
- `src/host/mod.rs:30` still calls the list "the union of each host family's names" and says handoff spawns remove all of it, while the key is not a host family's name and handoff drops it through the shim and the peer allowlist; TRUST-POLICY K5 (line 168) likewise says only "pane- and terminal-scoped variables". Comment wording only; behaviour matches the policy's limit entry.
