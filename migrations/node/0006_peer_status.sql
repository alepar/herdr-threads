-- Node status (ht-xg88): what this node last knew of each peer node, so a peer
-- it cannot reach shows its last-known version and sync positions instead of
-- vanishing. Written by the sync coordinator (throttled); display only.
CREATE TABLE sync_peer_status (
    node_uuid TEXT PRIMARY KEY CHECK(length(node_uuid) = 32),
    version TEXT CHECK(version IS NULL OR length(version) BETWEEN 1 AND 32),
    boot TEXT NOT NULL CHECK(length(boot) = 36),
    tx_acked INTEGER CHECK(tx_acked IS NULL OR tx_acked >= 0),
    tx_head INTEGER CHECK((tx_head IS NULL) = (tx_acked IS NULL)),
    rx_have INTEGER CHECK(rx_have IS NULL OR rx_have >= 0),
    rx_head INTEGER CHECK((rx_head IS NULL) = (rx_have IS NULL)),
    app_seen_at INTEGER CHECK(app_seen_at IS NULL OR app_seen_at >= 0),
    updated_at INTEGER NOT NULL
) STRICT;
