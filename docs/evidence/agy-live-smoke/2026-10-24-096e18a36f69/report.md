# agy live smoke (live)

- SHA: `096e18a36f690bd8880c004728fb5535b71866cb`
- agy: 1.4.6, sha512 `f1982f560c40dbd7`
- herdr: herdr 0.9.5
- model: google/gemini-2.5-flash; main requests: 6; cost: $0.01434975
- ACK phase: None
- result: **FAIL**

## Phases

- enroll: ok in 10.4s
- send: ok in 1.6s
- wake: ok in 8.0s
- act: failed in 180.2s
- fallback: failed in 180.5s

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
- [ ] the agy seat ACKed through run_command (tool calls=[{'name': 'read_file', 'args_trimmed': '{"Path": "<root>/home/.gemini/config/plugins/herdr-threads/skills/herdr-threads/SKILL.md"}', 'seq': 3}, {'name': 'run_command', 'args_trimmed': '{"toolSummary": "Reply to herdr-threads message", "WaitMsBeforeAsync": 5000, "CommandLine": "herdr-threads send dPC4UBJsrsZ --body \\"SMOKE-OK\\""}', 'seq': 4}, {'name': 'run_command', 'args_trimmed': '{"WaitMsBeforeAsync": 5000, "toolSummary": "Check herdr-threads inbox", "CommandLine": "herdr-threads inbox"}', 'seq': 6}])
- [x] the receipt is acked
- [x] no permission prompt: the agent was never blocked
- [x] the agy seat replied SMOKE-OK in the thread
- [x] the delivery receipt is settled (cooperative_hook_delivery, or superseded by the ACK)
- [x] no relay refusal

Leak check: clean
