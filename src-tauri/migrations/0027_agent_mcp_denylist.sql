CREATE TABLE agent_disabled_mcp_servers (
    agent_id  TEXT NOT NULL,
    server_id TEXT NOT NULL CHECK (length(server_id) BETWEEN 1 AND 128),

    PRIMARY KEY (agent_id, server_id),

    FOREIGN KEY (agent_id)
        REFERENCES agents(id)
        ON DELETE CASCADE
);

CREATE TRIGGER retire_threads_after_agent_mcp_denylist_add
AFTER INSERT ON agent_disabled_mcp_servers
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_POLICY_CHANGED',
        claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_after_agent_mcp_denylist_remove
AFTER DELETE ON agent_disabled_mcp_servers
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_POLICY_CHANGED',
        claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state = 'ACTIVE';
END;
