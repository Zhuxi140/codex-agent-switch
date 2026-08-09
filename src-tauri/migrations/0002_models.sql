CREATE TABLE models (
    id                        TEXT PRIMARY KEY,
    provider_id               TEXT NOT NULL,
    model_id                  TEXT NOT NULL,
    display_name              TEXT NOT NULL,
    enabled                   INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    source                    TEXT NOT NULL CHECK (source IN ('PRESET', 'USER')),
    lifecycle                 TEXT NOT NULL DEFAULT 'UNKNOWN'
                                      CHECK (lifecycle IN ('ACTIVE', 'DEPRECATED', 'PREVIEW', 'UNKNOWN')),
    compatibility_level       TEXT NOT NULL DEFAULT 'UNKNOWN'
                                      CHECK (compatibility_level IN ('NATIVE', 'COMPATIBLE', 'GATEWAY_REQUIRED', 'UNSUPPORTED', 'UNKNOWN')),
    compatibility_source      TEXT NOT NULL DEFAULT 'UNKNOWN',
    minimum_codex_version     TEXT,
    compatibility_verified_at TEXT,
    context_window            INTEGER CHECK (context_window IS NULL OR context_window > 0),
    max_output_tokens         INTEGER CHECK (max_output_tokens IS NULL OR max_output_tokens > 0),
    reasoning_supported       INTEGER CHECK (reasoning_supported IS NULL OR reasoning_supported IN (0, 1)),
    default_reasoning         TEXT,
    metadata_source           TEXT,
    metadata_json             TEXT,
    created_at                TEXT NOT NULL,
    updated_at                TEXT NOT NULL,

    FOREIGN KEY(provider_id)
        REFERENCES providers(id)
        ON DELETE RESTRICT,

    UNIQUE(provider_id, model_id)
);

CREATE TABLE model_reasoning_efforts (
    model_id TEXT NOT NULL,
    effort   TEXT NOT NULL,
    ordinal  INTEGER NOT NULL DEFAULT 0,

    PRIMARY KEY(model_id, effort),

    FOREIGN KEY(model_id)
        REFERENCES models(id)
        ON DELETE CASCADE
);

CREATE TABLE model_capabilities (
    model_id         TEXT NOT NULL,
    capability       TEXT NOT NULL,
    status           TEXT NOT NULL CHECK (status IN ('SUPPORTED', 'UNSUPPORTED', 'UNKNOWN')),
    source           TEXT NOT NULL,
    confidence       TEXT NOT NULL,
    verified_at      TEXT,
    evidence_version TEXT,
    details_json     TEXT,

    PRIMARY KEY(model_id, capability),

    FOREIGN KEY(model_id)
        REFERENCES models(id)
        ON DELETE CASCADE
);

CREATE INDEX idx_models_provider_enabled ON models(provider_id, enabled);
