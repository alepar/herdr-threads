-- `locator_host`: the Herdr server of a pane `locator_hint`, as the caller reported it; a hint,
-- never authority. NULL for a terminal hint and for a caller with no server to report.
ALTER TABLE occupant_bindings ADD COLUMN locator_host TEXT;
