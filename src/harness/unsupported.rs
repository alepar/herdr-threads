//! Agent harnesses with no adapter. Their agents export these environment markers to the
//! commands they run; without them such a caller has no registered marker and would be read as
//! a person. Registered here, a set marker refuses the caller as an agent (`session_claim_required`) and is
//! TRUST-POLICY A4 person-pane refusal evidence. Nothing here attributes a call to the harness.
use crate::harness::adapter::AgentEnvMarker;

pub(crate) static AGENT_ENV_MARKERS: &[AgentEnvMarker] = &[
    // Not `AI_AGENT`: Claude Code sets it too, so it names no harness without an adapter.
    // Pi: `PI_CODING_AGENT=true`, set process-wide at startup (pi-mono
    // packages/coding-agent/src/cli/setup.ts).
    AgentEnvMarker {
        name: "PI_CODING_AGENT",
        person_guard: true,
    },
    // OpenCode: `OPENCODE=1`, process-wide through 1.x (packages/opencode/src/index.ts), on the
    // shell tool's environment on dev (packages/core/src/tool/plugin/shell.ts).
    AgentEnvMarker {
        name: "OPENCODE",
        person_guard: true,
    },
];
