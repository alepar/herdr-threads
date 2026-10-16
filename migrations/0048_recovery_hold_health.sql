-- Health reads only open holds in the current recovery epoch, independently
-- of released targets and open holds retained from previous incarnations.
CREATE INDEX recovery_holds_current_open ON recovery_holds(instance_id,baseline_boot,baseline_epoch,target_id) WHERE released_at IS NULL;
