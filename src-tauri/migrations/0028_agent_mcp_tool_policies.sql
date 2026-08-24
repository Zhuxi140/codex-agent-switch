CREATE TABLE agent_mcp_tool_policies (
    agent_id  TEXT NOT NULL,
    server_id TEXT NOT NULL CHECK (length(server_id) BETWEEN 1 AND 128),
    mode      TEXT NOT NULL CHECK (mode IN ('ALLOW_ONLY', 'DENY')),
    tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 256),

    PRIMARY KEY (agent_id, server_id, tool_name),

    FOREIGN KEY (agent_id)
        REFERENCES agents(id)
        ON DELETE CASCADE
);

CREATE TRIGGER retire_threads_after_agent_mcp_tool_policy_add
AFTER INSERT ON agent_mcp_tool_policies
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_TOOL_POLICY_CHANGED',
        claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_after_agent_mcp_tool_policy_remove
AFTER DELETE ON agent_mcp_tool_policies
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_TOOL_POLICY_CHANGED',
        claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state = 'ACTIVE';
END;
