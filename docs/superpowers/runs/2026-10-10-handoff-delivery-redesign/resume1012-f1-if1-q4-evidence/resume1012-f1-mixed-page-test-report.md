# F1 mixed-page negative boundary — scratch only

Tests-only append to `tests/service/normal_unrenderable_controls.rs`; exact current source and before/after/patch hashes are in the seal. Existing F1 positive controls and corrected manifest-first pending oracle are byte-preserved.

Real `OfferWindowFixture` registers a Watch lease, publishes canonical Lazy bodies through actual service Send, and pulls real Watch-rendered V2 pages. Two full bodies contain8192 and4096 NUL bytes. JSON body expansion alone is49152+24576=73728: each fits65536 individually, both do not fit together. No custom sizer, prefix, truncated marker, prior offer, clock advance, owner replay, SQL write or manufactured storage input supplies the condition.

The first actual page must emit the full first body, keep the second unoffered, report continuation, and retain the second's exact identity in bounded ordinary carry. Later actual pulls on the same lease must emit the second exact full body once and become quiet. The test checks actual serialized32/65536 bounds, sizing352, fresh-work433, carry155731, offer256/count ceilings, canonical Lazy Pending for both identities, and unchanged own-binding read cursor. Existing fixture confirmation settles only actual nonmessage control tokens; it never ACKs or marks message display.

This guards against incorrectly applying standalone-unrenderable deferral to any nonempty-page sizing failure. Actual retained carry plus later full output prove this candidate gets its next ordinary opportunity; no private deferral representation is prescribed. Unchanged-source PASS is predicted for this negative boundary, not observed by this author. The original independent starvation RED remains required and unchanged.

No candidate/source edits, Cargo/build/project runtime, SQL execution, Git mutation, process lifecycle actions, Herdr writes or nested agents. Parent owns review, apply, formatting and execution. No runtime PASS claimed.
