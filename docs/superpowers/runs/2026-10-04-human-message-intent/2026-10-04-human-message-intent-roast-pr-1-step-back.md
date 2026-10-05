decision: patch
summary: Add bounded cleanup of completed bundle snapshots after their valid replay window; the single retention gap does not justify replacing the approved frozen-input design.
pattern: r1 [Should-fix] src/store/summary.rs:1388 identifies one lifecycle omission: cumulative snapshots survive after lease-fenced replay ends. No recurring findings or compensating-fix pattern exists in this first code round.
clusters: none
