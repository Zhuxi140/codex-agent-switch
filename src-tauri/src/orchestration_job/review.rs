use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{OrchestrationJobService, find_attempt_by_id, find_job_by_id, find_latest_attempt};
use crate::delivery_receipt::{
    DeliveryReceiptAppendRequest, ReceiptEvidence, append_in_transaction,
};
use crate::orchestration_contract::{
    AttemptState, ExecutionKind, JobState, OrchestrationError, OrchestrationErrorCode,
    OrchestrationJob, ReceiptEvidenceSource, ReceiptStage, ReviewDecision, ReviewOutcome,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobReviewRequest {
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    pub(crate) decision: ReviewOutcome,
    pub(crate) reviewer_thread_id: String,
    pub(crate) reason: String,
    pub(crate) evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobReviewResponse {
    pub(crate) review: ReviewDecision,
    pub(crate) job: OrchestrationJob,
}

impl OrchestrationJobService {
    pub(crate) fn review(
        &self,
        mut request: OrchestrationJobReviewRequest,
    ) -> Result<OrchestrationJobReviewResponse, OrchestrationError> {
        normalize_request(&mut request)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;

        if let Some(review) = find_review(&transaction, &request.attempt_id)? {
            if review_matches(&review, &request) {
                let job = find_job_by_id(&transaction, &request.job_id)
                    .map_err(|_| persistence_error())?
                    .ok_or_else(|| invariant_error(&request.job_id, &request.attempt_id))?;
                transaction.commit().map_err(|_| persistence_error())?;
                return Ok(OrchestrationJobReviewResponse { review, job });
            }
            return Err(review_conflict(&request.job_id, &request.attempt_id));
        }

        let job = find_job_by_id(&transaction, &request.job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invalid_transition(&request.job_id, &request.attempt_id))?;
        if request.reviewer_thread_id != job.parent_thread_id {
            return Err(field_error("reviewer_thread_id"));
        }
        if job.state != JobState::ReviewPending {
            return Err(invalid_transition(&request.job_id, &request.attempt_id));
        }
        let attempt = find_attempt_by_id(&transaction, &request.attempt_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| attempt_not_current(&request.job_id, &request.attempt_id))?;
        let latest = find_latest_attempt(&transaction, &request.job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| attempt_not_current(&request.job_id, &request.attempt_id))?;
        if attempt.job_id != request.job_id
            || latest.attempt_id != request.attempt_id
            || attempt.state != AttemptState::Succeeded
        {
            return Err(attempt_not_current(&request.job_id, &request.attempt_id));
        }
        let result_observed = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM delivery_receipts
                    WHERE job_id=?1 AND attempt_id=?2 AND stage='RESULT_OBSERVED'
                 )",
                params![request.job_id, request.attempt_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| persistence_error())?;
        if !result_observed {
            return Err(result_not_observed(&request.job_id, &request.attempt_id));
        }
        let execution = load_execution(&transaction, &request.attempt_id)?
            .ok_or_else(|| invariant_error(&request.job_id, &request.attempt_id))?;
        if execution.lease_state != "ACTIVE"
            || execution.lease_thread_id.as_deref() != Some(&execution.codex_thread_id)
            || execution.reuse_state != "HELD_FOR_REVIEW"
        {
            return Err(invariant_error(&request.job_id, &request.attempt_id));
        }

        let now = timestamp(&transaction)?;
        let review = ReviewDecision {
            review_id: Uuid::new_v4().to_string(),
            job_id: request.job_id.clone(),
            attempt_id: request.attempt_id.clone(),
            decision: request.decision,
            reviewer_thread_id: request.reviewer_thread_id.clone(),
            reason: request.reason.clone(),
            evidence_refs: request.evidence_refs.clone(),
            created_at: now.clone(),
        };
        let evidence_refs = serde_json::to_string(&review.evidence_refs)
            .map_err(|_| field_error("evidence_refs"))?;
        transaction
            .execute(
                "INSERT INTO review_decisions (
                    review_id, job_id, attempt_id, decision, reviewer_thread_id,
                    reason, evidence_refs, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    review.review_id,
                    review.job_id,
                    review.attempt_id,
                    outcome_name(review.decision),
                    review.reviewer_thread_id,
                    review.reason,
                    evidence_refs,
                    review.created_at,
                ],
            )
            .map_err(|_| persistence_error())?;
        append_in_transaction(
            &transaction,
            DeliveryReceiptAppendRequest {
                job_id: request.job_id.clone(),
                attempt_id: request.attempt_id.clone(),
                evidence: vec![ReceiptEvidence {
                    stage: ReceiptStage::ParentAcknowledged,
                    execution_kind: Some(execution.execution_kind),
                    evidence_source: ReceiptEvidenceSource::PrimaryReview,
                    evidence_ref: review.review_id.clone(),
                    parent_thread_id: request.reviewer_thread_id,
                    codex_thread_id: Some(execution.codex_thread_id.clone()),
                    codex_turn_id: execution.codex_turn_id.clone(),
                    schema_profile: None,
                    evidence_at: now.clone(),
                }],
            },
        )?;

        match request.decision {
            ReviewOutcome::Approve => {
                update_job(
                    &transaction,
                    &request.job_id,
                    "REVIEW_PENDING",
                    "APPROVED",
                    None,
                    &now,
                )?;
                release_execution(&transaction, &execution, "REVIEW_APPROVED", &now)?;
                let (reuse_state, reason) = if execution.is_reusable_after_review() {
                    ("ACTIVE", None)
                } else {
                    (
                        "RETIRED",
                        Some(
                            execution
                                .reuse_state_reason
                                .as_deref()
                                .unwrap_or("REVIEW_RELEASE_INVALID_IDENTITY"),
                        ),
                    )
                };
                update_thread_release(
                    &transaction,
                    &execution.thread_instance_id,
                    reuse_state,
                    reason,
                )?;
                update_job(
                    &transaction,
                    &request.job_id,
                    "APPROVED",
                    "COMPLETED",
                    Some(&now),
                    &now,
                )?;
            }
            ReviewOutcome::RevisionRequired => {
                update_job(
                    &transaction,
                    &request.job_id,
                    "REVIEW_PENDING",
                    "REVISION_REQUIRED",
                    None,
                    &now,
                )?;
                release_execution(&transaction, &execution, "REVISION_REQUIRED", &now)?;
                update_thread_release(
                    &transaction,
                    &execution.thread_instance_id,
                    "HELD_FOR_REVIEW",
                    execution.reuse_state_reason.as_deref(),
                )?;
            }
            ReviewOutcome::Reject => {
                update_job(
                    &transaction,
                    &request.job_id,
                    "REVIEW_PENDING",
                    "REJECTED",
                    Some(&now),
                    &now,
                )?;
                release_execution(&transaction, &execution, "REVIEW_REJECTED", &now)?;
                update_thread_release(
                    &transaction,
                    &execution.thread_instance_id,
                    "RETIRED",
                    Some("REVIEW_REJECTED"),
                )?;
            }
        }

        let job = find_job_by_id(&transaction, &request.job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invariant_error(&request.job_id, &request.attempt_id))?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(OrchestrationJobReviewResponse { review, job })
    }
}

struct ReviewExecution {
    lease_id: String,
    lease_state: String,
    lease_thread_id: Option<String>,
    thread_instance_id: String,
    codex_thread_id: String,
    codex_turn_id: Option<String>,
    execution_kind: ExecutionKind,
    thread_status: String,
    reuse_state: String,
    reuse_state_reason: Option<String>,
    identity_matches: bool,
}

impl ReviewExecution {
    fn is_reusable_after_review(&self) -> bool {
        self.thread_status == "IDLE"
            && self.reuse_state == "HELD_FOR_REVIEW"
            && self.reuse_state_reason.is_none()
            && self.identity_matches
    }
}

fn load_execution(
    connection: &Connection,
    attempt_id: &str,
) -> Result<Option<ReviewExecution>, OrchestrationError> {
    connection
        .query_row(
            "SELECT attempt.lease_id, lease.state, lease.codex_agent_id,
                    instance.id, instance.codex_thread_id, attempt.codex_turn_id,
                    attempt.execution_kind, instance.status, instance.reuse_state,
                    instance.reuse_state_reason,
                    instance.agent_id = job.agent_id
                      AND instance.parent_thread_id = job.parent_thread_id
                      AND instance.scope_key = job.workspace_scope_key
                      AND instance.task_scope_key = job.task_scope_key
                      AND instance.runtime_fingerprint = decision.runtime_fingerprint
                      AND instance.execution_kind = attempt.execution_kind
                      AND agent.enabled = 1
             FROM job_attempts attempt
             JOIN orchestration_jobs job ON job.job_id=attempt.job_id
             JOIN runtime_delegation_leases lease ON lease.id=attempt.lease_id
             JOIN agent_thread_instances instance ON instance.id=attempt.thread_instance_id
             JOIN agent_schedule_decisions decision ON decision.id=attempt.schedule_decision_id
             JOIN agents agent ON agent.id=job.agent_id
             WHERE attempt.attempt_id=?1",
            [attempt_id],
            |row| {
                let kind: String = row.get(6)?;
                let execution_kind = match kind.as_str() {
                    "NATIVE_CHILD" => ExecutionKind::NativeChild,
                    "MANAGED_WORKER" => ExecutionKind::ManagedWorker,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                Ok(ReviewExecution {
                    lease_id: row.get(0)?,
                    lease_state: row.get(1)?,
                    lease_thread_id: row.get(2)?,
                    thread_instance_id: row.get(3)?,
                    codex_thread_id: row.get(4)?,
                    codex_turn_id: row.get(5)?,
                    execution_kind,
                    thread_status: row.get(7)?,
                    reuse_state: row.get(8)?,
                    reuse_state_reason: row.get(9)?,
                    identity_matches: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|_| persistence_error())
}

fn release_execution(
    transaction: &Transaction<'_>,
    execution: &ReviewExecution,
    reason: &str,
    now: &str,
) -> Result<(), OrchestrationError> {
    let changed = transaction
        .execute(
            "UPDATE runtime_delegation_leases
             SET state='RELEASED', released_at=?2, updated_at=?2, release_reason=?3
             WHERE id=?1 AND state='ACTIVE'",
            params![execution.lease_id, now, reason],
        )
        .map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(persistence_error());
    }
    transaction
        .execute(
            "UPDATE runtime_hook_turns
             SET lease_state='RELEASED', stopped_at=COALESCE(stopped_at,?2), updated_at=?2
             WHERE lease_id=?1",
            params![execution.lease_id, now],
        )
        .map_err(|_| persistence_error())?;
    transaction
        .execute(
            "DELETE FROM agent_spawn_reservations WHERE lease_id=?1",
            [&execution.lease_id],
        )
        .map_err(|_| persistence_error())?;
    Ok(())
}

fn update_thread_release(
    transaction: &Transaction<'_>,
    instance_id: &str,
    reuse_state: &str,
    reason: Option<&str>,
) -> Result<(), OrchestrationError> {
    let changed = transaction
        .execute(
            "UPDATE agent_thread_instances
             SET reuse_state=?2, reuse_state_reason=?3,
                 claimed_until=NULL, claim_lease_id=NULL
             WHERE id=?1",
            params![instance_id, reuse_state, reason],
        )
        .map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(persistence_error());
    }
    Ok(())
}

fn update_job(
    transaction: &Transaction<'_>,
    job_id: &str,
    expected: &str,
    next: &str,
    terminal_at: Option<&str>,
    now: &str,
) -> Result<(), OrchestrationError> {
    let changed = transaction
        .execute(
            "UPDATE orchestration_jobs
             SET state=?3, updated_at=?4, terminal_at=?5
             WHERE job_id=?1 AND state=?2",
            params![job_id, expected, next, now, terminal_at],
        )
        .map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(invalid_transition(job_id, ""));
    }
    Ok(())
}

fn normalize_request(
    request: &mut OrchestrationJobReviewRequest,
) -> Result<(), OrchestrationError> {
    for (field, value) in [
        ("job_id", &request.job_id),
        ("attempt_id", &request.attempt_id),
        ("reviewer_thread_id", &request.reviewer_thread_id),
    ] {
        if value.trim().is_empty() {
            return Err(field_error(field));
        }
    }
    request.reason = request.reason.trim().to_owned();
    if request.reason.is_empty() {
        return Err(field_error("reason"));
    }
    request.evidence_refs = request
        .evidence_refs
        .drain(..)
        .map(|reference| reference.trim().to_owned())
        .collect();
    let unique = request
        .evidence_refs
        .iter()
        .filter(|reference| !reference.is_empty())
        .collect::<BTreeSet<_>>();
    if request.evidence_refs.len() < 2
        || unique.len() != request.evidence_refs.len()
        || unique.len() < 2
    {
        return Err(field_error("evidence_refs"));
    }
    Ok(())
}

fn find_review(
    connection: &Connection,
    attempt_id: &str,
) -> Result<Option<ReviewDecision>, OrchestrationError> {
    connection
        .query_row(
            "SELECT review_id, job_id, attempt_id, decision, reviewer_thread_id,
                    reason, evidence_refs, created_at
             FROM review_decisions WHERE attempt_id=?1",
            [attempt_id],
            |row| {
                let decision: String = row.get(3)?;
                let decision = match decision.as_str() {
                    "APPROVE" => ReviewOutcome::Approve,
                    "REVISION_REQUIRED" => ReviewOutcome::RevisionRequired,
                    "REJECT" => ReviewOutcome::Reject,
                    _ => return Err(rusqlite::Error::InvalidQuery),
                };
                let refs: String = row.get(6)?;
                Ok(ReviewDecision {
                    review_id: row.get(0)?,
                    job_id: row.get(1)?,
                    attempt_id: row.get(2)?,
                    decision,
                    reviewer_thread_id: row.get(4)?,
                    reason: row.get(5)?,
                    evidence_refs: serde_json::from_str(&refs).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            refs.len(),
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    created_at: row.get(7)?,
                })
            },
        )
        .optional()
        .map_err(|_| persistence_error())
}

fn review_matches(review: &ReviewDecision, request: &OrchestrationJobReviewRequest) -> bool {
    review.job_id == request.job_id
        && review.attempt_id == request.attempt_id
        && review.decision == request.decision
        && review.reviewer_thread_id == request.reviewer_thread_id
        && review.reason == request.reason
        && review.evidence_refs == request.evidence_refs
}

fn outcome_name(outcome: ReviewOutcome) -> &'static str {
    match outcome {
        ReviewOutcome::Approve => "APPROVE",
        ReviewOutcome::RevisionRequired => "REVISION_REQUIRED",
        ReviewOutcome::Reject => "REJECT",
    }
}

fn timestamp(connection: &Connection) -> Result<String, OrchestrationError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .map_err(|_| persistence_error())
}

fn field_error(path: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::TaskPacketFieldInvalid,
        message: format!("Review 字段无效：{path}。"),
        field_path: Some(path.to_owned()),
        job_id: None,
        attempt_id: None,
    }
}

fn attempt_not_current(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::AttemptNotCurrent,
        message: "Review 目标不是当前已完成 Attempt。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn result_not_observed(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::ResultNotObserved,
        message: "缺少 RESULT_OBSERVED，不能 Review。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn review_conflict(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::ReviewDecisionConflict,
        message: "该 Attempt 已有不同权威 ReviewDecision。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn invalid_transition(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InvalidStateTransition,
        message: "当前 Job 状态不允许 Review。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: (!attempt_id.is_empty()).then(|| attempt_id.to_owned()),
    }
}

fn invariant_error(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InternalInvariantViolation,
        message: "Review 所需 Job、Attempt、Thread 或 Lease 关系不完整。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn persistence_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::PersistenceError,
        message: "Review 持久化失败。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-09-10T00:00:00.000Z";

    fn service_with_observed_result() -> OrchestrationJobService {
        let service = OrchestrationJobService::in_memory();
        service
            .connection()
            .unwrap()
            .execute_batch(&format!(
                r#"INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    orchestration_phase, created_at, updated_at
                 ) VALUES (
                    'agent-1', 'executor', 'Executor', 'test', 'test', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, 'executor', 'EXECUTION',
                    '{NOW}', '{NOW}'
                 );
                 INSERT INTO orchestration_jobs (
                    job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    created_at, updated_at
                 ) VALUES (
                    'job-1', 'key-1',
                    '{{"schema_version":1,"job_id":"job-1","idempotency_key":"key-1","agent_id":"agent-1","parent_thread_id":"parent-1","workspace_scope_key":"c:/workspace","task_scope_key":"task-1","objective":"test","allowed_scope":["src"],"constraints":[],"success_criteria":["pass"],"allowed_tools":[],"permission_policy":"WORKSPACE_WRITE","execution_kind_policy":"MANAGED_WORKER_REQUIRED","context_references":[],"output_contract":"STANDARD_V1","review_policy":"PRIMARY_REQUIRED"}}',
                    '{hash}', 'agent-1', 'parent-1',
                    'c:/workspace', 'task-1', 'REVIEW_PENDING', '{NOW}', '{NOW}'
                 );
                 INSERT INTO agent_schedule_decisions (
                    id, created_at, source, agent_id, agent_name_snapshot,
                    workspace_scope_key, parent_thread_id, decision, reason_code,
                    runtime_fingerprint, cache_hint, claimed, task_scope_key, job_id
                 ) VALUES (
                    'decision-1', '{NOW}', 'CAS', 'agent-1', 'Executor',
                    'c:/workspace', 'parent-1', 'SPAWN', 'TEST', 'fingerprint-1',
                    'UNKNOWN', 0, 'task-1', 'job-1'
                 );
                 INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id,
                    codex_agent_id, workspace_scope_key, task_scope_key,
                    schedule_decision_id, state, expires_at, agent_type
                 ) VALUES (
                    'lease-1', '{NOW}', '{NOW}', 'agent-1', 'parent-1', 'thread-1',
                    'c:/workspace', 'task-1', 'decision-1', 'ACTIVE',
                    '2099-01-01T00:00:00.000Z', 'executor'
                 );
                 INSERT INTO agent_thread_instances (
                    id, agent_id, agent_name_snapshot, codex_thread_id,
                    parent_thread_id, scope_key, status, runtime_fingerprint,
                    created_at, last_used_at, last_observed_at, task_scope_key,
                    reuse_state, execution_kind, claim_lease_id
                 ) VALUES (
                    'instance-1', 'agent-1', 'Executor', 'thread-1', 'parent-1',
                    'c:/workspace', 'IDLE', 'fingerprint-1', '{NOW}', '{NOW}', '{NOW}',
                    'task-1', 'HELD_FOR_REVIEW', 'MANAGED_WORKER', 'lease-1'
                 );
                 INSERT INTO job_attempts (
                    attempt_id, job_id, attempt_no, schedule_decision_id, lease_id,
                    route_action, planned_execution_kind, execution_kind,
                    thread_instance_id, codex_turn_id, state, created_at, updated_at,
                    dispatch_recorded_at, accepted_at, terminal_at
                 ) VALUES (
                    'attempt-1', 'job-1', 1, 'decision-1', 'lease-1', 'SPAWN',
                    'MANAGED_WORKER', 'MANAGED_WORKER', 'instance-1', 'turn-1',
                    'SUCCEEDED', '{NOW}', '{NOW}', '{NOW}', '{NOW}', '{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id, job_id, attempt_id, stage, evidence_source,
                    evidence_ref, parent_thread_id, evidence_at, created_at
                 ) VALUES (
                    'receipt-1', 'job-1', 'attempt-1', 'DISPATCH_RECORDED',
                    'CAS_TRANSACTION', 'decision-1', 'parent-1', '{NOW}', '{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id, job_id, attempt_id, stage, execution_kind,
                    evidence_source, evidence_ref, parent_thread_id, codex_thread_id,
                    codex_turn_id, schema_profile, evidence_at, created_at
                 ) VALUES (
                    'receipt-2', 'job-1', 'attempt-1', 'TURN_ACCEPTED',
                    'MANAGED_WORKER', 'APP_SERVER_RESPONSE', 'accept-1', 'parent-1',
                    'thread-1', 'turn-1', 'APP_SERVER', '{NOW}', '{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id, job_id, attempt_id, stage, execution_kind,
                    evidence_source, evidence_ref, parent_thread_id, codex_thread_id,
                    codex_turn_id, schema_profile, evidence_at, created_at
                 ) VALUES (
                    'receipt-3', 'job-1', 'attempt-1', 'RESULT_OBSERVED',
                    'MANAGED_WORKER', 'RUNTIME_EVENT', 'result-1', 'parent-1',
                    'thread-1', 'turn-1', 'MODERN', '{NOW}', '{NOW}'
                  );"#,
                hash = "0".repeat(64)
            ))
            .unwrap();
        service
    }

    fn request(decision: ReviewOutcome) -> OrchestrationJobReviewRequest {
        OrchestrationJobReviewRequest {
            job_id: "job-1".to_owned(),
            attempt_id: "attempt-1".to_owned(),
            decision,
            reviewer_thread_id: "parent-1".to_owned(),
            reason: "验收通过".to_owned(),
            evidence_refs: vec!["child-result:1".to_owned(), "test-run:1".to_owned()],
        }
    }

    fn scalar(service: &OrchestrationJobService, sql: &str) -> String {
        service
            .connection()
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    fn count(service: &OrchestrationJobService, sql: &str) -> i64 {
        service
            .connection()
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn approve_is_atomic_idempotent_and_reopens_only_healthy_thread() {
        let service = service_with_observed_result();
        let first = service.review(request(ReviewOutcome::Approve)).unwrap();
        let second = service.review(request(ReviewOutcome::Approve)).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.job.state, JobState::Completed);
        assert_eq!(
            scalar(&service, "SELECT state FROM orchestration_jobs"),
            "COMPLETED"
        );
        assert_eq!(
            scalar(
                &service,
                "SELECT state || ':' || release_reason FROM runtime_delegation_leases"
            ),
            "RELEASED:REVIEW_APPROVED"
        );
        assert_eq!(
            scalar(
                &service,
                "SELECT reuse_state FROM agent_thread_instances WHERE id='instance-1'"
            ),
            "ACTIVE"
        );
        assert_eq!(count(&service, "SELECT COUNT(*) FROM review_decisions"), 1);
        assert_eq!(count(&service, "SELECT COUNT(*) FROM delivery_receipts"), 4);
        assert!(
            service
                .connection()
                .unwrap()
                .execute("UPDATE review_decisions SET reason='rewrite'", [])
                .is_err()
        );

        let mut conflict = request(ReviewOutcome::Reject);
        conflict.reason = "不同决定".to_owned();
        assert_eq!(
            service.review(conflict).unwrap_err().code,
            OrchestrationErrorCode::ReviewDecisionConflict
        );
    }

    #[test]
    fn revision_holds_thread_and_reject_retires_it() {
        let revision = service_with_observed_result();
        let response = revision
            .review(request(ReviewOutcome::RevisionRequired))
            .unwrap();
        assert_eq!(response.job.state, JobState::RevisionRequired);
        assert_eq!(
            scalar(&revision, "SELECT reuse_state FROM agent_thread_instances"),
            "HELD_FOR_REVIEW"
        );
        assert_eq!(
            scalar(&revision, "SELECT state FROM runtime_delegation_leases"),
            "RELEASED"
        );

        let reject = service_with_observed_result();
        let response = reject.review(request(ReviewOutcome::Reject)).unwrap();
        assert_eq!(response.job.state, JobState::Rejected);
        assert_eq!(
            scalar(
                &reject,
                "SELECT reuse_state || ':' || reuse_state_reason FROM agent_thread_instances"
            ),
            "RETIRED:REVIEW_REJECTED"
        );
    }

    #[test]
    fn parent_acknowledge_failure_rolls_back_review_and_release() {
        let service = service_with_observed_result();
        service
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_parent_acknowledge
                 BEFORE INSERT ON delivery_receipts
                 WHEN NEW.stage='PARENT_ACKNOWLEDGED'
                 BEGIN
                    SELECT RAISE(ABORT, 'injected parent acknowledge failure');
                 END;",
            )
            .unwrap();
        assert_eq!(
            service
                .review(request(ReviewOutcome::Approve))
                .unwrap_err()
                .code,
            OrchestrationErrorCode::PersistenceError
        );
        assert_eq!(count(&service, "SELECT COUNT(*) FROM review_decisions"), 0);
        assert_eq!(
            scalar(&service, "SELECT state FROM orchestration_jobs"),
            "REVIEW_PENDING"
        );
        assert_eq!(
            scalar(&service, "SELECT state FROM runtime_delegation_leases"),
            "ACTIVE"
        );
        assert_eq!(
            scalar(&service, "SELECT reuse_state FROM agent_thread_instances"),
            "HELD_FOR_REVIEW"
        );
    }

    #[test]
    fn invalid_primary_or_evidence_is_rejected_without_writes() {
        let service = service_with_observed_result();
        let mut wrong_primary = request(ReviewOutcome::Approve);
        wrong_primary.reviewer_thread_id = "other-parent".to_owned();
        assert_eq!(
            service.review(wrong_primary).unwrap_err().code,
            OrchestrationErrorCode::TaskPacketFieldInvalid
        );
        let mut missing_evidence = request(ReviewOutcome::Approve);
        missing_evidence.evidence_refs.pop();
        assert_eq!(
            service.review(missing_evidence).unwrap_err().code,
            OrchestrationErrorCode::TaskPacketFieldInvalid
        );
        assert_eq!(count(&service, "SELECT COUNT(*) FROM review_decisions"), 0);

        let missing_result = service_with_observed_result();
        missing_result
            .connection()
            .unwrap()
            .execute_batch(
                "DROP TRIGGER delivery_receipts_are_append_only_delete;
                 DELETE FROM delivery_receipts WHERE stage='RESULT_OBSERVED';",
            )
            .unwrap();
        assert_eq!(
            missing_result
                .review(request(ReviewOutcome::Approve))
                .unwrap_err()
                .code,
            OrchestrationErrorCode::ResultNotObserved
        );

        let invalid_state = service_with_observed_result();
        invalid_state
            .connection()
            .unwrap()
            .execute(
                "UPDATE orchestration_jobs SET state='RESULT_RECEIVED' WHERE job_id='job-1'",
                [],
            )
            .unwrap();
        assert_eq!(
            invalid_state
                .review(request(ReviewOutcome::Approve))
                .unwrap_err()
                .code,
            OrchestrationErrorCode::InvalidStateTransition
        );
    }

    #[test]
    fn approve_retires_thread_when_identity_was_invalidated() {
        let service = service_with_observed_result();
        service
            .connection()
            .unwrap()
            .execute(
                "UPDATE agent_thread_instances
                 SET reuse_state_reason='AGENT_RUNTIME_CHANGED'
                 WHERE id='instance-1'",
                [],
            )
            .unwrap();
        service.review(request(ReviewOutcome::Approve)).unwrap();
        assert_eq!(
            scalar(
                &service,
                "SELECT reuse_state || ':' || reuse_state_reason FROM agent_thread_instances"
            ),
            "RETIRED:AGENT_RUNTIME_CHANGED"
        );
    }
}
