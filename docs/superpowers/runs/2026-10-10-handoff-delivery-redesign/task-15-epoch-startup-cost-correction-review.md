# Upgrade40 epoch startup cost correction: independent focused review

Spec: CLEAN. Quality: CLEAN. Scope ONLY the proposed accepted-limit/comment/TRUST correction in task-15-epoch-startup-cost-correction.md SHA2562a845cc555287d13069c8d5d24c0577b51535971c2c6528dbc48b2f6ffd208c2. This is not final D source/schema approval, permission to change the epoch algorithm, or runtime/readiness evidence. Parent exact ruling remains required before the proposed accepted-limit text is adopted.

## Evidence inspected

Read complete correction artifact and actual test source `tests/store/invitation_content.rs` SHA256b6b76187d9762252c7ce92f69fe9458ab65930fa5e2e1c7db59b4e38d1067d3b. Read actual raw log `/private/tmp/ht-task15-healthy-channel-sqlite-batch.log` SHA256483ebdac11a4360da75fc8de5de88336a7db9f3c7bfe52f3869176d0a96c9610. The specific released-v39 epoch test PASS is present: 1/32/128 invitation rows, outer VM1913 for each, nested progress5951/8245/15354. The broader four-test batch contained a separate ordinary-ACK channel failure; this review makes no whole-batch PASS claim.

Test initializes exact released1..39 SQL, constructs FK-valid retained invitation/membership graphs, enables/verifies FK before data, extracts the exact40 epoch ALTER, and verifies all original invitation values, pending projection values, other catalog objects/rootpages, invitation rootpage/FK definitions, FK check and default-zero new values remain unchanged. user_version stays39 because this is one ALTER probe, not a full40/41 migration test. The per-connection progress handler interval1 surrounds actual execute and sees nested work; outer statement VmStep is reported separately. Callback lifetime/unregistration is guarded and no test calls back into SQLite from the progress callback. Samples establish actual measurements, not exact asymptotic constants or all-table work.

Actual dependency source is bundled libsqlite3-sys0.37.0 selected by rusqlite0.39.0 with bundled feature. Cached sqlite3.c SHA2569512509b1bccb7461f79bea8aad6280ae4699e925fa4804381b71f59e7efb0c5 declares SQLite3.51.3/source737ae4a34738ffa0c3ff7f9bb18df914dd1cad163f28fd6b6e114a344fe6d618. Direct source audit:

- sqlite3AlterFinishAddColumn119451–119611 modifies sqlite_schema SQL then reloads schema. Its119585 condition includes existing-table TF_Strict independently of any added CHECK, and generates named-table `pragma_quick_check(zTab,zDb)`.
- tableSkipIntegrityCheck142992 selects ONLY the specified table. Named-table pragma semantics144279 and generated root inventory144340–144372 cover that table and its index B-trees, not every retained table.
-144455 onward checks each named-table row header, NOTNULL/type constraints, and144630 existing CHECK expressions. Quick mode omits subsequent per-row index-key cross-checks but retains B-tree/index-page integrity work. Catalog/schema reload/parse remains separate.

Consequently no added CHECK does NOT imply no row scan on the released STRICT relation. Corrected wording honestly accepts one-time retained invitation row/index-page/byte validation plus catalog/schema work; it does not infer machine work solely from row-count samples, claim exactly I VMsteps, claim an all-database retained-row scan, or hide it in runtime quantum budgets. Existing fixed historical schema/index fanout bounds the structural categories; larger retained content/catalog still costs startup work.

## Disposition

The proposed correction explicitly supersedes simplified48-outer-VM/no-retained-row-validation inference. Keeping exact ADD COLUMN behavior, no rebuild/history rewrite/projection/epoch-policy change, immutable1..39, and runtime H/D/I fairness after successful migration is coherent. Existing40 SQL SHA83e7348a425907dc7ad55021960812e933f4126cdaacefbb7f90d28382118326 was inspected; only its misleading explanatory comment is proposed to change. No workaround or new algorithm is needed for this cost correction.

Remaining final D review still requires full frozen cumulative source, actual40→revised41/39→41/reopen/rollback evidence, final guards/serializer/probes/VM proofs, private-loader I1 correction and exact U+10FFFF sentinel qualification for ONLY the two after-thread cursor fields. No final CLEAN or merge recommendation is supplied here.

Reviewer ran no Cargo/source/git/bd/thread mutation; only this review artifact was written.
