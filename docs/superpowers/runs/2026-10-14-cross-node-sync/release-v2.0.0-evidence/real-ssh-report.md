# real-ssh two-node check d33d5f32

| row | result | detail |
|---|---|---|
| local-bin/no-key-backoff | PASS | peer add rc=0; row={'target_id': 1, 'ssh_dest': 'tester@ht-rs-d33d5f32-local-bin-b', 'source': 'explicit', 'enabled': True, 'state': 'backoff', 'stopped_reason': None, 'remote_command': None, 'node':  |
| local-bin/default-command | PASS | precondition (not on PATH, present at ~/.local/bin): True; target: {'target_id': 1, 'ssh_dest': 'tester@ht-rs-d33d5f32-local-bin-b', 'source': 'explicit', 'enabled': True, 'state': 'linked', 'stopped_ |
| local-bin/converge | PASS | A=[('Ipq', 1027291150, 1, 'b538656c'), ('Ixq', 3750788387, 1, 'e13c50be')] B=[('Ipq', 1027291150, 1, 'b538656c'), ('Ixq', 3750788387, 1, 'e13c50be')] |
| local-bin/send-ack | PASS | message mIxqq1r9FpEJ on B: True; reply mIpqxvrYL8ME on A: True |
| local-bin/daemon-stop-reaps-ssh | PASS | ssh children before stop: ['192']; after: []; zombies: [] |
| local-bin/no-leaks | PASS | leaked: {} |
| on-path/default-command | PASS | precondition (on PATH: '/usr/local/bin/herdr-threads', none at ~/.local/bin): True; target: {'target_id': 1, 'ssh_dest': 'tester@ht-rs-d33d5f32-on-path-b', 'source': 'explicit', 'enabled': True, 'stat |
| on-path/converge | PASS | A=[('OjH', 2035406326, 1, '9a7dac70'), ('uK8', 3094045025, 1, '81efb8c1')] B=[('OjH', 2035406326, 1, '9a7dac70'), ('uK8', 3094045025, 1, '81efb8c1')] |
| on-path/explicit-remote-command | PASS | peer add rc=0; wrapper marker on B: True; target: {'target_id': 1, 'ssh_dest': 'tester@ht-rs-d33d5f32-on-path-b', 'source': 'explicit', 'enabled': True, 'state': 'linked', 'stopped_reason': None, 'rem |
| on-path/no-leaks | PASS | leaked: {} |
| cleanup/containers | PASS |  |

