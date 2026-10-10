use super::*;
use crate::harness::adapter::IdleWake;

struct Policy(DeliveryMechanism);
impl DeliveryPolicy for Policy {
    fn mechanism(&self) -> DeliveryMechanism {
        self.0
    }
    fn provenance(&self) -> &'static str {
        "test"
    }
    fn idle_wake(&self) -> IdleWake {
        IdleWake::Native
    }
}

#[test]
fn hook_pull_keeps_the_lifecycle_digest_and_omits_the_tool_digest_whatever_the_channel() {
    let pull = Policy(DeliveryMechanism::HookPull);
    for channel_live in [false, true] {
        assert!(!omit_digest(Some(&pull), channel_live, Boundary::Lifecycle));
        assert!(omit_digest(Some(&pull), channel_live, Boundary::Tool));
    }
}

#[test]
fn push_channel_and_no_policy_omit_exactly_while_the_channel_is_live() {
    let push = Policy(DeliveryMechanism::PushChannel);
    let policies: [Option<&dyn DeliveryPolicy>; 2] = [Some(&push), None];
    for policy in policies {
        for boundary in [Boundary::Lifecycle, Boundary::Tool] {
            for channel_live in [false, true] {
                assert_eq!(omit_digest(policy, channel_live, boundary), channel_live);
            }
        }
    }
}
