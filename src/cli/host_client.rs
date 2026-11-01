//! The CLI-side launch handle: the one place `launch` and `handoff` obtain their host from.
//!
//! Responsibility (LaunchHandle)
//! - Owns: building the selected host's adapter for an endpoint and handing out its
//!   [`CreateTargetPort`] (target creation) and its Herdr pane operations (labels, guarded
//!   start, topology reads) as separate views of the one connection.
//! - Hides: which adapter type answers; creation code holds a `&dyn CreateTargetPort`.
//! - Must not: create anything itself, write the journal or the store, or branch on the host
//!   type in launch and handoff code.
//! - Role: handle over the host seam. A new host's launch arrives as another constructor here
//!   plus its [`CreateTargetPort`] impl; callers stay as they are.

use std::{path::Path, path::PathBuf, sync::Arc};

use crate::{
    host::{native::NativeCli, target_creator::TargetCreator},
    ports::{CreateTargetPort, HostKind, HostWitness},
    protocol::{
        results::ApiError,
        time::{CallBudget, Clock},
    },
};

pub struct LaunchHandle {
    herdr: Option<NativeCli>,
    creator: Option<TargetCreator>,
}

impl LaunchHandle {
    /// The Herdr server on `endpoint`.
    pub fn herdr(endpoint: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            herdr: Some(crate::host::target_creator::herdr_host(endpoint, clock)),
            creator: None,
        }
    }

    /// The server on `endpoint`, probed: either host. The probe gives a witness with no target
    /// pane; the gate argv rides each creation request, so the port is built with none.
    pub fn probed(
        endpoint: &Path,
        clock: Arc<dyn Clock>,
        budget: &CallBudget,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            herdr: None,
            creator: Some(crate::host::target_creator::connect(
                endpoint,
                Vec::new(),
                clock,
                budget,
            )?),
        })
    }

    /// Creation of launch and handoff targets.
    pub fn create_port(&self) -> &dyn CreateTargetPort {
        match (&self.creator, &self.herdr) {
            (Some(creator), _) => creator.port.as_ref(),
            (None, Some(herdr)) => herdr,
            (None, None) => unreachable!("a handle is built by one constructor"),
        }
    }

    /// The Herdr pane operations (`launch --pane`, labels, topology) on the same endpoint.
    /// Refused on a handle that is not a Herdr server.
    pub fn host(&self) -> Result<&NativeCli, ApiError> {
        self.herdr
            .as_ref()
            .ok_or_else(|| ApiError::invalid_request("this host has no Herdr pane operations"))
    }

    /// What a creation on this handle must be witnessed against (a probed handle only).
    pub fn witness(&self) -> Option<&HostWitness> {
        self.creator.as_ref().map(|c| &c.witness)
    }

    pub fn kind(&self) -> Option<HostKind> {
        self.creator.as_ref().map(|c| c.kind)
    }
}
