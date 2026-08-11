CREATE TABLE agent_thread_instances (
    id                  TEXT PRIMARY KEY,
    agent_id            TEXT,
    agent_name_snapshot TEXT,
    codex_thread_id     TEXT NOT NULL UNIQUE,
    parent_thread_id    TEXT,
    scope_key           TEXT,
    status              TEXT NOT NULL CHECK (
        status IN ('RUNNING', 'IDLE', 'RECOVERY_REQUIRED', 'CLOSED', 'UNKNOWN')
    ),
    input_tokens        INTEGER NOT NULL DEFAULT 0 CHECK (input_tokens >= 0),
    cached_input_tokens INTEGER NOT NULL DEFAULT 0 CHECK (cached_input_tokens >= 0),
    output_tokens       INTEGER NOT NULL DEFAULT 0 CHECK (output_tokens >= 0),
    total_tokens        INTEGER NOT NULL DEFAULT 0 CHECK (total_tokens >= 0),
    context_window      INTEGER CHECK (context_window IS NULL OR context_window > 0),
    created_at          TEXT NOT NULL,
    last_used_at        TEXT NOT NULL,
    closed_at           TEXT
);

CREATE INDEX idx_agent_thread_instances_agent
    ON agent_thread_instances(agent_id, last_used_at DESC);

CREATE INDEX idx_agent_thread_instances_status
    ON agent_thread_instances(status, last_used_at DESC);

INSERT INTO agent_thread_instances (
    id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
    scope_key, status, input_tokens, cached_input_tokens, output_tokens,
    total_tokens, context_window, created_at, last_used_at, closed_at
)
SELECT
    'usage-' || id,
    agent_id,
    agent_name_snapshot,
    codex_thread_id,
    parent_thread_id,
    NULL,
    CASE usage_status
        WHEN 'LIVE' THEN 'RUNNING'
        WHEN 'FINAL' THEN 'IDLE'
        WHEN 'PARTIAL' THEN 'RECOVERY_REQUIRED'
        ELSE 'UNKNOWN'
    END,
    input_tokens,
    cached_input_tokens,
    output_tokens,
    total_tokens,
    model_context_window,
    started_at,
    updated_at,
    NULL
FROM token_usage_records
WHERE agent_id IS NOT NULL;
