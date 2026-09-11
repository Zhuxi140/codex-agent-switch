-- D-01（设计方案 §24.5）：Receipt 每阶段仅追加一行；UNKNOWN 只属于查询态。
CREATE TABLE delivery_receipts (
    receipt_id       TEXT PRIMARY KEY NOT NULL CHECK (length(receipt_id) > 0),
    job_id           TEXT NOT NULL CHECK (length(job_id) > 0),
    attempt_id       TEXT NOT NULL CHECK (length(attempt_id) > 0),
    stage            TEXT NOT NULL CHECK (stage IN (
                         'DISPATCH_RECORDED', 'TURN_ACCEPTED',
                         'RESULT_OBSERVED', 'PARENT_ACKNOWLEDGED'
                     )),
    execution_kind   TEXT CHECK (execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER')),
    evidence_source  TEXT NOT NULL CHECK (evidence_source IN (
                         'CAS_TRANSACTION', 'APP_SERVER_RESPONSE',
                         'NATIVE_PARENT_CHILD_EVENT', 'NATIVE_STATE_DB',
                         'RUNTIME_EVENT', 'RECOVERY_READ', 'PRIMARY_REVIEW'
                     )),
    evidence_ref     TEXT NOT NULL CHECK (length(trim(evidence_ref)) > 0),
    parent_thread_id TEXT NOT NULL CHECK (length(trim(parent_thread_id)) > 0),
    codex_thread_id  TEXT CHECK (codex_thread_id IS NULL OR length(trim(codex_thread_id)) > 0),
    codex_turn_id    TEXT CHECK (codex_turn_id IS NULL OR length(trim(codex_turn_id)) > 0),
    schema_profile   TEXT CHECK (schema_profile IS NULL OR length(trim(schema_profile)) > 0),
    evidence_at      TEXT NOT NULL CHECK (length(trim(evidence_at)) > 0),
    created_at       TEXT NOT NULL CHECK (length(trim(created_at)) > 0),
    CHECK (stage = 'DISPATCH_RECORDED' OR execution_kind IS NOT NULL),
    CHECK (stage = 'DISPATCH_RECORDED' OR codex_thread_id IS NOT NULL),
    CHECK (
        stage = 'DISPATCH_RECORDED'
        OR execution_kind <> 'MANAGED_WORKER'
        OR codex_turn_id IS NOT NULL
    ),
    UNIQUE (attempt_id, stage),
    FOREIGN KEY (job_id) REFERENCES orchestration_jobs(job_id) ON DELETE RESTRICT,
    FOREIGN KEY (job_id, attempt_id)
        REFERENCES job_attempts(job_id, attempt_id) ON DELETE RESTRICT
);

CREATE INDEX idx_delivery_receipts_job_attempt
    ON delivery_receipts(job_id, attempt_id, created_at);

-- Receipt 的 Parent 和已绑定的 Attempt 身份必须一致。触发器保护绕过 Repository 的写入。
CREATE TRIGGER delivery_receipts_parent_matches_job
BEFORE INSERT ON delivery_receipts
FOR EACH ROW
WHEN NEW.parent_thread_id <> (
    SELECT parent_thread_id FROM orchestration_jobs WHERE job_id = NEW.job_id
)
BEGIN
    SELECT RAISE(ABORT, 'delivery receipt parent does not match job');
END;

CREATE TRIGGER delivery_receipts_execution_kind_matches_attempt
BEFORE INSERT ON delivery_receipts
FOR EACH ROW
WHEN NEW.execution_kind IS NOT NULL
 AND EXISTS (
    SELECT 1 FROM job_attempts
    WHERE attempt_id = NEW.attempt_id
      AND execution_kind IS NOT NULL
      AND execution_kind <> NEW.execution_kind
 )
BEGIN
    SELECT RAISE(ABORT, 'delivery receipt execution kind does not match attempt');
END;

CREATE TRIGGER delivery_receipts_turn_matches_attempt
BEFORE INSERT ON delivery_receipts
FOR EACH ROW
WHEN NEW.codex_turn_id IS NOT NULL
 AND EXISTS (
    SELECT 1 FROM job_attempts
    WHERE attempt_id = NEW.attempt_id
      AND codex_turn_id IS NOT NULL
      AND codex_turn_id <> NEW.codex_turn_id
 )
BEGIN
    SELECT RAISE(ABORT, 'delivery receipt turn does not match attempt');
END;

CREATE TRIGGER delivery_receipts_thread_matches_instance
BEFORE INSERT ON delivery_receipts
FOR EACH ROW
WHEN NEW.codex_thread_id IS NOT NULL
 AND EXISTS (
    SELECT 1
    FROM job_attempts attempt
    JOIN agent_thread_instances instance ON instance.id = attempt.thread_instance_id
    WHERE attempt.attempt_id = NEW.attempt_id
      AND instance.codex_thread_id <> NEW.codex_thread_id
 )
BEGIN
    SELECT RAISE(ABORT, 'delivery receipt thread does not match attempt');
END;

CREATE TRIGGER delivery_receipts_stage_is_contiguous
BEFORE INSERT ON delivery_receipts
FOR EACH ROW
WHEN (
    (NEW.stage = 'TURN_ACCEPTED' AND NOT EXISTS (
        SELECT 1 FROM delivery_receipts
        WHERE attempt_id = NEW.attempt_id AND stage = 'DISPATCH_RECORDED'
    ))
    OR (NEW.stage = 'RESULT_OBSERVED' AND NOT EXISTS (
        SELECT 1 FROM delivery_receipts
        WHERE attempt_id = NEW.attempt_id AND stage = 'TURN_ACCEPTED'
    ))
    OR (NEW.stage = 'PARENT_ACKNOWLEDGED' AND NOT EXISTS (
        SELECT 1 FROM delivery_receipts
        WHERE attempt_id = NEW.attempt_id AND stage = 'RESULT_OBSERVED'
    ))
)
BEGIN
    SELECT RAISE(ABORT, 'delivery receipt stages must be contiguous');
END;

CREATE TRIGGER delivery_receipts_are_append_only_update
BEFORE UPDATE ON delivery_receipts
FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'delivery receipts are append only');
END;

CREATE TRIGGER delivery_receipts_are_append_only_delete
BEFORE DELETE ON delivery_receipts
FOR EACH ROW
BEGIN
    SELECT RAISE(ABORT, 'delivery receipts are append only');
END;
