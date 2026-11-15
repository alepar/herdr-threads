# Manual packet fixture source review

**Verdict: Approved.** Reviewed immutable `91e233980d0ce81b4a022d9f733b0ac4e07d61fd`, limited to `tests/cli/watch_manual.rs`. No findings. Production approval at `5b2713226` is unchanged.

The extracted maximum-page builder preserves the existing actual-serializer size and exact-u64 receipt checks (`tests/cli/watch_manual.rs:41`, `:58`). Ordinary tests now hash the real `encode_page` bytes for both the maximum page and a mixed Notice/AttentionCleared page; the latter also checks both emitted slots and exact decoded control stamps (`:95`). This couples the checked fixtures to production serialization rather than a second packet implementation.

The exporter reuses these same builders and production encoder, is explicitly ignored unless selected, checks the transport bound, and writes only framed stdout (`:109`). It creates no filesystem artifacts or children. The diff contains only this test file.

I independently compared each retained `fixture-export.log` payload byte-for-byte with the shared JavaScript fixture, including its final LF. Both match:

| Fixture | Bytes | SHA256 |
| --- | ---: | --- |
| manual-maximum.jsonl | 70886 | b38c97768e05e3e1e90bb11c031e7a848b8b4c923775e9fbd590ade175daa60e |
| manual-notice-clear.jsonl | 769 | 718b5f490cac71a45e9bf3c2067cdd75c6ffc5b5993ac6d5c6736476339b0c6a |

The JavaScript interoperability tests independently pin those same hashes and retain the opaque receipt bytes. Reading that narrow coupling and fixture README was a provenance check, not a separate extension implementation approval.

Retained evidence inspected: ordinary tests 7 passed/1 ignored; selected exporter 1 passed; clippy finished in 37.70 seconds; neutral/wire guards report zero violations; final format completed in 7.74 seconds (implementer reports exit 0); scoped leak check reports no leaked processes or temporary files. No tests/builds were rerun for this review. An additional builder-side leak check was pending when the implementer sent its final status; it does not alter this source verdict.

These fixtures establish serializer compatibility and test causality. They do not establish native model uptake, daemon delivery, wake behavior, or full-suite qualification.
