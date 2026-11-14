// herdr-threads plugin for opencode (epic ht-vxdp).
//
// Placeholder: the plugin body lands with the setup and delivery tasks. `herdr-threads setup
// opencode` embeds this file as `plugins/herdr-threads.js` under the opencode config root.
//
// Hook command contract (src/harness/opencode/decode.rs): the plugin runs
// `herdr-threads ... hook-event opencode <Event>` with one JSON object on stdin,
// `{"event", "source"?, "session_id", "parent_session_id"?, "cwd"}`, for the events
// SessionStart (source startup|clear|resume|compact), UserPromptSubmit and SubagentStart.
// Stdout is the context text itself (UTF-8, no envelope, at most 4096 bytes); empty stdout,
// a non-zero exit or a timeout means inject nothing.
