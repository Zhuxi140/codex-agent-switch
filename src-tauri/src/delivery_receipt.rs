use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::orchestration_contract::{
    DeliveryReceipt, ExecutionKind, OrchestrationError, OrchestrationErrorCode,
    ReceiptEvidenceSource, ReceiptProgress, ReceiptStage,
};
use crate::persistence::{PersistenceError, open_database};

pub(crate) struct DeliveryReceiptRepository {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReceiptEvidence {
    pub(crate) stage: ReceiptStage,
    pub(crate) execution_kind: Option<ExecutionKind>,
    pub(crate) evidence_source: ReceiptEvidenceSource,
    pub(crate) evidence_ref: String,
    pub(crate) parent_thread_id: String,
    pub(crate) codex_thread_id: Option<String>,
    pub(crate) codex_turn_id: Option<String>,
    pub(crate) schema_profile: Option<String>,
    pub(crate) evidence_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeliveryReceiptAppendRequest {
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    /// 可同时提供缺失低阶段的证据；Repository 只接受连续追加。
    pub(crate) evidence: Vec<ReceiptEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeliveryReceiptAppendResult {
    pub(crate) receipts: Vec<DeliveryReceipt>,
}

impl DeliveryReceiptRepository {
    pub(crate) fn open(database_path: &Path) -> Result<Self, PersistenceError> {
        Ok(Self {
            connection: Mutex::new(open_database(database_path)?),
        })
    }

    pub(crate) fn progress(&self, attempt_id: &str) -> Result<ReceiptProgress, OrchestrationError> {
        let connection = self.connection()?;
        progress_in_connection(&connection, attempt_id)
    }

    pub(crate) fn append(
        &self,
        request: DeliveryReceiptAppendRequest,
    ) -> Result<DeliveryReceiptAppendResult, OrchestrationError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let result = append_in_transaction(&transaction, request)?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(result)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, OrchestrationError> {
        self.connection.lock().map_err(|_| persistence_error())
    }
}

/// 在调用方已有事务中追加 Receipt，供调度、事件映射和 Review 与状态更新原子提交。
pub(crate) fn append_in_transaction(
    transaction: &Transaction<'_>,
    request: DeliveryReceiptAppendRequest,
) -> Result<DeliveryReceiptAppendResult, OrchestrationError> {
    if request.job_id.trim().is_empty() {
        return Err(field_error("job_id"));
    }
    if request.attempt_id.trim().is_empty() {
        return Err(field_error("attempt_id"));
    }
    if request.evidence.is_empty() {
        return Err(field_error("evidence"));
    }

    let mut evidence = request.evidence;
    evidence.sort_by_key(|item| stage_order(item.stage));
    if evidence
        .windows(2)
        .any(|items| items[0].stage == items[1].stage)
    {
        return Err(invalid_transition(
            Some(&request.job_id),
            Some(&request.attempt_id),
        ));
    }
    let identity = find_attempt_identity(transaction, &request.job_id, &request.attempt_id)
        .map_err(|_| persistence_error())?
        .ok_or_else(|| attempt_not_current(&request.attempt_id))?;
    for item in &evidence {
        validate_evidence(item, &identity, &request.job_id, &request.attempt_id)?;
    }

    let mut stored =
        list_receipts(transaction, &request.attempt_id).map_err(|_| persistence_error())?;
    validate_stored_sequence(&stored, &request.job_id, &request.attempt_id)?;
    let mut result = Vec::with_capacity(evidence.len());
    for item in evidence {
        if let Some(existing) = stored.iter().find(|receipt| receipt.stage == item.stage) {
            result.push(existing.clone());
            continue;
        }
        let expected = next_stage(stored.last().map(|receipt| receipt.stage));
        if expected != Some(item.stage) {
            return Err(invalid_transition(
                Some(&request.job_id),
                Some(&request.attempt_id),
            ));
        }
        let receipt = DeliveryReceipt {
            receipt_id: Uuid::new_v4().to_string(),
            job_id: request.job_id.clone(),
            attempt_id: request.attempt_id.clone(),
            stage: item.stage,
            execution_kind: item.execution_kind,
            evidence_source: item.evidence_source,
            evidence_ref: item.evidence_ref,
            parent_thread_id: item.parent_thread_id,
            codex_thread_id: item.codex_thread_id,
            codex_turn_id: item.codex_turn_id,
            schema_profile: item.schema_profile,
            evidence_at: item.evidence_at,
            created_at: current_timestamp(transaction).map_err(|_| persistence_error())?,
        };
        insert_receipt(transaction, &receipt).map_err(|_| persistence_error())?;
        stored.push(receipt.clone());
        result.push(receipt);
    }
    Ok(DeliveryReceiptAppendResult { receipts: result })
}

/// `Transaction` 自动解引用为 `Connection`，可用于与调用方状态变更同一事务内的读取。
pub(crate) fn progress_in_connection(
    connection: &Connection,
    attempt_id: &str,
) -> Result<ReceiptProgress, OrchestrationError> {
    if attempt_id.trim().is_empty() {
        return Err(field_error("attempt_id"));
    }
    let exists = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM job_attempts WHERE attempt_id = ?1)",
            [attempt_id],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| persistence_error())?;
    if !exists {
        return Err(attempt_not_current(attempt_id));
    }
    let stage = connection
        .query_row(
            "SELECT stage FROM delivery_receipts
             WHERE attempt_id = ?1
             ORDER BY CASE stage
                WHEN 'DISPATCH_RECORDED' THEN 1
                WHEN 'TURN_ACCEPTED' THEN 2
                WHEN 'RESULT_OBSERVED' THEN 3
                WHEN 'PARENT_ACKNOWLEDGED' THEN 4
             END DESC
             LIMIT 1",
            [attempt_id],
            |row| decode_required_enum(row, 0),
        )
        .optional()
        .map_err(|_| persistence_error())?;
    Ok(ReceiptProgress::from(stage))
}

struct AttemptIdentity {
    parent_thread_id: String,
    execution_kind: Option<ExecutionKind>,
    codex_turn_id: Option<String>,
    codex_thread_id: Option<String>,
}

fn find_attempt_identity(
    connection: &Connection,
    job_id: &str,
    attempt_id: &str,
) -> rusqlite::Result<Option<AttemptIdentity>> {
    connection
        .query_row(
            "SELECT job.parent_thread_id, attempt.execution_kind, attempt.codex_turn_id,
                    instance.codex_thread_id
             FROM job_attempts attempt
             JOIN orchestration_jobs job ON job.job_id = attempt.job_id
             LEFT JOIN agent_thread_instances instance ON instance.id = attempt.thread_instance_id
             WHERE attempt.job_id = ?1 AND attempt.attempt_id = ?2",
            params![job_id, attempt_id],
            |row| {
                Ok(AttemptIdentity {
                    parent_thread_id: row.get(0)?,
                    execution_kind: decode_optional_enum(row, 1)?,
                    codex_turn_id: row.get(2)?,
                    codex_thread_id: row.get(3)?,
                })
            },
        )
        .optional()
}

fn validate_evidence(
    evidence: &ReceiptEvidence,
    identity: &AttemptIdentity,
    job_id: &str,
    attempt_id: &str,
) -> Result<(), OrchestrationError> {
    if evidence.evidence_ref.trim().is_empty()
        || evidence.parent_thread_id.trim().is_empty()
        || evidence.evidence_at.trim().is_empty()
        || evidence
            .schema_profile
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return Err(field_error("receipt_evidence"));
    }
    if evidence.parent_thread_id != identity.parent_thread_id {
        return Err(invariant_error(Some(job_id), Some(attempt_id)));
    }
    if evidence.stage == ReceiptStage::DispatchRecorded {
        if evidence.evidence_source != ReceiptEvidenceSource::CasTransaction {
            return Err(invalid_transition(Some(job_id), Some(attempt_id)));
        }
        if let (Some(actual), Some(expected)) = (evidence.execution_kind, identity.execution_kind) {
            if actual != expected {
                return Err(execution_kind_mismatch(job_id, attempt_id));
            }
        }
        return Ok(());
    }

    let execution_kind = evidence
        .execution_kind
        .ok_or_else(|| execution_kind_mismatch(job_id, attempt_id))?;
    if identity.execution_kind != Some(execution_kind) {
        return Err(execution_kind_mismatch(job_id, attempt_id));
    }
    let thread_id = evidence
        .codex_thread_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| thread_id_missing(job_id, attempt_id))?;
    if identity
        .codex_thread_id
        .as_deref()
        .is_some_and(|expected| expected != thread_id)
    {
        return Err(thread_id_mismatch(job_id, attempt_id));
    }
    if execution_kind == ExecutionKind::ManagedWorker {
        let turn_id = evidence
            .codex_turn_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| turn_id_missing(job_id, attempt_id))?;
        if identity
            .codex_turn_id
            .as_deref()
            .is_some_and(|expected| expected != turn_id)
        {
            return Err(turn_id_mismatch(job_id, attempt_id));
        }
    } else if let Some(expected) = identity.codex_turn_id.as_deref() {
        if evidence.codex_turn_id.as_deref() != Some(expected) {
            return Err(turn_id_mismatch(job_id, attempt_id));
        }
    }
    if evidence.stage == ReceiptStage::ParentAcknowledged
        && evidence.evidence_source != ReceiptEvidenceSource::PrimaryReview
    {
        return Err(invalid_transition(Some(job_id), Some(attempt_id)));
    }
    Ok(())
}

fn list_receipts(
    connection: &Connection,
    attempt_id: &str,
) -> rusqlite::Result<Vec<DeliveryReceipt>> {
    let mut statement = connection.prepare(
        "SELECT receipt_id, job_id, attempt_id, stage, execution_kind, evidence_source,
                evidence_ref, parent_thread_id, codex_thread_id, codex_turn_id,
                schema_profile, evidence_at, created_at
         FROM delivery_receipts
         WHERE attempt_id = ?1
         ORDER BY CASE stage
            WHEN 'DISPATCH_RECORDED' THEN 1
            WHEN 'TURN_ACCEPTED' THEN 2
            WHEN 'RESULT_OBSERVED' THEN 3
            WHEN 'PARENT_ACKNOWLEDGED' THEN 4
         END ASC",
    )?;
    statement
        .query_map([attempt_id], map_receipt)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn map_receipt(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryReceipt> {
    Ok(DeliveryReceipt {
        receipt_id: row.get(0)?,
        job_id: row.get(1)?,
        attempt_id: row.get(2)?,
        stage: decode_required_enum(row, 3)?,
        execution_kind: decode_optional_enum(row, 4)?,
        evidence_source: decode_required_enum(row, 5)?,
        evidence_ref: row.get(6)?,
        parent_thread_id: row.get(7)?,
        codex_thread_id: row.get(8)?,
        codex_turn_id: row.get(9)?,
        schema_profile: row.get(10)?,
        evidence_at: row.get(11)?,
        created_at: row.get(12)?,
    })
}

fn insert_receipt(connection: &Connection, receipt: &DeliveryReceipt) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO delivery_receipts (
            receipt_id, job_id, attempt_id, stage, execution_kind, evidence_source,
            evidence_ref, parent_thread_id, codex_thread_id, codex_turn_id,
            schema_profile, evidence_at, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            receipt.receipt_id,
            receipt.job_id,
            receipt.attempt_id,
            enum_name(receipt.stage),
            receipt.execution_kind.map(enum_name),
            enum_name(receipt.evidence_source),
            receipt.evidence_ref,
            receipt.parent_thread_id,
            receipt.codex_thread_id,
            receipt.codex_turn_id,
            receipt.schema_profile,
            receipt.evidence_at,
            receipt.created_at,
        ],
    )?;
    Ok(())
}

fn validate_stored_sequence(
    receipts: &[DeliveryReceipt],
    job_id: &str,
    attempt_id: &str,
) -> Result<(), OrchestrationError> {
    if receipts
        .iter()
        .enumerate()
        .any(|(index, receipt)| stage_order(receipt.stage) != index + 1)
    {
        return Err(invariant_error(Some(job_id), Some(attempt_id)));
    }
    Ok(())
}

fn stage_order(stage: ReceiptStage) -> usize {
    match stage {
        ReceiptStage::DispatchRecorded => 1,
        ReceiptStage::TurnAccepted => 2,
        ReceiptStage::ResultObserved => 3,
        ReceiptStage::ParentAcknowledged => 4,
    }
}

fn next_stage(stage: Option<ReceiptStage>) -> Option<ReceiptStage> {
    match stage {
        None => Some(ReceiptStage::DispatchRecorded),
        Some(ReceiptStage::DispatchRecorded) => Some(ReceiptStage::TurnAccepted),
        Some(ReceiptStage::TurnAccepted) => Some(ReceiptStage::ResultObserved),
        Some(ReceiptStage::ResultObserved) => Some(ReceiptStage::ParentAcknowledged),
        Some(ReceiptStage::ParentAcknowledged) => None,
    }
}

fn current_timestamp(connection: &Connection) -> rusqlite::Result<String> {
    connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get(0)
    })
}

fn enum_name<T: Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .expect("冻结枚举必须可序列化")
        .as_str()
        .expect("冻结枚举必须序列化为字符串")
        .to_owned()
}

fn decode_required_enum<T: DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let raw = row.get::<_, String>(index)?;
    serde_json::from_str(&format!("\"{raw}\"")).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn decode_optional_enum<T: DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    row.get::<_, Option<String>>(index)?
        .map(|raw| serde_json::from_str(&format!("\"{raw}\"")))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                index,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
}

fn field_error(field: &'static str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::TaskPacketFieldInvalid,
        message: format!("{field} 无效"),
        field_path: Some(field.to_owned()),
        job_id: None,
        attempt_id: None,
    }
}

fn attempt_not_current(attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::AttemptNotCurrent,
        message: "目标 Attempt 不存在或不属于指定 Job。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn invalid_transition(job_id: Option<&str>, attempt_id: Option<&str>) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InvalidStateTransition,
        message: "Receipt 阶段必须连续追加。".to_owned(),
        field_path: None,
        job_id: job_id.map(str::to_owned),
        attempt_id: attempt_id.map(str::to_owned),
    }
}

fn execution_kind_mismatch(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::ExecutionKindMismatch,
        message: "Receipt 执行身份与 Attempt 不一致或尚未证明。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn thread_id_missing(job_id: &str, attempt_id: &str) -> OrchestrationError {
    receipt_error(OrchestrationErrorCode::ThreadIdMissing, job_id, attempt_id)
}

fn thread_id_mismatch(job_id: &str, attempt_id: &str) -> OrchestrationError {
    receipt_error(OrchestrationErrorCode::ThreadIdMismatch, job_id, attempt_id)
}

fn turn_id_missing(job_id: &str, attempt_id: &str) -> OrchestrationError {
    receipt_error(OrchestrationErrorCode::TurnIdMissing, job_id, attempt_id)
}

fn turn_id_mismatch(job_id: &str, attempt_id: &str) -> OrchestrationError {
    receipt_error(OrchestrationErrorCode::TurnIdMismatch, job_id, attempt_id)
}

fn receipt_error(
    code: OrchestrationErrorCode,
    job_id: &str,
    attempt_id: &str,
) -> OrchestrationError {
    OrchestrationError {
        code,
        message: "Receipt 身份证据不完整或不一致。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn invariant_error(job_id: Option<&str>, attempt_id: Option<&str>) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InternalInvariantViolation,
        message: "Receipt 与已持久化 Job/Attempt 身份不一致。".to_owned(),
        field_path: None,
        job_id: job_id.map(str::to_owned),
        attempt_id: attempt_id.map(str::to_owned),
    }
}

fn persistence_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::PersistenceError,
        message: "Receipt 持久化失败。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rusqlite::Connection;

    use super::*;

    fn repository_with_managed_attempt() -> (DeliveryReceiptRepository, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("cas-delivery-receipt-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        let repository = DeliveryReceiptRepository::open(&database_path).unwrap();
        let connection = Connection::open(&database_path).unwrap();
        connection
            .pragma_update(None, "foreign_keys", true)
            .unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                 ) VALUES (
                    'agent-1', 'agent-1', 'Agent 1', 'test', 'test', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'INHERIT', 'CAS', 1,
                    '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z'
                 );
                 INSERT INTO orchestration_jobs (
                    job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    created_at, updated_at
                 ) VALUES (
                    'job-1', 'receipt-1', '{\"schema_version\":1}', lower(hex(zeroblob(32))),
                    'agent-1', 'parent-1', 'workspace-1', 'receipt-1', 'DISPATCHED',
                    '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z'
                 );
                 INSERT INTO agent_schedule_decisions (
                    id, created_at, source, workspace_scope_key, decision, reason_code, cache_hint
                 ) VALUES (
                    'decision-1', '2026-09-10T00:00:00Z', 'TEST', 'workspace-1',
                    'SPAWN', 'TEST', 'NONE'
                 );
                 INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id, workspace_scope_key,
                    schedule_decision_id, state, expires_at, agent_type
                 ) VALUES (
                    'lease-1', '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z', 'agent-1',
                    'parent-1', 'workspace-1', 'decision-1', 'PENDING',
                    '2026-09-11T00:00:00Z', 'executor'
                 );
                 INSERT INTO job_attempts (
                    attempt_id, job_id, attempt_no, previous_attempt_id, schedule_decision_id,
                    lease_id, route_action, planned_execution_kind, execution_kind, codex_turn_id,
                    state, created_at, updated_at, dispatch_recorded_at
                 ) VALUES (
                    'attempt-1', 'job-1', 1, NULL, 'decision-1', 'lease-1', 'SPAWN',
                    'MANAGED_WORKER', 'MANAGED_WORKER', 'turn-1', 'DISPATCHING',
                    '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z',
                    '2026-09-10T00:00:00Z'
                 );",
            )
            .unwrap();
        (repository, root)
    }

    fn evidence(stage: ReceiptStage) -> ReceiptEvidence {
        let (execution_kind, evidence_source, codex_thread_id, codex_turn_id) = match stage {
            ReceiptStage::DispatchRecorded => {
                (None, ReceiptEvidenceSource::CasTransaction, None, None)
            }
            ReceiptStage::TurnAccepted => (
                Some(ExecutionKind::ManagedWorker),
                ReceiptEvidenceSource::AppServerResponse,
                Some("thread-1".to_owned()),
                Some("turn-1".to_owned()),
            ),
            ReceiptStage::ResultObserved => (
                Some(ExecutionKind::ManagedWorker),
                ReceiptEvidenceSource::RuntimeEvent,
                Some("thread-1".to_owned()),
                Some("turn-1".to_owned()),
            ),
            ReceiptStage::ParentAcknowledged => (
                Some(ExecutionKind::ManagedWorker),
                ReceiptEvidenceSource::PrimaryReview,
                Some("thread-1".to_owned()),
                Some("turn-1".to_owned()),
            ),
        };
        ReceiptEvidence {
            stage,
            execution_kind,
            evidence_source,
            evidence_ref: format!("evidence-{stage:?}"),
            parent_thread_id: "parent-1".to_owned(),
            codex_thread_id,
            codex_turn_id,
            schema_profile: Some("modern".to_owned()),
            evidence_at: "2026-09-10T00:00:00Z".to_owned(),
        }
    }

    fn request(evidence: Vec<ReceiptEvidence>) -> DeliveryReceiptAppendRequest {
        DeliveryReceiptAppendRequest {
            job_id: "job-1".to_owned(),
            attempt_id: "attempt-1".to_owned(),
            evidence,
        }
    }

    #[test]
    fn no_receipt_is_unknown_and_high_stage_cannot_jump() {
        let (repository, _root) = repository_with_managed_attempt();
        assert_eq!(
            repository.progress("attempt-1").unwrap(),
            ReceiptProgress::Unknown
        );
        assert_eq!(
            repository
                .append(request(vec![evidence(ReceiptStage::TurnAccepted)]))
                .unwrap_err()
                .code,
            OrchestrationErrorCode::InvalidStateTransition
        );
        assert_eq!(
            repository.progress("attempt-1").unwrap(),
            ReceiptProgress::Unknown
        );
    }

    #[test]
    fn missing_lower_stages_can_only_be_filled_continuously_in_one_transaction() {
        let (repository, _root) = repository_with_managed_attempt();
        let appended = repository
            .append(request(vec![
                evidence(ReceiptStage::ParentAcknowledged),
                evidence(ReceiptStage::ResultObserved),
                evidence(ReceiptStage::TurnAccepted),
                evidence(ReceiptStage::DispatchRecorded),
            ]))
            .unwrap();
        assert_eq!(
            appended
                .receipts
                .iter()
                .map(|receipt| receipt.stage)
                .collect::<Vec<_>>(),
            vec![
                ReceiptStage::DispatchRecorded,
                ReceiptStage::TurnAccepted,
                ReceiptStage::ResultObserved,
                ReceiptStage::ParentAcknowledged,
            ]
        );
        assert_eq!(
            repository.progress("attempt-1").unwrap(),
            ReceiptProgress::ParentAcknowledged
        );
    }

    #[test]
    fn duplicate_stage_returns_existing_receipt_without_mutating_it() {
        let (repository, _root) = repository_with_managed_attempt();
        let first = repository
            .append(request(vec![evidence(ReceiptStage::DispatchRecorded)]))
            .unwrap()
            .receipts
            .into_iter()
            .next()
            .unwrap();
        let mut duplicate = evidence(ReceiptStage::DispatchRecorded);
        duplicate.evidence_ref = "new evidence is ignored".to_owned();
        let repeated = repository
            .append(request(vec![duplicate]))
            .unwrap()
            .receipts
            .into_iter()
            .next()
            .unwrap();
        assert_eq!(repeated, first);
    }

    #[test]
    fn parent_and_turn_identity_must_match_the_job_and_attempt() {
        let (repository, _root) = repository_with_managed_attempt();
        let mut wrong_parent = evidence(ReceiptStage::DispatchRecorded);
        wrong_parent.parent_thread_id = "other-parent".to_owned();
        assert_eq!(
            repository
                .append(request(vec![wrong_parent]))
                .unwrap_err()
                .code,
            OrchestrationErrorCode::InternalInvariantViolation
        );

        let mut wrong_turn = evidence(ReceiptStage::TurnAccepted);
        wrong_turn.codex_turn_id = Some("other-turn".to_owned());
        assert_eq!(
            repository
                .append(request(vec![
                    evidence(ReceiptStage::DispatchRecorded),
                    wrong_turn
                ]))
                .unwrap_err()
                .code,
            OrchestrationErrorCode::TurnIdMismatch
        );
        assert_eq!(
            repository.progress("attempt-1").unwrap(),
            ReceiptProgress::Unknown
        );
    }
}
