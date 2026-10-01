# Codex 0.158.0 hook-input fixtures

Copied from `docs/evidence/codex-158-hook-capture/payloads/`.
They are **schema-conformant, not live captures**: each satisfies the hook input
schema embedded in the Codex 0.158.0 binary (required keys, no extra keys,
enum/const values; see `fixture-schema-check.out.txt` there). Only the
SessionStart `startup` key set is corroborated by a live 0.158.0 capture
(`prior-live-0.158.0-sessionstart.sanitized.jsonl`). The embedded input/output
schemas are byte-identical to 0.157.1's, which is why 0.158.0 shares the
`codex-hooks-v1` recipe. `05-sessionstart-fork.json` is valid native input that
the adapter deliberately refuses (fork is known-unsupported: never captured live).
Live 0.158.0 payloads for every owned event are in `../codex-0.158.0-live/`.
