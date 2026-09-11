-- D-03：Primary ReviewDecision、父确认 Receipt 与 HELD_FOR_REVIEW。
-- 通过重命名旧列再删除，避免重建被 job_attempts 外键引用的 Thread 表。
DROP TRIGGER retire_threads_after_agent_disable;
DROP TRIGGER retire_threads_before_agent_delete;
DROP TRIGGER retire_threads_after_agent_runtime_change;
DROP TRIGGER retire_threads_after_binding_runtime_change;
DROP TRIGGER retire_threads_before_binding_delete;
DROP TRIGGER retire_threads_after_agent_skill_add;
DROP TRIGGER retire_threads_after_agent_skill_remove;
DROP TRIGGER retire_threads_after_agent_mcp_denylist_add;
DROP TRIGGER retire_threads_after_agent_mcp_denylist_remove;
DROP TRIGGER retire_threads_after_agent_mcp_tool_policy_add;
DROP TRIGGER retire_threads_after_agent_mcp_tool_policy_remove;
DROP INDEX idx_agent_thread_instances_reuse_state;

ALTER TABLE agent_thread_instances RENAME COLUMN reuse_state TO legacy_reuse_state;
ALTER TABLE agent_thread_instances
ADD COLUMN reuse_state TEXT NOT NULL DEFAULT 'ACTIVE'
CHECK (reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW', 'RETIRE_PENDING', 'RETIRED'));
UPDATE agent_thread_instances SET reuse_state = legacy_reuse_state;
ALTER TABLE agent_thread_instances DROP COLUMN legacy_reuse_state;

CREATE INDEX idx_agent_thread_instances_reuse_state
ON agent_thread_instances(reuse_state, status, last_used_at DESC);

CREATE TRIGGER retire_threads_after_agent_disable
AFTER UPDATE OF enabled ON agents
WHEN OLD.enabled != NEW.enabled AND NEW.enabled = 0
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_DISABLED', claimed_until = NULL
    WHERE agent_id = NEW.id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_before_agent_delete
BEFORE DELETE ON agents
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_DELETED', claimed_until = NULL
    WHERE agent_id = OLD.id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_runtime_change
AFTER UPDATE OF instruction, sandbox_policy, reasoning_policy ON agents
WHEN OLD.instruction != NEW.instruction
  OR OLD.sandbox_policy != NEW.sandbox_policy
  OR OLD.reasoning_policy != NEW.reasoning_policy
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_RUNTIME_CHANGED', claimed_until = NULL
    WHERE agent_id = NEW.id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_binding_runtime_change
AFTER UPDATE OF model_id, enabled ON agent_model_bindings
WHEN OLD.model_id != NEW.model_id OR OLD.enabled != NEW.enabled
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'MODEL_BINDING_CHANGED', claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_before_binding_delete
BEFORE DELETE ON agent_model_bindings
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'MODEL_BINDING_REMOVED', claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_skill_add
AFTER INSERT ON agent_skill_bindings
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_SKILLS_CHANGED', claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_skill_remove
AFTER DELETE ON agent_skill_bindings
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_SKILLS_CHANGED', claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_mcp_denylist_add
AFTER INSERT ON agent_disabled_mcp_servers
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_POLICY_CHANGED', claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_mcp_denylist_remove
AFTER DELETE ON agent_disabled_mcp_servers
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_POLICY_CHANGED', claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_mcp_tool_policy_add
AFTER INSERT ON agent_mcp_tool_policies
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_TOOL_POLICY_CHANGED', claimed_until = NULL
    WHERE agent_id = NEW.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TRIGGER retire_threads_after_agent_mcp_tool_policy_remove
AFTER DELETE ON agent_mcp_tool_policies
BEGIN
    UPDATE agent_thread_instances
    SET reuse_state = CASE WHEN status = 'RUNNING' THEN 'RETIRE_PENDING' ELSE 'RETIRED' END,
        reuse_state_reason = 'AGENT_MCP_TOOL_POLICY_CHANGED', claimed_until = NULL
    WHERE agent_id = OLD.agent_id AND reuse_state IN ('ACTIVE', 'HELD_FOR_REVIEW');
END;

CREATE TABLE review_decisions (
    review_id           TEXT PRIMARY KEY NOT NULL CHECK (length(review_id) > 0),
    job_id              TEXT NOT NULL CHECK (length(job_id) > 0),
    attempt_id          TEXT NOT NULL CHECK (length(attempt_id) > 0),
    decision            TEXT NOT NULL CHECK (
                            decision IN ('APPROVE', 'REVISION_REQUIRED', 'REJECT')
                        ),
    reviewer_thread_id  TEXT NOT NULL CHECK (length(reviewer_thread_id) > 0),
    reason              TEXT NOT NULL CHECK (length(trim(reason)) > 0),
    evidence_refs       TEXT NOT NULL CHECK (
                            json_valid(evidence_refs)
                            AND json_type(evidence_refs) = 'array'
                            AND json_array_length(evidence_refs) >= 2
                        ),
    created_at          TEXT NOT NULL,
    UNIQUE (attempt_id),
    FOREIGN KEY (job_id) REFERENCES orchestration_jobs(job_id) ON DELETE RESTRICT,
    FOREIGN KEY (job_id, attempt_id)
        REFERENCES job_attempts(job_id, attempt_id) ON DELETE RESTRICT
);

CREATE INDEX idx_review_decisions_job
ON review_decisions(job_id, created_at, review_id);

CREATE TRIGGER review_decisions_validate_primary_and_result
BEFORE INSERT ON review_decisions
WHEN NOT EXISTS (
    SELECT 1
    FROM orchestration_jobs job
    JOIN job_attempts attempt
      ON attempt.job_id = job.job_id AND attempt.attempt_id = NEW.attempt_id
    WHERE job.job_id = NEW.job_id
      AND job.parent_thread_id = NEW.reviewer_thread_id
      AND job.state = 'REVIEW_PENDING'
      AND attempt.state = 'SUCCEEDED'
      AND attempt.attempt_no = (
          SELECT MAX(current_attempt.attempt_no)
          FROM job_attempts current_attempt
          WHERE current_attempt.job_id = job.job_id
      )
      AND EXISTS (
          SELECT 1 FROM delivery_receipts receipt
          WHERE receipt.job_id = job.job_id
            AND receipt.attempt_id = attempt.attempt_id
            AND receipt.stage = 'RESULT_OBSERVED'
      )
)
BEGIN
    SELECT RAISE(ABORT, 'review decision requires current observed result and primary reviewer');
END;

CREATE TRIGGER review_decisions_no_update
BEFORE UPDATE ON review_decisions
BEGIN
    SELECT RAISE(ABORT, 'review decisions are append-only');
END;

CREATE TRIGGER review_decisions_no_delete
BEFORE DELETE ON review_decisions
BEGIN
    SELECT RAISE(ABORT, 'review decisions are append-only');
END;
