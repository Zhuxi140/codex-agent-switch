CREATE TABLE providers_new (
    id                  TEXT PRIMARY KEY,
    provider_key        TEXT NOT NULL UNIQUE,
    name                TEXT NOT NULL,
    provider_type       TEXT NOT NULL CHECK (provider_type IN ('PRESET', 'CUSTOM')),
    base_url            TEXT NOT NULL,
    protocol            TEXT NOT NULL CHECK (protocol IN ('RESPONSES', 'CHAT_COMPLETIONS')),
    auth_type           TEXT NOT NULL CHECK (auth_type = 'BEARER_TOKEN'),
    enabled             INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    source              TEXT NOT NULL CHECK (source IN ('BUILT_IN', 'USER')),
    preset_id           TEXT,
    custom_headers_json TEXT,
    metadata_json       TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    cache_support       TEXT NOT NULL DEFAULT 'UNKNOWN'
        CHECK (cache_support IN ('UNKNOWN', 'SUPPORTED', 'UNSUPPORTED')),
    cache_retention_type TEXT NOT NULL DEFAULT 'UNKNOWN'
        CHECK (cache_retention_type IN ('UNKNOWN', 'APPROXIMATE', 'GUARANTEED')),
    cache_retention_hint_seconds INTEGER
        CHECK (cache_retention_hint_seconds IS NULL OR cache_retention_hint_seconds > 0),
    cache_profile_source TEXT,
    cache_profile_verified_at TEXT
);

INSERT INTO providers_new SELECT * FROM providers;
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
CREATE INDEX idx_providers_enabled ON providers(enabled);
