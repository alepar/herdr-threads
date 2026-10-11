-- Capture a finite repair workload once, without rewriting the send-time cutover.
-- Every visited physical/prepared ordinal consumes the worker's row allowance.
CREATE TABLE receipt_deadline_backfill (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    physical_after INTEGER NOT NULL CHECK(physical_after>=0),
    physical_high_water INTEGER NOT NULL CHECK(physical_high_water>=physical_after),
    prepared_after INTEGER NOT NULL CHECK(prepared_after>=0),
    prepared_high_water INTEGER NOT NULL CHECK(prepared_high_water>=prepared_after),
    completed INTEGER NOT NULL CHECK(completed IN (0,1)),
    CHECK(completed=0 OR (physical_after=physical_high_water AND prepared_after=prepared_high_water))
) STRICT;

INSERT INTO receipt_deadline_backfill
SELECT 1,0,COALESCE((SELECT MAX(ordinal) FROM receipts),0),
         0,COALESCE((SELECT MAX(ordinal) FROM prepared_recipients),0),0;
