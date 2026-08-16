-- F-13：只追加的调度决策审计记录。
-- helper 预检与桌面预览/执行在决策同时写入；代码中不提供 UPDATE/DELETE 路径。
-- runtime_fingerprint 固化决策当时的 Agent 运行时配置指纹，供事后追溯。
CREATE TABLE agent_schedule_decisions (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    source TEXT NOT NULL,
    agent_id TEXT,
    agent_name_snapshot TEXT,
    workspace_scope_key TEXT NOT NULL,
    parent_thread_id TEXT,
    candidate_thread_id TEXT,
    decision TEXT NOT NULL,
    reason_code TEXT NOT NULL,
    runtime_fingerprint TEXT,
    context_pressure_percent INTEGER,
    context_pressure_limit_percent INTEGER,
    cache_hint TEXT NOT NULL,
    candidate_age_seconds INTEGER,
    claimed INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_agent_schedule_decisions_created_at
    ON agent_schedule_decisions (created_at DESC);
