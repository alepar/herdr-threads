# agy delivery fixtures

Used by `tests/harness/agy_delivery.rs`.

- `transcript-*.jsonl`: transcript excerpts. No genuine `transcript_full.jsonl` was recoverable from
  the research probes (only the model requests, which carry the injected `<SYSTEM_MESSAGE>` wrapper
  text, were recorded), so these are SYNTHETIC: one JSON object per line, with the wrapper text of
  probe `runs/excerpts.txt` around a D4 frame. The scan is schema-agnostic (it matches the frame
  header line `[herdr-threads] message|lazy <id> in ...` in the raw bytes), so only the frame text
  matters. `transcript-truncated.jsonl` ends mid-string with invalid UTF-8.
- `tool-output-line.jsonl`: one tool-output line; the test repeats it to push a frame past the old
  fixed tail and past the 8 MiB bound.
- `stop-*.json`: Stop payloads (shape of `payloads/stop-*.json`; `@TRANSCRIPT@` is replaced by the
  test). `stop-error-unknown-step.json` is the recorded probe error (`tui-04`); the cancelled
  `terminationReason` value is synthetic.
- `output-*.json`: byte-locked hook stdout for the text in `output-text.txt` (no trailing newline).
- Identity layouts (symlinked binary, launcher-wrapped binary) are built in a temp dir by the test:
  git does not keep portable symlinks.
