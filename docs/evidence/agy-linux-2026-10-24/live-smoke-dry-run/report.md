# agy live smoke (dry-run)

- SHA: `120060fae813238e3a885ed5f42a97f91ead23bd` (dirty)
- agy: 1.4.6, sha512 `f1982f560c40dbd7`
- herdr: herdr 0.9.5
- model: fake-upstream (scripted, no model); main requests: 6; cost: $0.0
- ACK phase: wake
- result: **PASS**

## Phases

- enroll: ok in 10.1s
- send: ok in 0.3s
- wake: ok in 6.0s
- act: ok in 0.0s

## Checks

- [x] setup agy succeeds
- [x] agy onboarding walked to the composer
- [x] the agy seat is enrolled
- [x] agy reached idle after the READY prompt
- [x] the ACK-required message was sent on the pair channel
- [x] the message is stored
- [x] wake: a new model request carried the injected SYSTEM_MESSAGE with the message id (the runner typed no prompt)
- [x] the injected SYSTEM_MESSAGE holds the frame header with the message id
- [x] no USER_REQUEST of the wake turn is a runner prompt
- [x] the agy seat ACKed through run_command
- [x] the receipt is acked
- [x] no permission prompt: the agent was never blocked
- [x] the agy seat replied SMOKE-OK in the thread
- [x] the delivery receipt is settled (cooperative_hook_delivery, or superseded by the ACK)
- [x] no relay refusal

Leak check: clean
