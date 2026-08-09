ALTER TABLE configuration_state
ADD COLUMN active_agent_id TEXT REFERENCES agents(id) ON DELETE RESTRICT;

-- V0.1 的 enabled 是多 Agent 投影开关；新运行模式改为 active_agent_id 单选。
UPDATE agents SET enabled = 1 WHERE enabled = 0;
