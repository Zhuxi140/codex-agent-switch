-- F-02（设计方案 §19）：区分「Cached Input 未提供」与「Cached Input = 0」。
-- NULL 表示历史记录无法证明（UNKNOWN）；新写入由 Runtime 事件填充 0/1。
ALTER TABLE token_usage_records
    ADD COLUMN cached_input_provided INTEGER
    CHECK (cached_input_provided IS NULL OR cached_input_provided IN (0, 1));

ALTER TABLE agent_thread_instances
    ADD COLUMN cached_input_provided INTEGER
    CHECK (cached_input_provided IS NULL OR cached_input_provided IN (0, 1));
