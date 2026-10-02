-- 旧精简绑定合并为一个；完整版 caveman 优先，Ponytail 完全退出。
INSERT OR IGNORE INTO agent_skill_bindings (agent_id, skill_key)
SELECT DISTINCT old.agent_id, 'cas-slim'
FROM agent_skill_bindings old
WHERE old.skill_key IN ('caveman-slim', 'ponytail-slim')
  AND NOT EXISTS (
      SELECT 1 FROM agent_skill_bindings existing_full
      WHERE existing_full.agent_id = old.agent_id AND existing_full.skill_key = 'caveman'
  );

DELETE FROM agent_skill_bindings
WHERE skill_key IN ('caveman-slim', 'ponytail', 'ponytail-slim');

-- 不存用户消息内容，仅记录同一 Primary 会话的首回合与注入回合。
CREATE TABLE primary_prompt_injections (
    session_id TEXT PRIMARY KEY,
    first_turn_id TEXT NOT NULL,
    injected_turn_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
