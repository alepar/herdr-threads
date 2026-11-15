# Manual presentation serializer fixtures

These files are exact UTF-8 bytes, including final LF, exported by production
`src/cli/watch/manual.rs::encode_page` at reviewed source 5b2713226. The opt-in
Rust test `cli::watch::manual::tests::export_manual_presentation_packet_fixtures`
produced them; ordinary Rust serializer tests pin these same SHA256 hashes.
The Pi test separately pins each hash and consumes the bytes through its actual
bounded line parser and native callback state machine. Keys are Rust-produced;
no JavaScript key/hash or numeric control stamp reconstruction participates.

| File | Bytes | SHA256 |
| --- | ---: | --- |
| manual-maximum.jsonl | 70886 | b38c97768e05e3e1e90bb11c031e7a848b8b4c923775e9fbd590ade175daa60e |
| manual-notice-clear.jsonl | 769 | 718b5f490cac71a45e9bf3c2067cdd75c6ffc5b5993ac6d5c6736476339b0c6a |

Maximum uses the full 65536-byte Watch item rendering budget, 32 maximum-u64
Attention stamps, escaping, and a 2802-byte opaque receipt. Notice/Cleared mixes
a body slot and an applied-state slot with a maximum-u64 control stamp. These
are serializer compatibility fixtures, not a daemon or native uptake run.
