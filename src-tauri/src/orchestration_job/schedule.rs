use std::convert::TryFrom;

use cas_scheduler::hard_gates::{
    AgentExecutionInput, CacheRequirement, CacheState, CandidateHardGateInput, CandidateScope,
    Capability, ContextHealth, ContextPressure, ExclusionInput, ExecutionCapabilityInput,
    ExecutionKind as SchedulerExecutionKind, HardGateContext, HardGateReasonCode, RequiredIdentity,
    RequiredScope, ThreadState,
};
use cas_scheduler::scoring::{
    CacheEvidenceSource, ReuseSelection, ReuseStrategy, ScoringCandidate, ScoringPolicy,
    select_reuse,
};
use cas_scheduler::runtime_policy::{
    AdmissionDecision, OrchestrationFailurePolicy, RuntimePolicyInput, RuntimePolicyMode,
    RuntimePolicyReasonCode, evaluate_runtime_policy,
};
use cas_scheduler::{
    Profile as LegacySchedulingProfile, REUSE_CLAIM_TTL_SECONDS, SPAWN_RESERVATION_TTL_SECONDS,
    cache_hint, context_pressure_limit, effective_cache_retention,
    effective_model_reasoning_efforts, effective_reuse_strategy,
    render_delegated_agent_instructions_for_phase, resolve_agent_reasoning_effort,
    runtime_fingerprint as shared_runtime_fingerprint, skill_fingerprint_values,
    workspace_is_within,
};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::delivery_receipt::{
    DeliveryReceiptAppendRequest, ReceiptEvidence, append_in_transaction,
};
use crate::orchestration_contract::{
    ExecutionKind, JobAttempt, JobState, OrchestrationError, OrchestrationErrorCode,
    OrchestrationJob, PermissionPolicy, ReceiptEvidenceSource, ReceiptStage, RouteAction,
    TaskPacket,
};

use super::{
    OrchestrationJobCreateResponse, OrchestrationJobService, classify_existing, enum_name,
    find_job_by_id, find_job_by_idempotency_scope, find_latest_attempt, map_attempt,
    persistence_error, update_job_state,
};

const EXECUTION_RECOMMENDATION_SOURCE: &str = "ORCHESTRATION_EXECUTE_RECOMMENDATION";
const EXECUTION_FINAL_SOURCE: &str = "ORCHESTRATION_EXECUTE_FINAL";

#[derive(Debug, Clone)]
pub(crate) struct AtomicScheduleRequest {
    pub(crate) task_packet: TaskPacket,
    pub(crate) expected_decision: RouteAction,
    pub(crate) expected_candidate_thread_id: Option<String>,
    pub(crate) planned_execution_kind: ExecutionKind,
    pub(crate) admission: DispatchAdmission,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DispatchAdmission {
    pub(crate) runtime: Capability,
    pub(crate) agent_execution: Capability,
    pub(crate) event: Capability,
    pub(crate) runtime_healthy: bool,
    pub(crate) permission_allowed: bool,
    pub(crate) scope_allowed: bool,
    pub(crate) schedule_certain: bool,
    pub(crate) lease_certain: bool,
    pub(crate) receipt_certain: bool,
    pub(crate) schema_verified: bool,
    pub(crate) global_concurrency_available: bool,
    pub(crate) workspace_excluded: bool,
    pub(crate) conversation_excluded: bool,
    pub(crate) cache_requirement: CacheRequirement,
}

/// 原子调度事务内读取并以 Fingerprint 固化的 App Server 派发参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DispatchAgentProfile {
    pub(crate) agent_id: String,
    pub(crate) agent_key: String,
    pub(crate) agent_name: String,
    pub(crate) instruction: String,
    pub(crate) orchestration_phase: String,
    pub(crate) sandbox_policy: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) model_slug: String,
    pub(crate) model_provider: Option<String>,
    pub(crate) runtime_fingerprint: String,
}

impl DispatchAdmission {
    #[cfg(test)]
    fn allowed() -> Self {
        Self {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AtomicScheduleOutcome {
    Ready(DispatchPermit),
    Waiting(ScheduleStop),
    Blocked(ScheduleStop),
    Existing(OrchestrationJobCreateResponse),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScheduleStop {
    pub(crate) job: OrchestrationJob,
    pub(crate) schedule_decision_id: String,
    pub(crate) admission_decision: AdmissionDecision,
    pub(crate) reason_code: String,
    pub(crate) error_code: OrchestrationErrorCode,
}

/// 只能由成功提交的原子调度事务构造。Runtime 必须先持有此 Permit，才能进入派发边界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DispatchPermit {
    job: OrchestrationJob,
    attempt: JobAttempt,
    agent_type: String,
    candidate_thread_id: Option<String>,
    reason_code: String,
    profile: DispatchAgentProfile,
}

impl DispatchPermit {
    pub(crate) fn job(&self) -> &OrchestrationJob {
        &self.job
    }

    pub(crate) fn attempt(&self) -> &JobAttempt {
        &self.attempt
    }

    pub(crate) fn agent_type(&self) -> &str {
        &self.agent_type
    }

    pub(crate) fn candidate_thread_id(&self) -> Option<&str> {
        self.candidate_thread_id.as_deref()
    }

    pub(crate) fn reason_code(&self) -> &str {
        &self.reason_code
    }

    pub(crate) fn profile(&self) -> &DispatchAgentProfile {
        &self.profile
    }
}

impl OrchestrationJobService {
    /// C-03：在单个 `BEGIN IMMEDIATE` 中重算、占用并建立完整派发前审计。
    /// 返回 `Ready` 时事务已经提交；本函数内部不接触 App Server。
    pub(crate) fn schedule_atomic(
        &self,
        request: AtomicScheduleRequest,
    ) -> Result<AtomicScheduleOutcome, OrchestrationError> {
        request.task_packet.validate()?;
        if !request.planned_execution_kind.is_dispatchable()
            || !request
                .task_packet
                .execution_kind_policy
                .allows(request.planned_execution_kind)
        {
            return Err(schedule_error(
                OrchestrationErrorCode::ExecutionKindUnsupported,
                "TaskPacket 不允许请求的执行身份。",
                Some(&request.task_packet.job_id),
            ));
        }
        let canonical_packet = request.task_packet.canonical_form()?;
        let packet_hash = request.task_packet.task_packet_hash()?;
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let now = current_timestamp(&transaction)?;
        schedule_in_transaction(transaction, request, canonical_packet, packet_hash, &now)
    }

    /// 写入不可逆的派发边界。调用方只有拿到成功返回后才可调用 App Server。
    pub(crate) fn authorize_dispatch(
        &self,
        permit: &DispatchPermit,
    ) -> Result<JobAttempt, OrchestrationError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let now = current_timestamp(&transaction)?;
        let changed = transaction
            .execute(
                "UPDATE job_attempts
                 SET state = 'DISPATCHING', updated_at = ?2,
                     dispatch_recorded_at = COALESCE(dispatch_recorded_at, ?2)
                 WHERE attempt_id = ?1 AND state = 'PLANNED'
                   AND EXISTS (
                       SELECT 1 FROM runtime_delegation_leases lease
                       WHERE lease.id = job_attempts.lease_id
                         AND lease.state = 'PENDING'
                   )",
                params![permit.attempt.attempt_id, now],
            )
            .map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(schedule_error(
                OrchestrationErrorCode::AttemptNotCurrent,
                "Attempt 已被派发、释放或不再是当前派发对象。",
                Some(&permit.job.job_id),
            ));
        }
        update_job_state(
            &transaction,
            &permit.job.job_id,
            JobState::Claimed,
            JobState::Dispatched,
            None,
            &now,
        )?;
        append_in_transaction(
            &transaction,
            DeliveryReceiptAppendRequest {
                job_id: permit.job.job_id.clone(),
                attempt_id: permit.attempt.attempt_id.clone(),
                evidence: vec![ReceiptEvidence {
                    stage: ReceiptStage::DispatchRecorded,
                    execution_kind: None,
                    evidence_source: ReceiptEvidenceSource::CasTransaction,
                    evidence_ref: permit.attempt.schedule_decision_id.clone(),
                    parent_thread_id: permit.job.parent_thread_id.clone(),
                    codex_thread_id: None,
                    codex_turn_id: None,
                    schema_profile: None,
                    evidence_at: now.clone(),
                }],
            },
        )?;
        let attempt = transaction
            .query_row(
                "SELECT attempt_id, job_id, attempt_no, previous_attempt_id,
                        schedule_decision_id, lease_id, route_action, planned_execution_kind,
                        execution_kind, thread_instance_id, codex_turn_id, state,
                        recovery_count, last_error_code, created_at, updated_at,
                        dispatch_recorded_at, accepted_at, terminal_at
                 FROM job_attempts WHERE attempt_id = ?1",
                [&permit.attempt.attempt_id],
                map_attempt,
            )
            .map_err(|_| persistence_error())?;
        transaction.commit().map_err(|_| persistence_error())?;
        Ok(attempt)
    }
}

fn schedule_in_transaction(
    transaction: Transaction<'_>,
    request: AtomicScheduleRequest,
    canonical_packet: String,
    packet_hash: String,
    now: &str,
) -> Result<AtomicScheduleOutcome, OrchestrationError> {
    let packet = &request.task_packet;
    let mut revision = None;
    let existing = find_job_by_idempotency_scope(
        &transaction,
        &packet.workspace_scope_key,
        &packet.parent_thread_id,
        &packet.idempotency_key,
    )
    .map_err(|_| persistence_error())?;
    let job_preexisted = existing.is_some();
    if let Some(job) = existing.as_ref() {
        if job.task_packet_hash != packet_hash {
            return Err(schedule_error(
                OrchestrationErrorCode::IdempotencyKeyConflict,
                "同一幂等范围已存在不同 TaskPacket。",
                Some(&job.job_id),
            ));
        }
        let current_attempt =
            find_latest_attempt(&transaction, &job.job_id).map_err(|_| persistence_error())?;
        let waiting_for_revision = if job.state == JobState::Waiting {
            if let Some(attempt) = current_attempt.as_ref().filter(|attempt| {
                attempt.state == crate::orchestration_contract::AttemptState::Succeeded
            }) {
                has_revision_decision(&transaction, &job.job_id, &attempt.attempt_id)
                    .map_err(|_| persistence_error())?
            } else {
                false
            }
        } else {
            false
        };
        if job.state == JobState::RevisionRequired || waiting_for_revision {
            revision = Some(load_revision_schedule_context(
                &transaction,
                job,
                current_attempt.as_ref(),
            )?);
        } else if current_attempt.is_some()
            || !matches!(
                job.state,
                JobState::Created | JobState::Routed | JobState::Waiting
            )
        {
            let outcome = classify_existing(&transaction, job, current_attempt.as_ref())
                .map_err(|_| persistence_error())?;
            transaction.commit().map_err(|_| persistence_error())?;
            return Ok(AtomicScheduleOutcome::Existing(
                OrchestrationJobCreateResponse {
                    outcome,
                    job: job.clone(),
                    current_attempt,
                },
            ));
        }
    }

    let Some(agent) =
        load_agent_snapshot(&transaction, &packet.agent_id, packet.permission_policy)?
    else {
        return Err(schedule_error(
            OrchestrationErrorCode::AgentNotExecutable,
            "TaskPacket 指定的 Agent 不存在。",
            Some(&packet.job_id),
        ));
    };
    if agent.is_reviewer
        && !reviewer_assignment_exists(&transaction, &packet.job_id)
            .map_err(|_| persistence_error())?
    {
        return Err(schedule_error(
            OrchestrationErrorCode::AgentNotExecutable,
            "Reviewer 只能通过已绑定的独立审查任务派发。",
            Some(&packet.job_id),
        ));
    }

    if !job_preexisted {
        if find_job_by_id(&transaction, &packet.job_id)
            .map_err(|_| persistence_error())?
            .is_some()
        {
            return Err(schedule_error(
                OrchestrationErrorCode::IdempotencyKeyConflict,
                "job_id 已被另一 TaskPacket 使用。",
                Some(&packet.job_id),
            ));
        }
        insert_job(&transaction, packet, &canonical_packet, &packet_hash, now)?;
    }

    expire_proven_undispatched_leases(&transaction, now)?;
    let mut scheduled_agent = agent;
    let initial_recommendation = match evaluate_recommendation(
        &transaction,
        &request,
        &scheduled_agent,
        packet,
        now,
        &[],
        revision.as_ref(),
    )? {
        RecommendationEvaluation::Recommendation(recommendation) => recommendation,
        RecommendationEvaluation::Stop {
            state,
            admission_decision,
            reason_code,
            error_code,
        } => {
            return finish_stopped_with_admission(
                transaction,
                packet,
                now,
                state,
                &reason_code,
                error_code,
                None,
                admission_decision,
            );
        }
    };

    // Expected 仅与本次事务首次重算结果比较。后续 Claim 竞争是执行事实变化，不能误报 stale。
    validate_expected(&request, &initial_recommendation, &packet.job_id)?;
    let initial_decision_id = Uuid::new_v4().to_string();
    insert_schedule_decision(
        &transaction,
        &initial_decision_id,
        EXECUTION_RECOMMENDATION_SOURCE,
        packet,
        &scheduled_agent,
        &initial_recommendation,
        false,
        None,
        now,
    )?;

    let lease_id = Uuid::new_v4().to_string();
    let attempt_id = Uuid::new_v4().to_string();
    let final_decision_id = Uuid::new_v4().to_string();
    let mut final_recommendation = initial_recommendation;
    let mut rejected_instance_ids = Vec::new();
    loop {
        match final_recommendation.action {
            RouteAction::Reuse => {
                let candidate = final_recommendation.candidate.as_ref().ok_or_else(|| {
                    schedule_error(
                        OrchestrationErrorCode::InternalInvariantViolation,
                        "REUSE 缺少候选 Thread。",
                        Some(&packet.job_id),
                    )
                })?;
                let changed = if let Some(revision) = revision.as_ref() {
                    claim_revision_candidate(
                        &transaction,
                        candidate,
                        revision,
                        &lease_id,
                        &scheduled_agent,
                        packet,
                        now,
                    )?
                } else {
                    claim_candidate(
                        &transaction,
                        candidate,
                        &lease_id,
                        &scheduled_agent,
                        packet,
                        now,
                    )?
                };
                if changed == 1 {
                    break;
                }

                // 条件 Claim 失败即视为该候选已失效；重读所有调度事实，不能复用旧预览。
                rejected_instance_ids.push(candidate.instance_id.clone());
                let Some(refreshed_agent) =
                    load_agent_snapshot(&transaction, &packet.agent_id, packet.permission_policy)?
                else {
                    return finish_stopped_with_supersedes(
                        transaction,
                        packet,
                        now,
                        JobState::Blocked,
                        "AGENT_NOT_EXECUTABLE",
                        OrchestrationErrorCode::AgentNotExecutable,
                        Some(&initial_decision_id),
                    );
                };
                scheduled_agent = refreshed_agent;
                final_recommendation = match evaluate_recommendation(
                    &transaction,
                    &request,
                    &scheduled_agent,
                    packet,
                    now,
                    &rejected_instance_ids,
                    revision.as_ref(),
                )? {
                    RecommendationEvaluation::Recommendation(recommendation) => recommendation,
                    RecommendationEvaluation::Stop {
                        state,
                        admission_decision,
                        reason_code,
                        error_code,
                    } => {
                        return finish_stopped_with_admission(
                            transaction,
                            packet,
                            now,
                            state,
                            &reason_code,
                            error_code,
                            Some(&initial_decision_id),
                            admission_decision,
                        );
                    }
                };
            }
            RouteAction::Spawn => {
                if reserve_spawn(&transaction, packet, &lease_id, now)? {
                    break;
                }
                return finish_stopped_with_supersedes(
                    transaction,
                    packet,
                    now,
                    JobState::Waiting,
                    "SPAWN_RESERVED",
                    OrchestrationErrorCode::ConcurrencyLimitReached,
                    Some(&initial_decision_id),
                );
            }
        }
    }

    insert_schedule_decision(
        &transaction,
        &final_decision_id,
        EXECUTION_FINAL_SOURCE,
        packet,
        &scheduled_agent,
        &final_recommendation,
        final_recommendation.action == RouteAction::Reuse,
        Some(&initial_decision_id),
        now,
    )?;
    insert_pending_lease(
        &transaction,
        &lease_id,
        &final_decision_id,
        packet,
        &scheduled_agent.agent_type,
        final_recommendation.candidate_thread_id(),
        now,
    )?;
    insert_attempt(
        &transaction,
        &attempt_id,
        revision
            .as_ref()
            .map(|context| context.previous_attempt_no + 1)
            .unwrap_or(1),
        revision
            .as_ref()
            .map(|context| context.previous_attempt_id.as_str()),
        &final_decision_id,
        &lease_id,
        packet,
        &final_recommendation,
        request.planned_execution_kind,
        now,
    )?;
    move_job_to_claimed(&transaction, &packet.job_id, now)?;
    let job = find_job_by_id(&transaction, &packet.job_id)
        .map_err(|_| persistence_error())?
        .ok_or_else(|| {
            schedule_error(
                OrchestrationErrorCode::InternalInvariantViolation,
                "事务提交前缺少 Job。",
                Some(&packet.job_id),
            )
        })?;
    let attempt = transaction
        .query_row(
            "SELECT attempt_id, job_id, attempt_no, previous_attempt_id,
                    schedule_decision_id, lease_id, route_action, planned_execution_kind,
                    execution_kind, thread_instance_id, codex_turn_id, state,
                    recovery_count, last_error_code, created_at, updated_at,
                    dispatch_recorded_at, accepted_at, terminal_at
             FROM job_attempts WHERE attempt_id = ?1",
            [&attempt_id],
            map_attempt,
        )
        .map_err(|_| persistence_error())?;
    let dispatch_profile = scheduled_agent.dispatch_profile.ok_or_else(|| {
        schedule_error(
            OrchestrationErrorCode::InternalInvariantViolation,
            "可派发 Agent 缺少事务内 Runtime Profile。",
            Some(&packet.job_id),
        )
    })?;
    let candidate_thread_id = final_recommendation
        .candidate_thread_id()
        .map(str::to_owned);
    let reason_code = final_recommendation.reason_code;
    transaction.commit().map_err(|_| persistence_error())?;
    Ok(AtomicScheduleOutcome::Ready(DispatchPermit {
        job,
        attempt,
        agent_type: scheduled_agent.agent_type,
        candidate_thread_id,
        reason_code,
        profile: dispatch_profile,
    }))
}

#[derive(Debug, Clone)]
struct AgentSnapshot {
    agent_type: String,
    is_reviewer: bool,
    active: bool,
    enabled: bool,
    model_available: bool,
    provider_available: bool,
    permission_allowed: bool,
    profile: LegacySchedulingProfile,
    dispatch_profile: Option<DispatchAgentProfile>,
}

#[derive(Debug, Clone)]
struct CandidateSnapshot {
    instance_id: String,
    reuse_state: String,
    reuse_state_reason: Option<String>,
    scoring: ScoringCandidate,
    context_pressure_percent: Option<i64>,
    context_pressure_limit_percent: i64,
    cache_hint: String,
    candidate_age_seconds: Option<i64>,
}

#[derive(Debug, Clone)]
struct RevisionScheduleContext {
    previous_attempt_id: String,
    previous_attempt_no: u32,
    thread_instance_id: String,
}

#[derive(Debug, Clone)]
struct RouteRecommendation {
    action: RouteAction,
    candidate: Option<CandidateSnapshot>,
    reason_code: String,
}

enum RecommendationEvaluation {
    Recommendation(RouteRecommendation),
    Stop {
        state: JobState,
        admission_decision: AdmissionDecision,
        reason_code: String,
        error_code: OrchestrationErrorCode,
    },
}

impl RouteRecommendation {
    fn candidate_thread_id(&self) -> Option<&str> {
        self.candidate
            .as_ref()
            .map(|candidate| candidate.scoring.hard_gate.thread_id.as_str())
    }
}

fn load_revision_schedule_context(
    connection: &Connection,
    job: &OrchestrationJob,
    current_attempt: Option<&JobAttempt>,
) -> Result<RevisionScheduleContext, OrchestrationError> {
    let attempt = current_attempt.ok_or_else(|| {
        schedule_error(
            OrchestrationErrorCode::InternalInvariantViolation,
            "REVISION_REQUIRED 缺少前序 Attempt。",
            Some(&job.job_id),
        )
    })?;
    let thread_instance_id = attempt.thread_instance_id.clone().ok_or_else(|| {
        schedule_error(
            OrchestrationErrorCode::InternalInvariantViolation,
            "REVISION_REQUIRED 的前序 Attempt 缺少 Thread。",
            Some(&job.job_id),
        )
    })?;
    let has_revision_decision = has_revision_decision(connection, &job.job_id, &attempt.attempt_id)
        .map_err(|_| persistence_error())?;
    if attempt.job_id != job.job_id
        || attempt.state != crate::orchestration_contract::AttemptState::Succeeded
        || attempt.attempt_no == u32::MAX
        || !has_revision_decision
    {
        return Err(schedule_error(
            OrchestrationErrorCode::InternalInvariantViolation,
            "REVISION_REQUIRED 缺少当前成功 Attempt 或权威修订决定。",
            Some(&job.job_id),
        ));
    }
    Ok(RevisionScheduleContext {
        previous_attempt_id: attempt.attempt_id.clone(),
        previous_attempt_no: attempt.attempt_no,
        thread_instance_id,
    })
}

fn has_revision_decision(
    connection: &Connection,
    job_id: &str,
    attempt_id: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM review_decisions review
            WHERE review.job_id=?1 AND review.attempt_id=?2
              AND review.decision='REVISION_REQUIRED'
         )",
        params![job_id, attempt_id],
        |row| row.get(0),
    )
}

/// 每次 Route/Claim 失败后的复核都从 SQLite 读取新事实；此函数不写入任何占用。
fn evaluate_recommendation(
    connection: &Connection,
    request: &AtomicScheduleRequest,
    agent: &AgentSnapshot,
    packet: &TaskPacket,
    now: &str,
    excluded_instance_ids: &[String],
    revision: Option<&RevisionScheduleContext>,
) -> Result<RecommendationEvaluation, OrchestrationError> {
    let project_excluded = project_is_excluded(connection, &packet.workspace_scope_key)?;
    let policy = evaluate_runtime_policy(&RuntimePolicyInput {
        mode: runtime_policy_mode(connection)?,
        delegation_required: true,
        failure_policy: runtime_failure_policy(connection)?,
        workspace_excluded: request.admission.workspace_excluded,
        project_excluded,
        conversation_excluded: request.admission.conversation_excluded,
        capability_supported: request.admission.agent_execution == Capability::Supported,
        runtime_healthy: request.admission.runtime_healthy
            && request.admission.runtime == Capability::Supported,
        permission_allowed: request.admission.permission_allowed
            && agent_permission_allows(agent, packet.permission_policy),
        scope_allowed: request.admission.scope_allowed,
        schedule_certain: request.admission.schedule_certain,
        lease_certain: request.admission.lease_certain,
        receipt_certain: request.admission.receipt_certain,
        schema_verified: request.admission.schema_verified
            && request.admission.event == Capability::Supported,
    });
    // E-02：Decision=ALLOW 仍可能代表 Default/排除；只有 reservation_allowed
    // 才能进入 Claim、Reservation 或 Pending Lease 创建路径。
    if !policy.reservation_allowed {
        return Ok(RecommendationEvaluation::Stop {
            state: JobState::Blocked,
            admission_decision: policy.decision,
            reason_code: policy.reason_code.as_str().to_owned(),
            error_code: orchestration_error_for_policy(policy.reason_code),
        });
    }
    if !request.admission.global_concurrency_available {
        return Ok(RecommendationEvaluation::Stop {
            state: JobState::Blocked,
            admission_decision: AdmissionDecision::Deny,
            reason_code: "GLOBAL_CONCURRENCY_LIMIT_REACHED".to_owned(),
            error_code: OrchestrationErrorCode::ConcurrencyLimitReached,
        });
    }

    let type_conflict = live_agent_type_lease_exists(
        connection,
        &packet.workspace_scope_key,
        &packet.parent_thread_id,
        &agent.agent_type,
    )?;
    let hard_context = HardGateContext {
        agent: AgentExecutionInput {
            active: agent.active,
            enabled: agent.enabled,
            model_available: agent.model_available,
            provider_available: agent.provider_available,
        },
        exclusions: ExclusionInput {
            workspace_excluded: request.admission.workspace_excluded,
            project_excluded,
            conversation_excluded: request.admission.conversation_excluded,
        },
        execution: ExecutionCapabilityInput {
            selected_kind: scheduler_execution_kind(request.planned_execution_kind),
            runtime: request.admission.runtime,
            agent_execution: request.admission.agent_execution,
            event: request.admission.event,
        },
        required_identity: RequiredIdentity {
            agent_id: packet.agent_id.clone(),
            runtime_fingerprint: agent.profile.runtime_fingerprint.clone(),
        },
        required_scope: RequiredScope {
            workspace_scope_key: packet.workspace_scope_key.clone(),
            parent_thread_id: packet.parent_thread_id.clone(),
            task_scope_key: packet.task_scope_key.clone(),
        },
        cache_requirement: request.admission.cache_requirement,
        requested_agent_type: agent.agent_type.clone(),
        conflicting_agent_type_lease: type_conflict,
    };
    let mut candidates = load_candidates(connection, agent, packet, now)?
        .into_iter()
        .filter(|candidate| !excluded_instance_ids.contains(&candidate.instance_id))
        .collect::<Vec<_>>();
    if let Some(revision) = revision {
        candidates.retain(|candidate| candidate.instance_id == revision.thread_instance_id);
        for candidate in &mut candidates {
            if candidate.reuse_state == "HELD_FOR_REVIEW" && candidate.reuse_state_reason.is_none()
            {
                candidate.scoring.hard_gate.review_pending = false;
            }
        }
    }
    let scoring_candidates = candidates
        .iter()
        .map(|candidate| candidate.scoring.clone())
        .collect::<Vec<_>>();
    match select_reuse(
        &hard_context,
        ScoringPolicy {
            reuse_strategy: scoring_strategy(&agent.profile),
        },
        &scoring_candidates,
    ) {
        ReuseSelection::Reuse(selected) => Ok(RecommendationEvaluation::Recommendation(
            RouteRecommendation {
                action: RouteAction::Reuse,
                candidate: candidates
                    .iter()
                    .find(|candidate| candidate.scoring.stable_key == selected.stable_key)
                    .cloned(),
                reason_code: if revision.is_some() {
                    "REVISION_EXACT_THREAD_HEALTHY".to_owned()
                } else {
                    "EXACT_WORKSPACE_SCOPE_IDLE".to_owned()
                },
            },
        )),
        ReuseSelection::SpawnAllowed { reason } => Ok(RecommendationEvaluation::Recommendation(
            RouteRecommendation {
                action: RouteAction::Spawn,
                candidate: None,
                reason_code: reason
                    .map(HardGateReasonCode::as_str)
                    .unwrap_or("NO_WORKSPACE_SCOPE_MATCH")
                    .to_owned(),
            },
        )),
        ReuseSelection::Blocked { reason }
            if reason == HardGateReasonCode::AgentTypeLeaseConflict =>
        {
            Ok(RecommendationEvaluation::Stop {
                state: JobState::Waiting,
                admission_decision: AdmissionDecision::Deny,
                reason_code: reason.as_str().to_owned(),
                error_code: OrchestrationErrorCode::ConcurrencyLimitReached,
            })
        }
        ReuseSelection::Blocked { reason } => Ok(RecommendationEvaluation::Stop {
            state: JobState::Blocked,
            admission_decision: AdmissionDecision::Deny,
            reason_code: reason.as_str().to_owned(),
            error_code: orchestration_error_for_reason(reason),
        }),
    }
}

fn agent_permission_allows(agent: &AgentSnapshot, requested: PermissionPolicy) -> bool {
    let _ = requested;
    agent.permission_allowed
}

fn effective_sandbox_policy(configured: &str, requested: PermissionPolicy) -> Option<String> {
    let configured = match configured {
        "READ_ONLY" | "WORKSPACE_WRITE" | "DANGER_FULL_ACCESS" => configured,
        _ => return None,
    };
    match requested {
        PermissionPolicy::Inherit => Some(configured.to_owned()),
        PermissionPolicy::ReadOnly => Some("READ_ONLY".to_owned()),
        PermissionPolicy::WorkspaceWrite
            if matches!(configured, "WORKSPACE_WRITE" | "DANGER_FULL_ACCESS") =>
        {
            Some("WORKSPACE_WRITE".to_owned())
        }
        PermissionPolicy::DangerFullAccess if configured == "DANGER_FULL_ACCESS" => {
            Some("DANGER_FULL_ACCESS".to_owned())
        }
        _ => None,
    }
}

fn load_agent_snapshot(
    connection: &Connection,
    agent_id: &str,
    permission_policy: PermissionPolicy,
) -> Result<Option<AgentSnapshot>, OrchestrationError> {
    type AgentRow = (
        String,
        String,
        Option<String>,
        bool,
        bool,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<bool>,
        String,
        Option<String>,
        String,
        String,
        Option<i64>,
        String,
        String,
        Option<i64>,
        String,
        Option<bool>,
        Option<bool>,
        String,
        String,
    );
    let row: Option<AgentRow> = connection
        .query_row(
            "SELECT a.agent_key, a.name, a.role_key, a.enabled,
                    EXISTS(SELECT 1 FROM active_agent_bindings active WHERE active.agent_id = a.id),
                    binding.model_id, model.model_id, provider.provider_key, provider.preset_id,
                    provider.base_url, provider.protocol, provider.custom_headers_json,
                    model.default_reasoning, model.reasoning_supported,
                    a.reasoning_policy, a.orchestration_phase, a.sandbox_policy, a.instruction,
                    a.cache_retention_override_seconds, a.reuse_strategy,
                    COALESCE(provider.cache_support, 'UNKNOWN'),
                    provider.cache_retention_hint_seconds,
                    COALESCE(provider.cache_retention_type, 'UNKNOWN'),
                    model.enabled, provider.enabled, a.agent_type, a.source
             FROM agents a
             LEFT JOIN agent_model_bindings binding
               ON binding.agent_id = a.id AND binding.enabled = 1
             LEFT JOIN models model ON model.id = binding.model_id
             LEFT JOIN providers provider ON provider.id = model.provider_id
             WHERE a.id = ?1",
            [agent_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, i64>(4)? != 0,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    row.get(10)?,
                    row.get(11)?,
                    row.get(12)?,
                    row.get::<_, Option<i64>>(13)?.map(|value| value != 0),
                    row.get(14)?,
                    row.get(15)?,
                    row.get(16)?,
                    row.get(17)?,
                    row.get(18)?,
                    row.get(19)?,
                    row.get(20)?,
                    row.get(21)?,
                    row.get(22)?,
                    row.get::<_, Option<i64>>(23)?.map(|value| value != 0),
                    row.get::<_, Option<i64>>(24)?.map(|value| value != 0),
                    row.get(25)?,
                    row.get(26)?,
                ))
            },
        )
        .optional()
        .map_err(|_| persistence_error())?;
    let Some((
        agent_key,
        agent_name,
        role_key,
        enabled,
        active,
        model_id,
        model_slug,
        provider_key,
        preset_id,
        base_url,
        protocol,
        custom_headers_json,
        model_default_reasoning,
        reasoning_supported,
        reasoning_policy,
        orchestration_phase,
        sandbox_policy,
        instruction,
        agent_cache_retention_override_seconds,
        reuse_strategy,
        cache_support,
        cache_retention_hint_seconds,
        cache_retention_type,
        model_enabled,
        provider_enabled,
        agent_record_type,
        agent_source,
    )) = row
    else {
        return Ok(None);
    };
    let model_available = model_id.is_some() && model_slug.is_some() && model_enabled == Some(true);
    let provider_available = provider_key.is_some()
        && base_url.is_some()
        && protocol.is_some()
        && provider_enabled == Some(true);
    let effective_sandbox = effective_sandbox_policy(&sandbox_policy, permission_policy);
    let (runtime_fingerprint, reasoning_effort) = match (
        model_id.as_deref(),
        model_slug.as_deref(),
        provider_key.as_deref(),
        base_url.as_deref(),
        protocol.as_deref(),
        effective_sandbox.as_deref(),
    ) {
        (
            Some(model_id),
            Some(model_slug),
            Some(provider_key),
            Some(base_url),
            Some(protocol),
            Some(effective_sandbox),
        ) => {
            let configured_efforts = fingerprint_values(
                connection,
                "SELECT effort FROM model_reasoning_efforts WHERE model_id = ?1",
                model_id,
            )?;
            let supported_efforts = effective_model_reasoning_efforts(
                reasoning_supported,
                model_default_reasoning.as_deref(),
                &configured_efforts,
            );
            let reasoning_effort = resolve_agent_reasoning_effort(
                &reasoning_policy,
                model_default_reasoning.as_deref(),
                &supported_efforts,
            );
            let fingerprint = runtime_fingerprint(
                connection,
                agent_id,
                model_id,
                provider_key,
                preset_id.as_deref(),
                base_url,
                protocol,
                custom_headers_json.as_deref(),
                model_slug,
                reasoning_effort.as_deref().unwrap_or_default(),
                effective_sandbox,
                &instruction,
                orchestration_phase.as_deref().unwrap_or_default(),
            )?;
            (Some(fingerprint), reasoning_effort)
        }
        _ => (None, None),
    };
    let agent_type = role_key
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .unwrap_or_else(|| agent_key.clone());
    let dispatch_profile = match (
        runtime_fingerprint.as_ref(),
        model_slug.as_ref(),
        provider_key.as_ref(),
    ) {
        (Some(runtime_fingerprint), Some(model_slug), Some(provider_key)) => {
            Some(DispatchAgentProfile {
                agent_id: agent_id.to_owned(),
                agent_key: agent_key.clone(),
                agent_name,
                instruction: instruction.clone(),
                orchestration_phase: orchestration_phase.clone().unwrap_or_default(),
                sandbox_policy: effective_sandbox.clone().unwrap_or_default(),
                reasoning_effort,
                model_slug: model_slug.clone(),
                model_provider: (preset_id.as_deref() != Some("codex-native"))
                    .then(|| format!("cas_{provider_key}")),
                runtime_fingerprint: runtime_fingerprint.clone(),
            })
        }
        _ => None,
    };
    Ok(Some(AgentSnapshot {
        agent_type,
        is_reviewer: agent_record_type == "PRESET"
            && agent_source == "CAS"
            && role_key.as_deref() == Some("reviewer")
            && orchestration_phase.as_deref() == Some("REVIEW")
            && sandbox_policy == "READ_ONLY",
        active,
        enabled,
        model_available,
        provider_available,
        permission_allowed: effective_sandbox.is_some(),
        profile: LegacySchedulingProfile {
            reuse_strategy,
            orchestration_phase,
            cache_support,
            cache_retention_type,
            cache_retention_hint_seconds,
            agent_cache_retention_override_seconds,
            runtime_fingerprint,
        },
        dispatch_profile,
    }))
}

fn reviewer_assignment_exists(
    connection: &Connection,
    reviewer_job_id: &str,
) -> rusqlite::Result<bool> {
    connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM reviewer_assignments WHERE reviewer_job_id=?1)",
        [reviewer_job_id],
        |row| row.get(0),
    )
}

#[allow(clippy::too_many_arguments)]
fn runtime_fingerprint(
    connection: &Connection,
    agent_id: &str,
    model_id: &str,
    provider_key: &str,
    preset_id: Option<&str>,
    base_url: &str,
    protocol: &str,
    custom_headers_json: Option<&str>,
    model_slug: &str,
    reasoning_effort: &str,
    sandbox_policy: &str,
    instruction: &str,
    orchestration_phase: &str,
) -> Result<String, OrchestrationError> {
    Ok(shared_runtime_fingerprint(&[
        ("provider_key", vec![provider_key.to_owned()]),
        (
            "provider_preset_id",
            vec![preset_id.unwrap_or_default().to_owned()],
        ),
        ("provider_base_url", vec![base_url.to_owned()]),
        ("provider_protocol", vec![protocol.to_owned()]),
        (
            "provider_custom_headers_json",
            vec![custom_headers_json.unwrap_or_default().to_owned()],
        ),
        ("model", vec![model_slug.to_owned()]),
        ("reasoning", vec![reasoning_effort.to_owned()]),
        ("sandbox", vec![sandbox_policy.to_owned()]),
        ("orchestration_phase", vec![orchestration_phase.to_owned()]),
        (
            "instruction",
            vec![render_delegated_agent_instructions_for_phase(
                instruction,
                Some(orchestration_phase),
            )],
        ),
        (
            "required_capabilities",
            fingerprint_values(
                connection,
                "SELECT capability FROM agent_required_capabilities WHERE agent_id = ?1",
                agent_id,
            )?,
        ),
        (
            "preferred_capabilities",
            fingerprint_values(
                connection,
                "SELECT capability FROM agent_preferred_capabilities WHERE agent_id = ?1",
                agent_id,
            )?,
        ),
        (
            "skills",
            skill_fingerprint_values(fingerprint_values(
                connection,
                "SELECT skill_key FROM agent_skill_bindings WHERE agent_id = ?1",
                agent_id,
            )?),
        ),
        (
            "disabled_mcp_servers",
            fingerprint_values(
                connection,
                "SELECT server_id FROM agent_disabled_mcp_servers WHERE agent_id = ?1",
                agent_id,
            )?,
        ),
        (
            "mcp_tool_policies",
            fingerprint_values(
                connection,
                "SELECT server_id || '=' || mode || '=' || tool_name
                 FROM agent_mcp_tool_policies WHERE agent_id = ?1",
                agent_id,
            )?,
        ),
        (
            "model_capabilities",
            fingerprint_values(
                connection,
                "SELECT capability || '=' || status FROM model_capabilities WHERE model_id = ?1",
                model_id,
            )?,
        ),
    ]))
}

fn fingerprint_values(
    connection: &Connection,
    sql: &str,
    parameter: &str,
) -> Result<Vec<String>, OrchestrationError> {
    connection
        .prepare(sql)
        .and_then(|mut statement| {
            statement
                .query_map([parameter], |row| row.get(0))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|_| persistence_error())
}

fn load_candidates(
    connection: &Connection,
    agent: &AgentSnapshot,
    packet: &TaskPacket,
    now: &str,
) -> Result<Vec<CandidateSnapshot>, OrchestrationError> {
    let mut statement = connection
        .prepare(
            "SELECT instance.id, instance.agent_id, instance.codex_thread_id,
                    instance.parent_thread_id, instance.scope_key, instance.task_scope_key,
                    instance.status, instance.current_context_tokens, instance.context_window,
                    instance.runtime_fingerprint, instance.last_model_usage_at,
                    instance.cached_input_tokens,
                    CASE WHEN instance.claimed_until IS NOT NULL
                              AND julianday(instance.claimed_until) > julianday(?2)
                         THEN 1 ELSE 0 END,
                    instance.reuse_state, instance.reuse_state_reason, instance.execution_kind,
                    CAST(MAX(0, (julianday(?2) - julianday(instance.last_model_usage_at)) * 86400) AS INTEGER),
                    EXISTS(
                        SELECT 1 FROM runtime_delegation_leases lease
                        WHERE lease.codex_agent_id = instance.codex_thread_id
                          AND lease.state IN ('PENDING', 'ACTIVE')
                    )
             FROM agent_thread_instances instance
             WHERE instance.agent_id = ?1
             ORDER BY instance.codex_thread_id, instance.id",
        )
        .map_err(|_| persistence_error())?;
    statement
        .query_map(params![packet.agent_id, now], |row| {
            let instance_id: String = row.get(0)?;
            let candidate_agent_id: Option<String> = row.get(1)?;
            let thread_id: String = row.get(2)?;
            let parent_thread_id: Option<String> = row.get(3)?;
            let workspace_scope_key: Option<String> = row.get(4)?;
            let task_scope_key: Option<String> = row.get(5)?;
            let status: String = row.get(6)?;
            let current_context_tokens: Option<i64> = row.get(7)?;
            let context_window: Option<i64> = row.get(8)?;
            let runtime_fingerprint: Option<String> = row.get(9)?;
            let cached_input_tokens: i64 = row.get(11)?;
            let active_claim: bool = row.get::<_, i64>(12)? != 0;
            let reuse_state: String = row.get(13)?;
            let reuse_state_reason: Option<String> = row.get(14)?;
            let execution_kind: String = row.get(15)?;
            let age_seconds: Option<i64> = row.get(16)?;
            let active_lease: bool = row.get::<_, i64>(17)? != 0;
            let context_limit = context_pressure_limit(&agent.profile, age_seconds);
            let context_pressure =
                context_pressure(current_context_tokens, context_window, context_limit);
            let cache_hint_value = cache_hint(&agent.profile, age_seconds).to_owned();
            let cache_state = match cache_hint_value.as_str() {
                "WITHIN_RETENTION_HINT" => CacheState::WithinRequiredWindow,
                "OUTSIDE_RETENTION_HINT" => CacheState::OutsideRequiredWindow,
                "SUPPORTED_NO_RETENTION_HINT" | "UNKNOWN" => CacheState::Unknown,
                _ => CacheState::NotObserved,
            };
            let cache_source = if cached_input_tokens > 0 {
                CacheEvidenceSource::Observed
            } else {
                match effective_cache_retention(&agent.profile).1 {
                    "PROVIDER" => CacheEvidenceSource::Provider,
                    "AGENT_OVERRIDE" => CacheEvidenceSource::AgentOverride,
                    _ => CacheEvidenceSource::Unknown,
                }
            };
            let thread_state = match status.as_str() {
                "IDLE" | "RECOVERY_REQUIRED" => ThreadState::Idle,
                "RUNNING" => ThreadState::Active,
                _ => ThreadState::Unknown,
            };
            let context_health = if matches!(context_pressure, ContextPressure::Known { .. }) {
                ContextHealth::Healthy
            } else {
                ContextHealth::Unknown
            };
            let execution_kind = match execution_kind.as_str() {
                "NATIVE_CHILD" => SchedulerExecutionKind::NativeChild,
                "MANAGED_WORKER" => SchedulerExecutionKind::ManagedWorker,
                _ => SchedulerExecutionKind::ObservedExternal,
            };
            let review_pending = reuse_state != "ACTIVE";
            Ok(CandidateSnapshot {
                instance_id: instance_id.clone(),
                reuse_state,
                reuse_state_reason,
                scoring: ScoringCandidate {
                    hard_gate: CandidateHardGateInput {
                        thread_id,
                        agent_id: candidate_agent_id.unwrap_or_default(),
                        runtime_fingerprint,
                        execution_kind,
                        scope: CandidateScope {
                            workspace_scope_key: workspace_scope_key.unwrap_or_default(),
                            parent_thread_id: parent_thread_id.unwrap_or_default(),
                            task_scope_key: task_scope_key.unwrap_or_default(),
                        },
                        thread_state,
                        context_health,
                        active_claim,
                        active_lease,
                        recovery_required: status == "RECOVERY_REQUIRED",
                        review_pending,
                        context_pressure,
                        cache_state,
                    },
                    age_seconds: age_seconds.and_then(|value| u64::try_from(value).ok()),
                    cached_input_tokens: u64::try_from(cached_input_tokens).ok(),
                    cache_evidence_source: cache_source,
                    stable_key: instance_id,
                },
                context_pressure_percent: observed_context_percent(
                    current_context_tokens,
                    context_window,
                ),
                context_pressure_limit_percent: context_limit,
                cache_hint: cache_hint_value,
                candidate_age_seconds: age_seconds,
            })
        })
        .map_err(|_| persistence_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| persistence_error())
}

fn context_pressure(tokens: Option<i64>, window: Option<i64>, limit: i64) -> ContextPressure {
    let Some(percent) = observed_context_percent(tokens, window) else {
        return ContextPressure::Unknown;
    };
    let observed_percent = if percent > 100 {
        101
    } else {
        u8::try_from(percent).unwrap_or(101)
    };
    ContextPressure::Known {
        observed_percent,
        limit_percent: u8::try_from(limit).unwrap_or(101),
    }
}

fn observed_context_percent(tokens: Option<i64>, window: Option<i64>) -> Option<i64> {
    tokens.zip(window).and_then(|(tokens, window)| {
        (tokens >= 0 && window > 0).then(|| tokens.saturating_mul(100) / window)
    })
}

fn scoring_strategy(profile: &LegacySchedulingProfile) -> ReuseStrategy {
    match effective_reuse_strategy(profile) {
        "HOT" => ReuseStrategy::Hot,
        "COLD" => ReuseStrategy::Cold,
        _ => ReuseStrategy::Auto,
    }
}

fn scheduler_execution_kind(kind: ExecutionKind) -> SchedulerExecutionKind {
    match kind {
        ExecutionKind::NativeChild => SchedulerExecutionKind::NativeChild,
        ExecutionKind::ManagedWorker => SchedulerExecutionKind::ManagedWorker,
        ExecutionKind::ObservedExternal => SchedulerExecutionKind::ObservedExternal,
    }
}

fn validate_expected(
    request: &AtomicScheduleRequest,
    recommendation: &RouteRecommendation,
    job_id: &str,
) -> Result<(), OrchestrationError> {
    if request.expected_decision != recommendation.action {
        return Err(schedule_error(
            OrchestrationErrorCode::StaleExpectedDecision,
            "执行期 Recommendation 已变化。",
            Some(job_id),
        ));
    }
    if request.expected_candidate_thread_id.as_deref() != recommendation.candidate_thread_id() {
        return Err(schedule_error(
            OrchestrationErrorCode::StaleExpectedCandidate,
            "执行期候选 Thread 已变化。",
            Some(job_id),
        ));
    }
    Ok(())
}

fn claim_candidate(
    transaction: &Transaction<'_>,
    candidate: &CandidateSnapshot,
    lease_id: &str,
    agent: &AgentSnapshot,
    packet: &TaskPacket,
    now: &str,
) -> Result<usize, OrchestrationError> {
    transaction
        .execute(
            "UPDATE agent_thread_instances
             SET claimed_until = strftime('%Y-%m-%dT%H:%M:%fZ', ?6, ?7),
                 claim_lease_id = ?2
             WHERE id = ?1
               AND agent_id = ?3
               AND scope_key = ?4
               AND parent_thread_id = ?5
               AND task_scope_key = ?8
               AND status = 'IDLE'
               AND reuse_state = 'ACTIVE'
               AND execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER')
               AND runtime_fingerprint = ?9
               AND (claimed_until IS NULL OR julianday(claimed_until) <= julianday(?6))
               AND NOT EXISTS (
                   SELECT 1 FROM runtime_delegation_leases lease
                   WHERE lease.codex_agent_id = agent_thread_instances.codex_thread_id
                     AND lease.state IN ('PENDING', 'ACTIVE')
               )",
            params![
                candidate.instance_id,
                lease_id,
                packet.agent_id,
                packet.workspace_scope_key,
                packet.parent_thread_id,
                now,
                format!("+{REUSE_CLAIM_TTL_SECONDS} seconds"),
                packet.task_scope_key,
                agent.profile.runtime_fingerprint.as_deref(),
            ],
        )
        .map_err(|_| persistence_error())
}

/// `HELD_FOR_REVIEW` 不是普通复用池成员。只有产生该 Hold 的同一 Job 下一 Attempt
/// 可以在仍满足身份、Scope、健康与占用条件时原子取得它。
fn claim_revision_candidate(
    transaction: &Transaction<'_>,
    candidate: &CandidateSnapshot,
    revision: &RevisionScheduleContext,
    lease_id: &str,
    agent: &AgentSnapshot,
    packet: &TaskPacket,
    now: &str,
) -> Result<usize, OrchestrationError> {
    transaction
        .execute(
            "UPDATE agent_thread_instances AS instance
             SET claimed_until = strftime('%Y-%m-%dT%H:%M:%fZ', ?6, ?7),
                 claim_lease_id = ?2
             WHERE instance.id = ?1
               AND instance.agent_id = ?3
               AND instance.scope_key = ?4
               AND instance.parent_thread_id = ?5
               AND instance.task_scope_key = ?8
               AND instance.status = 'IDLE'
               AND instance.reuse_state = 'HELD_FOR_REVIEW'
               AND instance.reuse_state_reason IS NULL
               AND instance.execution_kind IN ('NATIVE_CHILD', 'MANAGED_WORKER')
               AND instance.runtime_fingerprint = ?9
               AND (instance.claimed_until IS NULL OR julianday(instance.claimed_until) <= julianday(?6))
               AND NOT EXISTS (
                   SELECT 1 FROM runtime_delegation_leases lease
                   WHERE lease.codex_agent_id = instance.codex_thread_id
                     AND lease.state IN ('PENDING', 'ACTIVE')
               )
               AND EXISTS (
                   SELECT 1
                   FROM orchestration_jobs job
                   JOIN job_attempts previous
                     ON previous.job_id = job.job_id
                    AND previous.attempt_id = ?11
                   JOIN review_decisions review
                     ON review.job_id = job.job_id
                    AND review.attempt_id = previous.attempt_id
                   WHERE job.job_id = ?10
                     AND job.state IN ('REVISION_REQUIRED', 'WAITING')
                     AND previous.state = 'SUCCEEDED'
                     AND previous.attempt_no = ?12
                     AND previous.thread_instance_id = instance.id
                     AND review.decision = 'REVISION_REQUIRED'
               )",
            params![
                candidate.instance_id,
                lease_id,
                packet.agent_id,
                packet.workspace_scope_key,
                packet.parent_thread_id,
                now,
                format!("+{REUSE_CLAIM_TTL_SECONDS} seconds"),
                packet.task_scope_key,
                agent.profile.runtime_fingerprint.as_deref(),
                packet.job_id,
                revision.previous_attempt_id,
                revision.previous_attempt_no,
            ],
        )
        .map_err(|_| persistence_error())
}

fn reserve_spawn(
    transaction: &Transaction<'_>,
    packet: &TaskPacket,
    lease_id: &str,
    now: &str,
) -> Result<bool, OrchestrationError> {
    transaction
        .execute(
            "DELETE FROM agent_spawn_reservations
             WHERE agent_id = ?1 AND parent_thread_id = ?2
               AND workspace_scope_key = ?3 AND task_scope_key = ?4
               AND julianday(reserved_until) <= julianday(?5)
               AND (
                   lease_id IS NULL OR NOT EXISTS (
                       SELECT 1 FROM runtime_delegation_leases lease
                       WHERE lease.id = agent_spawn_reservations.lease_id
                         AND lease.state IN ('PENDING', 'ACTIVE')
                   )
               )",
            params![
                packet.agent_id,
                packet.parent_thread_id,
                packet.workspace_scope_key,
                packet.task_scope_key,
                now,
            ],
        )
        .map_err(|_| persistence_error())?;
    let changed = transaction
        .execute(
            "INSERT INTO agent_spawn_reservations (
                agent_id, parent_thread_id, workspace_scope_key, task_scope_key,
                reserved_until, created_at, lease_id, job_id
             ) VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ', ?5, ?6), ?5, ?7, ?8)
             ON CONFLICT(agent_id, parent_thread_id, workspace_scope_key, task_scope_key)
             DO NOTHING",
            params![
                packet.agent_id,
                packet.parent_thread_id,
                packet.workspace_scope_key,
                packet.task_scope_key,
                now,
                format!("+{SPAWN_RESERVATION_TTL_SECONDS} seconds"),
                lease_id,
                packet.job_id,
            ],
        )
        .map_err(|_| persistence_error())?;
    Ok(changed == 1)
}

fn insert_job(
    transaction: &Transaction<'_>,
    packet: &TaskPacket,
    canonical_packet: &str,
    packet_hash: &str,
    now: &str,
) -> Result<(), OrchestrationError> {
    transaction
        .execute(
            "INSERT INTO orchestration_jobs (
                job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                parent_thread_id, workspace_scope_key, task_scope_key, state,
                last_error_code, created_at, updated_at, terminal_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'CREATED', NULL, ?9, ?9, NULL)",
            params![
                packet.job_id,
                packet.idempotency_key,
                canonical_packet,
                packet_hash,
                packet.agent_id,
                packet.parent_thread_id,
                packet.workspace_scope_key,
                packet.task_scope_key,
                now,
            ],
        )
        .map_err(|_| persistence_error())?;
    Ok(())
}

fn insert_schedule_decision(
    transaction: &Transaction<'_>,
    decision_id: &str,
    source: &str,
    packet: &TaskPacket,
    agent: &AgentSnapshot,
    recommendation: &RouteRecommendation,
    claimed: bool,
    supersedes_decision_id: Option<&str>,
    now: &str,
) -> Result<(), OrchestrationError> {
    let candidate = recommendation.candidate.as_ref();
    transaction
        .execute(
            "INSERT INTO agent_schedule_decisions (
                id, created_at, source, agent_id, agent_name_snapshot, workspace_scope_key,
                parent_thread_id, candidate_thread_id, decision, reason_code,
                runtime_fingerprint, context_pressure_percent, context_pressure_limit_percent,
                cache_hint, candidate_age_seconds, claimed, task_scope_key, job_id,
                supersedes_decision_id
             ) VALUES (
                ?1, ?2, ?3, ?4, (SELECT name FROM agents WHERE id = ?4), ?5,
                ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
             )",
            params![
                decision_id,
                now,
                source,
                packet.agent_id,
                packet.workspace_scope_key,
                packet.parent_thread_id,
                recommendation.candidate_thread_id(),
                enum_name(recommendation.action),
                recommendation.reason_code,
                agent.profile.runtime_fingerprint.as_deref(),
                candidate.and_then(|candidate| candidate.context_pressure_percent),
                candidate
                    .map(|candidate| candidate.context_pressure_limit_percent)
                    .unwrap_or(0),
                candidate
                    .map(|candidate| candidate.cache_hint.as_str())
                    .unwrap_or("UNKNOWN"),
                candidate.and_then(|candidate| candidate.candidate_age_seconds),
                i64::from(claimed),
                packet.task_scope_key,
                packet.job_id,
                supersedes_decision_id,
            ],
        )
        .map_err(|_| persistence_error())?;
    Ok(())
}

fn insert_pending_lease(
    transaction: &Transaction<'_>,
    lease_id: &str,
    decision_id: &str,
    packet: &TaskPacket,
    agent_type: &str,
    candidate_thread_id: Option<&str>,
    now: &str,
) -> Result<(), OrchestrationError> {
    transaction
        .execute(
            "INSERT INTO runtime_delegation_leases (
                id, created_at, updated_at, agent_id, parent_thread_id, codex_agent_id,
                workspace_scope_key, task_scope_key, schedule_decision_id, state,
                expires_at, agent_type
             ) VALUES (
                ?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'PENDING',
                strftime('%Y-%m-%dT%H:%M:%fZ', ?2, ?9), ?10
             )",
            params![
                lease_id,
                now,
                packet.agent_id,
                packet.parent_thread_id,
                candidate_thread_id,
                packet.workspace_scope_key,
                packet.task_scope_key,
                decision_id,
                format!("+{SPAWN_RESERVATION_TTL_SECONDS} seconds"),
                agent_type,
            ],
        )
        .map_err(|_| persistence_error())?;
    Ok(())
}

fn insert_attempt(
    transaction: &Transaction<'_>,
    attempt_id: &str,
    attempt_no: u32,
    previous_attempt_id: Option<&str>,
    decision_id: &str,
    lease_id: &str,
    packet: &TaskPacket,
    recommendation: &RouteRecommendation,
    planned_execution_kind: ExecutionKind,
    now: &str,
) -> Result<(), OrchestrationError> {
    transaction
        .execute(
            "INSERT INTO job_attempts (
                attempt_id, job_id, attempt_no, previous_attempt_id,
                schedule_decision_id, lease_id, route_action, planned_execution_kind,
                execution_kind, thread_instance_id, codex_turn_id, state, recovery_count,
                last_error_code, created_at, updated_at, dispatch_recorded_at,
                accepted_at, terminal_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9, NULL,
                'PLANNED', 0, NULL, ?10, ?10, NULL, NULL, NULL
             )",
            params![
                attempt_id,
                packet.job_id,
                attempt_no,
                previous_attempt_id,
                decision_id,
                lease_id,
                enum_name(recommendation.action),
                enum_name(planned_execution_kind),
                recommendation
                    .candidate
                    .as_ref()
                    .map(|candidate| candidate.instance_id.as_str()),
                now,
            ],
        )
        .map_err(|_| persistence_error())?;
    Ok(())
}

fn finish_stopped(
    transaction: Transaction<'_>,
    packet: &TaskPacket,
    now: &str,
    state: JobState,
    reason_code: &str,
    error_code: OrchestrationErrorCode,
) -> Result<AtomicScheduleOutcome, OrchestrationError> {
    finish_stopped_with_admission(
        transaction,
        packet,
        now,
        state,
        reason_code,
        error_code,
        None,
        AdmissionDecision::Deny,
    )
}

fn finish_stopped_with_supersedes(
    transaction: Transaction<'_>,
    packet: &TaskPacket,
    now: &str,
    state: JobState,
    reason_code: &str,
    error_code: OrchestrationErrorCode,
    supersedes_decision_id: Option<&str>,
) -> Result<AtomicScheduleOutcome, OrchestrationError> {
    finish_stopped_with_admission(
        transaction,
        packet,
        now,
        state,
        reason_code,
        error_code,
        supersedes_decision_id,
        AdmissionDecision::Deny,
    )
}

fn finish_stopped_with_admission(
    transaction: Transaction<'_>,
    packet: &TaskPacket,
    now: &str,
    state: JobState,
    reason_code: &str,
    error_code: OrchestrationErrorCode,
    supersedes_decision_id: Option<&str>,
    admission_decision: AdmissionDecision,
) -> Result<AtomicScheduleOutcome, OrchestrationError> {
    let decision_id = Uuid::new_v4().to_string();
    transaction
        .execute(
            "INSERT INTO agent_schedule_decisions (
                id, created_at, source, agent_id, agent_name_snapshot, workspace_scope_key,
                parent_thread_id, candidate_thread_id, decision, reason_code,
                runtime_fingerprint, context_pressure_percent, context_pressure_limit_percent,
                cache_hint, candidate_age_seconds, claimed, task_scope_key, job_id,
                supersedes_decision_id
             ) VALUES (
                ?1, ?2, ?3, ?4, (SELECT name FROM agents WHERE id = ?4), ?5,
                ?6, NULL, ?7, ?8, NULL, NULL, 0, 'UNKNOWN', NULL, 0, ?9, ?10, ?11
             )",
            params![
                decision_id,
                now,
                EXECUTION_FINAL_SOURCE,
                packet.agent_id,
                packet.workspace_scope_key,
                packet.parent_thread_id,
                if state == JobState::Waiting {
                    "WAIT"
                } else {
                    "BLOCK"
                },
                reason_code,
                packet.task_scope_key,
                packet.job_id,
                supersedes_decision_id,
            ],
        )
        .map_err(|_| persistence_error())?;
    let job = find_job_by_id(&transaction, &packet.job_id)
        .map_err(|_| persistence_error())?
        .ok_or_else(|| persistence_error())?;
    if job.state != state {
        update_job_state(
            &transaction,
            &packet.job_id,
            job.state,
            state,
            Some(error_code),
            now,
        )?;
    }
    let job = find_job_by_id(&transaction, &packet.job_id)
        .map_err(|_| persistence_error())?
        .ok_or_else(|| persistence_error())?;
    transaction.commit().map_err(|_| persistence_error())?;
    let stop = ScheduleStop {
        job,
        schedule_decision_id: decision_id,
        admission_decision,
        reason_code: reason_code.to_owned(),
        error_code,
    };
    Ok(if state == JobState::Waiting {
        AtomicScheduleOutcome::Waiting(stop)
    } else {
        AtomicScheduleOutcome::Blocked(stop)
    })
}

fn move_job_to_claimed(
    transaction: &Transaction<'_>,
    job_id: &str,
    now: &str,
) -> Result<(), OrchestrationError> {
    let job = find_job_by_id(transaction, job_id)
        .map_err(|_| persistence_error())?
        .ok_or_else(|| persistence_error())?;
    let mut state = job.state;
    if state != JobState::Routed {
        update_job_state(transaction, job_id, state, JobState::Routed, None, now)?;
        state = JobState::Routed;
    }
    update_job_state(transaction, job_id, state, JobState::Claimed, None, now)
}

fn expire_proven_undispatched_leases(
    transaction: &Transaction<'_>,
    now: &str,
) -> Result<(), OrchestrationError> {
    transaction
        .execute(
            "UPDATE runtime_delegation_leases AS lease
             SET state = 'EXPIRED', released_at = ?1, updated_at = ?1,
                 release_reason = 'TTL_EXPIRED_UNDISPATCHED'
             WHERE lease.state = 'PENDING'
               AND julianday(lease.expires_at) <= julianday(?1)
               AND (
                   EXISTS (
                       SELECT 1 FROM job_attempts attempt
                       WHERE attempt.lease_id = lease.id
                         AND attempt.state = 'PLANNED'
                         AND attempt.dispatch_recorded_at IS NULL
                   )
                   OR (
                       lease.state = 'PENDING'
                       AND lease.codex_agent_id IS NULL
                       AND lease.admission_tool_use_id IS NULL
                       AND NOT EXISTS (
                           SELECT 1 FROM job_attempts attempt WHERE attempt.lease_id = lease.id
                       )
                   )
               )",
            [now],
        )
        .map_err(|_| persistence_error())?;
    transaction
        .execute(
            "UPDATE agent_thread_instances
             SET claimed_until = NULL, claim_lease_id = NULL
             WHERE claim_lease_id IN (
                 SELECT id FROM runtime_delegation_leases
                 WHERE state = 'EXPIRED' AND release_reason = 'TTL_EXPIRED_UNDISPATCHED'
             )",
            [],
        )
        .map_err(|_| persistence_error())?;
    transaction
        .execute(
            "DELETE FROM agent_spawn_reservations
             WHERE lease_id IN (
                 SELECT id FROM runtime_delegation_leases
                 WHERE state = 'EXPIRED' AND release_reason = 'TTL_EXPIRED_UNDISPATCHED'
             )",
            [],
        )
        .map_err(|_| persistence_error())?;
    Ok(())
}

fn live_agent_type_lease_exists(
    connection: &Connection,
    workspace_scope_key: &str,
    parent_thread_id: &str,
    agent_type: &str,
) -> Result<bool, OrchestrationError> {
    connection
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM runtime_delegation_leases
                WHERE workspace_scope_key = ?1 AND parent_thread_id = ?2
                  AND agent_type = ?3 AND state IN ('PENDING', 'ACTIVE')
             )",
            params![workspace_scope_key, parent_thread_id, agent_type],
            |row| row.get::<_, bool>(0),
        )
        .map_err(|_| persistence_error())
}

fn project_is_excluded(
    connection: &Connection,
    workspace_scope_key: &str,
) -> Result<bool, OrchestrationError> {
    let mut statement = connection
        .prepare(
            "SELECT project_path FROM project_orchestration_exclusions ORDER BY normalized_path",
        )
        .map_err(|_| persistence_error())?;
    let paths = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|_| persistence_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| persistence_error())?;
    Ok(paths.iter().any(|excluded| {
        cas_scheduler::normalize_workspace_scope_key(excluded)
            .is_some_and(|excluded| workspace_is_within(workspace_scope_key, &excluded))
    }))
}

fn runtime_policy_mode(
    connection: &Connection,
) -> Result<RuntimePolicyMode, OrchestrationError> {
    connection
        .query_row(
            "SELECT EXISTS(
                    SELECT 1
                    FROM active_agent_bindings binding
                    JOIN agents agent ON agent.id = binding.agent_id AND agent.enabled = 1
                 ) OR EXISTS(
                    SELECT 1 FROM configuration_state
                    WHERE active_agent_id IS NOT NULL
                 )",
            [],
            |row| row.get::<_, bool>(0),
        )
        .map(|active| {
            if active {
                RuntimePolicyMode::Orchestration
            } else {
                RuntimePolicyMode::Default
            }
        })
        .map_err(|_| persistence_error())
}

fn runtime_failure_policy(
    connection: &Connection,
) -> Result<OrchestrationFailurePolicy, OrchestrationError> {
    let value = connection
        .query_row(
            "SELECT setting_value FROM application_settings
             WHERE setting_key = 'orchestration_failure_policy'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map_err(|_| persistence_error())?
        .flatten();
    Ok(match value.as_deref() {
        Some("PRIMARY_FALLBACK") => OrchestrationFailurePolicy::PrimaryFallback,
        _ => OrchestrationFailurePolicy::StrictStop,
    })
}

fn orchestration_error_for_policy(reason: RuntimePolicyReasonCode) -> OrchestrationErrorCode {
    match reason {
        RuntimePolicyReasonCode::DefaultMode
        | RuntimePolicyReasonCode::WorkspaceExcluded
        | RuntimePolicyReasonCode::ProjectExcluded
        | RuntimePolicyReasonCode::ConversationExcluded
        | RuntimePolicyReasonCode::DelegationNotRequired
        | RuntimePolicyReasonCode::ScopeDenied => OrchestrationErrorCode::ScopeExcluded,
        RuntimePolicyReasonCode::CapabilityUnsupported => {
            OrchestrationErrorCode::ExecutionKindUnsupported
        }
        RuntimePolicyReasonCode::RuntimeUnhealthy => OrchestrationErrorCode::RuntimeUnavailable,
        RuntimePolicyReasonCode::PermissionDenied => OrchestrationErrorCode::PermissionDenied,
        RuntimePolicyReasonCode::ScheduleUncertain
        | RuntimePolicyReasonCode::LeaseUncertain
        | RuntimePolicyReasonCode::ReceiptUncertain => OrchestrationErrorCode::RecoveryRequired,
        RuntimePolicyReasonCode::SchemaUnverified => OrchestrationErrorCode::SchemaUnverified,
        RuntimePolicyReasonCode::AdmissionAllowed => {
            OrchestrationErrorCode::InternalInvariantViolation
        }
    }
}

fn orchestration_error_for_reason(reason: HardGateReasonCode) -> OrchestrationErrorCode {
    match reason {
        HardGateReasonCode::AgentNotActive
        | HardGateReasonCode::AgentNotEnabled
        | HardGateReasonCode::ModelUnavailable
        | HardGateReasonCode::ProviderUnavailable => OrchestrationErrorCode::AgentNotExecutable,
        HardGateReasonCode::WorkspaceExcluded
        | HardGateReasonCode::ProjectExcluded
        | HardGateReasonCode::ConversationExcluded => OrchestrationErrorCode::ScopeExcluded,
        HardGateReasonCode::ExecutionKindNotDispatchable
        | HardGateReasonCode::CandidateExecutionKindNotDispatchable
        | HardGateReasonCode::CandidateExecutionKindMismatch => {
            OrchestrationErrorCode::ExecutionKindUnsupported
        }
        HardGateReasonCode::RuntimeCapabilityUnsupported
        | HardGateReasonCode::RuntimeCapabilityUnknown => {
            OrchestrationErrorCode::RuntimeUnavailable
        }
        HardGateReasonCode::AgentExecutionCapabilityUnsupported
        | HardGateReasonCode::AgentExecutionCapabilityUnknown
        | HardGateReasonCode::EventCapabilityUnsupported
        | HardGateReasonCode::EventCapabilityUnknown => OrchestrationErrorCode::SchemaUnverified,
        HardGateReasonCode::AgentTypeUnknown | HardGateReasonCode::AgentTypeLeaseConflict => {
            OrchestrationErrorCode::ConcurrencyLimitReached
        }
        HardGateReasonCode::RecoveryRequired => OrchestrationErrorCode::RecoveryRequired,
        _ => OrchestrationErrorCode::InternalInvariantViolation,
    }
}

fn current_timestamp(connection: &Connection) -> Result<String, OrchestrationError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(|_| persistence_error())
}

fn schedule_error(
    code: OrchestrationErrorCode,
    message: &str,
    job_id: Option<&str>,
) -> OrchestrationError {
    OrchestrationError {
        code,
        message: message.to_owned(),
        field_path: None,
        job_id: job_id.map(str::to_owned),
        attempt_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration_contract::{
        AttemptState, ExecutionKindPolicy, IdempotencyOutcome, OutputContract, PermissionPolicy,
        ReviewOutcome, ReviewPolicy, TASK_PACKET_SCHEMA_VERSION,
    };
    use crate::orchestration_job::OrchestrationJobReviewRequest;

    const NOW: &str = "2026-09-09T00:00:00.000Z";

    fn packet(job_id: &str, key: &str, agent_id: &str, task_scope: &str) -> TaskPacket {
        TaskPacket {
            schema_version: TASK_PACKET_SCHEMA_VERSION,
            job_id: job_id.to_owned(),
            idempotency_key: key.to_owned(),
            agent_id: agent_id.to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            workspace_scope_key: "c:/workspace".to_owned(),
            task_scope_key: task_scope.to_owned(),
            objective: "验证原子调度".to_owned(),
            allowed_scope: vec!["src-tauri/src/orchestration_job".to_owned()],
            constraints: Vec::new(),
            success_criteria: vec!["事务完整".to_owned()],
            allowed_tools: Vec::new(),
            permission_policy: PermissionPolicy::WorkspaceWrite,
            execution_kind_policy: ExecutionKindPolicy::ManagedWorkerRequired,
            context_references: Vec::new(),
            output_contract: OutputContract::StandardV1,
            review_policy: ReviewPolicy::PrimaryRequired,
        }
    }

    fn request(packet: TaskPacket, expected: RouteAction) -> AtomicScheduleRequest {
        AtomicScheduleRequest {
            task_packet: packet,
            expected_decision: expected,
            expected_candidate_thread_id: None,
            planned_execution_kind: ExecutionKind::ManagedWorker,
            admission: DispatchAdmission::allowed(),
        }
    }

    fn seed_agent(service: &OrchestrationJobService, agent_id: &str, agent_type: &str) {
        let connection = service.connection().unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, preset_id, created_at, updated_at
                 ) VALUES (
                    'provider-1', 'openai', 'OpenAI', 'PRESET', 'https://api.example/v1',
                    'RESPONSES', 'BEARER_TOKEN', 1, 'BUILT_IN', 'codex-native', ?1, ?1
                 )",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT OR IGNORE INTO models (
                    id, provider_id, model_id, display_name, enabled, source,
                    created_at, updated_at
                 ) VALUES ('model-1', 'provider-1', 'gpt-test', 'GPT Test', 1, 'PRESET', ?1, ?1)",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    orchestration_phase, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?2, 'test', '执行任务', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, ?3,
                    'EXECUTION', ?4, ?4
                 )",
                params![agent_id, format!("key-{agent_id}"), agent_type, NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, enabled, priority, source, created_at, updated_at
                 ) VALUES (?1, ?2, 'model-1', 1, 0, 'CAS', ?3, ?3)",
                params![format!("binding-{agent_id}"), agent_id, NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO active_agent_bindings (role_key, agent_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3)",
                params![agent_type, agent_id, NOW],
            )
            .unwrap();
    }

    fn seed_candidate(service: &OrchestrationJobService, agent_id: &str, thread_id: &str) {
        let connection = service.connection().unwrap();
        let fingerprint =
            load_agent_snapshot(&connection, agent_id, PermissionPolicy::WorkspaceWrite)
                .unwrap()
                .unwrap()
                .profile
                .runtime_fingerprint
                .unwrap();
        connection
            .execute(
                "INSERT INTO agent_thread_instances (
                    id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                    scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, current_context_tokens, context_window, runtime_fingerprint,
                    created_at, last_used_at, last_model_usage_at, last_observed_at,
                    task_scope_key, reuse_state, execution_kind
                 ) VALUES (
                    ?1, ?2, 'Agent', ?3, 'parent-1', 'c:/workspace', 'IDLE',
                    10, 5, 5, 20, 20, 100, ?4, ?5, ?5, ?5, ?5,
                    'task-1', 'ACTIVE', 'MANAGED_WORKER'
                 )",
                params![
                    format!("instance-{thread_id}"),
                    agent_id,
                    thread_id,
                    fingerprint,
                    NOW
                ],
            )
            .unwrap();
    }

    fn seed_revision_job(
        service: &OrchestrationJobService,
        task_packet: &TaskPacket,
        thread_id: &str,
    ) {
        seed_candidate(service, &task_packet.agent_id, thread_id);
        let canonical = task_packet.canonical_form().unwrap();
        let hash = task_packet.task_packet_hash().unwrap();
        let connection = service.connection().unwrap();
        let fingerprint: String = connection
            .query_row(
                "SELECT runtime_fingerprint FROM agent_thread_instances
                 WHERE codex_thread_id=?1",
                [thread_id],
                |row| row.get(0),
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO orchestration_jobs (
                    job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    created_at, updated_at
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'REVIEW_PENDING',?9,?9)",
                params![
                    task_packet.job_id,
                    task_packet.idempotency_key,
                    canonical,
                    hash,
                    task_packet.agent_id,
                    task_packet.parent_thread_id,
                    task_packet.workspace_scope_key,
                    task_packet.task_scope_key,
                    NOW,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_schedule_decisions (
                    id, created_at, source, agent_id, agent_name_snapshot,
                    workspace_scope_key, parent_thread_id, candidate_thread_id,
                    decision, reason_code, runtime_fingerprint, cache_hint, claimed,
                    task_scope_key, job_id
                 ) VALUES (
                    'decision-previous',?1,'TEST',?2,'Agent',?3,?4,?5,
                    'SPAWN','TEST',?6,'UNKNOWN',0,?7,?8
                 )",
                params![
                    NOW,
                    task_packet.agent_id,
                    task_packet.workspace_scope_key,
                    task_packet.parent_thread_id,
                    thread_id,
                    fingerprint,
                    task_packet.task_scope_key,
                    task_packet.job_id,
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id,
                    codex_agent_id, workspace_scope_key, task_scope_key,
                    schedule_decision_id, state, expires_at, agent_type
                 ) VALUES (
                    'lease-previous',?1,?1,?2,?3,?4,?5,?6,
                    'decision-previous','ACTIVE','2099-01-01T00:00:00.000Z','executor'
                 )",
                params![
                    NOW,
                    task_packet.agent_id,
                    task_packet.parent_thread_id,
                    thread_id,
                    task_packet.workspace_scope_key,
                    task_packet.task_scope_key,
                ],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE agent_thread_instances
                 SET reuse_state='HELD_FOR_REVIEW', claim_lease_id='lease-previous'
                 WHERE codex_thread_id=?1",
                [thread_id],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO job_attempts (
                    attempt_id, job_id, attempt_no, schedule_decision_id, lease_id,
                    route_action, planned_execution_kind, execution_kind,
                    thread_instance_id, codex_turn_id, state, created_at, updated_at,
                    dispatch_recorded_at, accepted_at, terminal_at
                 ) VALUES (
                    'attempt-previous',?1,1,'decision-previous','lease-previous',
                    'SPAWN','MANAGED_WORKER','MANAGED_WORKER',
                    (SELECT id FROM agent_thread_instances WHERE codex_thread_id=?2),
                    'turn-previous','SUCCEEDED',?3,?3,?3,?3,?3
                 )",
                params![task_packet.job_id, thread_id, NOW],
            )
            .unwrap();
        for (receipt_id, stage, execution_kind, source, reference, codex_thread, codex_turn) in [
            (
                "receipt-dispatch",
                "DISPATCH_RECORDED",
                None,
                "CAS_TRANSACTION",
                "decision-previous",
                None,
                None,
            ),
            (
                "receipt-accepted",
                "TURN_ACCEPTED",
                Some("MANAGED_WORKER"),
                "APP_SERVER_RESPONSE",
                "accepted-previous",
                Some(thread_id),
                Some("turn-previous"),
            ),
            (
                "receipt-result",
                "RESULT_OBSERVED",
                Some("MANAGED_WORKER"),
                "RUNTIME_EVENT",
                "result-previous",
                Some(thread_id),
                Some("turn-previous"),
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO delivery_receipts (
                        receipt_id, job_id, attempt_id, stage, execution_kind,
                        evidence_source, evidence_ref, parent_thread_id,
                        codex_thread_id, codex_turn_id, schema_profile,
                        evidence_at, created_at
                     ) VALUES (?1,?2,'attempt-previous',?3,?4,?5,?6,?7,?8,?9,'TEST',?10,?10)",
                    params![
                        receipt_id,
                        task_packet.job_id,
                        stage,
                        execution_kind,
                        source,
                        reference,
                        task_packet.parent_thread_id,
                        codex_thread,
                        codex_turn,
                        NOW,
                    ],
                )
                .unwrap();
        }
        drop(connection);
        service
            .review(OrchestrationJobReviewRequest {
                job_id: task_packet.job_id.clone(),
                attempt_id: "attempt-previous".to_owned(),
                decision: ReviewOutcome::RevisionRequired,
                reviewer_thread_id: task_packet.parent_thread_id.clone(),
                reason: "补充修订".to_owned(),
                evidence_refs: vec![
                    "child-result:previous".to_owned(),
                    "test:previous".to_owned(),
                ],
            })
            .unwrap();
    }

    fn ready(outcome: AtomicScheduleOutcome) -> DispatchPermit {
        match outcome {
            AtomicScheduleOutcome::Ready(permit) => permit,
            other => panic!("expected ready, got {other:?}"),
        }
    }

    fn table_count(service: &OrchestrationJobService, table: &str) -> i64 {
        service
            .connection()
            .unwrap()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn assert_no_delegation_occupancy(service: &OrchestrationJobService) {
        assert_eq!(table_count(service, "job_attempts"), 0);
        assert_eq!(table_count(service, "runtime_delegation_leases"), 0);
        assert_eq!(table_count(service, "agent_spawn_reservations"), 0);
    }

    #[test]
    fn spawn_commits_job_decision_attempt_lease_and_reservation_before_permit() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");

        let permit = ready(
            service
                .schedule_atomic(request(
                    packet("job-1", "atomic-spawn", "agent-1", "task-1"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );

        assert_eq!(permit.job().state, JobState::Claimed);
        assert_eq!(permit.attempt().state, AttemptState::Planned);
        assert_eq!(permit.attempt().route_action, RouteAction::Spawn);
        assert_eq!(permit.attempt().execution_kind, None);
        assert_eq!(permit.agent_type(), "executor");
        assert_eq!(permit.candidate_thread_id(), None);
        let connection = service.connection().unwrap();
        let lease: (String, String, String) = connection
            .query_row(
                "SELECT state, agent_type, schedule_decision_id
                 FROM runtime_delegation_leases WHERE id = ?1",
                [&permit.attempt().lease_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(lease.0, "PENDING");
        assert_eq!(lease.1, "executor");
        assert_eq!(lease.2, permit.attempt().schedule_decision_id);
        let reservation_owner: (String, String) = connection
            .query_row(
                "SELECT lease_id, job_id FROM agent_spawn_reservations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(reservation_owner.0, permit.attempt().lease_id);
        assert_eq!(reservation_owner.1, "job-1");
        drop(connection);
        assert_eq!(table_count(&service, "agent_schedule_decisions"), 2);
        assert_eq!(table_count(&service, "job_attempts"), 1);
    }

    #[test]
    fn reuse_claim_is_owned_by_pending_lease_and_has_no_reservation() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        seed_candidate(&service, "agent-1", "thread-1");
        let mut request = request(
            packet("job-1", "atomic-reuse", "agent-1", "task-1"),
            RouteAction::Reuse,
        );
        request.expected_candidate_thread_id = Some("thread-1".to_owned());

        let permit = ready(service.schedule_atomic(request).unwrap());

        assert_eq!(permit.attempt().route_action, RouteAction::Reuse);
        assert_eq!(permit.candidate_thread_id(), Some("thread-1"));
        let owner: String = service
            .connection()
            .unwrap()
            .query_row(
                "SELECT claim_lease_id FROM agent_thread_instances WHERE codex_thread_id = 'thread-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(owner, permit.attempt().lease_id);
        assert_eq!(table_count(&service, "agent_spawn_reservations"), 0);
        let claimed: Vec<i64> = service
            .connection()
            .unwrap()
            .prepare(
                "SELECT claimed FROM agent_schedule_decisions WHERE job_id = 'job-1' ORDER BY rowid",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(claimed, vec![0, 1]);
    }

    #[test]
    fn revision_reuses_only_the_exact_healthy_thread_and_links_attempts() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let task_packet = packet("job-1", "revision-reuse", "agent-1", "task-1");
        seed_revision_job(&service, &task_packet, "thread-1");
        let immutable_before: (String, String, String, i64, i64) = service
            .connection()
            .unwrap()
            .query_row(
                "SELECT attempt.state, attempt.execution_kind, review.decision,
                        (SELECT COUNT(*) FROM delivery_receipts WHERE attempt_id=attempt.attempt_id),
                        (SELECT COUNT(*) FROM review_decisions WHERE attempt_id=attempt.attempt_id)
                 FROM job_attempts attempt
                 JOIN review_decisions review ON review.attempt_id=attempt.attempt_id
                 WHERE attempt.attempt_id='attempt-previous'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        let mut revision_request = request(task_packet, RouteAction::Reuse);
        revision_request.expected_candidate_thread_id = Some("thread-1".to_owned());

        let permit = ready(service.schedule_atomic(revision_request).unwrap());

        assert_eq!(permit.reason_code(), "REVISION_EXACT_THREAD_HEALTHY");
        assert_eq!(permit.candidate_thread_id(), Some("thread-1"));
        assert_eq!(permit.attempt().attempt_no, 2);
        assert_eq!(
            permit.attempt().previous_attempt_id.as_deref(),
            Some("attempt-previous")
        );
        assert_eq!(permit.attempt().route_action, RouteAction::Reuse);
        assert_eq!(
            permit.attempt().planned_execution_kind,
            ExecutionKind::ManagedWorker
        );
        let immutable_after: (String, String, String, i64, i64) = service
            .connection()
            .unwrap()
            .query_row(
                "SELECT attempt.state, attempt.execution_kind, review.decision,
                        (SELECT COUNT(*) FROM delivery_receipts WHERE attempt_id=attempt.attempt_id),
                        (SELECT COUNT(*) FROM review_decisions WHERE attempt_id=attempt.attempt_id)
                 FROM job_attempts attempt
                 JOIN review_decisions review ON review.attempt_id=attempt.attempt_id
                 WHERE attempt.attempt_id='attempt-previous'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .unwrap();
        assert_eq!(immutable_after, immutable_before);
        assert_eq!(table_count(&service, "job_attempts"), 2);
    }

    #[test]
    fn unhealthy_revision_thread_spawns_replacement_but_keeps_attempt_chain() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let task_packet = packet("job-1", "revision-spawn", "agent-1", "task-1");
        seed_revision_job(&service, &task_packet, "thread-1");
        service
            .connection()
            .unwrap()
            .execute(
                "UPDATE agent_thread_instances
                 SET current_context_tokens=100, context_window=100
                 WHERE codex_thread_id='thread-1'",
                [],
            )
            .unwrap();

        let permit = ready(
            service
                .schedule_atomic(request(task_packet, RouteAction::Spawn))
                .unwrap(),
        );

        assert_eq!(permit.attempt().route_action, RouteAction::Spawn);
        assert_eq!(permit.attempt().attempt_no, 2);
        assert_eq!(
            permit.attempt().previous_attempt_id.as_deref(),
            Some("attempt-previous")
        );
        assert_eq!(permit.attempt().thread_instance_id, None);
        assert_eq!(permit.candidate_thread_id(), None);
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row(
                    "SELECT reuse_state FROM agent_thread_instances WHERE codex_thread_id='thread-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "HELD_FOR_REVIEW"
        );
    }

    #[test]
    fn unrelated_job_cannot_claim_a_held_revision_thread() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let revision_packet = packet("job-1", "held-owner", "agent-1", "task-1");
        seed_revision_job(&service, &revision_packet, "thread-1");

        let permit = ready(
            service
                .schedule_atomic(request(
                    packet("job-2", "unrelated", "agent-1", "task-1"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );

        assert_eq!(permit.attempt().route_action, RouteAction::Spawn);
        assert_eq!(permit.candidate_thread_id(), None);
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row(
                    "SELECT claim_lease_id IS NULL FROM agent_thread_instances
                     WHERE codex_thread_id='thread-1'",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
            true
        );
    }

    #[test]
    fn repeated_revision_schedule_creates_only_one_next_attempt() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let task_packet = packet("job-1", "revision-idempotent", "agent-1", "task-1");
        seed_revision_job(&service, &task_packet, "thread-1");
        let mut revision_request = request(task_packet, RouteAction::Reuse);
        revision_request.expected_candidate_thread_id = Some("thread-1".to_owned());

        let first = ready(service.schedule_atomic(revision_request.clone()).unwrap());
        let second = service.schedule_atomic(revision_request).unwrap();

        assert_eq!(first.attempt().attempt_no, 2);
        match second {
            AtomicScheduleOutcome::Existing(existing) => {
                assert_eq!(
                    existing.current_attempt.unwrap().attempt_id,
                    first.attempt().attempt_id
                );
            }
            other => panic!("expected existing, got {other:?}"),
        }
        assert_eq!(table_count(&service, "job_attempts"), 2);
    }

    #[test]
    fn waiting_revision_can_retry_after_known_occupancy_is_released() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let task_packet = packet("job-1", "revision-wait", "agent-1", "task-1");
        seed_revision_job(&service, &task_packet, "thread-1");
        service
            .connection()
            .unwrap()
            .execute_batch(&format!(
                r#"INSERT INTO agent_schedule_decisions (
                    id,created_at,source,agent_id,agent_name_snapshot,workspace_scope_key,
                    parent_thread_id,decision,reason_code,cache_hint,claimed,task_scope_key
                 ) VALUES (
                    'conflict-decision','{NOW}','TEST','agent-1','Agent','c:/workspace',
                    'parent-1','SPAWN','TEST','UNKNOWN',0,'other-task'
                 );
                 INSERT INTO runtime_delegation_leases (
                    id,created_at,updated_at,agent_id,parent_thread_id,codex_agent_id,
                    workspace_scope_key,task_scope_key,schedule_decision_id,state,
                    expires_at,agent_type
                 ) VALUES (
                    'conflict-lease','{NOW}','{NOW}','agent-1','parent-1','other-thread',
                    'c:/workspace','other-task','conflict-decision','ACTIVE',
                    '2099-01-01T00:00:00.000Z','executor'
                 );"#,
            ))
            .unwrap();
        let mut revision_request = request(task_packet, RouteAction::Reuse);
        revision_request.expected_candidate_thread_id = Some("thread-1".to_owned());

        match service.schedule_atomic(revision_request.clone()).unwrap() {
            AtomicScheduleOutcome::Waiting(stop) => {
                assert_eq!(stop.reason_code, "AGENT_TYPE_LEASE_CONFLICT");
            }
            other => panic!("expected waiting, got {other:?}"),
        }
        assert_eq!(table_count(&service, "job_attempts"), 1);
        service
            .connection()
            .unwrap()
            .execute(
                "UPDATE runtime_delegation_leases
                 SET state='RELEASED', released_at=?1, updated_at=?1,
                     release_reason='TEST_RELEASE'
                 WHERE id='conflict-lease'",
                [NOW],
            )
            .unwrap();

        let permit = ready(service.schedule_atomic(revision_request).unwrap());
        assert_eq!(permit.attempt().attempt_no, 2);
        assert_eq!(permit.attempt().route_action, RouteAction::Reuse);
        assert_eq!(permit.candidate_thread_id(), Some("thread-1"));
    }

    #[test]
    fn concurrent_revision_schedule_creates_one_next_attempt() {
        let root = std::env::temp_dir().join(format!("cas-revision-race-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        let seed = OrchestrationJobService::open(&database_path).unwrap();
        seed_agent(&seed, "agent-1", "executor");
        let task_packet = packet("job-1", "revision-race", "agent-1", "task-1");
        seed_revision_job(&seed, &task_packet, "thread-1");
        drop(seed);

        let first = OrchestrationJobService::open(&database_path).unwrap();
        let second = OrchestrationJobService::open(&database_path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let first_barrier = std::sync::Arc::clone(&barrier);
        let first_packet = task_packet.clone();
        let first_handle = std::thread::spawn(move || {
            let mut request = request(first_packet, RouteAction::Reuse);
            request.expected_candidate_thread_id = Some("thread-1".to_owned());
            first_barrier.wait();
            first.schedule_atomic(request)
        });
        let second_handle = std::thread::spawn(move || {
            let mut request = request(task_packet, RouteAction::Reuse);
            request.expected_candidate_thread_id = Some("thread-1".to_owned());
            barrier.wait();
            second.schedule_atomic(request)
        });

        let outcomes = [first_handle.join().unwrap(), second_handle.join().unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(AtomicScheduleOutcome::Ready(_))))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(AtomicScheduleOutcome::Existing(_))))
                .count(),
            1
        );
        let verifier = OrchestrationJobService::open(&database_path).unwrap();
        assert_eq!(table_count(&verifier, "job_attempts"), 2);
        assert_eq!(
            verifier
                .connection()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM job_attempts
                     WHERE job_id='job-1' AND state IN (
                        'PLANNED','DISPATCHING','ACCEPTED','RUNNING','UNCERTAIN'
                     )",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(verifier);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reviewer_agent_cannot_be_dispatched_without_assignment() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "reviewer-agent", "reviewer");
        service
            .connection()
            .unwrap()
            .execute(
                "UPDATE agents
                 SET agent_type='PRESET', source='CAS', role_key='reviewer',
                     orchestration_phase='REVIEW', sandbox_policy='READ_ONLY'
                 WHERE id='reviewer-agent'",
                [],
            )
            .unwrap();
        let mut task_packet = packet(
            "unassigned-reviewer-job",
            "unassigned-reviewer",
            "reviewer-agent",
            "task-1",
        );
        task_packet.permission_policy = PermissionPolicy::ReadOnly;

        let error = service
            .schedule_atomic(request(task_packet, RouteAction::Spawn))
            .unwrap_err();

        assert_eq!(error.code, OrchestrationErrorCode::AgentNotExecutable);
        assert_eq!(table_count(&service, "orchestration_jobs"), 0);
        assert_eq!(table_count(&service, "runtime_delegation_leases"), 0);
    }

    #[test]
    fn invalidated_reuse_claim_recomputes_to_spawn_and_keeps_decision_chain() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        seed_candidate(&service, "agent-1", "thread-1");
        service
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER invalidate_reuse_claim
                 BEFORE UPDATE OF claim_lease_id ON agent_thread_instances
                 WHEN OLD.codex_thread_id = 'thread-1'
                 BEGIN SELECT RAISE(IGNORE); END;",
            )
            .unwrap();
        let mut schedule_request = request(
            packet("job-1", "claim-fallback", "agent-1", "task-1"),
            RouteAction::Reuse,
        );
        schedule_request.expected_candidate_thread_id = Some("thread-1".to_owned());

        let permit = ready(service.schedule_atomic(schedule_request).unwrap());

        assert_eq!(permit.attempt().route_action, RouteAction::Spawn);
        assert_eq!(permit.candidate_thread_id(), None);
        let connection = service.connection().unwrap();
        let decisions: Vec<(String, String, String, i64, Option<String>)> = connection
            .prepare(
                "SELECT id, source, decision, claimed, supersedes_decision_id
                 FROM agent_schedule_decisions WHERE job_id = 'job-1'
                 ORDER BY CASE source
                    WHEN 'ORCHESTRATION_EXECUTE_RECOMMENDATION' THEN 0 ELSE 1 END",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[0].1, EXECUTION_RECOMMENDATION_SOURCE);
        assert_eq!(decisions[0].2, "REUSE");
        assert_eq!(decisions[0].3, 0);
        assert_eq!(decisions[0].4, None);
        assert_eq!(decisions[1].1, EXECUTION_FINAL_SOURCE);
        assert_eq!(decisions[1].2, "SPAWN");
        assert_eq!(decisions[1].3, 0);
        assert_eq!(decisions[1].4.as_deref(), Some(decisions[0].0.as_str()));
        assert_eq!(permit.attempt().schedule_decision_id, decisions[1].0);
        drop(connection);
        assert_eq!(table_count(&service, "agent_thread_instances"), 1);
        assert_eq!(table_count(&service, "agent_spawn_reservations"), 1);
    }

    #[test]
    fn invalidated_reuse_claim_still_honors_hard_gate_and_reservation() {
        let hard_gate = OrchestrationJobService::in_memory();
        seed_agent(&hard_gate, "agent-1", "executor");
        seed_candidate(&hard_gate, "agent-1", "thread-1");
        hard_gate
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER invalidate_claim_and_disable_agent
                 BEFORE UPDATE OF claim_lease_id ON agent_thread_instances
                 BEGIN
                    UPDATE agents SET enabled = 0 WHERE id = 'agent-1';
                    SELECT RAISE(IGNORE);
                 END;",
            )
            .unwrap();
        let mut hard_gate_request = request(
            packet("job-1", "claim-hard-gate", "agent-1", "task-1"),
            RouteAction::Reuse,
        );
        hard_gate_request.expected_candidate_thread_id = Some("thread-1".to_owned());
        match hard_gate.schedule_atomic(hard_gate_request).unwrap() {
            AtomicScheduleOutcome::Blocked(stop) => {
                assert_eq!(stop.reason_code, "AGENT_NOT_ENABLED");
            }
            other => panic!("expected blocked, got {other:?}"),
        }
        assert_eq!(table_count(&hard_gate, "agent_schedule_decisions"), 2);
        assert_eq!(table_count(&hard_gate, "job_attempts"), 0);
        assert_eq!(table_count(&hard_gate, "runtime_delegation_leases"), 0);

        let reservation = OrchestrationJobService::in_memory();
        seed_agent(&reservation, "agent-1", "executor");
        seed_candidate(&reservation, "agent-1", "thread-1");
        reservation
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER invalidate_claim_and_reserve_spawn
                 BEFORE UPDATE OF claim_lease_id ON agent_thread_instances
                 BEGIN
                    INSERT INTO agent_spawn_reservations (
                        agent_id, parent_thread_id, workspace_scope_key, task_scope_key,
                        reserved_until, created_at, lease_id, job_id
                    ) VALUES (
                        'agent-1', 'parent-1', 'c:/workspace', 'task-1',
                        '2099-01-01T00:00:00.000Z', '2026-09-09T00:00:00.000Z', NULL, NULL
                    );
                    SELECT RAISE(IGNORE);
                 END;",
            )
            .unwrap();
        let mut reservation_request = request(
            packet("job-1", "claim-reservation", "agent-1", "task-1"),
            RouteAction::Reuse,
        );
        reservation_request.expected_candidate_thread_id = Some("thread-1".to_owned());
        match reservation.schedule_atomic(reservation_request).unwrap() {
            AtomicScheduleOutcome::Waiting(stop) => {
                assert_eq!(stop.reason_code, "SPAWN_RESERVED");
            }
            other => panic!("expected waiting, got {other:?}"),
        }
        assert_eq!(table_count(&reservation, "agent_schedule_decisions"), 2);
        assert_eq!(table_count(&reservation, "job_attempts"), 0);
        assert_eq!(table_count(&reservation, "runtime_delegation_leases"), 0);
        assert_eq!(table_count(&reservation, "agent_spawn_reservations"), 1);
    }

    #[test]
    fn stale_expected_values_roll_back_every_write() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let error = service
            .schedule_atomic(request(
                packet("job-1", "stale-decision", "agent-1", "task-1"),
                RouteAction::Reuse,
            ))
            .unwrap_err();
        assert_eq!(error.code, OrchestrationErrorCode::StaleExpectedDecision);
        assert_eq!(table_count(&service, "orchestration_jobs"), 0);

        seed_candidate(&service, "agent-1", "thread-1");
        let mut stale_candidate = request(
            packet("job-2", "stale-candidate", "agent-1", "task-1"),
            RouteAction::Reuse,
        );
        stale_candidate.expected_candidate_thread_id = Some("thread-old".to_owned());
        let error = service.schedule_atomic(stale_candidate).unwrap_err();
        assert_eq!(error.code, OrchestrationErrorCode::StaleExpectedCandidate);
        for table in [
            "orchestration_jobs",
            "agent_schedule_decisions",
            "runtime_delegation_leases",
            "job_attempts",
        ] {
            assert_eq!(table_count(&service, table), 0, "{table}");
        }
    }

    #[test]
    fn persistence_failure_rolls_back_all_audit_and_occupancy() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        service
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_pending_lease
                 BEFORE INSERT ON runtime_delegation_leases
                 BEGIN SELECT RAISE(ABORT, 'injected lease failure'); END;",
            )
            .unwrap();

        let error = service
            .schedule_atomic(request(
                packet("job-1", "rollback", "agent-1", "task-1"),
                RouteAction::Spawn,
            ))
            .unwrap_err();
        assert_eq!(error.code, OrchestrationErrorCode::PersistenceError);
        for table in [
            "orchestration_jobs",
            "agent_schedule_decisions",
            "runtime_delegation_leases",
            "job_attempts",
            "agent_spawn_reservations",
        ] {
            assert_eq!(table_count(&service, table), 0, "{table}");
        }
    }

    #[test]
    fn retry_returns_existing_job_and_authorization_is_single_use() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let schedule_request = request(
            packet("job-1", "retry", "agent-1", "task-1"),
            RouteAction::Spawn,
        );
        let permit = ready(service.schedule_atomic(schedule_request.clone()).unwrap());
        match service.schedule_atomic(schedule_request).unwrap() {
            AtomicScheduleOutcome::Existing(existing) => {
                assert_eq!(existing.outcome, IdempotencyOutcome::ExistingNotDispatched);
                assert_eq!(
                    existing.current_attempt.unwrap().attempt_id,
                    permit.attempt().attempt_id
                );
            }
            other => panic!("expected existing, got {other:?}"),
        }
        let dispatching = service.authorize_dispatch(&permit).unwrap();
        assert_eq!(dispatching.state, AttemptState::Dispatching);
        assert_eq!(table_count(&service, "delivery_receipts"), 1);
        assert_eq!(
            service
                .connection()
                .unwrap()
                .query_row(
                    "SELECT stage FROM delivery_receipts WHERE attempt_id = ?1",
                    [&permit.attempt().attempt_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "DISPATCH_RECORDED"
        );
        assert_eq!(
            service.authorize_dispatch(&permit).unwrap_err().code,
            OrchestrationErrorCode::AttemptNotCurrent
        );
        assert_eq!(table_count(&service, "job_attempts"), 1);
    }

    #[test]
    fn dispatch_receipt_failure_rolls_back_attempt_and_job_state_together() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let permit = ready(
            service
                .schedule_atomic(request(
                    packet("job-1", "receipt-rollback", "agent-1", "task-1"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );
        service
            .connection()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER fail_dispatch_receipt
                 BEFORE INSERT ON delivery_receipts
                 BEGIN
                    SELECT RAISE(ABORT, 'injected receipt failure');
                 END;",
            )
            .unwrap();

        assert_eq!(
            service.authorize_dispatch(&permit).unwrap_err().code,
            OrchestrationErrorCode::PersistenceError
        );
        let connection = service.connection().unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM job_attempts WHERE attempt_id = ?1",
                    [&permit.attempt().attempt_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "PLANNED"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM orchestration_jobs WHERE job_id = ?1",
                    [&permit.job().job_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "CLAIMED"
        );
        drop(connection);
        assert_eq!(table_count(&service, "delivery_receipts"), 0);
    }

    #[test]
    fn same_agent_type_waits_while_different_type_can_run() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let first = ready(
            service
                .schedule_atomic(request(
                    packet("job-1", "type-one", "agent-1", "task-1"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );
        assert_eq!(first.agent_type(), "executor");

        match service
            .schedule_atomic(request(
                packet("job-2", "type-two", "agent-1", "task-2"),
                RouteAction::Spawn,
            ))
            .unwrap()
        {
            AtomicScheduleOutcome::Waiting(stop) => {
                assert_eq!(stop.job.state, JobState::Waiting);
                assert_eq!(
                    stop.error_code,
                    OrchestrationErrorCode::ConcurrencyLimitReached
                );
                assert_eq!(stop.reason_code, "AGENT_TYPE_LEASE_CONFLICT");
            }
            other => panic!("expected waiting, got {other:?}"),
        }

        seed_agent(&service, "agent-2", "tester");
        let other = ready(
            service
                .schedule_atomic(request(
                    packet("job-3", "type-three", "agent-2", "task-3"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );
        assert_eq!(other.agent_type(), "tester");
        assert_eq!(table_count(&service, "runtime_delegation_leases"), 2);
    }

    #[test]
    fn concurrent_connections_grant_only_one_same_type_occupancy() {
        let root = std::env::temp_dir().join(format!("cas-atomic-race-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        let seed = OrchestrationJobService::open(&database_path).unwrap();
        seed_agent(&seed, "agent-1", "executor");
        drop(seed);

        let first = OrchestrationJobService::open(&database_path).unwrap();
        let second = OrchestrationJobService::open(&database_path).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let first_barrier = std::sync::Arc::clone(&barrier);
        let first_handle = std::thread::spawn(move || {
            first_barrier.wait();
            first.schedule_atomic(request(
                packet("job-1", "concurrent-1", "agent-1", "task-1"),
                RouteAction::Spawn,
            ))
        });
        let second_handle = std::thread::spawn(move || {
            barrier.wait();
            second.schedule_atomic(request(
                packet("job-2", "concurrent-2", "agent-1", "task-2"),
                RouteAction::Spawn,
            ))
        });

        let outcomes = [first_handle.join().unwrap(), second_handle.join().unwrap()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(AtomicScheduleOutcome::Ready(_))))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(AtomicScheduleOutcome::Waiting(_))))
                .count(),
            1
        );

        let verifier = OrchestrationJobService::open(&database_path).unwrap();
        assert_eq!(table_count(&verifier, "runtime_delegation_leases"), 1);
        assert_eq!(table_count(&verifier, "agent_spawn_reservations"), 1);
        assert_eq!(table_count(&verifier, "job_attempts"), 1);
        drop(verifier);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn expired_unknown_dispatch_keeps_slot_but_proven_undispatched_releases_it() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let first = ready(
            service
                .schedule_atomic(request(
                    packet("job-1", "unknown-owner", "agent-1", "task-1"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );
        service.authorize_dispatch(&first).unwrap();
        service
            .connection()
            .unwrap()
            .execute(
                "UPDATE runtime_delegation_leases SET expires_at = '2020-01-01T00:00:00Z'
                 WHERE id = ?1",
                [&first.attempt().lease_id],
            )
            .unwrap();
        assert!(matches!(
            service
                .schedule_atomic(request(
                    packet("job-2", "unknown-waiter", "agent-1", "task-2"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
            AtomicScheduleOutcome::Waiting(_)
        ));
        assert_eq!(table_count(&service, "runtime_delegation_leases"), 1);

        let fresh = OrchestrationJobService::in_memory();
        seed_agent(&fresh, "agent-1", "executor");
        let undispatched = ready(
            fresh
                .schedule_atomic(request(
                    packet("job-1", "safe-owner", "agent-1", "task-1"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );
        fresh
            .connection()
            .unwrap()
            .execute(
                "UPDATE runtime_delegation_leases SET expires_at = '2020-01-01T00:00:00Z'
                 WHERE id = ?1",
                [&undispatched.attempt().lease_id],
            )
            .unwrap();
        let replacement = ready(
            fresh
                .schedule_atomic(request(
                    packet("job-2", "safe-replacement", "agent-1", "task-2"),
                    RouteAction::Spawn,
                ))
                .unwrap(),
        );
        assert_ne!(
            replacement.attempt().lease_id,
            undispatched.attempt().lease_id
        );
        let states: (i64, i64) = fresh
            .connection()
            .unwrap()
            .query_row(
                "SELECT SUM(state = 'EXPIRED'), SUM(state = 'PENDING')
                 FROM runtime_delegation_leases",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(states, (1, 1));
    }

    #[test]
    fn hard_policy_failures_block_without_attempt_or_lease() {
        let service = OrchestrationJobService::in_memory();
        seed_agent(&service, "agent-1", "executor");
        let mut denied = request(
            packet("job-1", "permission", "agent-1", "task-1"),
            RouteAction::Spawn,
        );
        denied.admission.permission_allowed = false;
        match service.schedule_atomic(denied).unwrap() {
            AtomicScheduleOutcome::Blocked(stop) => {
                assert_eq!(stop.job.state, JobState::Blocked);
                assert_eq!(stop.admission_decision, AdmissionDecision::Deny);
                assert_eq!(stop.reason_code, "PERMISSION_DENIED");
                assert_eq!(stop.error_code, OrchestrationErrorCode::PermissionDenied);
            }
            other => panic!("expected blocked, got {other:?}"),
        }
        assert_no_delegation_occupancy(&service);
    }

    #[test]
    fn runtime_policy_default_and_project_exclusion_never_reserve() {
        let default = OrchestrationJobService::in_memory();
        seed_agent(&default, "agent-1", "executor");
        default
            .connection()
            .unwrap()
            .execute("DELETE FROM active_agent_bindings", [])
            .unwrap();
        match default
            .schedule_atomic(request(
                packet("job-default", "policy-default", "agent-1", "task-1"),
                RouteAction::Spawn,
            ))
            .unwrap()
        {
            AtomicScheduleOutcome::Blocked(stop) => {
                assert_eq!(stop.admission_decision, AdmissionDecision::Allow);
                assert_eq!(stop.reason_code, "DEFAULT_MODE");
                assert_eq!(stop.error_code, OrchestrationErrorCode::ScopeExcluded);
            }
            other => panic!("expected default policy stop, got {other:?}"),
        }
        assert_no_delegation_occupancy(&default);

        let excluded = OrchestrationJobService::in_memory();
        seed_agent(&excluded, "agent-1", "executor");
        excluded
            .connection()
            .unwrap()
            .execute(
                "INSERT INTO project_orchestration_exclusions (
                    id, project_path, normalized_path, config_existed,
                    baseline_json, created_at, updated_at
                 ) VALUES ('excluded-1', 'C:/workspace', 'c:/workspace', 0, '{}', ?1, ?1)",
                [NOW],
            )
            .unwrap();
        match excluded
            .schedule_atomic(request(
                packet("job-excluded", "policy-excluded", "agent-1", "task-1"),
                RouteAction::Spawn,
            ))
            .unwrap()
        {
            AtomicScheduleOutcome::Blocked(stop) => {
                assert_eq!(stop.admission_decision, AdmissionDecision::Allow);
                assert_eq!(stop.reason_code, "PROJECT_EXCLUDED");
                assert_eq!(stop.error_code, OrchestrationErrorCode::ScopeExcluded);
            }
            other => panic!("expected exclusion policy stop, got {other:?}"),
        }
        assert_no_delegation_occupancy(&excluded);
    }

    #[test]
    fn runtime_policy_strict_and_fallback_stop_before_reservation() {
        for (name, setting, decision) in [
            ("strict", None, AdmissionDecision::Deny),
            (
                "fallback",
                Some("PRIMARY_FALLBACK"),
                AdmissionDecision::Warn,
            ),
        ] {
            let service = OrchestrationJobService::in_memory();
            seed_agent(&service, "agent-1", "executor");
            if let Some(setting) = setting {
                service
                    .connection()
                    .unwrap()
                    .execute(
                        "INSERT INTO application_settings (
                            setting_key, setting_value, value_type, source, updated_at
                         ) VALUES (
                            'orchestration_failure_policy', ?1, 'STRING', 'USER', ?2
                         )",
                        params![setting, NOW],
                    )
                    .unwrap();
            }
            let mut request = request(
                packet(
                    &format!("job-{name}"),
                    &format!("policy-{name}"),
                    "agent-1",
                    "task-1",
                ),
                RouteAction::Spawn,
            );
            request.admission.runtime_healthy = false;
            match service.schedule_atomic(request).unwrap() {
                AtomicScheduleOutcome::Blocked(stop) => {
                    assert_eq!(stop.admission_decision, decision, "{name}");
                    assert_eq!(stop.reason_code, "RUNTIME_UNHEALTHY", "{name}");
                    assert_eq!(
                        stop.error_code,
                        OrchestrationErrorCode::RuntimeUnavailable,
                        "{name}"
                    );
                }
                other => panic!("expected {name} policy stop, got {other:?}"),
            }
            assert_no_delegation_occupancy(&service);
        }
    }
}
