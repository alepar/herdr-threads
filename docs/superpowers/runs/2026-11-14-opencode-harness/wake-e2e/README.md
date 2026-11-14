# opencode wake e2e evidence

Real opencode TUIs (opencode 1.18.1, `openrouter/deepseek/deepseek-v4-pro`) in a private Herdr 0.9.5 server (`scripts/wake-e2e-herdr 0.9.5 --harness opencode`).

| Directory | Tree | Result |
|---|---|---|
| `pre-fix-9980c97/` | before the PR roast fix round | 57/57 PASS |
| `final-67a5ad27d/` | final commit `67a5ad27d` | 53/57 PASS, exit 1 |
| `final-67a5ad27d-rerun/` | same commit, only the four failing cases, fresh sessions | 12/12 PASS |

The four failures of the full final run: `busy-send-p1/1:1`, `busy-send-p0/1:1`, `busy-send-ack/1:1` at T=3 and `mixed-order/group` at T=-1. In the three busy rows the plugin ledger (`ledger-oc-t3.jsonl`) shows the item appended and acked inside the busy turn and no wake during or after it; the row fails `delivered_in_turn` because the model's reply did not echo the item (`pane-oc-t3.txt`: it re-ran hold commands of earlier cases and printed an invented line). They were the last cases of that recipient's session. The same cases pass in fresh sessions on the same commit, so they are recorded as model behaviour in a long context, not as a delivery defect. No check was weakened.

`commands.jsonl` (4 MB per run) is left out. No file here contains the credential.
