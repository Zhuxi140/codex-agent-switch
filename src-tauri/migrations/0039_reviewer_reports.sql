-- D-05：独立 Reviewer 只产生可供 Primary 采纳的意见，绝不写主 Job 的裁决或 Receipt。
CREATE TABLE reviewer_assignments (
    reviewer_job_id    TEXT PRIMARY KEY NOT NULL,
    primary_job_id     TEXT NOT NULL,
    primary_attempt_id TEXT NOT NULL,
    created_at         TEXT NOT NULL,
    UNIQUE (primary_job_id, primary_attempt_id),
    UNIQUE (reviewer_job_id, primary_job_id, primary_attempt_id),
    FOREIGN KEY (reviewer_job_id) REFERENCES orchestration_jobs(job_id) ON DELETE RESTRICT,
    FOREIGN KEY (primary_job_id, primary_attempt_id)
        REFERENCES job_attempts(job_id, attempt_id) ON DELETE RESTRICT
);

CREATE TABLE reviewer_reports (
    report_id           TEXT PRIMARY KEY NOT NULL CHECK (length(report_id) > 0),
    reviewer_job_id     TEXT NOT NULL UNIQUE,
    primary_job_id      TEXT NOT NULL,
    primary_attempt_id  TEXT NOT NULL,
    reviewer_thread_id  TEXT NOT NULL CHECK (length(reviewer_thread_id) > 0),
    summary             TEXT NOT NULL CHECK (length(trim(summary)) > 0),
    findings            TEXT NOT NULL CHECK (json_valid(findings) AND json_type(findings) = 'array'),
    evidence_refs       TEXT NOT NULL CHECK (
                            json_valid(evidence_refs)
                            AND json_type(evidence_refs) = 'array'
                            AND json_array_length(evidence_refs) >= 1
                        ),
    created_at          TEXT NOT NULL,
    FOREIGN KEY (reviewer_job_id, primary_job_id, primary_attempt_id)
        REFERENCES reviewer_assignments(reviewer_job_id, primary_job_id, primary_attempt_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (primary_job_id, primary_attempt_id)
        REFERENCES job_attempts(job_id, attempt_id) ON DELETE RESTRICT
);

CREATE INDEX idx_reviewer_reports_primary_attempt
ON reviewer_reports(primary_job_id, primary_attempt_id, created_at, report_id);

CREATE TRIGGER reviewer_assignments_validate_policy_and_result
BEFORE INSERT ON reviewer_assignments
WHEN NOT EXISTS (
    SELECT 1
    FROM orchestration_jobs reviewer
    JOIN agents reviewer_agent
      ON reviewer_agent.id = reviewer.agent_id
    JOIN orchestration_jobs primary_job
      ON primary_job.job_id = NEW.primary_job_id
    JOIN job_attempts primary_attempt
      ON primary_attempt.job_id = primary_job.job_id
     AND primary_attempt.attempt_id = NEW.primary_attempt_id
    WHERE reviewer.job_id = NEW.reviewer_job_id
      AND reviewer.state = 'CREATED'
      AND reviewer.parent_thread_id = primary_job.parent_thread_id
      AND reviewer.workspace_scope_key = primary_job.workspace_scope_key
      AND reviewer.task_scope_key = primary_job.task_scope_key
      AND json_extract(reviewer.task_packet, '$.permission_policy') = 'READ_ONLY'
      AND json_extract(reviewer.task_packet, '$.review_policy') = 'PRIMARY_REQUIRED'
      AND json_array_length(json_extract(reviewer.task_packet, '$.allowed_scope')) > 0
      AND NOT EXISTS (
          SELECT 1
          FROM json_each(json_extract(reviewer.task_packet, '$.allowed_scope')) reviewer_scope
          WHERE reviewer_scope.value NOT IN (
              SELECT primary_scope.value
              FROM json_each(json_extract(primary_job.task_packet, '$.allowed_scope')) primary_scope
          )
      )
      AND reviewer_agent.agent_type = 'PRESET'
      AND reviewer_agent.source = 'CAS'
      AND reviewer_agent.role_key = 'reviewer'
      AND reviewer_agent.orchestration_phase = 'REVIEW'
      AND reviewer_agent.sandbox_policy = 'READ_ONLY'
      AND primary_job.state = 'REVIEW_PENDING'
      AND json_extract(primary_job.task_packet, '$.review_policy') =
          'PRIMARY_WITH_READ_ONLY_REVIEWER'
      AND primary_attempt.state = 'SUCCEEDED'
      AND primary_attempt.attempt_no = (
          SELECT MAX(current_attempt.attempt_no)
          FROM job_attempts current_attempt
          WHERE current_attempt.job_id = primary_job.job_id
      )
      AND EXISTS (
          SELECT 1 FROM delivery_receipts receipt
          WHERE receipt.job_id = primary_job.job_id
            AND receipt.attempt_id = primary_attempt.attempt_id
            AND receipt.stage = 'RESULT_OBSERVED'
      )
)
BEGIN
    SELECT RAISE(ABORT, 'reviewer assignment requires policy, read-only preset and current result');
END;

CREATE TRIGGER reviewer_assignments_no_update
BEFORE UPDATE ON reviewer_assignments
BEGIN
    SELECT RAISE(ABORT, 'reviewer assignments are append-only');
END;

CREATE TRIGGER reviewer_assignments_no_delete
BEFORE DELETE ON reviewer_assignments
BEGIN
    SELECT RAISE(ABORT, 'reviewer assignments are append-only');
END;

CREATE TRIGGER reviewer_reports_validate_observed_result
BEFORE INSERT ON reviewer_reports
WHEN NOT EXISTS (
    SELECT 1
    FROM reviewer_assignments assignment
    JOIN orchestration_jobs reviewer
      ON reviewer.job_id = assignment.reviewer_job_id
    JOIN agents reviewer_agent
      ON reviewer_agent.id = reviewer.agent_id
    JOIN job_attempts attempt
      ON attempt.job_id = reviewer.job_id
    JOIN agent_thread_instances instance
      ON instance.id = attempt.thread_instance_id
    JOIN delivery_receipts receipt
      ON receipt.job_id = reviewer.job_id
     AND receipt.attempt_id = attempt.attempt_id
     AND receipt.stage = 'RESULT_OBSERVED'
    WHERE assignment.reviewer_job_id = NEW.reviewer_job_id
      AND assignment.primary_job_id = NEW.primary_job_id
      AND assignment.primary_attempt_id = NEW.primary_attempt_id
      AND json_extract(reviewer.task_packet, '$.permission_policy') = 'READ_ONLY'
      AND json_extract(reviewer.task_packet, '$.review_policy') = 'PRIMARY_REQUIRED'
      AND reviewer_agent.agent_type = 'PRESET'
      AND reviewer_agent.source = 'CAS'
      AND reviewer_agent.role_key = 'reviewer'
      AND reviewer_agent.orchestration_phase = 'REVIEW'
      AND reviewer_agent.sandbox_policy = 'READ_ONLY'
      AND attempt.state = 'SUCCEEDED'
      AND attempt.attempt_no = (
          SELECT MAX(current_attempt.attempt_no)
          FROM job_attempts current_attempt
          WHERE current_attempt.job_id = reviewer.job_id
      )
      AND instance.codex_thread_id = NEW.reviewer_thread_id
)
BEGIN
    SELECT RAISE(ABORT, 'reviewer report requires the assigned read-only reviewer result');
END;

CREATE TRIGGER reviewer_reports_no_update
BEFORE UPDATE ON reviewer_reports
BEGIN
    SELECT RAISE(ABORT, 'reviewer reports are append-only');
END;

CREATE TRIGGER reviewer_reports_no_delete
BEFORE DELETE ON reviewer_reports
BEGIN
    SELECT RAISE(ABORT, 'reviewer reports are append-only');
END;
