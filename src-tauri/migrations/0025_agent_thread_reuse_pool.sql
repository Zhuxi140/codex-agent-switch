ALTER TABLE agent_thread_instances
ADD COLUMN reuse_state TEXT NOT NULL DEFAULT 'ACTIVE'
CHECK (reuse_state IN ('ACTIVE', 'RETIRE_PENDING', 'RETIRED'));

ALTER TABLE agent_thread_instances
ADD COLUMN reuse_state_reason TEXT;

CREATE INDEX idx_agent_thread_instances_reuse_state
ON agent_thread_instances(reuse_state, status, last_used_at DESC);

CREATE TRIGGER retire_threads_after_agent_disable
AFTER UPDATE OF enabled ON agents
WHEN OLD.enabled != NEW.enabled AND NEW.enabled = 0
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_DISABLED',
        claimed_until = NULL
    WHERE agent_id = NEW.id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_before_agent_delete
BEFORE DELETE ON agents
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_DELETED',
        claimed_until = NULL
    WHERE agent_id = OLD.id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_after_agent_runtime_change
AFTER UPDATE OF instruction, sandbox_policy, reasoning_policy ON agents
WHEN OLD.instruction != NEW.instruction
  OR OLD.sandbox_policy != NEW.sandbox_policy
  OR OLD.reasoning_policy != NEW.reasoning_policy
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_RUNTIME_CHANGED',
        claimed_until = NULL
    WHERE agent_id = NEW.id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_after_binding_runtime_change
AFTER UPDATE OF model_id, enabled ON agent_model_bindings
WHEN OLD.model_id != NEW.model_id OR OLD.enabled != NEW.enabled
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'MODEL_BINDING_CHANGED',
        claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state = 'ACTIVE';
END;

CREATE TRIGGER retire_threads_before_binding_delete
BEFORE DELETE ON agent_model_bindings
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'MODEL_BINDING_REMOVED',
        claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state = 'ACTIVE';
END;
