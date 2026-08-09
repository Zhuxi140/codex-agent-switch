CREATE TABLE managed_resources (
    id                 TEXT PRIMARY KEY,
    resource_type      TEXT NOT NULL,
    logical_key        TEXT NOT NULL,
    physical_location  TEXT NOT NULL,
    ownership          TEXT NOT NULL,
    semantic_hash      TEXT,
    content_hash       TEXT,
    fragment_hash      TEXT,
    origin_entity_type TEXT,
    origin_entity_id   TEXT,
    last_applied_at    TEXT,
    created_at         TEXT NOT NULL,
    updated_at         TEXT NOT NULL,

    UNIQUE(resource_type, logical_key)
);

CREATE TABLE configuration_snapshots (
    id            TEXT PRIMARY KEY,
    reason        TEXT NOT NULL,
    codex_home    TEXT NOT NULL,
    codex_version TEXT,
    snapshot_path TEXT NOT NULL,
    status        TEXT NOT NULL,
    created_at    TEXT NOT NULL
);

CREATE TABLE configuration_snapshot_resources (
    id             TEXT PRIMARY KEY,
    snapshot_id    TEXT NOT NULL,
    relative_path  TEXT NOT NULL,
    resource_type  TEXT NOT NULL,
    existed_before INTEGER NOT NULL,
    content_hash   TEXT,
    created_at     TEXT NOT NULL,

    FOREIGN KEY(snapshot_id)
        REFERENCES configuration_snapshots(id)
        ON DELETE CASCADE,

    UNIQUE(snapshot_id, relative_path)
);

CREATE TABLE configuration_state (
    id                        INTEGER PRIMARY KEY CHECK(id = 1),
    last_applied_desired_hash TEXT,
    last_applied_at           TEXT,
    last_apply_transaction_id TEXT
);

INSERT INTO configuration_state (id) VALUES (1);

CREATE TABLE apply_transactions (
    id           TEXT PRIMARY KEY,
    snapshot_id  TEXT,
    status       TEXT NOT NULL,
    codex_home   TEXT NOT NULL,
    desired_hash TEXT,
    started_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    completed_at TEXT,

    FOREIGN KEY(snapshot_id)
        REFERENCES configuration_snapshots(id)
        ON DELETE SET NULL
);

CREATE INDEX idx_managed_resources_origin
ON managed_resources(origin_entity_type, origin_entity_id);

CREATE INDEX idx_managed_resources_location
ON managed_resources(physical_location);

CREATE INDEX idx_snapshots_created_at
ON configuration_snapshots(created_at DESC);

CREATE INDEX idx_apply_transactions_status
ON apply_transactions(status);
