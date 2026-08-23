CREATE TABLE agent_spawn_reservations (
    agent_id           TEXT NOT NULL,
    parent_thread_id   TEXT NOT NULL,
    workspace_scope_key TEXT NOT NULL,
    task_scope_key     TEXT NOT NULL,
    reserved_until     TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    PRIMARY KEY (agent_id, parent_thread_id, workspace_scope_key, task_scope_key),
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE
);
