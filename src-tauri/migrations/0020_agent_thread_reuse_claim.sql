-- F-11：REUSE 短租约。schedule/prepare 选中候选后在同一事务内写入 claimed_until，
-- 有效租约内的 Thread 不参与复用，防止两个并发预检双重 REUSE 同一 IDLE Thread。
-- 租约仅覆盖「预检 → follow-up 进入 RUNNING」窗口，超时自动失效，无需显式清理。
ALTER TABLE agent_thread_instances
    ADD COLUMN claimed_until TEXT;
