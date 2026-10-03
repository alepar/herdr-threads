R1 Compaction, resume or clear of a top-level agent yields hook text naming its hot threads and the thread-summary procedure (Codex compact/resume/clear; Claude resume/clear, plus compact once evidence admits it)
R2 Accepting a thread that holds at least one full chunk shows a summary hint
R3 `herdr-threads summary <thread>` returns Ready (displayed cover plus raw tail up to a frozen frontier) or Work (leased jobs), planned by the daemon
R4 Agents run summary jobs with parallel small-model workers per the shipped skill procedure; the daemon validates submissions deterministically and stores immutable blocks reused by every reader of the thread
R5 Long threads roll up (fan-in 8, index-aligned) so the displayed summary fits about 10k tokens, with recent history at level 0
R6 User instructions (human-authored or relayed) are carried verbatim or by pointer and never dropped across levels; identifiers are daemon-extracted; open items carry forward mechanically
R7 Messages record author_kind and relays_user, and `send --relays-user` exists
R8 Catch-up holds pushed attention for messages after the frontier (warnings, invitations and priority messages bypass), and ends on Ready, stall or binding change
R9 Effective receipt deadlines extend by p99 job time per stored progress during catch-up; warnings and overdue use the effective deadline; senders see the deferral
R10 A soft-deadline poke reaches an eligible unfocused native agent before the deadline and is skipped whenever it is unsafe; composer stash and poke-during-turn only with captured evidence
R11 The hard-deadline overdue warning to joined seats keeps working, firing on the effective deadline
R12 TRUST-POLICY.md records derived_summary provenance, the deadline-extension fact, the poke rule and who may call the summary commands
R13 Native evidence on both harnesses shows the summary flow end to end and the poke behaviour
