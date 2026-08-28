CREATE TABLE runtime_delegation_leases (
    id                   TEXT PRIMARY KEY,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL,
    agent_id             TEXT NOT NULL,
    parent_thread_id     TEXT NOT NULL,
    codex_agent_id       TEXT,
    workspace_scope_key  TEXT NOT NULL,
    task_scope_key       TEXT,
    schedule_decision_id TEXT NOT NULL,
    state                TEXT NOT NULL
                         CHECK (state IN ('PENDING', 'ACTIVE', 'RELEASED', 'EXPIRED', 'REVOKED')),
    expires_at           TEXT NOT NULL,
    released_at          TEXT,
    release_reason       TEXT,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
    FOREIGN KEY (schedule_decision_id) REFERENCES agent_schedule_decisions(id) ON DELETE CASCADE
);

CREATE INDEX idx_runtime_delegation_leases_pending
    ON runtime_delegation_leases(
        agent_id, parent_thread_id, workspace_scope_key, task_scope_key, state, expires_at
    );

CREATE INDEX idx_runtime_delegation_leases_codex_agent
    ON runtime_delegation_leases(codex_agent_id, state, expires_at);

ALTER TABLE runtime_hook_turns ADD COLUMN codex_agent_id TEXT;
ALTER TABLE runtime_hook_turns ADD COLUMN lease_id TEXT;
ALTER TABLE runtime_hook_turns ADD COLUMN workspace_scope_key TEXT;
ALTER TABLE runtime_hook_turns ADD COLUMN task_scope_key TEXT;
ALTER TABLE runtime_hook_turns ADD COLUMN lease_state TEXT;
ALTER TABLE runtime_hook_turns ADD COLUMN lease_expires_at TEXT;
ALTER TABLE runtime_hook_turns ADD COLUMN stopped_at TEXT;

CREATE INDEX idx_runtime_hook_turns_codex_agent
    ON runtime_hook_turns(codex_agent_id, updated_at DESC);

ALTER TABLE runtime_enforcement_events ADD COLUMN codex_agent_id TEXT;
ALTER TABLE runtime_enforcement_events ADD COLUMN lease_id TEXT;
ALTER TABLE runtime_enforcement_events ADD COLUMN workspace_scope_key TEXT;
ALTER TABLE runtime_enforcement_events ADD COLUMN task_scope_key TEXT;
ALTER TABLE runtime_enforcement_events ADD COLUMN lease_state TEXT;
ALTER TABLE runtime_enforcement_events ADD COLUMN lease_expires_at TEXT;
