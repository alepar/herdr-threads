# ht-iygh restart-restore e2e (20261127T151551Z)

- expect: `fixed`  result: **PASS**
- candidate: `/var/home/alepar/AleCode/herdr-threads/.worktrees/ht-iygh/bin/herdr-threads` (herdr-threads 3.6.0)
- herdr: `/var/home/alepar/.local/bin/herdr` (herdr 0.9.5)

- PASS [both, gates] pre: A has a seat (Codex hook enrolled): `seat=s760FsylCIen session=01a21d93-c7e5-7f32-a9eb-19b771480f3c`
- PASS [both, gates] pre: B has a seat (stand-in SessionStart): `seat=s760FwCjspgd hook_rc=0`
- PASS [both, gates] pre: A resolved: `resolved`
- PASS [both, gates] restart: new Herdr server process: `pid 4148612 -> 4149364`
- PASS [both, gates] restart: pane ids kept: `A=w1:p1 B=w1:p2`
- PASS [both, gates] restart: terminal ids new: `before={'w1:p1': 'term_661438921898b1', 'w1:p2': 'term_66143892191c02'} after={'w1:p1': 'term_6614389e3bc1c1', 'w1:p2': 'term_6614389e3bfa82'}`
- PASS [both, gates] restart: pane A runs `codex ... resume <session>`: `/var/home/alepar/.local/bin/codex --no-daemon --approve-for-me --dangerously-bypass-hook-trust resume 01a21d93-c7e5-7f32-a9eb-19b771480f3c`
- PASS [both, gates] restart: herdr agent get A agent_session == session: `agent=codex agent_session=01a21d93-c7e5-7f32-a9eb-19b771480f3c`
- PASS [info, not gating] observe: codex stayed idle (no prompt sent): `idle`
- FAIL [current, not gating] current: A's seat unresolved, not restored: `state=resolved decisions=['host_session_continuity']`
- PASS [current, not gating] current: B's seat unresolved: `state=unresolved`
- PASS [fixed, gates] fixed: A restored by host_session_continuity, no operator step: `state=resolved decisions=['host_session_continuity']`
- PASS [info, not gating] observe: daemon health: `unresolved_seats=1 held_targets=1`
- PASS [info, not gating] observe: resumed Codex hook-trust gate answered: `gate_seen=False cleared=True`
- FAIL [current, not gating] current: fresh Claude in B refused (unresolved seat mapping): `rc=0 out='{"hookSpecificOutput":{"additionalContext":"Complete hook/channel deliveries and default text inbox displays ACK eligible receipts automatically after output is written and flushed. Do not reread or re-ACK them. Call inbox only for continua'`
- PASS [fixed, gates] fixed: fresh Claude takes B's seat by pane_continuity: `state=resolved decisions=['pane_continuity'] seat_at_B=s760FwCjspgd`
- PASS [both, gates] send: person sender seat made after restart: `rc=0 seat=s7606YuZaHDo `
- PASS [info, not gating] send: p1 accepted by the CLI: `rc=0 {"result":{"data":"m760NemOb2BR","kind":"message_sent","selection":{"invitation":null,"outcome":"already_joined"},"thread":"dy43gZbAYuO"},"version":9}`
- PASS [fixed, gates] fixed: p1 does not wake A: `wake=None`
- PASS [current, not gating] current: p1 does not wake A: `wake=None`
- PASS [info, not gating] send: p0 accepted by the CLI: `rc=0 {"result":{"data":"m760Ner9ijf8","kind":"message_sent","selection":{"invitation":null,"outcome":"already_joined"},"thread":"dy43gZbAYuO"},"version":9}`
- PASS [fixed, gates] fixed: p0 wakes Codex in A: `wake=106.6`
- PASS [fixed, gates] fixed: A's seat resolved and connected after the wake: `state=resolved connected=True`
- FAIL [current, not gating] current: p0 does not wake A (seat unresolved, never woken): `wake=106.6 send_rc=0`
- PASS [info, not gating] send: A's seat after sends: `state=resolved connected=True decisions=['host_session_continuity']`
- PASS [fixed, gates] fixed: daemon health shows 0 held targets: `held_targets=0`
- PASS [info, not gating] final: daemon health: `unresolved_seats=0 held_targets=0`

Facts: see facts.json; raw JSON and pane captures alongside.
