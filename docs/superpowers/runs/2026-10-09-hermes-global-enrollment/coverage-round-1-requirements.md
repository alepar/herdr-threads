R1 A bare Hermes CLI session in a Herdr pane whose selected profile has herdr-threads installed and enabled gets the pane's seat without `launch --kind hermes`.
R2 That session's first qualified top-level turn receives the herdr-threads context through a lifecycle check-in, as a launched one does.
R3 Delegated Hermes children (and non-CLI platforms) never enroll or check in.
R4 Every TRUST-POLICY guard (restore holds, collisions, A2 canonical decision) still applies; no new A3 provenance; TRUST-POLICY.md changes in the same commit as the behavior.
R5 Claude and Codex enrollment behavior is unchanged.
R6 A Hermes `/new` reset keeps the same seat (Clear on the same seat).
R7 Hermes runs outside Herdr stay silent.
R8 User-facing docs, help and skill text describe Hermes global enrollment, its limits (no resume continuity, macOS wake) and the re-setup step; no "launch only" claim remains.
R9 The launched Hermes path (`launch --kind hermes`, process hint) keeps working.
R10 An older installed Hermes bridge that does not send the enrollment witness keeps working (launch-only behavior) rather than being refused.
R11 `setup hermes --profile NAME` installs or refreshes the bridge that sends the enrollment fields, and status flags an older installed bridge.
R12 A bare-enrolled first-turn check-in has the same context form as a launched one.
