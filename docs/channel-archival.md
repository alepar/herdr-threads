# Automatic channel archival

The daemon applies the quiet-channel lifecycle policy with schema23 and wire6. Older clients are refused before create/invite/send dispatch; use a matching CLI and daemon.

Eligible channels require a full uninterrupted hour of quiet activity and repeated composer-aware idle observations for joined agents. Human, unresolved or uncertain occupants and protected work keep a channel open. Archival preserves memberships, identities and durable obligations; reopening starts fresh grace.

Legacy handoffs in this instance's pending-operation journal conservatively protect their channel, or the instance while an unresolved new-thread handoff has no canonical attachment. Completed canonical handoff identities are retained so exact cleanup-only retries cannot revive protection or launch again.

Every other published pending intent, even a valid Send or ACK, currently blocks legacy coverage and therefore automatic archival. Use the normal pending-ops and retry/recovery flow; the archival importer never changes or deletes journal files. Once the record is recovered or absent, a complete fresh scan can establish coverage. Malformed, inaccessible, oversized and partially scanned journals also keep channels open.

Coverage follows the journal's immutable publication contract: intents and progress publish by atomic rename, and completion removes the intent. Each read checks file metadata stability; source generations follow directory publication, removal and replacement. Arbitrary external in-place file changes after inspection are outside that publication contract. Files confer no action authority, binding evidence or receipt provenance.

Set `auto_archive_after_ms` in the selected instance's `settings.json` to choose the grace period (default `3600000`; `0` disables automatic archival). Settings take effect on that instance's next normal daemon start. Values must be nonnegative integers within the validated clock range. Manual archive and reopen commands remain available.

The archival lane uses one composer sample per eligible seat per minute and treats gaps longer than two minutes as interrupted evidence. Continuous unrelated mutations or channels too large to refresh within that window may remain open indefinitely. A global canonical mutation revision invalidates unfinished scans without synchronously updating every joined channel. The lane reports failures through ordinary daemon health/logging and stops with the daemon's cancellation token.

Retained inactive seats stay outside the recurring observation queue. Canonical seat, binding or membership changes queue one seat for rechecking; uncertain active seats remain scheduled. Successful nonidle composer observations block that seat’s channels while unrelated quiet channels keep their grace. A missing target, host uncertainty or failed sample write breaks global qualification before the lane can make its next archival decision, so a failed write cannot restore an older positive certificate. Restarted producers begin a fresh ordered sample interval.
