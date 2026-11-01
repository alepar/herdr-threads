//! Connects to the host on an endpoint and yields its [`CreateTargetPort`] with the
//! witness a creation there must carry (peer-requested launch, `internal
//! peer-target-create`).
//!
//! Responsibility (TargetCreator)
//! - Owns: telling a Herdr socket from a herdr-core socket (`probe_kind`), building that
//!   host's creation port, and reading the witness it expects.
//! - Hides: which host adapter answers; callers hold a port and a witness.
//! - Must not: create anything, write the store or keep a connection.
//! - Role: a new host's creation is one more arm here; CLI code stays generic over the port.

use std::{path::Path, sync::Arc, time::Duration};

use crate::ports::{CreateTargetPort, HostKind, HostWitness};
use crate::protocol::results::ApiError;
use crate::protocol::time::{CallBudget, Clock};

pub struct TargetCreator {
    pub port: Box<dyn CreateTargetPort>,
    pub witness: HostWitness,
    pub kind: HostKind,
}

/// The creation port of the server on `endpoint`. `gate_argv` is what a herdr-core terminal
/// starts (ignored for a Herdr socket, whose tab is started by a later `launch --pane`).
pub fn connect(
    endpoint: &Path,
    gate_argv: Vec<String>,
    clock: Arc<dyn Clock>,
    budget: &CallBudget,
) -> Result<TargetCreator, ApiError> {
    match super::probe::probe_kind(endpoint, super::probe::PROBE_BUDGET)? {
        HostKind::Hc => {
            let host = super::hc::HcHost::new(super::hc::HcClient::new(endpoint))
                .with_gate_argv(gate_argv);
            let witness = host.core_witness()?;
            Ok(TargetCreator {
                port: Box::new(host),
                witness,
                kind: HostKind::Hc,
            })
        }
        kind => {
            let witness = super::request_witnessed(
                endpoint,
                &uuid::Uuid::new_v4().to_string(),
                "ping",
                serde_json::json!({}),
                clock.as_ref(),
                budget,
                Duration::from_secs(5),
            )?
            .witness;
            Ok(TargetCreator {
                port: Box::new(super::native::NativeCli::new(endpoint.to_path_buf(), clock)),
                witness: HostWitness::HerdrSocket(witness),
                kind,
            })
        }
    }
}
