use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{
    OrchestrationJobCreateResponse, OrchestrationJobService, classify_existing, find_job_by_id,
    find_job_by_idempotency_scope, find_latest_attempt,
};
use crate::orchestration_contract::{
    AttemptState, IdempotencyOutcome, JobState, OrchestrationError, OrchestrationErrorCode,
    PermissionPolicy, ReviewPolicy, TaskPacket,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationReviewerCreateRequest {
    pub(crate) primary_job_id: String,
    pub(crate) primary_attempt_id: String,
    pub(crate) reviewer_job_id: String,
    pub(crate) reviewer_agent_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) allowed_scope: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationReviewerCreateResponse {
    pub(crate) reviewer_job: OrchestrationJobCreateResponse,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationReviewerReportSubmitRequest {
    pub(crate) reviewer_job_id: String,
    pub(crate) reviewer_thread_id: String,
    pub(crate) summary: String,
    pub(crate) findings: Vec<String>,
    pub(crate) evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationReviewerReport {
    pub(crate) report_id: String,
    pub(crate) reviewer_job_id: String,
    pub(crate) primary_job_id: String,
    pub(crate) primary_attempt_id: String,
    pub(crate) reviewer_thread_id: String,
    pub(crate) summary: String,
    pub(crate) findings: Vec<String>,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) created_at: String,
}

impl OrchestrationJobService {
    /// 创建独立的只读 Reviewer Job。它与主 Job 只有可审计引用关系，不能推进主 Job。
    pub(crate) fn create_reviewer(
        &self,
        mut request: OrchestrationReviewerCreateRequest,
    ) -> Result<OrchestrationReviewerCreateResponse, OrchestrationError> {
        normalize_create_request(&mut request)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;

        if let Some((primary_job_id, primary_attempt_id)) =
            find_assignment(&transaction, &request.reviewer_job_id)?
        {
            let reviewer_job = find_job_by_id(&transaction, &request.reviewer_job_id)
                .map_err(|_| persistence_error())?
                .ok_or_else(|| invalid_request("reviewer_job_id"))?;
            if primary_job_id != request.primary_job_id
                || primary_attempt_id != request.primary_attempt_id
                || reviewer_job.agent_id != request.reviewer_agent_id
                || reviewer_job.idempotency_key != request.idempotency_key
                || reviewer_job.task_packet.allowed_scope != request.allowed_scope
                || reviewer_job.task_packet.permission_policy != PermissionPolicy::ReadOnly
                || reviewer_job.task_packet.review_policy != ReviewPolicy::PrimaryRequired
            {
                return Err(reviewer_conflict(&request.reviewer_job_id));
            }
            let current_attempt = find_latest_attempt(&transaction, &reviewer_job.job_id)
                .map_err(|_| persistence_error())?;
            let outcome = classify_existing(&transaction, &reviewer_job, current_attempt.as_ref())
                .map_err(|_| persistence_error())?;
            transaction.commit().map_err(|_| persistence_error())?;
            return Ok(OrchestrationReviewerCreateResponse {
                reviewer_job: OrchestrationJobCreateResponse {
                    outcome,
                    job: reviewer_job,
                    current_attempt,
                },
            });
        }

        let primary = find_job_by_id(&transaction, &request.primary_job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invalid_request("primary_job_id"))?;
        let primary_attempt = find_latest_attempt(&transaction, &request.primary_job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invalid_request("primary_attempt_id"))?;
        if primary.task_packet.review_policy != ReviewPolicy::PrimaryWithReadOnlyReviewer
            || primary.state != JobState::ReviewPending
            || primary_attempt.attempt_id != request.primary_attempt_id
            || primary_attempt.state != AttemptState::Succeeded
        {
            return Err(invalid_state(
                &request.primary_job_id,
                &request.primary_attempt_id,
            ));
        }
        if request.reviewer_job_id == request.primary_job_id {
            return Err(invalid_request("reviewer_job_id"));
        }
        if !is_scope_subset(&request.allowed_scope, &primary.task_packet.allowed_scope) {
            return Err(invalid_request("allowed_scope"));
        }

        let result_observed: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM delivery_receipts
                    WHERE job_id=?1 AND attempt_id=?2 AND stage='RESULT_OBSERVED'
                 )",
                params![request.primary_job_id, request.primary_attempt_id],
                |row| row.get(0),
            )
            .map_err(|_| persistence_error())?;
        if !result_observed {
            return Err(result_not_observed(
                &request.primary_job_id,
                &request.primary_attempt_id,
            ));
        }

        let reviewer_is_read_only: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM agents
                    WHERE id=?1 AND agent_type='PRESET' AND source='CAS'
                      AND role_key='reviewer' AND orchestration_phase='REVIEW'
                      AND sandbox_policy='READ_ONLY'
                 )",
                [&request.reviewer_agent_id],
                |row| row.get(0),
            )
            .map_err(|_| persistence_error())?;
        if !reviewer_is_read_only {
            return Err(invalid_request("reviewer_agent_id"));
        }

        let mut context_references = primary.task_packet.context_references.clone();
        context_references.extend([
            format!("orchestration-job:{}", request.primary_job_id),
            format!("job-attempt:{}", request.primary_attempt_id),
            format!(
                "delivery-receipt:{}/RESULT_OBSERVED",
                request.primary_attempt_id
            ),
        ]);

        let reviewer_packet = TaskPacket {
            schema_version: primary.task_packet.schema_version,
            job_id: request.reviewer_job_id.clone(),
            idempotency_key: request.idempotency_key.clone(),
            agent_id: request.reviewer_agent_id.clone(),
            parent_thread_id: primary.parent_thread_id.clone(),
            workspace_scope_key: primary.workspace_scope_key.clone(),
            task_scope_key: primary.task_scope_key.clone(),
            objective: format!(
                "独立审查主 Job {} 的 Attempt {}。",
                request.primary_job_id, request.primary_attempt_id
            ),
            allowed_scope: request.allowed_scope.clone(),
            constraints: vec![
                "只读审查；不得修改工作区、主 Job、主 Attempt、Receipt 或 ReviewDecision。"
                    .to_owned(),
                "仅返回结构化审查意见；Primary 自行决定是否采纳。".to_owned(),
            ],
            success_criteria: vec![
                "返回摘要、发现项与证据引用；无发现时显式返回空发现项。".to_owned(),
            ],
            allowed_tools: primary.task_packet.allowed_tools.clone(),
            permission_policy: PermissionPolicy::ReadOnly,
            execution_kind_policy: primary.task_packet.execution_kind_policy,
            context_references,
            output_contract: primary.task_packet.output_contract,
            review_policy: ReviewPolicy::PrimaryRequired,
        };
        reviewer_packet.validate()?;
        let canonical = reviewer_packet.canonical_form()?;
        let hash = reviewer_packet.task_packet_hash()?;
        let existing = find_job_by_idempotency_scope(
            &transaction,
            &reviewer_packet.workspace_scope_key,
            &reviewer_packet.parent_thread_id,
            &reviewer_packet.idempotency_key,
        )
        .map_err(|_| persistence_error())?;
        let (reviewer_job, current_attempt, outcome) = if let Some(existing) = existing {
            if existing.job_id != reviewer_packet.job_id || existing.task_packet_hash != hash {
                return Err(reviewer_conflict(&request.reviewer_job_id));
            }
            let current_attempt = find_latest_attempt(&transaction, &existing.job_id)
                .map_err(|_| persistence_error())?;
            let outcome = classify_existing(&transaction, &existing, current_attempt.as_ref())
                .map_err(|_| persistence_error())?;
            (existing, current_attempt, outcome)
        } else {
            if find_job_by_id(&transaction, &reviewer_packet.job_id)
                .map_err(|_| persistence_error())?
                .is_some()
            {
                return Err(reviewer_conflict(&request.reviewer_job_id));
            }
            let now = timestamp(&transaction)?;
            transaction
                .execute(
                    "INSERT INTO orchestration_jobs (
                        job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                        parent_thread_id, workspace_scope_key, task_scope_key, state,
                        last_error_code, created_at, updated_at, terminal_at
                     ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'CREATED',NULL,?9,?9,NULL)",
                    params![
                        reviewer_packet.job_id,
                        reviewer_packet.idempotency_key,
                        canonical,
                        hash,
                        reviewer_packet.agent_id,
                        reviewer_packet.parent_thread_id,
                        reviewer_packet.workspace_scope_key,
                        reviewer_packet.task_scope_key,
                        now,
                    ],
                )
                .map_err(|_| persistence_error())?;
            let created = find_job_by_id(&transaction, &reviewer_packet.job_id)
                .map_err(|_| persistence_error())?
                .ok_or_else(|| reviewer_conflict(&request.reviewer_job_id))?;
            (created, None, IdempotencyOutcome::Created)
        };
        transaction
            .execute(
                "INSERT INTO reviewer_assignments (
                    reviewer_job_id, primary_job_id, primary_attempt_id, created_at
                 ) VALUES (?1,?2,?3,strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
                params![
                    request.reviewer_job_id,
                    request.primary_job_id,
                    request.primary_attempt_id,
                ],
            )
            .map_err(|_| reviewer_conflict(&request.reviewer_job_id))?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(OrchestrationReviewerCreateResponse {
            reviewer_job: OrchestrationJobCreateResponse {
                outcome,
                job: reviewer_job,
                current_attempt,
            },
        })
    }

    /// 追加 Reviewer 结构化报告；刻意不接触主 Job、主 Attempt、Receipt 与 ReviewDecision。
    pub(crate) fn submit_reviewer_report(
        &self,
        mut request: OrchestrationReviewerReportSubmitRequest,
    ) -> Result<OrchestrationReviewerReport, OrchestrationError> {
        normalize_report_request(&mut request)?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let assignment = find_assignment(&transaction, &request.reviewer_job_id)?;
        let Some((primary_job_id, primary_attempt_id)) = assignment else {
            return Err(invalid_request("reviewer_job_id"));
        };
        if let Some(report) = find_report(&transaction, &request.reviewer_job_id)? {
            if report_matches(&report, &request, &primary_job_id, &primary_attempt_id) {
                transaction.commit().map_err(|_| persistence_error())?;
                return Ok(report);
            }
            return Err(reviewer_conflict(&request.reviewer_job_id));
        }
        let reviewer = find_job_by_id(&transaction, &request.reviewer_job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invalid_request("reviewer_job_id"))?;
        let attempt = find_latest_attempt(&transaction, &request.reviewer_job_id)
            .map_err(|_| persistence_error())?
            .ok_or_else(|| invalid_state(&request.reviewer_job_id, ""))?;
        if reviewer.task_packet.review_policy != ReviewPolicy::PrimaryRequired
            || reviewer.task_packet.permission_policy != PermissionPolicy::ReadOnly
            || reviewer.state != JobState::ReviewPending
            || attempt.state != AttemptState::Succeeded
        {
            return Err(invalid_state(&request.reviewer_job_id, &attempt.attempt_id));
        }
        let observed_and_bound: bool = transaction
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM delivery_receipts receipt
                     JOIN job_attempts attempt ON attempt.attempt_id=receipt.attempt_id
                     JOIN agent_thread_instances instance ON instance.id=attempt.thread_instance_id
                     WHERE receipt.job_id=?1 AND receipt.attempt_id=?2
                       AND receipt.stage='RESULT_OBSERVED'
                       AND instance.codex_thread_id=?3
                 )",
                params![
                    request.reviewer_job_id,
                    attempt.attempt_id,
                    request.reviewer_thread_id
                ],
                |row| row.get(0),
            )
            .map_err(|_| persistence_error())?;
        if !observed_and_bound {
            return Err(result_not_observed(
                &request.reviewer_job_id,
                &attempt.attempt_id,
            ));
        }
        let created_at = timestamp(&transaction)?;
        let report = OrchestrationReviewerReport {
            report_id: Uuid::new_v4().to_string(),
            reviewer_job_id: request.reviewer_job_id,
            primary_job_id,
            primary_attempt_id,
            reviewer_thread_id: request.reviewer_thread_id,
            summary: request.summary,
            findings: request.findings,
            evidence_refs: request.evidence_refs,
            created_at,
        };
        let findings = serde_json::to_string(&report.findings).map_err(|_| persistence_error())?;
        let evidence_refs =
            serde_json::to_string(&report.evidence_refs).map_err(|_| persistence_error())?;
        transaction
            .execute(
                "INSERT INTO reviewer_reports (
                    report_id, reviewer_job_id, primary_job_id, primary_attempt_id,
                    reviewer_thread_id, summary, findings, evidence_refs, created_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    report.report_id,
                    report.reviewer_job_id,
                    report.primary_job_id,
                    report.primary_attempt_id,
                    report.reviewer_thread_id,
                    report.summary,
                    findings,
                    evidence_refs,
                    report.created_at,
                ],
            )
            .map_err(|_| persistence_error())?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(report)
    }
}

fn is_scope_subset(candidate: &[String], parent: &[String]) -> bool {
    !candidate.is_empty() && candidate.iter().all(|scope| parent.contains(scope))
}

fn find_assignment(
    connection: &rusqlite::Connection,
    reviewer_job_id: &str,
) -> Result<Option<(String, String)>, OrchestrationError> {
    connection
        .query_row(
            "SELECT primary_job_id, primary_attempt_id
             FROM reviewer_assignments WHERE reviewer_job_id=?1",
            [reviewer_job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|_| persistence_error())
}

fn report_matches(
    report: &OrchestrationReviewerReport,
    request: &OrchestrationReviewerReportSubmitRequest,
    primary_job_id: &str,
    primary_attempt_id: &str,
) -> bool {
    report.reviewer_job_id == request.reviewer_job_id
        && report.primary_job_id == primary_job_id
        && report.primary_attempt_id == primary_attempt_id
        && report.reviewer_thread_id == request.reviewer_thread_id
        && report.summary == request.summary
        && report.findings == request.findings
        && report.evidence_refs == request.evidence_refs
}

fn normalize_create_request(
    request: &mut OrchestrationReviewerCreateRequest,
) -> Result<(), OrchestrationError> {
    for (name, value) in [
        ("primary_job_id", &mut request.primary_job_id),
        ("primary_attempt_id", &mut request.primary_attempt_id),
        ("reviewer_job_id", &mut request.reviewer_job_id),
        ("reviewer_agent_id", &mut request.reviewer_agent_id),
        ("idempotency_key", &mut request.idempotency_key),
    ] {
        *value = value.trim().to_owned();
        if value.is_empty() {
            return Err(invalid_request(name));
        }
    }
    normalize_list(&mut request.allowed_scope, "allowed_scope", true)
}

fn normalize_report_request(
    request: &mut OrchestrationReviewerReportSubmitRequest,
) -> Result<(), OrchestrationError> {
    for (name, value) in [
        ("reviewer_job_id", &mut request.reviewer_job_id),
        ("reviewer_thread_id", &mut request.reviewer_thread_id),
    ] {
        *value = value.trim().to_owned();
        if value.is_empty() {
            return Err(invalid_request(name));
        }
    }
    request.summary = request.summary.trim().to_owned();
    if request.summary.is_empty() {
        return Err(invalid_request("summary"));
    }
    normalize_list(&mut request.findings, "findings", false)?;
    normalize_list(&mut request.evidence_refs, "evidence_refs", true)
}

fn normalize_list(
    values: &mut Vec<String>,
    field: &str,
    required: bool,
) -> Result<(), OrchestrationError> {
    for value in values.iter_mut() {
        *value = value.trim().to_owned();
    }
    let unique = values.iter().collect::<BTreeSet<_>>();
    if (required && unique.is_empty())
        || unique.len() != values.len()
        || unique.iter().any(|value| value.is_empty())
    {
        return Err(invalid_request(field));
    }
    Ok(())
}

fn find_report(
    connection: &rusqlite::Connection,
    reviewer_job_id: &str,
) -> Result<Option<OrchestrationReviewerReport>, OrchestrationError> {
    connection.query_row(
        "SELECT report_id, reviewer_job_id, primary_job_id, primary_attempt_id, reviewer_thread_id,
                summary, findings, evidence_refs, created_at FROM reviewer_reports WHERE reviewer_job_id=?1",
        [reviewer_job_id],
        |row| Ok(OrchestrationReviewerReport {
            report_id: row.get(0)?, reviewer_job_id: row.get(1)?, primary_job_id: row.get(2)?,
            primary_attempt_id: row.get(3)?, reviewer_thread_id: row.get(4)?, summary: row.get(5)?,
            findings: serde_json::from_str(&row.get::<_, String>(6)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
            evidence_refs: serde_json::from_str(&row.get::<_, String>(7)?).map_err(|_| rusqlite::Error::InvalidQuery)?,
            created_at: row.get(8)?,
        }),
    ).optional().map_err(|_| persistence_error())
}

fn timestamp(connection: &rusqlite::Connection) -> Result<String, OrchestrationError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .map_err(|_| persistence_error())
}

fn invalid_request(field: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::TaskPacketFieldInvalid,
        message: format!("Reviewer 字段无效：{field}。"),
        field_path: Some(field.to_owned()),
        job_id: None,
        attempt_id: None,
    }
}

fn reviewer_conflict(job_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::IdempotencyKeyConflict,
        message: "Reviewer Job、绑定或只追加报告与既有事实冲突。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: None,
    }
}
fn invalid_state(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InvalidStateTransition,
        message: "Reviewer 只能审查当前已观察完成的主 Attempt。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: (!attempt_id.is_empty()).then(|| attempt_id.to_owned()),
    }
}
fn result_not_observed(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::ResultNotObserved,
        message: "缺少 RESULT_OBSERVED，不能提交 Reviewer 报告。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}
fn persistence_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::PersistenceError,
        message: "Reviewer 持久化失败。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration_contract::{
        ExecutionKindPolicy, OutputContract, TASK_PACKET_SCHEMA_VERSION,
    };

    const NOW: &str = "2026-09-10T00:00:00.000Z";

    fn primary_packet(review_policy: ReviewPolicy) -> TaskPacket {
        TaskPacket {
            schema_version: TASK_PACKET_SCHEMA_VERSION,
            job_id: "primary-job".to_owned(),
            idempotency_key: "primary-key".to_owned(),
            agent_id: "primary-agent".to_owned(),
            parent_thread_id: "parent-thread".to_owned(),
            workspace_scope_key: "c:/workspace".to_owned(),
            task_scope_key: "task-1".to_owned(),
            objective: "实现功能".to_owned(),
            allowed_scope: vec!["src/".to_owned(), "tests/".to_owned()],
            constraints: Vec::new(),
            success_criteria: vec!["测试通过".to_owned()],
            allowed_tools: vec!["read_file".to_owned()],
            permission_policy: PermissionPolicy::WorkspaceWrite,
            execution_kind_policy: ExecutionKindPolicy::ManagedWorkerRequired,
            context_references: vec!["src/lib.rs".to_owned()],
            output_contract: OutputContract::StandardV1,
            review_policy,
        }
    }

    fn service_with_primary_result(review_policy: ReviewPolicy) -> OrchestrationJobService {
        let service = OrchestrationJobService::in_memory();
        let packet = primary_packet(review_policy);
        let canonical = packet.canonical_form().unwrap();
        let hash = packet.task_packet_hash().unwrap();
        service
            .connection()
            .unwrap()
            .execute_batch(&format!(
                r#"INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, preset_id, created_at, updated_at
                 ) VALUES (
                    'provider-1','openai','OpenAI','PRESET','https://api.example/v1',
                    'RESPONSES','BEARER_TOKEN',1,'BUILT_IN','codex-native','{NOW}','{NOW}'
                 );
                 INSERT INTO models (
                    id, provider_id, model_id, display_name, enabled, source, created_at, updated_at
                 ) VALUES (
                    'model-1','provider-1','gpt-test','GPT Test',1,'PRESET','{NOW}','{NOW}'
                 );
                 INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    orchestration_phase, created_at, updated_at
                 ) VALUES
                    ('primary-agent','executor','Executor','test','execute','CUSTOM',1,
                     'WORKSPACE_WRITE','MEDIUM','CAS',1,'executor','EXECUTION','{NOW}','{NOW}'),
                    ('reviewer-agent','custom-reviewer','Reviewer','test','review','PRESET',1,
                     'READ_ONLY','MEDIUM','CAS',1,'reviewer','REVIEW','{NOW}','{NOW}');
                 INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, enabled, priority, source, created_at, updated_at
                 ) VALUES (
                    'reviewer-binding','reviewer-agent','model-1',1,0,'CAS','{NOW}','{NOW}'
                 );
                 INSERT INTO active_agent_bindings (role_key, agent_id, created_at, updated_at)
                 VALUES ('reviewer','reviewer-agent','{NOW}','{NOW}');"#,
            ))
            .unwrap();
        let connection = service.connection().unwrap();
        connection
            .execute(
                "INSERT INTO orchestration_jobs (
                    job_id,idempotency_key,task_packet,task_packet_hash,agent_id,
                    parent_thread_id,workspace_scope_key,task_scope_key,state,created_at,updated_at
                 ) VALUES (
                    'primary-job','primary-key',?1,?2,'primary-agent','parent-thread',
                    'c:/workspace','task-1','REVIEW_PENDING',?3,?3
                 )",
                params![canonical, hash, NOW],
            )
            .unwrap();
        connection
            .execute_batch(&format!(
                r#"INSERT INTO agent_schedule_decisions (
                    id,created_at,source,agent_id,agent_name_snapshot,workspace_scope_key,
                    parent_thread_id,candidate_thread_id,decision,reason_code,
                    runtime_fingerprint,cache_hint,claimed,task_scope_key,job_id
                 ) VALUES (
                    'primary-decision','{NOW}','TEST','primary-agent','Executor','c:/workspace',
                    'parent-thread','child-thread','SPAWN','TEST','primary-fingerprint',
                    'UNKNOWN',0,'task-1','primary-job'
                 );
                 INSERT INTO runtime_delegation_leases (
                    id,created_at,updated_at,agent_id,parent_thread_id,codex_agent_id,
                    workspace_scope_key,task_scope_key,schedule_decision_id,state,expires_at,agent_type
                 ) VALUES (
                    'primary-lease','{NOW}','{NOW}','primary-agent','parent-thread','child-thread',
                    'c:/workspace','task-1','primary-decision','ACTIVE',
                    '2099-01-01T00:00:00.000Z','executor'
                 );
                 INSERT INTO agent_thread_instances (
                    id,agent_id,agent_name_snapshot,codex_thread_id,parent_thread_id,scope_key,
                    status,runtime_fingerprint,created_at,last_used_at,last_observed_at,
                    task_scope_key,reuse_state,execution_kind,claim_lease_id
                 ) VALUES (
                    'primary-instance','primary-agent','Executor','child-thread','parent-thread',
                    'c:/workspace','IDLE','primary-fingerprint','{NOW}','{NOW}','{NOW}',
                    'task-1','HELD_FOR_REVIEW','MANAGED_WORKER','primary-lease'
                 );
                 INSERT INTO job_attempts (
                    attempt_id,job_id,attempt_no,schedule_decision_id,lease_id,route_action,
                    planned_execution_kind,execution_kind,thread_instance_id,codex_turn_id,state,
                    created_at,updated_at,dispatch_recorded_at,accepted_at,terminal_at
                 ) VALUES (
                    'primary-attempt','primary-job',1,'primary-decision','primary-lease','SPAWN',
                    'MANAGED_WORKER','MANAGED_WORKER','primary-instance','primary-turn','SUCCEEDED',
                    '{NOW}','{NOW}','{NOW}','{NOW}','{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id,job_id,attempt_id,stage,evidence_source,evidence_ref,
                    parent_thread_id,evidence_at,created_at
                 ) VALUES (
                    'primary-dispatch','primary-job','primary-attempt','DISPATCH_RECORDED',
                    'CAS_TRANSACTION','primary-decision','parent-thread','{NOW}','{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id,job_id,attempt_id,stage,execution_kind,evidence_source,evidence_ref,
                    parent_thread_id,codex_thread_id,codex_turn_id,schema_profile,evidence_at,created_at
                 ) VALUES
                    ('primary-accepted','primary-job','primary-attempt','TURN_ACCEPTED',
                     'MANAGED_WORKER','APP_SERVER_RESPONSE','primary-accepted','parent-thread',
                     'child-thread','primary-turn','TEST','{NOW}','{NOW}'),
                    ('primary-result','primary-job','primary-attempt','RESULT_OBSERVED',
                     'MANAGED_WORKER','RUNTIME_EVENT','primary-result','parent-thread',
                     'child-thread','primary-turn','TEST','{NOW}','{NOW}');"#,
            ))
            .unwrap();
        drop(connection);
        service
    }

    fn create_request() -> OrchestrationReviewerCreateRequest {
        OrchestrationReviewerCreateRequest {
            primary_job_id: "primary-job".to_owned(),
            primary_attempt_id: "primary-attempt".to_owned(),
            reviewer_job_id: "reviewer-job".to_owned(),
            reviewer_agent_id: "reviewer-agent".to_owned(),
            idempotency_key: "reviewer-key".to_owned(),
            allowed_scope: vec!["src/".to_owned()],
        }
    }

    fn seed_reviewer_result(service: &OrchestrationJobService) {
        service
            .connection()
            .unwrap()
            .execute_batch(&format!(
                r#"UPDATE orchestration_jobs
                 SET state='REVIEW_PENDING',updated_at='{NOW}' WHERE job_id='reviewer-job';
                 INSERT INTO agent_schedule_decisions (
                    id,created_at,source,agent_id,agent_name_snapshot,workspace_scope_key,
                    parent_thread_id,candidate_thread_id,decision,reason_code,
                    runtime_fingerprint,cache_hint,claimed,task_scope_key,job_id
                 ) VALUES (
                    'reviewer-decision','{NOW}','TEST','reviewer-agent','Reviewer','c:/workspace',
                    'parent-thread','reviewer-thread','SPAWN','TEST','reviewer-fingerprint',
                    'UNKNOWN',0,'task-1','reviewer-job'
                 );
                 INSERT INTO runtime_delegation_leases (
                    id,created_at,updated_at,agent_id,parent_thread_id,codex_agent_id,
                    workspace_scope_key,task_scope_key,schedule_decision_id,state,expires_at,agent_type
                 ) VALUES (
                    'reviewer-lease','{NOW}','{NOW}','reviewer-agent','parent-thread','reviewer-thread',
                    'c:/workspace','task-1','reviewer-decision','ACTIVE',
                    '2099-01-01T00:00:00.000Z','reviewer'
                 );
                 INSERT INTO agent_thread_instances (
                    id,agent_id,agent_name_snapshot,codex_thread_id,parent_thread_id,scope_key,
                    status,runtime_fingerprint,created_at,last_used_at,last_observed_at,
                    task_scope_key,reuse_state,execution_kind,claim_lease_id
                 ) VALUES (
                    'reviewer-instance','reviewer-agent','Reviewer','reviewer-thread','parent-thread',
                    'c:/workspace','IDLE','reviewer-fingerprint','{NOW}','{NOW}','{NOW}',
                    'task-1','HELD_FOR_REVIEW','MANAGED_WORKER','reviewer-lease'
                 );
                 INSERT INTO job_attempts (
                    attempt_id,job_id,attempt_no,schedule_decision_id,lease_id,route_action,
                    planned_execution_kind,execution_kind,thread_instance_id,codex_turn_id,state,
                    created_at,updated_at,dispatch_recorded_at,accepted_at,terminal_at
                 ) VALUES (
                    'reviewer-attempt','reviewer-job',1,'reviewer-decision','reviewer-lease','SPAWN',
                    'MANAGED_WORKER','MANAGED_WORKER','reviewer-instance','reviewer-turn','SUCCEEDED',
                    '{NOW}','{NOW}','{NOW}','{NOW}','{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id,job_id,attempt_id,stage,evidence_source,evidence_ref,
                    parent_thread_id,evidence_at,created_at
                 ) VALUES (
                    'reviewer-dispatch','reviewer-job','reviewer-attempt','DISPATCH_RECORDED',
                    'CAS_TRANSACTION','reviewer-decision','parent-thread','{NOW}','{NOW}'
                 );
                 INSERT INTO delivery_receipts (
                    receipt_id,job_id,attempt_id,stage,execution_kind,evidence_source,evidence_ref,
                    parent_thread_id,codex_thread_id,codex_turn_id,schema_profile,evidence_at,created_at
                 ) VALUES
                    ('reviewer-accepted','reviewer-job','reviewer-attempt','TURN_ACCEPTED',
                     'MANAGED_WORKER','APP_SERVER_RESPONSE','reviewer-accepted','parent-thread',
                     'reviewer-thread','reviewer-turn','TEST','{NOW}','{NOW}'),
                    ('reviewer-result','reviewer-job','reviewer-attempt','RESULT_OBSERVED',
                     'MANAGED_WORKER','RUNTIME_EVENT','reviewer-result','parent-thread',
                     'reviewer-thread','reviewer-turn','TEST','{NOW}','{NOW}');"#,
            ))
            .unwrap();
    }

    #[test]
    fn reviewer_scope_must_be_the_same_or_narrower() {
        let parent = vec!["src/".to_owned(), "docs/".to_owned()];
        assert!(is_scope_subset(&["src/".to_owned()], &parent));
        assert!(is_scope_subset(&parent, &parent));
        assert!(!is_scope_subset(&["outside/".to_owned()], &parent));
        assert!(!is_scope_subset(&[], &parent));
    }

    #[test]
    fn report_allows_an_empty_findings_list_but_requires_evidence() {
        let mut valid = OrchestrationReviewerReportSubmitRequest {
            reviewer_job_id: "review-job".to_owned(),
            reviewer_thread_id: "review-thread".to_owned(),
            summary: "无问题".to_owned(),
            findings: vec![],
            evidence_refs: vec!["proof".to_owned()],
        };
        assert!(normalize_report_request(&mut valid).is_ok());
        valid.evidence_refs.clear();
        assert!(normalize_report_request(&mut valid).is_err());
    }

    #[test]
    fn reviewer_creation_is_policy_gated_atomic_and_idempotent() {
        let service = service_with_primary_result(ReviewPolicy::PrimaryWithReadOnlyReviewer);
        let first = service.create_reviewer(create_request()).unwrap();
        assert_eq!(first.reviewer_job.outcome, IdempotencyOutcome::Created);
        assert_eq!(
            first.reviewer_job.job.task_packet.permission_policy,
            PermissionPolicy::ReadOnly
        );
        assert_eq!(
            first.reviewer_job.job.task_packet.review_policy,
            ReviewPolicy::PrimaryRequired
        );
        assert_eq!(
            first.reviewer_job.job.task_packet.allowed_scope,
            vec!["src/"]
        );
        assert!(
            first
                .reviewer_job
                .job
                .task_packet
                .context_references
                .contains(&"job-attempt:primary-attempt".to_owned())
        );
        let second = service.create_reviewer(create_request()).unwrap();
        assert_eq!(
            second.reviewer_job.outcome,
            IdempotencyOutcome::ExistingNotDispatched
        );
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM reviewer_assignments", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );

        let simple = service_with_primary_result(ReviewPolicy::PrimaryRequired);
        assert_eq!(
            simple.create_reviewer(create_request()).unwrap_err().code,
            OrchestrationErrorCode::InvalidStateTransition
        );
        assert_eq!(
            simple
                .connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM orchestration_jobs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );

        let rollback = service_with_primary_result(ReviewPolicy::PrimaryWithReadOnlyReviewer);
        rollback
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_reviewer_assignment
                 BEFORE INSERT ON reviewer_assignments
                 BEGIN SELECT RAISE(ABORT,'injected assignment failure'); END;",
            )
            .unwrap();
        assert!(rollback.create_reviewer(create_request()).is_err());
        assert_eq!(
            rollback
                .connection()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM orchestration_jobs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn reviewer_report_is_non_authoritative_append_only_and_conflict_safe() {
        let service = service_with_primary_result(ReviewPolicy::PrimaryWithReadOnlyReviewer);
        service.create_reviewer(create_request()).unwrap();
        seed_reviewer_result(&service);
        let request = OrchestrationReviewerReportSubmitRequest {
            reviewer_job_id: "reviewer-job".to_owned(),
            reviewer_thread_id: "reviewer-thread".to_owned(),
            summary: "发现一项风险".to_owned(),
            findings: vec!["缺少边界测试".to_owned()],
            evidence_refs: vec!["reviewer-result:1".to_owned()],
        };

        let first = service.submit_reviewer_report(request.clone()).unwrap();
        let second = service.submit_reviewer_report(request.clone()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.primary_attempt_id, "primary-attempt");
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row(
                    "SELECT state FROM orchestration_jobs WHERE job_id='primary-job'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "REVIEW_PENDING"
        );
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM review_decisions WHERE job_id='primary-job'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM delivery_receipts WHERE job_id='primary-job'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            3
        );

        let mut conflict = request;
        conflict.summary = "改写结论".to_owned();
        assert_eq!(
            service.submit_reviewer_report(conflict).unwrap_err().code,
            OrchestrationErrorCode::IdempotencyKeyConflict
        );
        let connection = service.connection().unwrap();
        assert!(
            connection
                .execute("UPDATE reviewer_reports SET summary='rewrite'", [])
                .is_err()
        );
        assert!(
            connection
                .execute("DELETE FROM reviewer_assignments", [])
                .is_err()
        );
    }
}
