# Design review 2 disposition

R2-F1: D4 now sets replay-needed on every unretained eligible body, including discarded incoming overflow as well as displaced resident bodies. Any later capacity release schedules one fenced canonical replay; full buffers do not cause reconnect loops, failed replay preserves the flag with backoff, and new drops re-arm it. The required regression exhausts old-watch re-offers for lower-ranked overflow without any resident displacement, then releases space and verifies eventual replay.

This is a design repair, not an implemented or passing regression test. The revised design is submitted to the final independent late review round.
