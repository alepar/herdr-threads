-- Peer-requested launches (remote launch, TRUST-POLICY N15): every
-- `launch_request` a linked peer node sent this node, recorded before this
-- node acts, with its progress. The request id is the requester's, derived
-- from its frozen intent, so a repeat returns the recorded outcome and never
-- launches twice. Local, never replicated; rows are kept 7 days. Provenance of
-- every row is `peer_request:<node_uuid>`.
CREATE TABLE peer_requests (
    id TEXT PRIMARY KEY CHECK(length(id) = 36),
    node_uuid TEXT NOT NULL CHECK(length(node_uuid) = 32),
    requester_seat TEXT CHECK(requester_seat IS NULL OR length(CAST(requester_seat AS BLOB)) <= 1024),
    kind TEXT NOT NULL CHECK(kind IN ('pane', 'new_tab')),
    plan TEXT NOT NULL CHECK(length(CAST(plan AS BLOB)) <= 131072),
    state TEXT NOT NULL CHECK(state IN ('running', 'launched', 'refused', 'failed')),
    pane TEXT CHECK(pane IS NULL OR length(CAST(pane AS BLOB)) <= 1024),
    seat TEXT CHECK(seat IS NULL OR length(CAST(seat AS BLOB)) <= 1024),
    detail TEXT CHECK(detail IS NULL OR length(CAST(detail AS BLOB)) <= 1024),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;
CREATE INDEX peer_requests_node ON peer_requests(node_uuid, created_at);
