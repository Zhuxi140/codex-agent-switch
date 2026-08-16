-- 显式 Task Scope（peer-benchmarks #6：任务简报是唯一上下文通道）。
-- task_scope_key 由 Primary 从任务简报显式提取，bind 时固化；
-- 调度仅复用同键 Thread，无键任务不复用绑定了任务键的 Thread（fail-closed）。
ALTER TABLE agent_thread_instances
    ADD COLUMN task_scope_key TEXT;
ALTER TABLE agent_schedule_decisions
    ADD COLUMN task_scope_key TEXT;
