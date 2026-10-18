# Peer wire fixtures 2.1.1

- release: 2.1.1
- at: v2.1.1
- commit: fd959d7aa47b4a728cd6f5dddc0022b663bfd12b
- generator: tests/support/peer_wire_freeze.rs (sha256 52f4a1e25132ea5671e7cdfcb61c4aa4f614c7191d120c33ceeb49fac1c978e6)

These files are what release 2.1.1's own codec emits for deterministic inputs (seeded keys, fixed HLC, fixed hello nonce and boot); they are not a recorded live session.

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
