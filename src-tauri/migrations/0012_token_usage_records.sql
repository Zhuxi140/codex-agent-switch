CREATE TABLE token_usage_records (
    id                       TEXT PRIMARY KEY,
    codex_session_id         TEXT NOT NULL,
    codex_thread_id          TEXT NOT NULL UNIQUE,
    parent_thread_id         TEXT,
    agent_id                 TEXT,
    agent_name_snapshot      TEXT,
    provider_id              TEXT,
    provider_name_snapshot   TEXT,
    model_id                 TEXT,
    model_name_snapshot      TEXT,
    input_tokens             INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    cached_input_tokens      INTEGER NOT NULL DEFAULT 0 CHECK (cached_input_tokens >= 0),
    cache_write_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cache_write_input_tokens >= 0),
    output_tokens            INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    reasoning_output_tokens  INTEGER NOT NULL DEFAULT 0 CHECK (reasoning_output_tokens >= 0),
    total_tokens             INTEGER NOT NULL DEFAULT 0 CHECK (total_tokens >= 0),
    model_context_window     INTEGER CHECK (
        model_context_window IS NULL OR model_context_window > 0
    ),
    usage_status             TEXT NOT NULL CHECK (
        usage_status IN ('LIVE', 'FINAL', 'PARTIAL', 'UNKNOWN')
    ),
    source                   TEXT NOT NULL CHECK (
        source IN ('CODEX_APP_SERVER', 'CODEX_EXEC_JSONL', 'RESPONSES_PROXY')
    ),
    started_at               TEXT NOT NULL,
    completed_at             TEXT,
    updated_at               TEXT NOT NULL
);

CREATE INDEX idx_token_usage_session
    ON token_usage_records(codex_session_id, updated_at DESC);

CREATE INDEX idx_token_usage_parent
    ON token_usage_records(parent_thread_id, updated_at DESC);

CREATE INDEX idx_token_usage_agent
    ON token_usage_records(agent_id, updated_at DESC);

CREATE INDEX idx_token_usage_provider_model
    ON token_usage_records(provider_id, model_id, updated_at DESC);
