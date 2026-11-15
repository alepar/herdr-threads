//! Agent harnesses with no adapter. Their agents export these environment markers to the
//! commands they run; without them such a caller has no registered marker and would be read as
//! a person. Registered here, a set marker refuses the caller as an agent (`session_claim_required`) and is
//! TRUST-POLICY A4 person-pane refusal evidence. Nothing here attributes a call to the harness.
use crate::harness::adapter::AgentEnvMarker;

pub(crate) static AGENT_ENV_MARKERS: &[AgentEnvMarker] = &[];
