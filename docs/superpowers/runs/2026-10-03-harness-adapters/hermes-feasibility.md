# Hermes preliminary native feasibility

2026-10-03, isolated HOME/HERMES_HOME, no model invocation. These probes establish launcher/runtime identity plumbing only; they do not establish delivered model context, cooperative interaction, recipe verification or seat continuity.

The installed Hermes source resolver reported:

```json
{"base_version":"0.21.5","derived_version":"0.21.5+3962.g37daf85","commit":"37daf85b2ad0ee50ed45d7234dc47b7fa24cec09","dirty":false,"distance":3962,"source":"git"}
```

The exact development identity must remain distinct from release `0.21.5`. The resolver ran with the installed launcher's Python interpreter and source import root, with bytecode writes disabled and optional Git locks disabled; exit 0. An initial guessed virtualenv interpreter path was absent; inspection of the actual launcher corrected the probe without invoking a different PATH interpreter.

The installed published launcher also answered `--print-runtime-command` with a JSON argv array (four elements), exit 0. It binds the store Python interpreter, isolated `-I` mode, source root and Hermes bootstrap. This native machine boundary is preferable to parsing shell-wrapper quoting or assuming a `.venv` layout. Its current implementation is `hermes_cli/_launchers.py::print_runtime_command`; qualification must verify the same boundary in any supported distribution rather than generalizing from this one source installation.

Official and installed native hook catalogs distinguish fail-closed `pre_tool_call` callbacks from fail-open `post_tool_call` observers. The bridge uses the latter for nonconsuming evidence. Native tool callbacks omit parent-session evidence; exact session/turn role association must come from an earlier qualified `pre_llm_call`. Missing association remains unattributed.

Probe children exited. Successful probe temporary homes were removed; no shared Herdr process, real user configuration or model credentials were accessed or changed.
