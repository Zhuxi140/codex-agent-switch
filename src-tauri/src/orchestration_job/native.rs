use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::{OrchestrationJobService, current_timestamp, persistence_error};
use crate::delivery_receipt::{
    DeliveryReceiptAppendRequest, ReceiptEvidence, append_in_transaction,
};
use crate::orchestration_contract::{
    ExecutionKind, OrchestrationError, OrchestrationErrorCode, ReceiptEvidenceSource, ReceiptStage,
};

#[derive(Debug, Clone)]
pub(crate) struct NativeBindingEvidence {
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    pub(crate) child_thread_id: String,
    pub(crate) evidence_ref: String,
    pub(crate) schema_profile: String,
}

#[derive(Debug, Clone)]
pub(crate) struct NativeResultEvidence {
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    pub(crate) child_thread_id: String,
    pub(crate) evidence_ref: String,
    pub(crate) schema_profile: String,
    pub(crate) reusable: bool,
}

struct NativeIdentity {
    parent_thread_id: String,
    attempt_state: String,
    job_state: String,
    lease_state: String,
    lease_id: String,
    planned_execution_kind: String,
    execution_kind: Option<String>,
    instance_id: String,
    instance_thread_id: String,
    identity_matches: bool,
    lease_thread_id: Option<String>,
}

impl OrchestrationJobService {
    /// 接收 helper 已从 Codex 原生 ParentChild 状态库核验的 Child 身份。
    pub(crate) fn accept_native_binding(
        &self,
        evidence: NativeBindingEvidence,
    ) -> Result<(), OrchestrationError> {
        validate_evidence_fields(
            &evidence.job_id,
            &evidence.attempt_id,
            &evidence.child_thread_id,
            &evidence.evidence_ref,
            &evidence.schema_profile,
        )?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let now = current_timestamp(&transaction).map_err(|_| persistence_error())?;
        let identity = load_native_identity(
            &transaction,
            &evidence.job_id,
            &evidence.attempt_id,
            &evidence.child_thread_id,
        )?
        .ok_or_else(|| attempt_not_current(&evidence.job_id, &evidence.attempt_id))?;

        if !identity.identity_matches
            || identity.planned_execution_kind != "NATIVE_CHILD"
            || identity
                .execution_kind
                .as_deref()
                .is_some_and(|kind| kind != "NATIVE_CHILD")
            || identity
                .lease_thread_id
                .as_deref()
                .is_some_and(|thread_id| thread_id != evidence.child_thread_id)
        {
            return Err(execution_kind_mismatch(
                &evidence.job_id,
                &evidence.attempt_id,
            ));
        }

        if identity.attempt_state == "DISPATCHING"
            && identity.job_state == "DISPATCHED"
            && matches!(identity.lease_state.as_str(), "PENDING" | "ACTIVE")
        {
            let changed = transaction
                .execute(
                    "UPDATE agent_thread_instances
                     SET execution_kind='NATIVE_CHILD', status='RUNNING', last_observed_at=?2
                     WHERE id=?1 AND execution_kind IN ('OBSERVED_EXTERNAL','NATIVE_CHILD')",
                    params![identity.instance_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE job_attempts
                     SET execution_kind='NATIVE_CHILD', thread_instance_id=?2, state='ACCEPTED',
                         accepted_at=COALESCE(accepted_at,?3), updated_at=?3
                     WHERE attempt_id=?1 AND state='DISPATCHING'",
                    params![evidence.attempt_id, identity.instance_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE runtime_delegation_leases
                     SET codex_agent_id=?2, state='ACTIVE',
                         admitted_at=COALESCE(admitted_at,?3),
                         admission_confirmed_at=COALESCE(admission_confirmed_at,?3),
                         updated_at=?3
                     WHERE id=?1 AND state IN ('PENDING','ACTIVE')
                       AND (codex_agent_id IS NULL OR codex_agent_id=?2)",
                    params![identity.lease_id, evidence.child_thread_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE orchestration_jobs SET state='RUNNING', updated_at=?2
                     WHERE job_id=?1 AND state='DISPATCHED'",
                    params![evidence.job_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            transaction
                .execute(
                    "DELETE FROM agent_spawn_reservations WHERE lease_id=?1",
                    [&identity.lease_id],
                )
                .map_err(|_| persistence_error())?;
        } else if !matches!(
            (
                identity.attempt_state.as_str(),
                identity.job_state.as_str(),
                identity.lease_state.as_str(),
                identity.execution_kind.as_deref(),
            ),
            (
                "ACCEPTED" | "RUNNING",
                "RUNNING",
                "ACTIVE",
                Some("NATIVE_CHILD")
            ) | (
                "SUCCEEDED",
                "REVIEW_PENDING",
                "ACTIVE",
                Some("NATIVE_CHILD")
            )
        ) {
            return Err(attempt_not_current(&evidence.job_id, &evidence.attempt_id));
        }

        append_in_transaction(
            &transaction,
            DeliveryReceiptAppendRequest {
                job_id: evidence.job_id,
                attempt_id: evidence.attempt_id,
                evidence: vec![ReceiptEvidence {
                    stage: ReceiptStage::TurnAccepted,
                    execution_kind: Some(ExecutionKind::NativeChild),
                    evidence_source: ReceiptEvidenceSource::NativeStateDb,
                    evidence_ref: evidence.evidence_ref,
                    parent_thread_id: identity.parent_thread_id,
                    codex_thread_id: Some(identity.instance_thread_id),
                    codex_turn_id: None,
                    schema_profile: Some(evidence.schema_profile),
                    evidence_at: now,
                }],
            },
        )?;
        transaction.commit().map_err(|_| persistence_error())
    }

    /// 将同一原生 Child 的可信终态观察推进到 REVIEW_PENDING；不释放 Lease。
    pub(crate) fn observe_native_result(
        &self,
        evidence: NativeResultEvidence,
    ) -> Result<(), OrchestrationError> {
        validate_evidence_fields(
            &evidence.job_id,
            &evidence.attempt_id,
            &evidence.child_thread_id,
            &evidence.evidence_ref,
            &evidence.schema_profile,
        )?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let now = current_timestamp(&transaction).map_err(|_| persistence_error())?;
        let identity = load_native_identity(
            &transaction,
            &evidence.job_id,
            &evidence.attempt_id,
            &evidence.child_thread_id,
        )?
        .ok_or_else(|| attempt_not_current(&evidence.job_id, &evidence.attempt_id))?;
        if !identity.identity_matches
            || identity.execution_kind.as_deref() != Some("NATIVE_CHILD")
            || identity.lease_thread_id.as_deref() != Some(&evidence.child_thread_id)
        {
            return Err(execution_kind_mismatch(
                &evidence.job_id,
                &evidence.attempt_id,
            ));
        }

        if matches!(identity.attempt_state.as_str(), "ACCEPTED" | "RUNNING")
            && identity.job_state == "RUNNING"
            && identity.lease_state == "ACTIVE"
        {
            append_in_transaction(
                &transaction,
                DeliveryReceiptAppendRequest {
                    job_id: evidence.job_id.clone(),
                    attempt_id: evidence.attempt_id.clone(),
                    evidence: vec![ReceiptEvidence {
                        stage: ReceiptStage::ResultObserved,
                        execution_kind: Some(ExecutionKind::NativeChild),
                        evidence_source: ReceiptEvidenceSource::RecoveryRead,
                        evidence_ref: evidence.evidence_ref,
                        parent_thread_id: identity.parent_thread_id,
                        codex_thread_id: Some(identity.instance_thread_id),
                        codex_turn_id: None,
                        schema_profile: Some(evidence.schema_profile),
                        evidence_at: now.clone(),
                    }],
                },
            )?;
            let changed = transaction
                .execute(
                    "UPDATE job_attempts
                     SET state='SUCCEEDED', terminal_at=COALESCE(terminal_at,?2), updated_at=?2
                     WHERE attempt_id=?1 AND state IN ('ACCEPTED','RUNNING')",
                    params![evidence.attempt_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let (status, reuse_reason) = if evidence.reusable {
                ("IDLE", None)
            } else {
                ("CLOSED", Some("NATIVE_THREAD_CLOSED"))
            };
            let changed = transaction
                .execute(
                    "UPDATE agent_thread_instances
                     SET status=?2, reuse_state='HELD_FOR_REVIEW', reuse_state_reason=?3,
                         last_observed_at=?4
                     WHERE id=?1 AND execution_kind='NATIVE_CHILD'",
                    params![identity.instance_id, status, reuse_reason, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE orchestration_jobs SET state='RESULT_RECEIVED', updated_at=?2
                     WHERE job_id=?1 AND state='RUNNING'",
                    params![evidence.job_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE orchestration_jobs SET state='REVIEW_PENDING', updated_at=?2
                     WHERE job_id=?1 AND state='RESULT_RECEIVED'",
                    params![evidence.job_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
        } else if !(identity.attempt_state == "SUCCEEDED"
            && identity.job_state == "REVIEW_PENDING"
            && identity.lease_state == "ACTIVE")
        {
            return Err(attempt_not_current(&evidence.job_id, &evidence.attempt_id));
        }
        transaction.commit().map_err(|_| persistence_error())
    }
}

fn load_native_identity(
    connection: &rusqlite::Connection,
    job_id: &str,
    attempt_id: &str,
    child_thread_id: &str,
) -> Result<Option<NativeIdentity>, OrchestrationError> {
    connection
        .query_row(
            "SELECT job.parent_thread_id, attempt.state, job.state, lease.state,
                    attempt.lease_id, attempt.planned_execution_kind, attempt.execution_kind,
                    instance.id, instance.codex_thread_id,
                    instance.agent_id = job.agent_id
                      AND instance.parent_thread_id = job.parent_thread_id
                      AND instance.scope_key = job.workspace_scope_key
                      AND instance.task_scope_key = job.task_scope_key
                      AND instance.runtime_fingerprint = decision.runtime_fingerprint,
                    lease.codex_agent_id
             FROM job_attempts attempt
             JOIN orchestration_jobs job ON job.job_id=attempt.job_id
             JOIN runtime_delegation_leases lease ON lease.id=attempt.lease_id
             JOIN agent_schedule_decisions decision ON decision.id=attempt.schedule_decision_id
             JOIN agent_thread_instances instance ON instance.codex_thread_id=?3
             WHERE job.job_id=?1 AND attempt.attempt_id=?2",
            params![job_id, attempt_id, child_thread_id],
            |row| {
                Ok(NativeIdentity {
                    parent_thread_id: row.get(0)?,
                    attempt_state: row.get(1)?,
                    job_state: row.get(2)?,
                    lease_state: row.get(3)?,
                    lease_id: row.get(4)?,
                    planned_execution_kind: row.get(5)?,
                    execution_kind: row.get(6)?,
                    instance_id: row.get(7)?,
                    instance_thread_id: row.get(8)?,
                    identity_matches: row.get(9)?,
                    lease_thread_id: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(|_| persistence_error())
}

fn validate_evidence_fields(
    job_id: &str,
    attempt_id: &str,
    child_thread_id: &str,
    evidence_ref: &str,
    schema_profile: &str,
) -> Result<(), OrchestrationError> {
    if [
        job_id,
        attempt_id,
        child_thread_id,
        evidence_ref,
        schema_profile,
    ]
    .iter()
    .any(|value| value.trim().is_empty())
    {
        return Err(OrchestrationError {
            code: OrchestrationErrorCode::TaskPacketFieldInvalid,
            message: "Native Child 证据字段不能为空。".to_owned(),
            field_path: Some("native_evidence".to_owned()),
            job_id: Some(job_id.to_owned()),
            attempt_id: Some(attempt_id.to_owned()),
        });
    }
    Ok(())
}

fn attempt_not_current(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::AttemptNotCurrent,
        message: "Native Child Attempt 不再可接收证据。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn execution_kind_mismatch(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::ExecutionKindMismatch,
        message: "Native Child 证据与 Job/Attempt 身份不一致。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn invariant_error(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InternalInvariantViolation,
        message: "Native Child 状态迁移未满足冻结不变量。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}
