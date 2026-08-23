CREATE TABLE pending_credential_deletions (
    credential_id TEXT PRIMARY KEY,
    created_at    TEXT NOT NULL
);

CREATE INDEX idx_agent_thread_instances_scope
    ON agent_thread_instances(scope_key, last_used_at DESC);
