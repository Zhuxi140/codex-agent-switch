CREATE TABLE providers (
    id                  TEXT PRIMARY KEY,
    provider_key        TEXT NOT NULL UNIQUE,
    name                TEXT NOT NULL,
    provider_type       TEXT NOT NULL CHECK (provider_type IN ('PRESET', 'CUSTOM')),
    base_url            TEXT NOT NULL,
    protocol            TEXT NOT NULL CHECK (protocol = 'RESPONSES'),
    auth_type           TEXT NOT NULL CHECK (auth_type = 'BEARER_TOKEN'),
    enabled             INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    source              TEXT NOT NULL CHECK (source IN ('BUILT_IN', 'USER')),
    preset_id           TEXT,
    custom_headers_json TEXT,
    metadata_json       TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL
);

CREATE TABLE credentials (
    id                TEXT PRIMARY KEY,
    provider_id       TEXT NOT NULL,
    credential_key    TEXT NOT NULL,
    secret_type       TEXT NOT NULL CHECK (secret_type = 'BEARER_TOKEN'),
    storage_backend   TEXT NOT NULL CHECK (storage_backend = 'WINDOWS_CREDENTIAL_MANAGER'),
    storage_key       TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,

    FOREIGN KEY(provider_id)
        REFERENCES providers(id)
        ON DELETE RESTRICT,

    UNIQUE(provider_id, credential_key)
);

CREATE INDEX idx_providers_enabled ON providers(enabled);
CREATE INDEX idx_credentials_provider_id ON credentials(provider_id);
