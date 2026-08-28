CREATE TABLE runtime_hook_turns (
    turn_id             TEXT PRIMARY KEY,
    session_id          TEXT NOT NULL,
    agent_id            TEXT,
    agent_type          TEXT NOT NULL,
    orchestration_phase TEXT,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE SET NULL
);

CREATE TABLE runtime_enforcement_events (
    id                  TEXT PRIMARY KEY,
    created_at          TEXT NOT NULL,
    session_id          TEXT NOT NULL,
    turn_id             TEXT NOT NULL,
    agent_id            TEXT,
    agent_type          TEXT,
    orchestration_phase TEXT,
    tool_name           TEXT NOT NULL,
    decision            TEXT NOT NULL CHECK (decision IN ('ALLOW', 'WARN', 'DENY')),
    reason_code         TEXT NOT NULL,
    cwd                 TEXT,
    message             TEXT NOT NULL,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE SET NULL
);

CREATE INDEX idx_runtime_enforcement_events_created_at
    ON runtime_enforcement_events(created_at DESC);
