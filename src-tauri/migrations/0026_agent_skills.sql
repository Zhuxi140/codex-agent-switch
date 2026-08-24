CREATE TABLE agent_skill_bindings (
    agent_id  TEXT NOT NULL,
    skill_key TEXT NOT NULL CHECK (length(skill_key) BETWEEN 1 AND 64),

    PRIMARY KEY (agent_id, skill_key),

    FOREIGN KEY (agent_id)
        REFERENCES agents(id)
        ON DELETE CASCADE
);

CREATE TRIGGER retire_threads_after_agent_skill_add
AFTER INSERT ON agent_skill_bindings
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_SKILLS_CHANGED',
        claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_after_agent_skill_remove
AFTER DELETE ON agent_skill_bindings
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_SKILLS_CHANGED',
        claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state = 'ACTIVE';
END;
