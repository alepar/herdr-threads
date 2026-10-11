-- DP8: a per-seat wake preference (`nudge` keeps today's wake policy; `all`
-- also wakes for pending lazy messages) and the lazy frontier a wake reserve
-- has already covered. Neither changes receipts, ACKs or authority.
ALTER TABLE seats ADD COLUMN wake_on TEXT NOT NULL DEFAULT 'nudge' CHECK(wake_on IN ('nudge','all'));
ALTER TABLE wake_work ADD COLUMN last_lazy_ordinal INTEGER CHECK(last_lazy_ordinal IS NULL OR last_lazy_ordinal>0);
