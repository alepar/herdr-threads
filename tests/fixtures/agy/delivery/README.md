# agy delivery fixtures

Used by `tests/harness/agy_delivery.rs`.

- `transcript-genuine-system-message.jsonl`, `transcript-genuine-user-message.jsonl`: GENUINE records
  captured from agy 1.3.2 by the native gate `delivery` / `downgrade` flows (the isolated root path
  redacted, nothing else changed). A `systemMessage` hook injection is logged as
  `{"source":"SYSTEM_SDK","type":"SYSTEM_MESSAGE","content":...}`, a `userMessage` injection as
  `{"source":"SYSTEM_SDK","type":"USER_INPUT","content":"<USER_REQUEST>\n..."}`; the frame header sits at
  column 0 of a line of `content`.
- Every other `transcript-*.jsonl` and `tool-output-line.jsonl` is SYNTHETIC but uses the genuine record
  schema (`step_index`, `source`, `type`, `status`, `created_at`, `content`; model output is
  `MODEL`/`PLANNER_RESPONSE`, tool output `MODEL`/`GENERIC`, user input `USER_EXPLICIT`/`USER_INPUT`).
  The `transcript-header-in-*`, `transcript-forged-*` and `transcript-malformed-lines.jsonl` files are
  adversarial: a header in tool output, model output, a user-sourced step, after a literal backslash-n in a
  body, or in a record that does not decode. `transcript-truncated.jsonl` ends mid-string with invalid UTF-8.
- The scan decodes the transcript record by record, reads only hook-injected (`SYSTEM_SDK`) steps and matches
  a frame header only at column 0 of a line of their decoded text.
- `tool-output-line.jsonl`: one tool-output record; the test repeats it to push a frame past the old
  fixed tail and past the 8 MiB bound.
- `stop-*.json`: Stop payloads (shape of `payloads/stop-*.json`; `@TRANSCRIPT@` is replaced by the
  test). `stop-error-unknown-step.json` is the recorded probe error (`tui-04`); the cancelled
  `terminationReason` value is synthetic.
- `output-*.json`: byte-locked hook stdout for the text in `output-text.txt` (no trailing newline).
- Identity layouts (symlinked binary, launcher-wrapped binary) are built in a temp dir by the test:
  git does not keep portable symlinks.
