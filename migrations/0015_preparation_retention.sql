-- Last successful preparation quantum; publication removes the live expiry key.
ALTER TABLE send_preparations ADD COLUMN prepared_at INTEGER;
CREATE INDEX send_preparations_retention ON send_preparations(prepared_at, id)
WHERE prepared_at IS NOT NULL AND status IN ('building','sealed');
CREATE TRIGGER send_manifest_preparation_published AFTER INSERT ON send_manifests
BEGIN
    UPDATE send_preparations SET prepared_at=NULL WHERE id=NEW.preparation_id;
END;
