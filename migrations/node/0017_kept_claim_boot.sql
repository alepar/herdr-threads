-- Kept continuity claims carry the kernel boot they were kept under (ht-22wz, TRUST-POLICY C1),
-- stamped from node_kernel_boot beside the claim's liveness fingerprint, as bindings do. A claim
-- whose boot is not the running one, or is unknown (NULL, as on every row an older daemon kept),
-- is dropped undecided: a reboot can repeat a pid and start tick.
ALTER TABLE kept_continuity_claims ADD COLUMN liveness_boot TEXT
    CHECK(liveness_boot IS NULL OR length(liveness_boot) BETWEEN 1 AND 64);
