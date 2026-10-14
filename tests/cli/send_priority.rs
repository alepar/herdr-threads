use super::*;

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
