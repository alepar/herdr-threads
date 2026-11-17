# Native qualification on the v3.3.0 baseline

The ten selected cases passed on the v3.3.0 candidate built from `9f69b8d02`
on main baseline `4f2a32d3a`. The stable executable was
`target/debug/deps/herdr_threads-ee429b642d86e164`, SHA256
`a63ac7a7629f29c117d1968523d65f8253aaaa1a1bfbe5e66b29e0b81ac06ea4`.
Candidate, driver, fixture and retained official runtime hashes were checked
before and after every run. Exact identities, execution HEADs, run IDs,
installation digests and selected native proof are in
[v330-native-evidence.json](v330-native-evidence.json).

Each notice case used a separate fresh private host. The six standard cases ran
in one additional fresh host, with named last. All five successful runs exited 0;
each reaped its helper, removed its owned root and passed both its own and an
outside-sandbox same-ID process/temp scan. Cleanup reported no errors.

| Fresh run | Case time (seconds) | Total including startup/cleanup (seconds) | Driver SHA prefix |
| --- | ---: | ---: | --- |
| notice_reload |17.092|25.875|3cfdfed3|
| notice_page |19.602|28.565|3cfdfed3|
| notice_wake |16.909|27.238|3cfdfed3|
| notice_hol, two sinks |64.097|73.578|66212a01|
| standard six |see below|179.209|66212a01|

The standard cases passed: setup 0.040 s, receipts 3.162 s, busy 65.113 s,
lifecycle 2.641 s, channel 32.547 s and named 66.473 s. Named exercised its actual
separate tmux/session binding, receipt ACK, p1 silence while its wake lane was
eligible, next-turn delivery and busy p0 delivery without a trailing wake.
Herdr results and named results remain separate host evidence. hc native wake
was not exercised or added.

All five successful runs admitted a fresh owned shell sender through the new
person guard: `human me init` exited 0, selected that run's own sender pane and
returned a current top-level human context. No operator override was used.
The isolated environment allowlist and blank Bash sender are fixture ownership
boundaries; this does not authorize person commands from an actual agent seat.

The first three notice positives used driver `3cfdfed3…`. A subsequent HOL run
failed during mid-case sink recovery, before protected-page successor staging:
the controller retyped the sink draft after its seed turn settled, and a host
wake was appended and submitted as
`OWNED UNREAD NOTICE SINK DRAFTherdr-threads: wake up`. That failed run remains
recorded: exit 1, total 55.457 s, clean teardown and unchanged identities. Its actual
host gate read responses were not retained, so its classifier-versus-timing
cause remains unclassified. The shared composer check/type interval, including
composer fallback, is separately documented and characterized; this is not a
passing draft-preservation
claim.

Reviewed driver commit `ef8b36efe` leaves the recovered seed sink alone and
creates one distinct genuine successor sink. The seed's clear Notice was
already protected on the same primary binding before that successor started.
Both sinks have separate panes, profiles, native sessions and traces; each
cause freezes its own recipient identity for receipt and diagnostic reads.
The successor had no addressed canonical work before its first physically
verified draft guard. The correction passed 45 driver tests, including real
canonical-DDL query coverage, stale active-sink diagnostics and interrupted
cleanup before/after successor pane allocation. No production repair or
candidate rebuild was made to obtain the corrected HOL pass.

That fresh HOL run retained old clear Notice `ec2EFUPpc75y` at ordinal 3 in its
actual protected page. New warning ordinal 5 remained pending behind the same
token, absent from that old page. One Ctrl+C released only the primary's owned
draft, with no further controller input before proof. Two actual content-free
host wake prompts began at 1794890603061 and 1794890633992 ms, 30.931 s apart.
The exact old and new Notice frames appeared in the same primary native
session's branch and context at 1794890603072 and 1794890633998 ms. The frontier
advanced from 1 to 5, and a post-proof read still found the successor sink's
distinct receipt Pending. The observed path was **later automatic wake**.
Canonical frontier advancement is settlement evidence; no unobserved confirm
response, lease observation or first-packet arrival time is claimed.

The new driver changes qualification staging, cause identity and owned cleanup;
the first three notice results retain their earlier driver identity. This is a
selected compatibility delta, not a rerun of safety/threshold or the complete
historical fourteen-case matrix. Accepted custom-overlay, whitespace-home and
unrendered extension-set text limits, plus the shared composer check/type limit
(including composer fallback), remain in
[TRUST-POLICY.md](../../../../../TRUST-POLICY.md). Historical paid
provider/ordinary delivery evidence remains scoped to
[final-live.md](final-live.md); these runs used the scripted provider, read no
credentials and made no new paid request. Full-suite and final-review results
are recorded separately.
