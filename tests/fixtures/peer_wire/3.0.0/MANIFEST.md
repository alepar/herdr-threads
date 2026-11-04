# Peer wire fixtures 3.0.0

- release: 3.0.0
- at: 5a4e28abe
- commit: 5a4e28abed09f6337a49b506827b948742b3a085
- generator: tests/support/peer_wire_freeze.rs (sha256 8c8fd574bb7ecb5494b90af16657bb983c576adede0ee4d8273cdf949bb5ea91)

These files are what release 3.0.0's own codec emits for deterministic inputs (seeded keys, fixed HLC, fixed hello nonce and boot); they are not a recorded live session.

## Fact kinds

- node.genesis v1
- node.retag v1
- seat.created v1
- seat.retired v1
- thread.created v1
- thread.register v1
- thread.direct v1
- invitation.created v1
- invitation.answered v1
- membership.joined v1
- membership.left v1
- message.sent v1
- receipt.settled v1

## Files

1e604bd9a2b25697d8363abcc24a51260022b41c0957b6aaeaa8339679d567cd  envelopes.jsonl
5d5b3915e60b0c83c717348ba0950605248862edbf6a69feef4e1a6e132bfdef  session.jsonl
