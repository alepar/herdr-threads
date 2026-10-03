#!/bin/sh
# usage: db_dump.sh OUTDIR   read-only dumps of the smoke instance database (no message text, no narratives)
DB=/private/tmp/ht-summary-smoke/state/instances/9639634cd008ed0f1c61a379dd666732a771ab7f17494af1af87036b37bf06a6/threads.sqlite3
HT=/private/tmp/ht-summary-smoke/ht
OUT=$1
mkdir -p "$OUT"
sqlite3 -readonly -header "$DB" "select b.thread_id, b.level, b.idx, b.first_seq, b.last_seq, b.id as block_id, b.author_seat_id, b.model, b.prompt_version, b.fallback, b.job_id from summary_blocks b order by b.thread_id, b.level, b.idx" > "$OUT/db-summary-blocks.txt"
sqlite3 -readonly -header "$DB" "select thread_id, count(*) jobs, sum(block_id is not null) stored, sum(rejections) rejections, sum(attempts) attempts from summary_jobs group by thread_id" > "$OUT/db-summary-jobs.txt"
sqlite3 -readonly -header "$DB" "select seat_id, thread_id, frontier_seq, datetime(entered_at/1000,'unixepoch') entered, datetime(extension_until/1000,'unixepoch') extension_until, state, end_reason, datetime(ended_at/1000,'unixepoch') ended, release_seq from catch_up order by entered_at" > "$OUT/db-catch-up.txt"
sqlite3 -readonly -header "$DB" "select seat_id, reason_bits, retry_step, effective_delay_ms, datetime(last_reserved_at_utc/1000,'unixepoch') last_wake, last_outcome from wake_work" > "$OUT/db-wake-work.txt"
$HT seat list > "$OUT/seats.txt" 2>&1
$HT daemon health > "$OUT/daemon-health.txt" 2>&1
$HT doctor > "$OUT/doctor.txt" 2>&1
cp "$(dirname "$DB")/daemon.log" "$OUT/daemon.log"
