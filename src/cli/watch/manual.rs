//! Bounded local transport for explicit, whole emitted-page presentation.
//! Receipts carry no caller authority: confirmation reconstructs the current Watch claim.
use super::*;
use crate::protocol::delivery::{
    DeliveryPageV2, DeliveryPresentationConfirmV2, DeliveryPresentationConfirmationV2,
    PresentationConsumer, PresentationControlStampV2, PresentationItemV2,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(super) const INPUT_LIMIT: usize = 4096;
const METADATA_LIMIT: usize = 8192;
pub(super) const LINE_LIMIT: usize = 72 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    token: String,
    presentation_lease: String,
    emitted_controls: Vec<PresentationControlStampV2>,
    emitted_nonordinary: u8,
}
impl Receipt {
    fn validate(&self) -> Result<(), ApiError> {
        if !(1..=32).contains(&self.emitted_nonordinary)
            || !crate::protocol::delivery::valid_presentation_token(&self.token)
            || !crate::protocol::delivery::valid_presentation_token(&self.presentation_lease)
        {
            return Err(ApiError::invalid_request(
                "invalid manual presentation receipt",
            ));
        }
        crate::protocol::delivery::validate_control_stamps(&self.emitted_controls)
            .map_err(ApiError::invalid_request)
    }
    fn command(self, claim: CallerClaim) -> DeliveryPresentationConfirmV2 {
        DeliveryPresentationConfirmV2 {
            claim,
            consumer: PresentationConsumer::Watch,
            token: self.token,
            presentation_lease: Some(self.presentation_lease),
            emitted_controls: self.emitted_controls,
        }
    }
}

pub(super) fn read_receipt(input: &mut dyn Read) -> Result<Receipt, ApiError> {
    let mut bytes = Vec::new();
    input
        .take((INPUT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ApiError::invalid_request("manual presentation receipt input failed"))?;
    if bytes.len() > INPUT_LIMIT {
        return Err(ApiError::invalid_request(
            "manual presentation receipt exceeds 4096 bytes",
        ));
    }
    let receipt: Receipt = serde_json::from_slice(&bytes)
        .map_err(|_| ApiError::invalid_request("invalid manual presentation receipt"))?;
    receipt.validate()?;
    Ok(receipt)
}

#[derive(Serialize)]
struct Envelope<'a> {
    schema: u8,
    kind: &'static str,
    items: Vec<WatchLine>,
    keys: Vec<Option<String>>,
    receipt: &'a str,
}

fn nonordinary(item: &PresentationItemV2) -> bool {
    !matches!(item, PresentationItemV2::Message { .. })
}

fn key(token: &str, item: &PresentationItemV2) -> Result<String, ApiError> {
    let item =
        serde_json::to_vec(item).map_err(|_| ApiError::invalid_request("presentation key"))?;
    let mut hash = Sha256::new();
    for part in [
        b"herdr-threads/manual-presentation/v1".as_slice(),
        token.as_bytes(),
        &item,
    ] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    Ok(format!("{:x}", hash.finalize()))
}

pub(super) fn encode_page(page: &DeliveryPageV2, lease: &str) -> Result<Option<Vec<u8>>, ApiError> {
    let count = page.items.iter().filter(|item| nonordinary(item)).count();
    if count == 0 {
        return Ok(None);
    }
    let token = page
        .presentation_token
        .as_deref()
        .ok_or_else(|| ApiError::invalid_request("manual page has no token"))?;
    let receipt = Receipt {
        token: token.into(),
        presentation_lease: lease.into(),
        emitted_controls: page.emitted_controls.clone(),
        emitted_nonordinary: count
            .try_into()
            .map_err(|_| ApiError::invalid_request("manual page item bound"))?,
    };
    receipt.validate()?;
    let receipt = serde_json::to_string(&receipt)
        .map_err(|_| ApiError::invalid_request("manual receipt encoding"))?;
    let keys = page
        .items
        .iter()
        .map(|item| nonordinary(item).then(|| key(token, item)).transpose())
        .collect::<Result<Vec<_>, _>>()?;
    let mut envelope = Envelope {
        schema: 1,
        kind: "presentation_page",
        items: Vec::new(),
        keys,
        receipt: &receipt,
    };
    let metadata = serde_json::to_vec(&envelope)
        .map_err(|_| ApiError::invalid_request("manual envelope encoding"))?;
    if receipt.len() > INPUT_LIMIT || metadata.len() > METADATA_LIMIT {
        return Err(ApiError::invalid_request(
            "manual presentation metadata bound",
        ));
    }
    envelope.items = crate::delivery::presentation::presentation_watch_lines(&page.items);
    let mut bytes = serde_json::to_vec(&envelope)
        .map_err(|_| ApiError::invalid_request("manual envelope encoding"))?;
    bytes.push(b'\n');
    if bytes.len() > LINE_LIMIT {
        return Err(ApiError::invalid_request(
            "manual presentation transport bound",
        ));
    }
    Ok(Some(bytes))
}

pub(super) fn confirm(
    client: &dyn LocalClient,
    clock: &dyn Clock,
    claim: CallerClaim,
    receipt: Receipt,
    out: &mut dyn Write,
) -> Result<(), RunError> {
    if !matches!(
        probe_mod_watch(client, &cooperative_budget(clock)),
        Probe::Supported {
            presentation: true,
            deferred_notices: true,
            ..
        }
    ) {
        return Err(RunError::Exit(1));
    }
    let command = receipt.command(claim);
    command.validate().map_err(ApiError::invalid_request)?;
    let result = client
        .call(
            Command::DeliveryPresentationConfirmV2(command.clone()),
            &cooperative_budget(clock),
        )
        .map_err(RunError::Api)?;
    let CommandResult::DeliveryPresentationConfirmedV2(reply) = result else {
        return Err(RunError::Exit(1));
    };
    reply
        .validate_for(&command.emitted_controls)
        .map_err(|_| RunError::Exit(1))?;
    let status = match reply.status {
        DeliveryPresentationConfirmationV2::Confirmed => "confirmed",
        DeliveryPresentationConfirmationV2::AlreadyConfirmed => "already_confirmed",
        _ => return Err(RunError::Exit(1)),
    };
    serde_json::to_writer(
        &mut *out,
        &serde_json::json!({"status":status,"emitted_page_confirmed":true}),
    )
    .map_err(|_| RunError::Exit(1))?;
    out.write_all(b"\n").map_err(|_| RunError::Exit(1))
}

#[cfg(test)]
#[path = "../../../tests/cli/watch_manual.rs"]
mod tests;
