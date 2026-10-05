# README handoff rehearsal — 2026-10-04

The README “Try it yourself” scenario passed locally with the ordinary v0.2.6 candidate, Claude Code 2.1.289 and Codex 0.160.0 on macOS arm64 / Herdr 0.9.1. This used actual interactive agents in two new owned panes and an actual human shell pane. The existing v0.2.5 daemon spoke the same wire protocol 6; the shared Herdr server and user configuration were unchanged.

The human checked in, named the empty panes `alice` and `bob`, and ran the README's two handoffs with their exact assignments. Both native starts succeeded without adding approval-mode or broad-network flags. Both recipients accepted their invitations and read their assignments. The human sent the exact question with receipt requests for both panes. Canonical receipt records show four required assignment/question pairs ACKed by the addressed seats with `cooperative_top_level` and `cooperative_inbox_display` provenance. Alice and Bob each posted a recommendation and agreed on spaces with formatter/toolchain exceptions. Both captured and terminal `read review --follow` ran successfully and stopped with Ctrl-C, exit 0. History remained readable. The owned agent panes were closed; an environment-scoped process inspection found no survivors.

These ACKs prove receipt under the cooperative model, not agreement or task completion. Agreement was observed separately in the agents' messages. This is an initial-prompt native run, not evidence for prompt-less recovery, every profile, or Linux integration. The summarized measurement is in [result.json](result.json); raw status, screen, history and canonical receipt artifacts are retained privately, with the canonical-result hash recorded here.

## Permanent regression

[The model-free README integration test](../../../tests/integration/readme_tryout.rs) runs the public CLI and a real private daemon. It covers same-tab names, both handoffs, invitation acceptance, exact required recipients, read-only JSON/machine inboxes, displayed-only ACK provenance, and a followed conversation. External Herdr and harness version observations are scripted; no model is invoked. It is registered in the existing integration target and runs in the ordinary all-target/all-feature suite:

```sh
nice cargo test --locked --all-features --test integration readme_tryout
```

Restoring the original generated newline made the test fail at the first Alice handoff with the real native-argument validation error. The corrected bootstrap passes; the standalone handoff unit regression also demonstrates RED/GREEN. Keep native README rehearsal separate from the fast test when validating changes to launch guidance.
