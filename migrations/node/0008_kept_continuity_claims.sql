-- Kept continuity claims (ht-dtx0, TRUST-POLICY C1): a session's own
-- ContinuityCheckIn that the daemon refused only because its host evidence was
-- not ready (reconciliation lag, or a failed host read), kept so the
-- observation worker can decide it once the epoch has reconciled. One row per
-- (Herdr server, target, harness, native session); a repeated claim of the
-- same session replaces its own row. Bounded by count and age in code.
CREATE TABLE kept_continuity_claims (
    host_id INTEGER NOT NULL REFERENCES herdr_hosts(host_id),
    target_id TEXT NOT NULL CHECK(length(CAST(target_id AS BLOB)) BETWEEN 1 AND 512),
    harness TEXT NOT NULL CHECK(length(harness) BETWEEN 1 AND 64),
    native_session TEXT NOT NULL CHECK(length(CAST(native_session AS BLOB)) BETWEEN 1 AND 512),
    command_json TEXT NOT NULL CHECK(length(CAST(command_json AS BLOB)) <= 16384),
    received_at INTEGER NOT NULL CHECK(received_at >= 0),
    PRIMARY KEY(host_id, target_id, harness, native_session)
) STRICT;
CREATE INDEX kept_continuity_claims_age ON kept_continuity_claims(host_id, received_at);
