# Upgrade40 epoch startup work correction

Current measured source: task-15-healthy-channel-sqlite-sourcepoint.json, full manifest206da0b40bfca65fd53916c6ef6130e85bf9cc0c0ac9f02d6d2a910a059072c7, dirtydiffa4a29ff118d217cc2f8117340146a8951bad70a8a2554dd5562d8d5cfcd67c01. Source SQL is migrations/0040_invitation_presentation.sql:4, exact `ALTER TABLE invitations ADD COLUMN command_context_epoch INTEGER NOT NULL DEFAULT 0;`.

Actual registered combined test `invitation_content::released_v39_invitation_epoch_alter_records_nested_work_and_preserves_state` initializes all released migrations1..39, inserts FK-valid retained invitation graphs at1/32/128 cardinality, executes that exact ALTER with linked SQLite3.51.3, and preserves old values, catalog/FK state and default epoch values. It records outer statement VM steps AND progress-handler work including nested statements. It passed in the four-test owned twohost selection; build37.09s, total runtime1.309s. Rawlog `/private/tmp/ht-task15-healthy-channel-sqlite-batch.log`, SHA256483ebdac11a4360da75fc8de5de88336a7db9f3c7bfe52f3869176d0a96c9610.

| Retained invitations | Outer VM steps | Progress including nested work |
|---:|---:|---:|
|1|1913|5951|
|32|1913|8245|
|128|1913|15354|

Outer-only counters do not establish row-independent upgrade work. The earlier simplified-table48-outer-VM evidence and its metadata-only/no-retained-row-validation conclusion are explicitly superseded for the actual released invitation relation. Exact source remains byte-preserving/nonrebuilding ADD COLUMN, with no added CHECK, projection seeding, history rewrite or epoch algorithm change. The existing STRICT invitation table causes sqlite3AlterFinishAddColumn to invoke named-table pragma_quick_check. This validates retained invitation rows/types/existing CHECK expressions and invitation index pages/bytes, with separate catalog/schema work; it does not imply an all-retained-table scan. Runtime presentation bounds and healthy migration deadline semantics do not change.

Proposed TRUST replacement (awaiting fresh schema verification and exact parent disposition):

> Upgrade40 constructs exact ordinal-seek indexes over the existing canonical pending-invitation projection. Its index startup cost scales with indexed pending rows; it seeds no new projection and rewrites no invitation history. The invitation command epoch uses byte-preserving ADD COLUMN with DEFAULT0 and no added CHECK. On the actual released STRICT invitation relation, linked SQLite invokes named-table quick_check: one-time read validation covers retained invitation rows/types/existing CHECK expressions and invitation index pages/bytes, plus catalog/schema work. No constant-time, no-row-read or all-retained-table startup guarantee is claimed. Deadline processing resumes after healthy migration without deadline resets. Canonical readers/writers reject negative epochs and checked increments reject exhaustion atomically. New presentation tables enforce their own bounded-state constraints.

Proposed migration comment correction only: `ADD COLUMN preserves retained invitation bytes without rebuilding. The existing STRICT table invokes named-table quick_check over invitation rows/types/existing CHECKs and invitation index pages/bytes, plus catalog/schema work; no added epoch CHECK is installed. Epoch range/exhaustion are checked by canonical readers/writers.`

No runtime/schema behavior change is proposed. Fresh schema reviewer must verify the actual nested measurement and exact original SQL before the source comment/TRUST correction is adopted. Existing1..39 SQL bytes remain immutable.
