//! `cas-helper` 复用的 Runtime First 原生控制面。
//!
//! 这里只暴露窄 DTO；TaskPacket、Job、Attempt、Receipt 与 Review 的事务实现仍只有一份。

use std::fmt;
use std::path::Path;

use cas_scheduler::hard_gates::{CacheRequirement, Capability};
use rusqlite::OptionalExtension;
use serde::Deserialize;

use crate::orchestration_contract::{
    ExecutionKindPolicy, IdempotencyOutcome, OutputContract, PermissionPolicy, ReviewOutcome,
    ReviewPolicy, TaskPacket,
};
use crate::orchestration_job::{
    AtomicScheduleOutcome, DispatchAdmission, NativeBindingEvidence, NativeResultEvidence,
    OrchestrationJobReviewRequest, OrchestrationJobService,
};
use crate::persistence::open_database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeScheduleResult {
    pub action: String,
    pub thread_id: Option<String>,
    pub reason_code: String,
    pub job_id: String,
    pub attempt_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeReviewResult {
    pub review_id: String,
    pub job_id: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeControlError {
    pub code: String,
    pub message: String,
}

impl fmt::Display for NativeControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for NativeControlError {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeTaskPacketDraft {
    schema_version: u32,
    job_id: String,
    idempotency_key: String,
    task_scope_key: String,
    objective: String,
    allowed_scope: Vec<String>,
    constraints: Vec<String>,
    success_criteria: Vec<String>,
    allowed_tools: Vec<String>,
    permission_policy: PermissionPolicy,
    execution_kind_policy: ExecutionKindPolicy,
    context_references: Vec<String>,
    output_contract: OutputContract,
    review_policy: ReviewPolicy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReviewDraft {
    decision: ReviewOutcome,
    reason: String,
    evidence_refs: Vec<String>,
}

pub fn schedule_native_task(
    database_path: &Path,
    agent_key: &str,
    workspace_scope_key: &str,
    parent_thread_id: &str,
    payload: &[u8],
) -> Result<NativeScheduleResult, NativeControlError> {
    let draft = serde_json::from_slice::<NativeTaskPacketDraft>(payload)
        .map_err(|_| input_error("TaskPacket draft 不是冻结的 JSON 结构。"))?;
    if !is_protocol_id(&draft.job_id) {
        return Err(input_error(
            "job_id 必须是 1～64 位 ASCII 字母、数字、点、下划线、冒号或连字符。",
        ));
    }
    let workspace_scope_key = cas_scheduler::normalize_workspace_scope_key(workspace_scope_key)
        .ok_or_else(|| input_error("workspace_scope_key 不是可规范化的绝对路径。"))?;
    if parent_thread_id.trim().is_empty() {
        return Err(input_error("parent_thread_id 不能为空。"));
    }
    let connection = open_database(database_path).map_err(|_| persistence_error())?;
    let agent_id = connection
        .query_row(
            "SELECT agent.id
             FROM active_agent_bindings active
             JOIN agents agent ON agent.id=active.agent_id
             WHERE agent.agent_key=?1 AND agent.enabled=1",
            [agent_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| persistence_error())?
        .ok_or_else(|| NativeControlError {
            code: "AGENT_NOT_EXECUTABLE".to_owned(),
            message: "未找到匹配的 Active Agent。".to_owned(),
        })?;
    drop(connection);

    let packet = TaskPacket {
        schema_version: draft.schema_version,
        job_id: draft.job_id,
        idempotency_key: draft.idempotency_key,
        agent_id,
        parent_thread_id: parent_thread_id.to_owned(),
        workspace_scope_key,
        task_scope_key: draft.task_scope_key,
        objective: draft.objective,
        allowed_scope: draft.allowed_scope,
        constraints: draft.constraints,
        success_criteria: draft.success_criteria,
        allowed_tools: draft.allowed_tools,
        permission_policy: draft.permission_policy,
        execution_kind_policy: draft.execution_kind_policy,
        context_references: draft.context_references,
        output_contract: draft.output_contract,
        review_policy: draft.review_policy,
    };
    if !packet
        .execution_kind_policy
        .allows(crate::orchestration_contract::ExecutionKind::NativeChild)
    {
        return Err(NativeControlError {
            code: "EXECUTION_KIND_UNSUPPORTED".to_owned(),
            message: "TaskPacket 不允许 NATIVE_CHILD。".to_owned(),
        });
    }
    let service = OrchestrationJobService::open(database_path).map_err(|_| persistence_error())?;
    let outcome = service
        .schedule_native_atomic(packet, native_admission())
        .map_err(NativeControlError::from)?;
    match outcome {
        AtomicScheduleOutcome::Ready(permit) => {
            let action = enum_name(permit.attempt().route_action);
            let result = NativeScheduleResult {
                action,
                thread_id: permit.candidate_thread_id().map(str::to_owned),
                reason_code: permit.reason_code().to_owned(),
                job_id: permit.job().job_id.clone(),
                attempt_id: Some(permit.attempt().attempt_id.clone()),
            };
            service
                .authorize_dispatch(&permit)
                .map_err(NativeControlError::from)?;
            Ok(result)
        }
        AtomicScheduleOutcome::Waiting(stop) => Ok(NativeScheduleResult {
            action: "WAIT".to_owned(),
            thread_id: None,
            reason_code: stop.reason_code,
            job_id: stop.job.job_id,
            attempt_id: None,
        }),
        AtomicScheduleOutcome::Blocked(stop) => Ok(NativeScheduleResult {
            action: "BLOCK".to_owned(),
            thread_id: None,
            reason_code: stop.reason_code,
            job_id: stop.job.job_id,
            attempt_id: None,
        }),
        AtomicScheduleOutcome::Existing(existing) => {
            let (action, reason_code) = match existing.outcome {
                IdempotencyOutcome::ExistingUncertain => ("UNCERTAIN", "EXISTING_UNCERTAIN"),
                IdempotencyOutcome::ExistingNotDispatched => ("WAIT", "EXISTING_NOT_DISPATCHED"),
                IdempotencyOutcome::Created
                | IdempotencyOutcome::ExistingKnown
                | IdempotencyOutcome::KeyConflict => ("EXISTING", "EXISTING_KNOWN"),
            };
            Ok(NativeScheduleResult {
                action: action.to_owned(),
                thread_id: None,
                reason_code: reason_code.to_owned(),
                job_id: existing.job.job_id,
                attempt_id: existing.current_attempt.map(|attempt| attempt.attempt_id),
            })
        }
    }
}

pub fn accept_native_binding(
    database_path: &Path,
    job_id: &str,
    attempt_id: &str,
    child_thread_id: &str,
    evidence_ref: &str,
    schema_profile: &str,
) -> Result<(), NativeControlError> {
    OrchestrationJobService::open(database_path)
        .map_err(|_| persistence_error())?
        .accept_native_binding(NativeBindingEvidence {
            job_id: job_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
            child_thread_id: child_thread_id.to_owned(),
            evidence_ref: evidence_ref.to_owned(),
            schema_profile: schema_profile.to_owned(),
        })
        .map_err(NativeControlError::from)
}

pub fn observe_native_result(
    database_path: &Path,
    job_id: &str,
    attempt_id: &str,
    child_thread_id: &str,
    evidence_ref: &str,
    schema_profile: &str,
    reusable: bool,
) -> Result<(), NativeControlError> {
    OrchestrationJobService::open(database_path)
        .map_err(|_| persistence_error())?
        .observe_native_result(NativeResultEvidence {
            job_id: job_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
            child_thread_id: child_thread_id.to_owned(),
            evidence_ref: evidence_ref.to_owned(),
            schema_profile: schema_profile.to_owned(),
            reusable,
        })
        .map_err(NativeControlError::from)
}

pub fn review_native_task(
    database_path: &Path,
    job_id: &str,
    attempt_id: &str,
    parent_thread_id: &str,
    payload: &[u8],
) -> Result<NativeReviewResult, NativeControlError> {
    let draft = serde_json::from_slice::<NativeReviewDraft>(payload)
        .map_err(|_| input_error("Review draft 不是冻结的 JSON 结构。"))?;
    let response = OrchestrationJobService::open(database_path)
        .map_err(|_| persistence_error())?
        .review(OrchestrationJobReviewRequest {
            job_id: job_id.to_owned(),
            attempt_id: attempt_id.to_owned(),
            decision: draft.decision,
            reviewer_thread_id: parent_thread_id.to_owned(),
            reason: draft.reason,
            evidence_refs: draft.evidence_refs,
        })
        .map_err(NativeControlError::from)?;
    Ok(NativeReviewResult {
        review_id: response.review.review_id,
        job_id: response.job.job_id,
        state: enum_name(response.job.state),
    })
}

fn native_admission() -> DispatchAdmission {
    DispatchAdmission {
        runtime: Capability::Supported,
        agent_execution: Capability::Supported,
        event: Capability::Supported,
        runtime_healthy: true,
        permission_allowed: true,
        scope_allowed: true,
        schedule_certain: true,
        lease_certain: true,
        receipt_certain: true,
        schema_verified: true,
        global_concurrency_available: true,
        workspace_excluded: false,
        conversation_excluded: false,
        cache_requirement: CacheRequirement::NotRequired,
    }
}

fn enum_name<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "UNKNOWN".to_owned())
}

fn input_error(message: &str) -> NativeControlError {
    NativeControlError {
        code: "TASK_PACKET_FIELD_INVALID".to_owned(),
        message: message.to_owned(),
    }
}

fn is_protocol_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
}

fn persistence_error() -> NativeControlError {
    NativeControlError {
        code: "PERSISTENCE_ERROR".to_owned(),
        message: "CAS 编排数据库不可用。".to_owned(),
    }
}

impl From<crate::orchestration_contract::OrchestrationError> for NativeControlError {
    fn from(error: crate::orchestration_contract::OrchestrationError) -> Self {
        Self {
            code: error.code.as_str().to_owned(),
            message: error.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use uuid::Uuid;

    const NOW: &str = "2026-09-13T00:00:00.000Z";

    #[test]
    fn native_control_plane_persists_full_job_receipt_and_review_chain() {
        let root = std::env::temp_dir().join(format!("cas-native-control-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        let connection = open_database(&database_path).unwrap();
        connection
            .execute_batch(&format!(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, preset_id, created_at, updated_at
                 ) VALUES (
                    'provider-native', 'openai', 'OpenAI', 'PRESET',
                    'https://api.example/v1', 'RESPONSES', 'BEARER_TOKEN', 1,
                    'BUILT_IN', 'codex-native', '{NOW}', '{NOW}'
                 );
                 INSERT INTO models (
                    id, provider_id, model_id, display_name, enabled, source,
                    created_at, updated_at
                 ) VALUES (
                    'model-native', 'provider-native', 'gpt-test', 'GPT Test', 1,
                    'PRESET', '{NOW}', '{NOW}'
                 );
                 INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    orchestration_phase, created_at, updated_at
                 ) VALUES (
                    'agent-native', 'executor', 'Executor', 'test', '执行任务', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, 'executor', 'EXECUTION',
                    '{NOW}', '{NOW}'
                 );
                 INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, enabled, priority, source, created_at, updated_at
                 ) VALUES (
                    'binding-native', 'agent-native', 'model-native', 1, 0, 'CAS',
                    '{NOW}', '{NOW}'
                 );
                 INSERT INTO active_agent_bindings (role_key, agent_id, created_at, updated_at)
                 VALUES ('executor', 'agent-native', '{NOW}', '{NOW}');"
            ))
            .unwrap();
        drop(connection);

        let workspace =
            cas_scheduler::normalize_workspace_scope_key(&root.to_string_lossy()).unwrap();
        let packet = serde_json::json!({
            "schema_version": 1,
            "job_id": "job-native-1",
            "idempotency_key": "native-test-1",
            "task_scope_key": "native-test",
            "objective": "验证 Native Runtime First 完整链",
            "allowed_scope": [workspace],
            "constraints": ["不扩展范围"],
            "success_criteria": ["四阶段 Receipt 与 Review 均落库"],
            "allowed_tools": ["apply_patch"],
            "permission_policy": "WORKSPACE_WRITE",
            "execution_kind_policy": "NATIVE_CHILD_REQUIRED",
            "context_references": [],
            "output_contract": "STANDARD_V1",
            "review_policy": "PRIMARY_REQUIRED"
        });
        let schedule = schedule_native_task(
            &database_path,
            "executor",
            &root.to_string_lossy(),
            "parent-native",
            serde_json::to_string(&packet).unwrap().as_bytes(),
        )
        .unwrap();
        assert_eq!(schedule.action, "SPAWN");
        let attempt_id = schedule.attempt_id.unwrap();

        let connection = open_database(&database_path).unwrap();
        connection
            .execute(
                "INSERT INTO agent_thread_instances (
                    id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                    scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, runtime_fingerprint, created_at, last_used_at,
                    last_observed_at, task_scope_key, reuse_state, execution_kind
                 ) VALUES (
                    'instance-native', 'agent-native', 'Executor', 'child-native',
                    'parent-native', ?1, 'UNKNOWN', 0, 0, 0, 0,
                    (SELECT runtime_fingerprint FROM agent_schedule_decisions
                     WHERE id=(SELECT schedule_decision_id FROM job_attempts WHERE attempt_id=?2)),
                    ?3, ?3, ?3, 'native-test', 'ACTIVE', 'OBSERVED_EXTERNAL'
                 )",
                params![workspace, attempt_id, NOW],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE runtime_delegation_leases
                 SET state='ACTIVE', codex_agent_id='child-native',
                     admission_tool_use_id='native-spawn-tool'
                 WHERE id=(SELECT lease_id FROM job_attempts WHERE attempt_id=?1)",
                [&attempt_id],
            )
            .unwrap();
        drop(connection);

        accept_native_binding(
            &database_path,
            "job-native-1",
            &attempt_id,
            "child-native",
            "native-state:parent-native:child-native",
            "NATIVE_STATE_V1",
        )
        .unwrap();
        observe_native_result(
            &database_path,
            "job-native-1",
            &attempt_id,
            "child-native",
            "native-result:child-native:1",
            "NATIVE_STATE_V1",
            true,
        )
        .unwrap();
        let review = review_native_task(
            &database_path,
            "job-native-1",
            &attempt_id,
            "parent-native",
            r#"{
                "decision":"APPROVE",
                "reason":"所有验收证据通过",
                "evidence_refs":["child:child-native","verification:native-chain"]
            }"#
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(review.state, "COMPLETED");

        let connection = open_database(&database_path).unwrap();
        let audit = connection
            .query_row(
                "SELECT job.state, attempt.state, attempt.execution_kind,
                        instance.execution_kind, instance.reuse_state,
                        (SELECT COUNT(*) FROM delivery_receipts receipt
                         WHERE receipt.attempt_id=attempt.attempt_id),
                        (SELECT COUNT(*) FROM review_decisions review
                         WHERE review.attempt_id=attempt.attempt_id)
                 FROM orchestration_jobs job
                 JOIN job_attempts attempt ON attempt.job_id=job.job_id
                 JOIN agent_thread_instances instance ON instance.id=attempt.thread_instance_id
                 WHERE job.job_id='job-native-1'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            audit,
            (
                "COMPLETED".to_owned(),
                "SUCCEEDED".to_owned(),
                "NATIVE_CHILD".to_owned(),
                "NATIVE_CHILD".to_owned(),
                "ACTIVE".to_owned(),
                4,
                1,
            )
        );
        drop(connection);

        let tracking = OrchestrationJobService::open(&database_path)
            .unwrap()
            .list_tracking(crate::orchestration_job::OrchestrationJobListRequest {
                workspace_scope_key: Some(workspace),
                agent_id: Some("agent-native".to_owned()),
                page: 0,
                page_size: 20,
            })
            .unwrap();
        assert_eq!(tracking.total_count, 1);
        assert_eq!(tracking.jobs[0].state, "COMPLETED");
        assert_eq!(tracking.jobs[0].attempts[0].state, "SUCCEEDED");
        assert_eq!(
            tracking.jobs[0].attempts[0].execution_kind.as_deref(),
            Some("NATIVE_CHILD")
        );
        assert_eq!(
            tracking.jobs[0].attempts[0].receipt_stage.as_deref(),
            Some("PARENT_ACKNOWLEDGED")
        );
        assert_eq!(
            tracking.jobs[0].attempts[0].review_decision.as_deref(),
            Some("APPROVE")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
