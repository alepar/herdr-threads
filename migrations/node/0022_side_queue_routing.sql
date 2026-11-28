-- Queue wake routing with Codex /side sessions present (ht-z41e.7). The intent's target is the
-- process's root session (target_json.native_session); this records the other top-level sessions
-- of that process (never targets) as a JSON array of native session ids, NULL when there are none.
-- Diagnostic only: fulfillment still matches target_json by equality.
ALTER TABLE wake_intents ADD COLUMN side_sessions TEXT;
