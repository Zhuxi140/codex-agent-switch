-- C-03/C-05：冻结调度所有权，使 Claim、Reservation、Decision 与 Agent Type Lease
-- 能在同一事务中关联到唯一 Job/Attempt。
ALTER TABLE runtime_delegation_leases
ADD COLUMN agent_type TEXT NOT NULL DEFAULT '__UNKNOWN__'
CHECK (length(trim(agent_type)) > 0);

UPDATE runtime_delegation_leases
SET agent_type = COALESCE(
    (
        SELECT COALESCE(NULLIF(trim(agent.role_key), ''), agent.agent_key)
        FROM agents agent
        WHERE agent.id = runtime_delegation_leases.agent_id
    ),
    '__UNKNOWN__'
);

-- PENDING 已经持有派发权，也必须计入并发槽。若升级前已有同槽重复活跃 Lease，
-- 唯一索引创建会令迁移失败并回滚，避免静默猜测哪一个仍在运行。
CREATE UNIQUE INDEX idx_runtime_delegation_leases_one_live_agent_type
ON runtime_delegation_leases(workspace_scope_key, parent_thread_id, agent_type)
WHERE state IN ('PENDING', 'ACTIVE');

ALTER TABLE agent_thread_instances
ADD COLUMN claim_lease_id TEXT;

CREATE INDEX idx_agent_thread_instances_claim_lease
ON agent_thread_instances(claim_lease_id);

ALTER TABLE agent_spawn_reservations
ADD COLUMN lease_id TEXT;

ALTER TABLE agent_spawn_reservations
ADD COLUMN job_id TEXT;

CREATE UNIQUE INDEX idx_agent_spawn_reservations_lease
ON agent_spawn_reservations(lease_id)
WHERE lease_id IS NOT NULL;

ALTER TABLE agent_schedule_decisions
ADD COLUMN job_id TEXT REFERENCES orchestration_jobs(job_id) ON DELETE RESTRICT;

ALTER TABLE agent_schedule_decisions
ADD COLUMN supersedes_decision_id TEXT REFERENCES agent_schedule_decisions(id) ON DELETE RESTRICT;

CREATE INDEX idx_agent_schedule_decisions_job
ON agent_schedule_decisions(job_id, created_at, id);
