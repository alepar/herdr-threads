use super::*;
use crate::{
    delivery::presentation::{PresentationRenderProfile, emitted_controls, measure},
    protocol::{
        delivery::PresentationRenderSelection,
        ids::{MessageId, ThreadId},
    },
};
use sha2::{Digest, Sha256};

// Shared with integrations/pi/extension/tests/fixtures/manual-*.jsonl and its JS hash checks.
const MAXIMUM_FIXTURE_SHA256: &str =
    "b38c97768e05e3e1e90bb11c031e7a848b8b4c923775e9fbd590ade175daa60e";
const NOTICE_CLEAR_FIXTURE_SHA256: &str =
    "718b5f490cac71a45e9bf3c2067cdd75c6ffc5b5993ac6d5c6736476339b0c6a";
fn page(items: Vec<PresentationItemV2>) -> DeliveryPageV2 {
    let size = measure(&PresentationRenderProfile::WatchJsonLinesV1, &items)
        .unwrap()
        .1;
    DeliveryPageV2 {
        attention_version: 0,
        text: String::new(),
        offered: vec![],
        truncated: vec![],
        more: true,
        presentation_token: Some("11111111111111111111111111111111".into()),
        render: PresentationRenderSelection::WatchJsonLinesV1,
        output_bytes: size as u32,
        emitted_controls: emitted_controls(&items),
        items,
    }
}
fn notice(body: &str) -> PresentationItemV2 {
    PresentationItemV2::Notice {
        warning: MessageId::new("notice"),
        thread: ThreadId::new("t"),
        body: body.into(),
    }
}
const LEASE: &str = "0123456789abcdef0123456789abcdef";
fn maximum_page() -> DeliveryPageV2 {
    let mut items = (0..32)
        .map(|i| PresentationItemV2::Attention {
            epoch: (u64::MAX - i).to_string(),
            attention_version: u64::MAX,
            text: "\"\\\n".repeat(200),
        })
        .collect::<Vec<_>>();
    let size = measure(&PresentationRenderProfile::WatchJsonLinesV1, &items)
        .unwrap()
        .1;
    if let PresentationItemV2::Attention { text, .. } = &mut items[0] {
        text.push_str(&"x".repeat(65536 - size));
    }
    page(items)
}
#[test]
fn maximum_actual_serializers_preserve_64k_body_under_72k_transport_and_4k_receipt() {
    let page = maximum_page();
    assert_eq!(page.output_bytes, 65536);
    let bytes = encode_page(&page, LEASE).unwrap().unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        MAXIMUM_FIXTURE_SHA256
    );
    assert!(bytes.len() <= 70886);
    assert!(bytes.len() <= LINE_LIMIT);
    let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let receipt = envelope["receipt"].as_str().unwrap();
    assert_eq!(receipt.len(), 2802);
    assert_eq!(serde_json::to_string(receipt).unwrap().len(), 3136);
    assert_eq!(
        serde_json::to_string(&envelope["keys"]).unwrap().len(),
        2145
    );
    assert_eq!(bytes.len(), 70886);
    let mut metadata = envelope.clone();
    metadata["items"] = serde_json::json!([]);
    assert_eq!(serde_json::to_vec(&metadata).unwrap().len(), 5350);
    let decoded = read_receipt(&mut receipt.as_bytes()).unwrap();
    assert_eq!(decoded.emitted_controls, page.emitted_controls);
    assert!(receipt.contains(&u64::MAX.to_string()));
}

fn notice_clear_page() -> DeliveryPageV2 {
    page(vec![
        notice("canonical notice: \"quoted\" \\ escaped\nsecond line"),
        PresentationItemV2::AttentionCleared {
            epoch: u64::MAX.to_string(),
            attention_version: u64::MAX,
        },
    ])
}

#[test]
fn notice_clear_serializer_matches_shared_interoperability_fixture() {
    let page = notice_clear_page();
    let bytes = encode_page(&page, LEASE).unwrap().unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        NOTICE_CLEAR_FIXTURE_SHA256
    );
    let packet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let receipt = read_receipt(&mut packet["receipt"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(receipt.emitted_nonordinary, 2);
    assert_eq!(receipt.emitted_controls, page.emitted_controls);
}

/// Explicit interoperability fixture export through the production serializer. This writes
/// stdout only; the invoking harness owns any artifact redirection and extraction.
#[test]
#[ignore = "exports bounded JSONL fixtures; select explicitly with --ignored --nocapture"]
fn export_manual_presentation_packet_fixtures() {
    use std::io::Write;
    let mut stdout = std::io::stdout().lock();
    for (name, page) in [
        ("maximum", maximum_page()),
        ("notice-clear", notice_clear_page()),
    ] {
        let bytes = encode_page(&page, LEASE).unwrap().unwrap();
        assert!(bytes.len() <= LINE_LIMIT);
        write!(stdout, "HT_MANUAL_PACKET {name}\t").unwrap();
        stdout.write_all(&bytes).unwrap();
    }
    stdout.flush().unwrap();
}
#[test]
fn receipt_rejects_oversize_unknown_fields_empty_count_and_trailing_data() {
    let envelope: serde_json::Value = serde_json::from_slice(
        &encode_page(&page(vec![notice("body")]), LEASE)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let text = envelope["receipt"].as_str().unwrap();
    for bad in [
        "x".repeat(INPUT_LIMIT + 1),
        format!("{text}{{}}"),
        text.replace("\"emitted_nonordinary\":1", "\"emitted_nonordinary\":0"),
        text.replace("\"token\":", "\"foreign\":0,\"token\":"),
    ] {
        assert!(read_receipt(&mut bad.as_bytes()).is_err());
    }
    let padded = format!("{text}{}", " ".repeat(INPUT_LIMIT - text.len()));
    assert!(read_receipt(&mut padded.as_bytes()).is_ok());
    assert!(read_receipt(&mut format!("{padded} ").as_bytes()).is_err());
}
#[test]
fn opaque_keys_survive_pruning_but_new_token_starts_a_new_episode() {
    let mut page = page(vec![notice("first"), notice("second")]);
    let decode = |p: &DeliveryPageV2| {
        serde_json::from_slice::<serde_json::Value>(&encode_page(p, LEASE).unwrap().unwrap())
            .unwrap()
    };
    let first = decode(&page);
    page.items.remove(0);
    let pruned = decode(&page);
    assert_eq!(first["keys"][1], pruned["keys"][0]);
    page.presentation_token = Some("22222222222222222222222222222222".into());
    assert_ne!(pruned["keys"][0], decode(&page)["keys"][0]);
}
#[test]
fn empty_hidden_token_has_no_receipt_and_hidden_controls_are_never_captured() {
    let empty = page(vec![]);
    assert!(empty.presentation_token.is_some());
    assert!(encode_page(&empty, LEASE).unwrap().is_none());
    let visible = page(vec![notice("shown")]);
    let envelope: serde_json::Value =
        serde_json::from_slice(&encode_page(&visible, LEASE).unwrap().unwrap()).unwrap();
    let receipt = read_receipt(&mut envelope["receipt"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(receipt.emitted_nonordinary, 1);
    assert!(receipt.emitted_controls.is_empty());
}

#[test]
fn ordinary_only_page_has_no_receipt_or_empty_required_confirmation() {
    let content=serde_json::from_value(serde_json::json!({"thread":"t","relays_user":false,"body":"mail","body_len":4,"truncated":false,"ack_required":false})).unwrap();
    let ordinary = page(vec![PresentationItemV2::Message {
        id: MessageId::new("m"),
        lazy: false,
        content,
    }]);
    assert!(encode_page(&ordinary, LEASE).unwrap().is_none());
}

#[test]
fn clear_only_page_has_one_applied_state_slot_and_exact_opaque_control() {
    let page = page(vec![PresentationItemV2::AttentionCleared {
        epoch: u64::MAX.to_string(),
        attention_version: u64::MAX,
    }]);
    let bytes = encode_page(&page, LEASE).unwrap().unwrap();
    let packet: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(packet["keys"].as_array().unwrap().len(), 1);
    let receipt = read_receipt(&mut packet["receipt"].as_str().unwrap().as_bytes()).unwrap();
    assert_eq!(receipt.emitted_nonordinary, 1);
    assert_eq!(receipt.emitted_controls, page.emitted_controls);
}
