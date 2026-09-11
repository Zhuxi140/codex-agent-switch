use cas_scheduler::hard_gates::{
    AgentExecutionInput, CacheRequirement, CacheState, CandidateHardGateInput, CandidateScope,
    Capability, ContextHealth, ContextPressure, ExclusionInput, ExecutionCapabilityInput,
    ExecutionKind, Fallback, HardGateContext, HardGateDisposition, HardGateReasonCode,
    RequiredIdentity, RequiredScope, ThreadState, evaluate_hard_gates, evaluate_spawn_hard_gates,
};
use cas_scheduler::scoring::{
    CacheEvidenceSource, ReuseSelection, ReuseStrategy, ScoredReuseCandidate, ScoringCandidate,
    ScoringPolicy, select_reuse,
};

fn context() -> HardGateContext {
    HardGateContext {
        agent: AgentExecutionInput {
            active: true,
            enabled: true,
            model_available: true,
            provider_available: true,
        },
        exclusions: ExclusionInput {
            workspace_excluded: false,
            project_excluded: false,
            conversation_excluded: false,
        },
        execution: ExecutionCapabilityInput {
            selected_kind: ExecutionKind::ManagedWorker,
            runtime: Capability::Supported,
            agent_execution: Capability::Supported,
            event: Capability::Supported,
        },
        required_identity: RequiredIdentity {
            agent_id: "agent-1".to_owned(),
            runtime_fingerprint: Some("fp-1".to_owned()),
        },
        required_scope: RequiredScope {
            workspace_scope_key: "workspace-1".to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            task_scope_key: "task-1".to_owned(),
        },
        cache_requirement: CacheRequirement::NotRequired,
        requested_agent_type: "worker".to_owned(),
        conflicting_agent_type_lease: false,
    }
}

fn candidate() -> CandidateHardGateInput {
    CandidateHardGateInput {
        thread_id: "thread-1".to_owned(),
        agent_id: "agent-1".to_owned(),
        runtime_fingerprint: Some("fp-1".to_owned()),
        execution_kind: ExecutionKind::ManagedWorker,
        scope: CandidateScope {
            workspace_scope_key: "workspace-1".to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            task_scope_key: "task-1".to_owned(),
        },
        thread_state: ThreadState::Idle,
        context_health: ContextHealth::Healthy,
        active_claim: false,
        active_lease: false,
        recovery_required: false,
        review_pending: false,
        context_pressure: ContextPressure::Known {
            observed_percent: 20,
            limit_percent: 80,
        },
        cache_state: CacheState::WithinRequiredWindow,
    }
}

fn assert_reason(
    result: cas_scheduler::hard_gates::HardGateEvaluation,
    disposition: HardGateDisposition,
    reason: HardGateReasonCode,
) {
    assert_eq!(result.disposition, disposition);
    assert_eq!(result.reason_code, Some(reason));
}

#[test]
fn hard_gate_matrix_has_stable_first_reason_and_fallback() {
    type Case = (
        &'static str,
        fn(&mut HardGateContext, &mut CandidateHardGateInput),
        HardGateDisposition,
        HardGateReasonCode,
        &'static str,
    );
    let cases: &[Case] = &[
        (
            "agent inactive",
            |c, _| c.agent.active = false,
            HardGateDisposition::Blocked {
                fallback: Fallback::DoNotSpawn,
            },
            HardGateReasonCode::AgentNotActive,
            "AGENT_NOT_ACTIVE",
        ),
        (
            "workspace excluded",
            |c, _| c.exclusions.workspace_excluded = true,
            HardGateDisposition::Blocked {
                fallback: Fallback::DoNotSpawn,
            },
            HardGateReasonCode::WorkspaceExcluded,
            "WORKSPACE_EXCLUDED",
        ),
        (
            "runtime capability unknown",
            |c, _| c.execution.runtime = Capability::Unknown,
            HardGateDisposition::Blocked {
                fallback: Fallback::DoNotSpawn,
            },
            HardGateReasonCode::RuntimeCapabilityUnknown,
            "RUNTIME_CAPABILITY_UNKNOWN",
        ),
        (
            "candidate execution mismatch",
            |c, _| c.execution.selected_kind = ExecutionKind::NativeChild,
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::CandidateExecutionKindMismatch,
            "CANDIDATE_EXECUTION_KIND_MISMATCH",
        ),
        (
            "identity mismatch",
            |_, x| x.agent_id = "other-agent".to_owned(),
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::AgentIdMismatch,
            "AGENT_ID_MISMATCH",
        ),
        (
            "scope mismatch",
            |_, x| x.scope.task_scope_key = "other-task".to_owned(),
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::TaskScopeMismatch,
            "TASK_SCOPE_MISMATCH",
        ),
        (
            "thread not idle",
            |_, x| x.thread_state = ThreadState::Active,
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::ThreadNotIdle,
            "THREAD_NOT_IDLE",
        ),
        (
            "review pending",
            |_, x| x.review_pending = true,
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::ReviewPending,
            "REVIEW_PENDING",
        ),
        (
            "context pressure exceeded",
            |_, x| {
                x.context_pressure = ContextPressure::Known {
                    observed_percent: 81,
                    limit_percent: 80,
                }
            },
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::ContextPressureExceeded,
            "CONTEXT_PRESSURE_EXCEEDED",
        ),
        (
            "cache required unavailable",
            |c, x| {
                c.cache_requirement = CacheRequirement::CacheRequired;
                x.cache_state = CacheState::OutsideRequiredWindow;
            },
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            HardGateReasonCode::CacheRequiredUnavailable,
            "CACHE_REQUIRED_UNAVAILABLE",
        ),
    ];

    for (name, mutate, disposition, reason, reason_code) in cases {
        let mut c = context();
        let mut x = candidate();
        assert_eq!(
            evaluate_hard_gates(&c, &x).disposition,
            HardGateDisposition::EligibleForReuse,
            "allow sample failed for {name}",
        );
        mutate(&mut c, &mut x);
        let first = evaluate_hard_gates(&c, &x);
        let second = evaluate_hard_gates(&c, &x);
        assert_reason(first, *disposition, *reason);
        assert_eq!(reason.as_str(), *reason_code, "unstable code for {name}");
        assert_eq!(second, first, "reason code changed for {name}");
    }
}

#[test]
fn spawn_gate_matrix_distinguishes_same_type_conflict_from_available_type() {
    let mut same_type = context();
    same_type.conflicting_agent_type_lease = true;
    assert_reason(
        evaluate_spawn_hard_gates(&same_type),
        HardGateDisposition::Blocked {
            fallback: Fallback::DoNotSpawn,
        },
        HardGateReasonCode::AgentTypeLeaseConflict,
    );
    assert_eq!(
        HardGateReasonCode::AgentTypeLeaseConflict.as_str(),
        "AGENT_TYPE_LEASE_CONFLICT",
    );

    let different_type = context();
    assert_eq!(
        evaluate_spawn_hard_gates(&different_type).disposition,
        HardGateDisposition::SpawnAllowed
    );
}

#[test]
fn cache_matrix_keeps_expiry_soft_unless_cache_is_required() {
    let mut expired = candidate();
    expired.cache_state = CacheState::OutsideRequiredWindow;

    let normal = evaluate_hard_gates(&context(), &expired);
    assert_eq!(normal.disposition, HardGateDisposition::EligibleForReuse);
    assert_eq!(normal.reason_code, None);

    let mut required = context();
    required.cache_requirement = CacheRequirement::CacheRequired;
    let required_result = evaluate_hard_gates(&required, &expired);
    assert_reason(
        required_result,
        HardGateDisposition::CandidateRejected {
            fallback: Fallback::EvaluateSpawn,
        },
        HardGateReasonCode::CacheRequiredUnavailable,
    );
}

fn scored(thread_id: &str, stable_key: &str, age_seconds: Option<u64>) -> ScoringCandidate {
    ScoringCandidate {
        hard_gate: candidate_with_thread(thread_id),
        age_seconds,
        cached_input_tokens: Some(10_000),
        cache_evidence_source: CacheEvidenceSource::Observed,
        stable_key: stable_key.to_owned(),
    }
}

fn candidate_with_thread(thread_id: &str) -> CandidateHardGateInput {
    let mut value = candidate();
    value.thread_id = thread_id.to_owned();
    value
}

#[test]
fn scoring_matrix_is_order_independent_and_uses_fixed_time_tie_breakers() {
    let first = scored("thread-b", "stable-b", Some(10));
    let second = scored("thread-a", "stable-a", Some(10));
    let policy = ScoringPolicy {
        reuse_strategy: ReuseStrategy::Auto,
    };

    let left = select_reuse(&context(), policy, &[first.clone(), second.clone()]);
    let right = select_reuse(&context(), policy, &[second, first]);
    let expected = ReuseSelection::Reuse(ScoredReuseCandidate {
        thread_id: "thread-a".to_owned(),
        stable_key: "stable-a".to_owned(),
        score: 158,
        age_seconds: Some(10),
    });
    assert_eq!(left, expected);
    assert_eq!(right, expected);
}
