-- B-04（设计方案 §24.6）：Usage 与 Thread Observation 必须保存已证明的执行身份。
-- 既有记录没有可回溯的证据链，保守标记为外部观察，禁止事后猜测升级。
ALTER TABLE token_usage_records
ADD COLUMN execution_kind TEXT NOT NULL DEFAULT 'OBSERVED_EXTERNAL'
CHECK (execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER', 'OBSERVED_EXTERNAL'));

ALTER TABLE agent_thread_instances
ADD COLUMN execution_kind TEXT NOT NULL DEFAULT 'OBSERVED_EXTERNAL'
CHECK (execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER', 'OBSERVED_EXTERNAL'));
