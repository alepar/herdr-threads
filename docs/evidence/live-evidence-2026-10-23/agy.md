# agy: opt-in real-model case in the wake e2e

## What was added

`scripts/wake-e2e.py --harness agy` (opt-in; the default matrix stays `claude,codex`). An agy recipient per
threshold (`ag-t0`, `ag-t-1`, `ag-t3`) runs every selected case, judged like Codex (native content-free wake, a
hook-delivered page, so `mixed-order` is judged at T = 0):

- agy runs with `HOME` at the run's scratch home, where `setup agy --with-permissions` installs the plugin; its
  `settings.json` is seeded first the way the agy gate seeds it (`modelProvider: gemini`, the agents' working
  directory in `trustedWorkspaces`, `altScreenMode: never`), so setup merges its permission member into that file.
- Credential: `GEMINI_API_KEY` from the driver's environment (plus `GOOGLE_GEMINI_BASE_URL` when set, which is how
  the agy relay is reached). It goes to the pane through a mode-0600 env file in the scratch root that the launch
  line sources, never on a command line or screen, and it is redacted from evidence like the Claude credentials.
  No Google sign-in is read or used (a real Google login is a non-goal of the agy design).
- The first-run screens (colour scheme, Terms of Service) are answered with the same keys as
  `scripts/agy-gate/live_smoke.py` `walk_onboarding` while the driver waits for Herdr to detect agy.
- Preflight refuses (exit 2, before any process starts) without `agy` on PATH or without the credential.

Driver checks: `python3 scripts/wake-e2e.py --self-test` (164 tests, OK), including `AgyOptIn`: opt-in only, the
refusal, the launch line never carrying the key, private env and settings files, redaction, onboarding keys, the
`> ` wake-prompt echo, waiting for the composer, and removal of the env file on every cleanup path (also `--keep`).
agy starts are serial, like Codex: the recipients share one scratch HOME.

## Run on this host: not possible, refused as designed

```
$ python3 scripts/wake-e2e.py --harness agy --cases send-p0/group --evidence-dir <scratch>
wake-e2e: refused: agy not on PATH; no agy credential in the environment: --harness agy needs GEMINI_API_KEY (no Google sign-in is used)
exit=2
```

Why not:

- No agy on blacky. A Linux build exists (the auto-updater manifest `linux_amd64.json` lists agy 1.4.6,
  `cli_linux_x64.tar.gz`), but `scripts/agy-sandbox/fetch-agy` accepts only `darwin_*` platform keys, and
  `scripts/agy-live-smoke` refuses non-darwin hosts (its loopback sandbox is `sandbox-exec`).
- No Gemini credential here. The existing real-model agy evidence (`docs/evidence/agy-live-smoke/`, agy 1.3.2,
  `google/gemini-2.5-flash`) used the OpenRouter relay (`scripts/agy-relay`) with a key read on macOS. The user
  pointed to an OpenRouter key on the laptop; the coordinator is asking the user where it lives and whether the
  relay may run on blacky, and no key is copied between hosts until then.

To run it once a credential is available (the relay as the endpoint, or a Gemini API key directly):

```sh
export GEMINI_API_KEY=...                         # or the relay's token
export GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:N  # only with the relay
python3 scripts/wake-e2e.py --harness agy --cases send-p0/group,send-p1/group,send-ack/group
```

Unverified until that run: whether agy 1.4.x accepts `GEMINI_API_KEY` against Google's own endpoint without a
sign-in screen (the relay path is the one measured before), and its first-run screens on Linux.
