ALTER TABLE messages ADD COLUMN user_intent TEXT
    CHECK (user_intent IN ('query', 'request', 'rule'));
ALTER TABLE summary_transitions ADD COLUMN rule_change TEXT
    CHECK (rule_change IN ('withdrawn', 'replaced'));
ALTER TABLE summary_jobs ADD COLUMN fetched_bundle_json TEXT;

CREATE TRIGGER messages_user_intent_insert
BEFORE INSERT ON messages
WHEN NEW.user_intent IS NOT NULL
BEGIN
    SELECT CASE WHEN COALESCE(
        NEW.kind = 'ordinary' AND
        (NEW.author_role = 'human' OR
         (NEW.author_role = 'agent' AND NEW.relays_user = 1)), 0
    ) = 0 THEN RAISE(ABORT, 'invalid user intent attribution') END;
END;
