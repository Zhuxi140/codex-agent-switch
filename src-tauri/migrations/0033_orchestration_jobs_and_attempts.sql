-- B-02（设计方案 §24.4）：建立 orchestration_jobs 与 job_attempts 表，含每个 Job 同时最多一个非终态 Attempt 的部分唯一索引。
CREATE TABLE orchestration_jobs (
    job_id              TEXT PRIMARY KEY NOT NULL CHECK (length(job_id) > 0),
    idempotency_key     TEXT NOT NULL CHECK (
                            length(idempotency_key) BETWEEN 1 AND 128
                            AND substr(idempotency_key, 1, 1) GLOB '[A-Za-z0-9]'
                            AND idempotency_key NOT GLOB '*[^A-Za-z0-9._:-]*'
                        ),
    task_packet         TEXT NOT NULL CHECK (
                            json_valid(task_packet)
                            AND json_type(task_packet) = 'object'
                        ),
    task_packet_hash    TEXT NOT NULL CHECK (
                            length(task_packet_hash) = 64
                            AND task_packet_hash NOT GLOB '*[^0-9a-f]*'
                        ),
    agent_id            TEXT NOT NULL CHECK (length(agent_id) > 0),
    parent_thread_id    TEXT NOT NULL CHECK (length(parent_thread_id) > 0),
    workspace_scope_key TEXT NOT NULL CHECK (length(workspace_scope_key) > 0),
    task_scope_key      TEXT NOT NULL CHECK (
                            length(task_scope_key) BETWEEN 1 AND 64
                            AND substr(task_scope_key, 1, 1) GLOB '[a-z0-9]'
                            AND task_scope_key NOT GLOB '*[^a-z0-9_-]*'
                        ),
    state               TEXT NOT NULL
                        CHECK (state IN (
                            'CREATED', 'ROUTED', 'CLAIMED', 'DISPATCHED', 'RUNNING',
                            'RESULT_RECEIVED', 'REVIEW_PENDING', 'REVISION_REQUIRED',
                            'APPROVED', 'COMPLETED', 'WAITING', 'UNCERTAIN',
                            'BLOCKED', 'FAILED', 'CANCELLED', 'REJECTED'
                        )),
    last_error_code     TEXT CHECK (last_error_code IS NULL OR last_error_code IN (
                            'TASK_PACKET_FIELD_REQUIRED', 'TASK_PACKET_FIELD_INVALID',
                            'TASK_PACKET_SCOPE_MISMATCH', 'TASK_PACKET_CANONICALIZATION_FAILED',
                            'IDEMPOTENCY_KEY_CONFLICT', 'AGENT_NOT_EXECUTABLE', 'SCOPE_EXCLUDED',
                            'EXECUTION_KIND_UNSUPPORTED', 'RUNTIME_UNAVAILABLE', 'SCHEMA_UNVERIFIED',
                            'PERMISSION_DENIED', 'CONCURRENCY_LIMIT_REACHED',
                            'STALE_EXPECTED_DECISION', 'STALE_EXPECTED_CANDIDATE',
                            'ACTIVE_TURN_EXISTS', 'DISPATCH_REJECTED', 'DISPATCH_OUTCOME_UNKNOWN',
                            'THREAD_ID_MISSING', 'TURN_ID_MISSING', 'THREAD_ID_MISMATCH',
                            'TURN_ID_MISMATCH', 'NATIVE_PARENT_CHILD_EVIDENCE_MISSING',
                            'EXECUTION_KIND_MISMATCH', 'RECOVERY_REQUIRED', 'RECOVERY_LIMIT_REACHED',
                            'RESULT_NOT_OBSERVED', 'REVIEW_REQUIRED', 'REVIEW_DECISION_CONFLICT',
                            'ATTEMPT_NOT_CURRENT', 'INVALID_STATE_TRANSITION',
                            'CANCELLATION_UNCONFIRMED', 'PERSISTENCE_ERROR',
                            'INTERNAL_INVARIANT_VIOLATION'
                        )),
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    terminal_at         TEXT,
    CHECK (
        (state IN ('COMPLETED', 'BLOCKED', 'FAILED', 'CANCELLED', 'REJECTED'))
        = (terminal_at IS NOT NULL)
    ),
    UNIQUE (workspace_scope_key, parent_thread_id, idempotency_key),
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE RESTRICT
);

CREATE INDEX idx_orchestration_jobs_agent
    ON orchestration_jobs(agent_id, state, updated_at DESC);

CREATE INDEX idx_orchestration_jobs_parent
    ON orchestration_jobs(parent_thread_id, updated_at DESC);

CREATE TABLE job_attempts (
    attempt_id             TEXT PRIMARY KEY NOT NULL CHECK (length(attempt_id) > 0),
    job_id                 TEXT NOT NULL CHECK (length(job_id) > 0),
    attempt_no             INTEGER NOT NULL CHECK (attempt_no >= 1),
    previous_attempt_id    TEXT CHECK (previous_attempt_id IS NULL OR length(previous_attempt_id) > 0),
    schedule_decision_id   TEXT NOT NULL CHECK (length(schedule_decision_id) > 0),
    lease_id               TEXT NOT NULL CHECK (length(lease_id) > 0),
    route_action           TEXT NOT NULL
                           CHECK (route_action IN ('REUSE', 'SPAWN')),
    planned_execution_kind TEXT NOT NULL
                           CHECK (planned_execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER')),
    execution_kind         TEXT
                           CHECK (execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER')),
    thread_instance_id     TEXT CHECK (thread_instance_id IS NULL OR length(thread_instance_id) > 0),
    codex_turn_id          TEXT CHECK (codex_turn_id IS NULL OR length(codex_turn_id) > 0),
    state                  TEXT NOT NULL
                           CHECK (state IN (
                               'PLANNED', 'DISPATCHING', 'ACCEPTED', 'RUNNING',
                               'SUCCEEDED', 'UNCERTAIN', 'FAILED', 'CANCELLED'
                           )),
    recovery_count         INTEGER NOT NULL DEFAULT 0 CHECK (recovery_count BETWEEN 0 AND 3),
    last_error_code        TEXT CHECK (last_error_code IS NULL OR last_error_code IN (
                               'TASK_PACKET_FIELD_REQUIRED', 'TASK_PACKET_FIELD_INVALID',
                               'TASK_PACKET_SCOPE_MISMATCH', 'TASK_PACKET_CANONICALIZATION_FAILED',
                               'IDEMPOTENCY_KEY_CONFLICT', 'AGENT_NOT_EXECUTABLE', 'SCOPE_EXCLUDED',
                               'EXECUTION_KIND_UNSUPPORTED', 'RUNTIME_UNAVAILABLE', 'SCHEMA_UNVERIFIED',
                               'PERMISSION_DENIED', 'CONCURRENCY_LIMIT_REACHED',
                               'STALE_EXPECTED_DECISION', 'STALE_EXPECTED_CANDIDATE',
                               'ACTIVE_TURN_EXISTS', 'DISPATCH_REJECTED', 'DISPATCH_OUTCOME_UNKNOWN',
                               'THREAD_ID_MISSING', 'TURN_ID_MISSING', 'THREAD_ID_MISMATCH',
                               'TURN_ID_MISMATCH', 'NATIVE_PARENT_CHILD_EVIDENCE_MISSING',
                               'EXECUTION_KIND_MISMATCH', 'RECOVERY_REQUIRED', 'RECOVERY_LIMIT_REACHED',
                               'RESULT_NOT_OBSERVED', 'REVIEW_REQUIRED', 'REVIEW_DECISION_CONFLICT',
                               'ATTEMPT_NOT_CURRENT', 'INVALID_STATE_TRANSITION',
                               'CANCELLATION_UNCONFIRMED', 'PERSISTENCE_ERROR',
                               'INTERNAL_INVARIANT_VIOLATION'
                           )),
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    dispatch_recorded_at   TEXT,
    accepted_at            TEXT,
    terminal_at            TEXT,
    CHECK (
        (state IN ('SUCCEEDED', 'FAILED', 'CANCELLED')) = (terminal_at IS NOT NULL)
    ),
    CHECK (
        (attempt_no = 1 AND previous_attempt_id IS NULL)
        OR (attempt_no > 1 AND previous_attempt_id IS NOT NULL)
    ),
    UNIQUE (job_id, attempt_no),
    UNIQUE (job_id, attempt_id),
    UNIQUE (lease_id),
    FOREIGN KEY (job_id) REFERENCES orchestration_jobs(job_id) ON DELETE CASCADE,
    FOREIGN KEY (job_id, previous_attempt_id)
        REFERENCES job_attempts(job_id, attempt_id),
    FOREIGN KEY (schedule_decision_id)
        REFERENCES agent_schedule_decisions(id) ON DELETE RESTRICT,
    FOREIGN KEY (lease_id) REFERENCES runtime_delegation_leases(id) ON DELETE RESTRICT,
    FOREIGN KEY (thread_instance_id)
        REFERENCES agent_thread_instances(id) ON DELETE RESTRICT
);

-- 设计方案 §24.4：每个 Job 同时最多一个非终态 Attempt
CREATE UNIQUE INDEX idx_job_attempts_one_active_per_job
    ON job_attempts(job_id)
    WHERE state IN ('PLANNED', 'DISPATCHING', 'ACCEPTED', 'RUNNING', 'UNCERTAIN');

CREATE INDEX idx_job_attempts_thread_instance
    ON job_attempts(thread_instance_id);

CREATE INDEX idx_job_attempts_job_state_updated
    ON job_attempts(job_id, state, updated_at DESC);
