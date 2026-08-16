-- F-10：区分「最近可证明的模型使用时间」与「CAS 最近观察时间」。
-- last_model_usage_at 仅由 Runtime Bridge Usage 事件写入；缓存窗口判定只使用该列，
-- 原生同步 / 状态观察只推进 last_observed_at，不伪造模型使用时间。
-- 历史 last_used_at 混有两种语义，按可得的最优近似回填两列。
ALTER TABLE agent_thread_instances
    ADD COLUMN last_model_usage_at TEXT;
ALTER TABLE agent_thread_instances
    ADD COLUMN last_observed_at TEXT;
UPDATE agent_thread_instances
SET last_model_usage_at = last_used_at,
    last_observed_at = last_used_at;
