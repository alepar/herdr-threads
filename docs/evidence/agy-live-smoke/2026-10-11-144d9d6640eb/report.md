# agy live smoke (live)

- SHA: `144d9d6640eb2d326de4605f1d911d1379657e45`
- agy: 1.3.2, sha512 `2657939f20f386ff`
- herdr: herdr 0.9.1
- model: google/gemini-2.5-flash; main requests: 6; cost: $0.024786460000000003
- ACK phase: wake
- result: **PASS**

## Phases

- enroll: ok in 9.5s
- send: ok in 2.8s
- wake: ok in 4.1s
- act: ok in 7.3s

## Checks

- [x] setup agy succeeds
- [x] agy onboarding walked to the composer
- [x] the agy seat is enrolled
- [x] agy reached idle after the READY prompt
- [x] the sender created a thread
- [x] the sender invited the agy seat
- [x] the agy seat joined the thread (runner scaffolding)
- [x] the ACK-required message was sent
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
