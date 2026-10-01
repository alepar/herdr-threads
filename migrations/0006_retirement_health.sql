CREATE INDEX retirements_failed_pending ON retirements(ordinal)
WHERE status='pending' AND last_error IS NOT NULL;
