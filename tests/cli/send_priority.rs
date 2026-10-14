use super::*;
use crate::protocol::capabilities::SEND_PRIORITY_V1;
use crate::protocol::time::CallBudget;

fn p(n: u8) -> DeliveryPriority {
    DeliveryPriority::new(n).unwrap()
}

fn flags(
    priority: Option<u8>,
    nudge: bool,
    explicit_ack: bool,
    deadline: bool,
) -> SendPriorityFlags {
    SendPriorityFlags {
        priority: priority.map(p),
        nudge,
        explicit_ack,
        deadline,
    }
}

#[test]
fn resolve_matrix() {
    for (input, expected) in [
        (flags(None, false, false, false), Ok(1)),
        (flags(Some(5), false, false, false), Ok(5)),
        (flags(Some(0), false, false, false), Ok(0)),
        (flags(None, true, false, false), Ok(0)),
        (flags(None, false, true, false), Ok(0)),
        (flags(Some(0), false, true, false), Ok(0)),
        (flags(None, true, true, false), Ok(0)),
        (flags(None, false, true, true), Ok(0)),
        (
            flags(Some(2), false, true, false),
            Err("--require-ack implies priority 0; drop --priority"),
        ),
        (
            flags(None, false, false, true),
            Err("--deadline requires --require-ack recipients"),
        ),
        (
            flags(None, true, false, true),
            Err("--deadline requires --require-ack recipients"),
        ),
        (
            flags(Some(3), false, false, true),
            Err("--deadline requires --require-ack recipients"),
        ),
    ] {
        let got = resolve(input).map(DeliveryPriority::get);
        match (got, expected) {
            (Ok(g), Ok(e)) => assert_eq!(g, e, "{input:?}"),
            (Err(g), Err(e)) => {
                assert_eq!(g.detail, e, "{input:?}");
                assert_eq!(g.code, crate::protocol::results::ErrorCode::InvalidRequest);
            }
            (g, e) => panic!("{input:?}: got {g:?}, expected {e:?}"),
        }
    }
}

#[test]
fn shape_with_priority_v1_carries_the_priority_and_omits_the_mode() {
    for n in 0..=9 {
        let shape = shape(p(n), true);
        assert_eq!(shape, SendShape::Priority(p(n)));
        assert_eq!(shape.wire(), (DeliveryMode::Ordinary, Some(p(n))));
    }
}

#[test]
fn shape_without_priority_v1_maps_to_the_legacy_modes() {
    assert_eq!(
        shape(p(0), false),
        SendShape::Legacy {
            mode: DeliveryMode::Ordinary,
            lost: None
        }
    );
    assert_eq!(
        shape(p(1), false),
        SendShape::Legacy {
            mode: DeliveryMode::Lazy,
            lost: None
        }
    );
    for n in 2..=9 {
        let shape = shape(p(n), false);
        assert_eq!(
            shape,
            SendShape::Legacy {
                mode: DeliveryMode::Lazy,
                lost: Some(p(n))
            }
        );
        assert_eq!(shape.wire(), (DeliveryMode::Lazy, None));
    }
}

#[test]
fn the_lossy_notice_names_the_lost_priority_in_one_short_line() {
    let line = lossy_notice(p(4));
    assert_eq!(
        line,
        "herdr-threads: this daemon lacks send.priority_v1; priority 4 sent as lazy (p1)"
    );
    assert!(!line.contains('\n') && line.len() < 100);
}

struct Caps(bool);
impl LocalClient for Caps {
    fn supports_capability(&self, name: &str, _: &CallBudget) -> bool {
        self.0 && name == SEND_PRIORITY_V1
    }
    fn call(
        &self,
        _: crate::protocol::commands::Command,
        _: &CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, ApiError> {
        unreachable!("the probe asks only for the capability")
    }
    fn call_with_output(
        &self,
        _: crate::protocol::commands::Command,
        _: &crate::protocol::output::OutputSpec,
        _: &CallBudget,
    ) -> Result<crate::protocol::results::CommandResult, ApiError> {
        unreachable!("the probe asks only for the capability")
    }
}

#[test]
fn supports_follows_the_advertised_capability() {
    let budget = crate::cli::cooperative_budget(&crate::app::SystemClock::new());
    assert!(supports(&Caps(true), &budget));
    assert!(!supports(&Caps(false), &budget));
}
