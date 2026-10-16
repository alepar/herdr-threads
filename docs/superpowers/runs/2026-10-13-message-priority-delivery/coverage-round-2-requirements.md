R1 A sender sets a message's delivery priority 0..=9 (`-p N`, `--nudge` = 0, default 1), frozen per message across retries.
R2 A seat sets its nudge threshold T (`nudge-threshold`, `wake-on` alias), default 0, negative = -1.
R3 An idle seat is woken by a pending item iff its priority <= the seat's current T (p0 always; T<0 everything); busy sessions are never nudged.
R4 Receipts, deadlines and overdue warnings exist only for explicit --require-ack recipients; an ACK request implies p0 and refuses p>0.
R5 Every new invitation is p0, always nudges and requires acceptance by a deadline; no lazy invitations.
R6 1:1 direct pairs join both seats on creation with no invitation.
R7 Inbox pages list pending items in (priority, deadline none-last, created) order across all sources, bounded, with a continuation that resumes the order.
R8 Hook pages, watch drains and the Claude mod inject pending items in the same order.
R9 Model-visible framing uses kind `message` with a `[pN]` marker only when N != 1; lazy markers are gone.
R10 Old clients and old daemons keep working (ordinary<->p0, lazy<->p1, capability-gated) and stored data migrates (ordinary->0, lazy->1, wake_on all->-1/nudge->0).
R11 AGENTS.md delivery contract, TRUST-POLICY, skill, READMEs and agent docs describe priority, threshold, ordering and pair auto-join.
R12 The pre-release wake e2e matrix covers priorities x thresholds x ACK/invitation/1:1.
R13 Old clients keep working against a new daemon (legacy send mode -> priority, legacy wake-on -> threshold, items readable) and the new daemon handles and advertises the new commands/capabilities.
R14 In-flight pre-v47 state (pending receipts, pending lazy invitations, pending pair invitations) is handled as the spec decides, verified by a migration test.
R15 One mixed pending set comes out in the same order on every surface, verified end to end.
R16 Store/daemon decisions no longer read delivery_mode; legacy rows behave by the priority model.
