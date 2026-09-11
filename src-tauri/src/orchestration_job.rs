use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::orchestration_contract::{
    AttemptState, IdempotencyOutcome, JobAttempt, JobState, OrchestrationError,
    OrchestrationErrorCode, OrchestrationJob, TaskPacket,
};
use crate::persistence::{PersistenceError, open_database};

pub(crate) mod query;
mod review;
mod reviewer;
mod schedule;
pub(crate) use query::{OrchestrationJobListRequest, OrchestrationJobPageResponse};
pub(crate) use review::{OrchestrationJobReviewRequest, OrchestrationJobReviewResponse};
pub(crate) use reviewer::{
    OrchestrationReviewerCreateRequest, OrchestrationReviewerCreateResponse,
    OrchestrationReviewerReport, OrchestrationReviewerReportSubmitRequest,
};
pub(crate) use schedule::{
    AtomicScheduleOutcome, AtomicScheduleRequest, DispatchAdmission, DispatchAgentProfile,
    DispatchPermit, ScheduleStop, release_undispatched_occupancy_for_mode_switch,
};

pub(crate) struct OrchestrationJobService {
    connection: Mutex<Connection>,
}

impl OrchestrationJobService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, PersistenceError> {
        Ok(Self {
            connection: Mutex::new(open_database(database_path)?),
        })
    }

    pub(crate) fn create_or_get(
        &self,
        request: OrchestrationJobCreateRequest,
    ) -> Result<OrchestrationJobCreateResponse, OrchestrationError> {
        request.task_packet.validate()?;
        let canonical_packet = request.task_packet.canonical_form()?;
        let packet_hash = request.task_packet.task_packet_hash()?;
        let packet = request.task_packet;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;

        if let Some(job) = find_job_by_idempotency_scope(
            &transaction,
            &packet.workspace_scope_key,
            &packet.parent_thread_id,
            &packet.idempotency_key,
        )
        .map_err(|_| persistence_error())?
        {
            if job.task_packet_hash != packet_hash {
                return Err(idempotency_conflict(&job.job_id));
            }
            let current_attempt =
                find_latest_attempt(&transaction, &job.job_id).map_err(|_| persistence_error())?;
            let outcome = classify_existing(&transaction, &job, current_attempt.as_ref())
                .map_err(|_| persistence_error())?;
            transaction.commit().map_err(|_| persistence_error())?;
            return Ok(OrchestrationJobCreateResponse {
                outcome,
                job,
                current_attempt,
            });
        }

        if find_job_by_id(&transaction, &packet.job_id)
            .map_err(|_| persistence_error())?
            .is_some()
        {
            return Err(idempotency_conflict(&packet.job_id));
        }

        transaction
            .execute(
                "INSERT INTO orchestration_jobs (
                    job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    last_error_code, created_at, updated_at, terminal_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'CREATED', NULL,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL
                 )",
                params![
                    packet.job_id,
                    packet.idempotency_key,
                    canonical_packet,
                    packet_hash,
                    packet.agent_id,
                    packet.parent_thread_id,
                    packet.workspace_scope_key,
                    packet.task_scope_key,
                ],
            )
            .map_err(|_| persistence_error())?;
        let job = find_job_by_id(&transaction, &packet.job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(invariant_error)?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(OrchestrationJobCreateResponse {
            outcome: IdempotencyOutcome::Created,
            job,
            current_attempt: None,
        })
    }

    pub(crate) fn get(
        &self,
        request: OrchestrationJobGetRequest,
    ) -> Result<Option<OrchestrationJobDetailResponse>, OrchestrationError> {
        if request.job_id.trim().is_empty() {
            return Err(field_error("job_id"));
        }
        let connection = self.connection()?;
        let Some(job) =
            find_job_by_id(&connection, &request.job_id).map_err(|_| persistence_error())?
        else {
            return Ok(None);
        };
        let attempts =
            list_attempts(&connection, &request.job_id).map_err(|_| persistence_error())?;
        Ok(Some(OrchestrationJobDetailResponse { job, attempts }))
    }

    #[allow(dead_code)]
    pub(crate) fn transition_job(
        &self,
        job_id: &str,
        expected: JobState,
        next: JobState,
        last_error_code: Option<OrchestrationErrorCode>,
    ) -> Result<OrchestrationJob, OrchestrationError> {
        if !expected.can_transition_to(next) {
            return Err(invalid_transition(Some(job_id), None));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let current = find_job_by_id(&transaction, job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invalid_transition(Some(job_id), None))?;
        if current.state != expected {
            return Err(invalid_transition(Some(job_id), None));
        }
        let now = current_timestamp(&transaction).map_err(|_| persistence_error())?;
        update_job_state(&transaction, job_id, expected, next, last_error_code, &now)?;
        let job = find_job_by_id(&transaction, job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(invariant_error)?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(job)
    }

    /// Attempt 状态及其映射 Job 状态在同一事务提交；本方法只接收既有 Attempt，
    /// 创建 Attempt 仍属于后续原子 Route/Lease 事务（Phase C）。
    #[allow(dead_code)]
    pub(crate) fn transition_attempt(
        &self,
        attempt_id: &str,
        expected: AttemptState,
        next: AttemptState,
        last_error_code: Option<OrchestrationErrorCode>,
    ) -> Result<JobAttempt, OrchestrationError> {
        if !expected.can_transition_to(next) {
            return Err(invalid_transition(None, Some(attempt_id)));
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let current = find_attempt_by_id(&transaction, attempt_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| attempt_not_current(attempt_id))?;
        if current.state != expected {
            return Err(attempt_not_current(attempt_id));
        }
        let job = find_job_by_id(&transaction, &current.job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(invariant_error)?;
        let next_job_state = job_state_for_attempt(next);
        if job.state != next_job_state && !job.state.can_transition_to(next_job_state) {
            return Err(invalid_transition(Some(&current.job_id), Some(attempt_id)));
        }

        let now = current_timestamp(&transaction).map_err(|_| persistence_error())?;
        if job.state != next_job_state {
            update_job_state(
                &transaction,
                &current.job_id,
                job.state,
                next_job_state,
                last_error_code,
                &now,
            )?;
        }
        let changed = transaction
            .execute(
                "UPDATE job_attempts
                 SET state = ?2,
                     last_error_code = ?3,
                     updated_at = ?4,
                     dispatch_recorded_at = CASE
                         WHEN ?2 = 'DISPATCHING'
                         THEN COALESCE(dispatch_recorded_at, ?4)
                         ELSE dispatch_recorded_at
                     END,
                     accepted_at = CASE
                         WHEN ?2 = 'ACCEPTED'
                         THEN COALESCE(accepted_at, ?4)
                         ELSE accepted_at
                     END,
                     terminal_at = ?5
                 WHERE attempt_id = ?1 AND state = ?6",
                params![
                    attempt_id,
                    enum_name(next),
                    last_error_code.map(enum_name),
                    now,
                    next.is_terminal().then_some(now.as_str()),
                    enum_name(expected),
                ],
            )
            .map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(attempt_not_current(attempt_id));
        }
        let attempt = find_attempt_by_id(&transaction, attempt_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(invariant_error)?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(attempt)
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, OrchestrationError> {
        self.connection.lock().map_err(|_| persistence_error())
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            connection: Mutex::new(crate::persistence::open_in_memory().unwrap()),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobCreateRequest {
    pub(crate) task_packet: TaskPacket,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobCreateResponse {
    pub(crate) outcome: IdempotencyOutcome,
    pub(crate) job: OrchestrationJob,
    pub(crate) current_attempt: Option<JobAttempt>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobGetRequest {
    pub(crate) job_id: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobDetailResponse {
    pub(crate) job: OrchestrationJob,
    pub(crate) attempts: Vec<JobAttempt>,
}

fn find_job_by_idempotency_scope(
    connection: &Connection,
    workspace_scope_key: &str,
    parent_thread_id: &str,
    idempotency_key: &str,
) -> rusqlite::Result<Option<OrchestrationJob>> {
    connection
        .query_row(
            "SELECT job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    last_error_code, created_at, updated_at, terminal_at
             FROM orchestration_jobs
             WHERE workspace_scope_key = ?1
               AND parent_thread_id = ?2
               AND idempotency_key = ?3",
            params![workspace_scope_key, parent_thread_id, idempotency_key],
            map_job,
        )
        .optional()
}

fn find_job_by_id(
    connection: &Connection,
    job_id: &str,
) -> rusqlite::Result<Option<OrchestrationJob>> {
    connection
        .query_row(
            "SELECT job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    last_error_code, created_at, updated_at, terminal_at
             FROM orchestration_jobs
             WHERE job_id = ?1",
            [job_id],
            map_job,
        )
        .optional()
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrchestrationJob> {
    let packet_json = row.get::<_, String>(2)?;
    Ok(OrchestrationJob {
        job_id: row.get(0)?,
        idempotency_key: row.get(1)?,
        task_packet: decode_json(2, &packet_json)?,
        task_packet_hash: row.get(3)?,
        agent_id: row.get(4)?,
        parent_thread_id: row.get(5)?,
        workspace_scope_key: row.get(6)?,
        task_scope_key: row.get(7)?,
        state: decode_required_enum(row, 8)?,
        last_error_code: decode_optional_enum(row, 9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        terminal_at: row.get(12)?,
    })
}

fn find_latest_attempt(
    connection: &Connection,
    job_id: &str,
) -> rusqlite::Result<Option<JobAttempt>> {
    connection
        .query_row(
            "SELECT attempt_id, job_id, attempt_no, previous_attempt_id,
                    schedule_decision_id, lease_id, route_action, planned_execution_kind,
                    execution_kind, thread_instance_id, codex_turn_id, state,
                    recovery_count, last_error_code, created_at, updated_at,
                    dispatch_recorded_at, accepted_at, terminal_at
             FROM job_attempts
             WHERE job_id = ?1
             ORDER BY attempt_no DESC
             LIMIT 1",
            [job_id],
            map_attempt,
        )
        .optional()
}

fn find_attempt_by_id(
    connection: &Connection,
    attempt_id: &str,
) -> rusqlite::Result<Option<JobAttempt>> {
    connection
        .query_row(
            "SELECT attempt_id, job_id, attempt_no, previous_attempt_id,
                    schedule_decision_id, lease_id, route_action, planned_execution_kind,
                    execution_kind, thread_instance_id, codex_turn_id, state,
                    recovery_count, last_error_code, created_at, updated_at,
                    dispatch_recorded_at, accepted_at, terminal_at
             FROM job_attempts
             WHERE attempt_id = ?1",
            [attempt_id],
            map_attempt,
        )
        .optional()
}

fn list_attempts(connection: &Connection, job_id: &str) -> rusqlite::Result<Vec<JobAttempt>> {
    let mut statement = connection.prepare(
        "SELECT attempt_id, job_id, attempt_no, previous_attempt_id,
                schedule_decision_id, lease_id, route_action, planned_execution_kind,
                execution_kind, thread_instance_id, codex_turn_id, state,
                recovery_count, last_error_code, created_at, updated_at,
                dispatch_recorded_at, accepted_at, terminal_at
         FROM job_attempts
         WHERE job_id = ?1
         ORDER BY attempt_no ASC",
    )?;
    statement
        .query_map([job_id], map_attempt)?
        .collect::<rusqlite::Result<Vec<_>>>()
}

fn map_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobAttempt> {
    Ok(JobAttempt {
        attempt_id: row.get(0)?,
        job_id: row.get(1)?,
        attempt_no: row.get(2)?,
        previous_attempt_id: row.get(3)?,
        schedule_decision_id: row.get(4)?,
        lease_id: row.get(5)?,
        route_action: decode_required_enum(row, 6)?,
        planned_execution_kind: decode_required_enum(row, 7)?,
        execution_kind: decode_optional_enum(row, 8)?,
        thread_instance_id: row.get(9)?,
        codex_turn_id: row.get(10)?,
        state: decode_required_enum(row, 11)?,
        recovery_count: row.get(12)?,
        last_error_code: decode_optional_enum(row, 13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
        dispatch_recorded_at: row.get(16)?,
        accepted_at: row.get(17)?,
        terminal_at: row.get(18)?,
    })
}

fn classify_existing(
    connection: &Connection,
    job: &OrchestrationJob,
    current_attempt: Option<&JobAttempt>,
) -> rusqlite::Result<IdempotencyOutcome> {
    let dispatch_recorded = connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM job_attempts
            WHERE job_id = ?1 AND dispatch_recorded_at IS NOT NULL
         )",
        [&job.job_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !dispatch_recorded {
        return Ok(IdempotencyOutcome::ExistingNotDispatched);
    }
    if job.state == JobState::Uncertain
        || current_attempt.is_some_and(|attempt| {
            matches!(
                attempt.state,
                AttemptState::Dispatching | AttemptState::Uncertain
            )
        })
    {
        Ok(IdempotencyOutcome::ExistingUncertain)
    } else {
        Ok(IdempotencyOutcome::ExistingKnown)
    }
}

fn update_job_state(
    transaction: &Transaction<'_>,
    job_id: &str,
    expected: JobState,
    next: JobState,
    last_error_code: Option<OrchestrationErrorCode>,
    now: &str,
) -> Result<(), OrchestrationError> {
    let changed = transaction
        .execute(
            "UPDATE orchestration_jobs
             SET state = ?2,
                 last_error_code = ?3,
                 updated_at = ?4,
                 terminal_at = ?5
             WHERE job_id = ?1 AND state = ?6",
            params![
                job_id,
                enum_name(next),
                last_error_code.map(enum_name),
                now,
                next.is_terminal().then_some(now),
                enum_name(expected),
            ],
        )
        .map_err(|_| persistence_error())?;
    if changed == 1 {
        Ok(())
    } else {
        Err(invalid_transition(Some(job_id), None))
    }
}

fn current_timestamp(connection: &Connection) -> rusqlite::Result<String> {
    connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get(0)
    })
}

fn job_state_for_attempt(state: AttemptState) -> JobState {
    match state {
        AttemptState::Planned => JobState::Claimed,
        AttemptState::Dispatching => JobState::Dispatched,
        AttemptState::Accepted | AttemptState::Running => JobState::Running,
        AttemptState::Succeeded => JobState::ResultReceived,
        AttemptState::Uncertain => JobState::Uncertain,
        AttemptState::Failed => JobState::Failed,
        AttemptState::Cancelled => JobState::Cancelled,
    }
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
    decode_json(index, &format!("\"{raw}\""))
}

fn decode_optional_enum<T: DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<T>> {
    let raw = row.get::<_, Option<String>>(index)?;
    raw.map(|raw| decode_json(index, &format!("\"{raw}\"")))
        .transpose()
}

fn decode_json<T: DeserializeOwned>(index: usize, raw: &str) -> rusqlite::Result<T> {
    serde_json::from_str(raw).map_err(|error| {
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

fn idempotency_conflict(job_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::IdempotencyKeyConflict,
        message: "同一幂等范围已存在不同 TaskPacket。".to_owned(),
        field_path: Some("idempotency_key".to_owned()),
        job_id: Some(job_id.to_owned()),
        attempt_id: None,
    }
}

fn invalid_transition(job_id: Option<&str>, attempt_id: Option<&str>) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InvalidStateTransition,
        message: "请求的编排状态迁移不合法或当前状态已变化。".to_owned(),
        field_path: None,
        job_id: job_id.map(str::to_owned),
        attempt_id: attempt_id.map(str::to_owned),
    }
}

fn attempt_not_current(attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::AttemptNotCurrent,
        message: "目标 Attempt 不存在、已变化或不是当前可迁移 Attempt。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: Some(attempt_id.to_owned()),
    }
}

fn persistence_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::PersistenceError,
        message: "编排数据持久化失败。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

fn invariant_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InternalInvariantViolation,
        message: "编排数据违反内部不变量。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

#[cfg(test)]
mod tests {
    use crate::orchestration_contract::{
        ExecutionKindPolicy, OutputContract, PermissionPolicy, ReviewPolicy,
        TASK_PACKET_SCHEMA_VERSION,
    };

    use super::*;

    fn sample_packet() -> TaskPacket {
        TaskPacket {
            schema_version: TASK_PACKET_SCHEMA_VERSION,
            job_id: "job-1".to_owned(),
            idempotency_key: "b03-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            workspace_scope_key: "workspace-1".to_owned(),
            task_scope_key: "b03".to_owned(),
            objective: "验证 Job 幂等服务".to_owned(),
            allowed_scope: vec!["src-tauri/src/orchestration_job.rs".to_owned()],
            constraints: Vec::new(),
            success_criteria: vec!["重试不创建第二个 Job".to_owned()],
            allowed_tools: Vec::new(),
            permission_policy: PermissionPolicy::WorkspaceWrite,
            execution_kind_policy: ExecutionKindPolicy::ManagedWorkerRequired,
            context_references: Vec::new(),
            output_contract: OutputContract::StandardV1,
            review_policy: ReviewPolicy::PrimaryRequired,
        }
    }

    fn service() -> OrchestrationJobService {
        let service = OrchestrationJobService::in_memory();
        service
            .connection
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                 ) VALUES (
                    'agent-1', 'agent-1', 'Agent 1', 'test', 'test', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'INHERIT', 'CAS', 1,
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 )",
                [],
            )
            .unwrap();
        service
    }

    fn create(service: &OrchestrationJobService) -> OrchestrationJobCreateResponse {
        service
            .create_or_get(OrchestrationJobCreateRequest {
                task_packet: sample_packet(),
            })
            .unwrap()
    }

    fn seed_attempt(service: &OrchestrationJobService, state: AttemptState, dispatched: bool) {
        let connection = service.connection.lock().unwrap();
        connection
            .execute(
                "INSERT INTO agent_schedule_decisions (
                    id, created_at, source, agent_id, workspace_scope_key,
                    parent_thread_id, decision, reason_code, cache_hint
                 ) VALUES (
                    'decision-1', '2026-09-09T00:00:00Z', 'TEST', 'agent-1',
                    'workspace-1', 'parent-1', 'SPAWN', 'TEST', 'UNKNOWN'
                 )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id,
                    workspace_scope_key, task_scope_key, schedule_decision_id,
                    state, expires_at
                 ) VALUES (
                    'lease-1', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z',
                    'agent-1', 'parent-1', 'workspace-1', 'b03', 'decision-1',
                    'PENDING', '2026-09-09T01:00:00Z'
                 )",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO job_attempts (
                    attempt_id, job_id, attempt_no, schedule_decision_id, lease_id,
                    route_action, planned_execution_kind, state, recovery_count,
                    created_at, updated_at, dispatch_recorded_at
                 ) VALUES (
                    'attempt-1', 'job-1', 1, 'decision-1', 'lease-1',
                    'SPAWN', 'MANAGED_WORKER', ?1, 0,
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z', ?2
                 )",
                params![
                    enum_name(state),
                    dispatched.then_some("2026-09-09T00:00:00Z")
                ],
            )
            .unwrap();
    }

    #[test]
    fn identical_retry_returns_existing_job_without_duplicate() {
        let service = service();
        let created = create(&service);
        assert_eq!(created.outcome, IdempotencyOutcome::Created);

        let retried = create(&service);
        assert_eq!(retried.outcome, IdempotencyOutcome::ExistingNotDispatched);
        assert_eq!(retried.job.job_id, "job-1");
        assert_eq!(retried.current_attempt, None);
        let count = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM orchestration_jobs", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn create_response_serialization_preserves_ipc_naming_contract() {
        let service = service();
        create(&service);
        seed_attempt(&service, AttemptState::Planned, false);
        service
            .connection
            .lock()
            .unwrap()
            .execute(
                "UPDATE job_attempts SET execution_kind = 'MANAGED_WORKER' WHERE attempt_id = 'attempt-1'",
                [],
            )
            .unwrap();

        let value = serde_json::to_value(create(&service)).unwrap();
        let object = value.as_object().unwrap();
        assert!(object.contains_key("currentAttempt"));
        assert!(!object.contains_key("current_attempt"));

        let job = object.get("job").unwrap().as_object().unwrap();
        assert!(job.contains_key("job_id"));
        assert!(job.contains_key("task_packet"));
        assert!(!job.contains_key("jobId"));
        let packet = job.get("task_packet").unwrap().as_object().unwrap();
        assert!(packet.contains_key("execution_kind_policy"));
        assert!(!packet.contains_key("executionKindPolicy"));

        let attempt = object.get("currentAttempt").unwrap().as_object().unwrap();
        assert_eq!(
            attempt
                .get("execution_kind")
                .and_then(|value| value.as_str()),
            Some("MANAGED_WORKER")
        );
        assert!(!attempt.contains_key("executionKind"));
    }

    #[test]
    fn same_idempotency_scope_with_different_hash_is_rejected() {
        let service = service();
        create(&service);
        let mut packet = sample_packet();
        packet.objective = "不同目标".to_owned();
        let error = service
            .create_or_get(OrchestrationJobCreateRequest {
                task_packet: packet,
            })
            .unwrap_err();
        assert_eq!(error.code, OrchestrationErrorCode::IdempotencyKeyConflict);
        assert_eq!(error.job_id.as_deref(), Some("job-1"));
    }

    #[test]
    fn job_read_returns_canonical_packet_and_rejects_illegal_transition() {
        let service = service();
        let created = create(&service);
        assert_eq!(
            created.job.task_packet_hash,
            sample_packet().task_packet_hash().unwrap()
        );
        let detail = service
            .get(OrchestrationJobGetRequest {
                job_id: "job-1".to_owned(),
            })
            .unwrap()
            .unwrap();
        assert_eq!(detail.job.task_packet, sample_packet());
        assert!(detail.attempts.is_empty());

        let error = service
            .transition_job("job-1", JobState::Created, JobState::Completed, None)
            .unwrap_err();
        assert_eq!(error.code, OrchestrationErrorCode::InvalidStateTransition);
        assert_eq!(
            service
                .get(OrchestrationJobGetRequest {
                    job_id: "job-1".to_owned()
                })
                .unwrap()
                .unwrap()
                .job
                .state,
            JobState::Created
        );
    }

    #[test]
    fn uncertain_retry_returns_existing_without_creating_attempt_or_turn() {
        let service = service();
        create(&service);
        service
            .transition_job("job-1", JobState::Created, JobState::Routed, None)
            .unwrap();
        service
            .transition_job("job-1", JobState::Routed, JobState::Uncertain, None)
            .unwrap();
        seed_attempt(&service, AttemptState::Uncertain, true);

        let retried = create(&service);
        assert_eq!(retried.outcome, IdempotencyOutcome::ExistingUncertain);
        assert_eq!(retried.current_attempt.unwrap().attempt_id, "attempt-1");
        let attempt_count = service
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM job_attempts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(attempt_count, 1);
    }

    #[test]
    fn attempt_transition_updates_job_atomically_and_classifies_dispatch_boundary() {
        let service = service();
        create(&service);
        service
            .transition_job("job-1", JobState::Created, JobState::Routed, None)
            .unwrap();
        service
            .transition_job("job-1", JobState::Routed, JobState::Claimed, None)
            .unwrap();
        seed_attempt(&service, AttemptState::Planned, false);

        let dispatching = service
            .transition_attempt(
                "attempt-1",
                AttemptState::Planned,
                AttemptState::Dispatching,
                None,
            )
            .unwrap();
        assert!(dispatching.dispatch_recorded_at.is_some());
        assert_eq!(
            create(&service).outcome,
            IdempotencyOutcome::ExistingUncertain
        );

        let illegal = service
            .transition_attempt(
                "attempt-1",
                AttemptState::Dispatching,
                AttemptState::Succeeded,
                None,
            )
            .unwrap_err();
        assert_eq!(illegal.code, OrchestrationErrorCode::InvalidStateTransition);

        service
            .transition_attempt(
                "attempt-1",
                AttemptState::Dispatching,
                AttemptState::Accepted,
                None,
            )
            .unwrap();
        let succeeded = service
            .transition_attempt(
                "attempt-1",
                AttemptState::Accepted,
                AttemptState::Succeeded,
                None,
            )
            .unwrap();
        assert!(succeeded.accepted_at.is_some());
        assert!(succeeded.terminal_at.is_some());
        let retried = create(&service);
        assert_eq!(retried.outcome, IdempotencyOutcome::ExistingKnown);
        assert_eq!(retried.job.state, JobState::ResultReceived);
    }
}
