# Default unknown-usage footer correction

The integrated remaining-case attempt on `3ad10e775` passed native lifecycle,
compaction and public handoff, then timed out waiting for a channel-disabled
native wake. The original recipient's manual turn and settlement preceded the
send; its host metadata was idle, unfocused and interactively ready. Its styled
editor was empty with the cursor at column zero. The captured default footer was
`?/100k`, while the parser required `%/`. The failed attempt and clean owned
cleanup remain preserved; the timeout is not credited as a pass.

The retained official native footer renderer emits `?/capacity` when context
usage is unknown after compaction (`footer.js`, lines 173–175), and numeric
`percent%/capacity` otherwise. A zero-usage scripted provider kept this normal
unknown state after the following turn. The captured plain/ANSI pane now supplies
a regression fixture. It failed before the correction because the blocked-screen
recognizer returned unknown instead of the known default editor.

The Pi-local parser now accepts the exact default unknown-usage token at a token
boundary. Numeric percentages still require a finite number; the unreachable
`?%/` spelling is rejected. Full matching rule widths, one editor row, current
cwd/stats footer and styled inverse cursor at column zero remain required.
No neutral core, scheduler, trust rule or extension delivery behavior changed.

All 15 Pi library tests pass. Draft, whitespace, multiline/wrapped and built-in
model/settings/inline selector captures refuse with both numeric and synthesized
unknown-usage footers. Unsupported footer spellings and mixed borders also refuse.
Independent review approved frozen patch SHA-256
`a9c8db29245e2de8b32a3e3d763995307b8e8cad9548b1b666bd116a20be409c`.
The warm candidate rebuild passed in 20.72 seconds.

The focused actual native compaction→channel scenario passed against candidate
SHA-256 `198e81ff9307f3e88c29cd47ba8ae425d0ed4f6283e5bb80a676636058895cce`:
public compaction, default unknown-usage composer, channel-disabled native wake,
no false context receipt, explicit inbox fallback receipt and restored channel
reload. Cleanup verified helper reaping, scratch-root removal and exact-run
process/temp leak status 0 with no errors. This new-binary focus and the 15 fixture
guards qualify the narrow production delta. Earlier broad native cases remain
explicitly identified as compatibility evidence on the preceding binary; they
are not represented as a second full matrix on this binary. Live and final suite
results are separate final evidence.
