# opencode wake e2e evidence

Real opencode TUIs (opencode 1.18.1, `openrouter/deepseek/deepseek-v4-pro`) in a private Herdr 0.9.5 server (`scripts/wake-e2e-herdr 0.9.5 --harness opencode`).

| Directory | Tree | Result |
|---|---|---|
| `pre-fix-9980c97/` | before the PR roast fix round | 57/57 PASS |
| `final-67a5ad27d/` | final commit `67a5ad27d` | 53/57 PASS, exit 1 |
| `final-67a5ad27d-rerun/` | same commit, only the four failing cases, fresh sessions | 12/12 PASS |
| `head-ef826683a/` | merge candidate `ef826683a` (after the second review's fixes) | 56/57 PASS, exit 1 |
| `head-ef826683a-rerun/` | same commit, `send-ack` on the pair and the group, fresh sessions | 6/6 PASS |

The four failures of the full final run: `busy-send-p1/1:1`, `busy-send-p0/1:1`, `busy-send-ack/1:1` at T=3 and `mixed-order/group` at T=-1. In the three busy rows the plugin ledger (`ledger-oc-t3.jsonl`) shows the item appended and acked inside the busy turn and no wake during or after it; the row fails `delivered_in_turn` because the model's reply did not echo the item (`pane-oc-t3.txt`: it re-ran hold commands of earlier cases and printed an invented line). They were the last cases of that recipient's session. The same cases pass in fresh sessions on the same commit, so they are recorded as model behaviour in a long context, not as a delivery defect. No check was weakened.

The one failure on `ef826683a`: `send-ack/1:1` at T=-1. The plugin ledger (`ledger-oc-t-1.jsonl`, item `mFrgKkFeiutr`) shows the item appended, the wake submitted and the ACK settled on the appended row; the recipient woke in 0.25 s and the woken turn produced no visible reply, so the row fails `delivered`. None of the four rows that failed on `67a5ad27d` failed here. The case passes in fresh sessions on the same commit; recorded as model behaviour. No check was weakened.

`commands.jsonl` (4 MB per run) is left out. No file here contains the credential.
