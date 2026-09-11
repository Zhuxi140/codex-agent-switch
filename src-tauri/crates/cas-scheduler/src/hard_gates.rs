//! 确定性复用候选硬门槛。此模块只计算结构化输入，不读取持久化状态或 Runtime。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionKind {
    NativeChild,
    ManagedWorker,
    ObservedExternal,
}

impl ExecutionKind {
    fn is_dispatchable(self) -> bool {
        !matches!(self, Self::ObservedExternal)
    }
}

/// 判断历史 Thread 身份能否承载新 Attempt 的计划身份。
/// Native Child 经 App Server resume 后可作为 Managed Worker 使用；反向升级不允许。
pub fn execution_kind_is_compatible(historical: ExecutionKind, planned: ExecutionKind) -> bool {
    matches!(
        (historical, planned),
        (ExecutionKind::NativeChild, ExecutionKind::NativeChild)
            | (ExecutionKind::NativeChild, ExecutionKind::ManagedWorker)
            | (ExecutionKind::ManagedWorker, ExecutionKind::ManagedWorker)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Idle,
    Active,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextHealth {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPressure {
    Known {
        observed_percent: u8,
        limit_percent: u8,
    },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRequirement {
    NotRequired,
    CacheRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheState {
    /// 缓存状态只供 C-02 评分；它不是默认的失败原因。
    NotObserved,
    WithinRequiredWindow,
    OutsideRequiredWindow,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentExecutionInput {
    pub active: bool,
    pub enabled: bool,
    pub model_available: bool,
    pub provider_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExclusionInput {
    pub workspace_excluded: bool,
    pub project_excluded: bool,
    pub conversation_excluded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionCapabilityInput {
    pub selected_kind: ExecutionKind,
    pub runtime: Capability,
    pub agent_execution: Capability,
    pub event: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredIdentity {
    pub agent_id: String,
    pub runtime_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredScope {
    pub workspace_scope_key: String,
    pub parent_thread_id: String,
    pub task_scope_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateScope {
    pub workspace_scope_key: String,
    pub parent_thread_id: String,
    pub task_scope_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateHardGateInput {
    pub thread_id: String,
    pub agent_id: String,
    pub runtime_fingerprint: Option<String>,
    pub execution_kind: ExecutionKind,
    pub scope: CandidateScope,
    pub thread_state: ThreadState,
    pub context_health: ContextHealth,
    pub active_claim: bool,
    pub active_lease: bool,
    pub recovery_required: bool,
    pub review_pending: bool,
    pub context_pressure: ContextPressure,
    pub cache_state: CacheState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HardGateContext {
    pub agent: AgentExecutionInput,
    pub exclusions: ExclusionInput,
    pub execution: ExecutionCapabilityInput,
    pub required_identity: RequiredIdentity,
    pub required_scope: RequiredScope,
    pub cache_requirement: CacheRequirement,
    pub requested_agent_type: String,
    pub conflicting_agent_type_lease: bool,
}

/// 冻结的检查组顺序。每次失败只返回第一个组内的第一个原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardGate {
    AgentExecutable,
    Exclusion,
    ExecutionCapability,
    Identity,
    WorkspaceScope,
    ParentScope,
    TaskScope,
    ThreadAndContextHealth,
    OccupancyAndReview,
    AgentTypeLease,
    ContextPressure,
    CacheRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardGateReasonCode {
    AgentNotActive,
    AgentNotEnabled,
    ModelUnavailable,
    ProviderUnavailable,
    WorkspaceExcluded,
    ProjectExcluded,
    ConversationExcluded,
    ExecutionKindNotDispatchable,
    CandidateExecutionKindNotDispatchable,
    CandidateExecutionKindMismatch,
    RuntimeCapabilityUnsupported,
    RuntimeCapabilityUnknown,
    AgentExecutionCapabilityUnsupported,
    AgentExecutionCapabilityUnknown,
    EventCapabilityUnsupported,
    EventCapabilityUnknown,
    ThreadIdUnknown,
    AgentIdUnknown,
    AgentIdMismatch,
    RuntimeFingerprintUnknown,
    RuntimeFingerprintMismatch,
    WorkspaceScopeUnknown,
    WorkspaceScopeMismatch,
    ParentThreadUnknown,
    ParentThreadMismatch,
    TaskScopeUnknown,
    TaskScopeMismatch,
    ThreadNotIdle,
    ThreadStateUnknown,
    ContextUnhealthy,
    ContextHealthUnknown,
    ActiveClaim,
    ActiveLease,
    RecoveryRequired,
    ReviewPending,
    AgentTypeUnknown,
    AgentTypeLeaseConflict,
    ContextPressureUnknown,
    ContextPressureInvalid,
    ContextPressureExceeded,
    CacheRequiredUnavailable,
}

impl HardGateReasonCode {
    /// 调度审计使用的冻结持久化值；不得依赖 `Debug` 输出生成协议字段。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentNotActive => "AGENT_NOT_ACTIVE",
            Self::AgentNotEnabled => "AGENT_NOT_ENABLED",
            Self::ModelUnavailable => "MODEL_UNAVAILABLE",
            Self::ProviderUnavailable => "PROVIDER_UNAVAILABLE",
            Self::WorkspaceExcluded => "WORKSPACE_EXCLUDED",
            Self::ProjectExcluded => "PROJECT_EXCLUDED",
            Self::ConversationExcluded => "CONVERSATION_EXCLUDED",
            Self::ExecutionKindNotDispatchable => "EXECUTION_KIND_NOT_DISPATCHABLE",
            Self::CandidateExecutionKindNotDispatchable => {
                "CANDIDATE_EXECUTION_KIND_NOT_DISPATCHABLE"
            }
            Self::CandidateExecutionKindMismatch => "CANDIDATE_EXECUTION_KIND_MISMATCH",
            Self::RuntimeCapabilityUnsupported => "RUNTIME_CAPABILITY_UNSUPPORTED",
            Self::RuntimeCapabilityUnknown => "RUNTIME_CAPABILITY_UNKNOWN",
            Self::AgentExecutionCapabilityUnsupported => "AGENT_EXECUTION_CAPABILITY_UNSUPPORTED",
            Self::AgentExecutionCapabilityUnknown => "AGENT_EXECUTION_CAPABILITY_UNKNOWN",
            Self::EventCapabilityUnsupported => "EVENT_CAPABILITY_UNSUPPORTED",
            Self::EventCapabilityUnknown => "EVENT_CAPABILITY_UNKNOWN",
            Self::ThreadIdUnknown => "THREAD_ID_UNKNOWN",
            Self::AgentIdUnknown => "AGENT_ID_UNKNOWN",
            Self::AgentIdMismatch => "AGENT_ID_MISMATCH",
            Self::RuntimeFingerprintUnknown => "RUNTIME_FINGERPRINT_UNKNOWN",
            Self::RuntimeFingerprintMismatch => "RUNTIME_FINGERPRINT_MISMATCH",
            Self::WorkspaceScopeUnknown => "WORKSPACE_SCOPE_UNKNOWN",
            Self::WorkspaceScopeMismatch => "WORKSPACE_SCOPE_MISMATCH",
            Self::ParentThreadUnknown => "PARENT_THREAD_UNKNOWN",
            Self::ParentThreadMismatch => "PARENT_THREAD_MISMATCH",
            Self::TaskScopeUnknown => "TASK_SCOPE_UNKNOWN",
            Self::TaskScopeMismatch => "TASK_SCOPE_MISMATCH",
            Self::ThreadNotIdle => "THREAD_NOT_IDLE",
            Self::ThreadStateUnknown => "THREAD_STATE_UNKNOWN",
            Self::ContextUnhealthy => "CONTEXT_UNHEALTHY",
            Self::ContextHealthUnknown => "CONTEXT_HEALTH_UNKNOWN",
            Self::ActiveClaim => "ACTIVE_CLAIM",
            Self::ActiveLease => "ACTIVE_LEASE",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::ReviewPending => "REVIEW_PENDING",
            Self::AgentTypeUnknown => "AGENT_TYPE_UNKNOWN",
            Self::AgentTypeLeaseConflict => "AGENT_TYPE_LEASE_CONFLICT",
            Self::ContextPressureUnknown => "CONTEXT_PRESSURE_UNKNOWN",
            Self::ContextPressureInvalid => "CONTEXT_PRESSURE_INVALID",
            Self::ContextPressureExceeded => "CONTEXT_PRESSURE_EXCEEDED",
            Self::CacheRequiredUnavailable => "CACHE_REQUIRED_UNAVAILABLE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fallback {
    EvaluateSpawn,
    DoNotSpawn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardGateDisposition {
    EligibleForReuse,
    SpawnAllowed,
    CandidateRejected { fallback: Fallback },
    Blocked { fallback: Fallback },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardGateEvaluation {
    pub disposition: HardGateDisposition,
    pub failed_gate: Option<HardGate>,
    pub reason_code: Option<HardGateReasonCode>,
}

impl HardGateEvaluation {
    fn eligible() -> Self {
        Self {
            disposition: HardGateDisposition::EligibleForReuse,
            failed_gate: None,
            reason_code: None,
        }
    }

    fn blocked(gate: HardGate, reason_code: HardGateReasonCode) -> Self {
        Self {
            disposition: HardGateDisposition::Blocked {
                fallback: Fallback::DoNotSpawn,
            },
            failed_gate: Some(gate),
            reason_code: Some(reason_code),
        }
    }

    fn rejected(gate: HardGate, reason_code: HardGateReasonCode) -> Self {
        Self {
            disposition: HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn,
            },
            failed_gate: Some(gate),
            reason_code: Some(reason_code),
        }
    }

    fn spawn_allowed() -> Self {
        Self {
            disposition: HardGateDisposition::SpawnAllowed,
            failed_gate: None,
            reason_code: None,
        }
    }
}

/// 按设计方案 §13.2 的固定顺序评估一个复用候选。
///
/// `CandidateRejected` 只表示候选不可复用，调用方必须再调用 `evaluate_spawn_hard_gates`
/// 判断能否 Spawn；此函数不把缓存过期作为默认门槛。
pub fn evaluate_hard_gates(
    context: &HardGateContext,
    candidate: &CandidateHardGateInput,
) -> HardGateEvaluation {
    if let Some(result) = evaluate_dispatch_prerequisites(context) {
        return result;
    }
    if !candidate.execution_kind.is_dispatchable() {
        return HardGateEvaluation::rejected(
            HardGate::ExecutionCapability,
            HardGateReasonCode::CandidateExecutionKindNotDispatchable,
        );
    }
    if !execution_kind_is_compatible(candidate.execution_kind, context.execution.selected_kind) {
        return HardGateEvaluation::rejected(
            HardGate::ExecutionCapability,
            HardGateReasonCode::CandidateExecutionKindMismatch,
        );
    }

    if !is_present(&candidate.thread_id) {
        return HardGateEvaluation::rejected(
            HardGate::Identity,
            HardGateReasonCode::ThreadIdUnknown,
        );
    }
    if !is_present(&context.required_identity.agent_id) || !is_present(&candidate.agent_id) {
        return HardGateEvaluation::rejected(
            HardGate::Identity,
            HardGateReasonCode::AgentIdUnknown,
        );
    }
    if context.required_identity.agent_id != candidate.agent_id {
        return HardGateEvaluation::rejected(
            HardGate::Identity,
            HardGateReasonCode::AgentIdMismatch,
        );
    }
    let (Some(required_fingerprint), Some(candidate_fingerprint)) = (
        context.required_identity.runtime_fingerprint.as_deref(),
        candidate.runtime_fingerprint.as_deref(),
    ) else {
        return HardGateEvaluation::rejected(
            HardGate::Identity,
            HardGateReasonCode::RuntimeFingerprintUnknown,
        );
    };
    if !is_present(required_fingerprint) || !is_present(candidate_fingerprint) {
        return HardGateEvaluation::rejected(
            HardGate::Identity,
            HardGateReasonCode::RuntimeFingerprintUnknown,
        );
    }
    if required_fingerprint != candidate_fingerprint {
        return HardGateEvaluation::rejected(
            HardGate::Identity,
            HardGateReasonCode::RuntimeFingerprintMismatch,
        );
    }

    if !is_present(&context.required_scope.workspace_scope_key)
        || !is_present(&candidate.scope.workspace_scope_key)
    {
        return HardGateEvaluation::rejected(
            HardGate::WorkspaceScope,
            HardGateReasonCode::WorkspaceScopeUnknown,
        );
    }
    if context.required_scope.workspace_scope_key != candidate.scope.workspace_scope_key {
        return HardGateEvaluation::rejected(
            HardGate::WorkspaceScope,
            HardGateReasonCode::WorkspaceScopeMismatch,
        );
    }

    if !is_present(&context.required_scope.parent_thread_id)
        || !is_present(&candidate.scope.parent_thread_id)
    {
        return HardGateEvaluation::rejected(
            HardGate::ParentScope,
            HardGateReasonCode::ParentThreadUnknown,
        );
    }
    if context.required_scope.parent_thread_id != candidate.scope.parent_thread_id {
        return HardGateEvaluation::rejected(
            HardGate::ParentScope,
            HardGateReasonCode::ParentThreadMismatch,
        );
    }

    if !is_present(&context.required_scope.task_scope_key)
        || !is_present(&candidate.scope.task_scope_key)
    {
        return HardGateEvaluation::rejected(
            HardGate::TaskScope,
            HardGateReasonCode::TaskScopeUnknown,
        );
    }
    if context.required_scope.task_scope_key != candidate.scope.task_scope_key {
        return HardGateEvaluation::rejected(
            HardGate::TaskScope,
            HardGateReasonCode::TaskScopeMismatch,
        );
    }

    match candidate.thread_state {
        ThreadState::Idle => {}
        ThreadState::Active => {
            return HardGateEvaluation::rejected(
                HardGate::ThreadAndContextHealth,
                HardGateReasonCode::ThreadNotIdle,
            );
        }
        ThreadState::Unknown => {
            return HardGateEvaluation::rejected(
                HardGate::ThreadAndContextHealth,
                HardGateReasonCode::ThreadStateUnknown,
            );
        }
    }
    match candidate.context_health {
        ContextHealth::Healthy => {}
        ContextHealth::Unhealthy => {
            return HardGateEvaluation::rejected(
                HardGate::ThreadAndContextHealth,
                HardGateReasonCode::ContextUnhealthy,
            );
        }
        ContextHealth::Unknown => {
            return HardGateEvaluation::rejected(
                HardGate::ThreadAndContextHealth,
                HardGateReasonCode::ContextHealthUnknown,
            );
        }
    }

    if candidate.active_claim {
        return HardGateEvaluation::rejected(
            HardGate::OccupancyAndReview,
            HardGateReasonCode::ActiveClaim,
        );
    }
    if candidate.active_lease {
        return HardGateEvaluation::rejected(
            HardGate::OccupancyAndReview,
            HardGateReasonCode::ActiveLease,
        );
    }
    if candidate.recovery_required {
        return HardGateEvaluation::rejected(
            HardGate::OccupancyAndReview,
            HardGateReasonCode::RecoveryRequired,
        );
    }
    if candidate.review_pending {
        return HardGateEvaluation::rejected(
            HardGate::OccupancyAndReview,
            HardGateReasonCode::ReviewPending,
        );
    }

    if !is_present(&context.requested_agent_type) {
        return HardGateEvaluation::blocked(
            HardGate::AgentTypeLease,
            HardGateReasonCode::AgentTypeUnknown,
        );
    }
    if context.conflicting_agent_type_lease {
        return HardGateEvaluation::blocked(
            HardGate::AgentTypeLease,
            HardGateReasonCode::AgentTypeLeaseConflict,
        );
    }

    match candidate.context_pressure {
        ContextPressure::Unknown => {
            return HardGateEvaluation::rejected(
                HardGate::ContextPressure,
                HardGateReasonCode::ContextPressureUnknown,
            );
        }
        ContextPressure::Known {
            observed_percent,
            limit_percent,
        } if observed_percent > 100 || limit_percent > 100 => {
            return HardGateEvaluation::rejected(
                HardGate::ContextPressure,
                HardGateReasonCode::ContextPressureInvalid,
            );
        }
        ContextPressure::Known {
            observed_percent,
            limit_percent,
        } if observed_percent > limit_percent => {
            return HardGateEvaluation::rejected(
                HardGate::ContextPressure,
                HardGateReasonCode::ContextPressureExceeded,
            );
        }
        ContextPressure::Known { .. } => {}
    }

    if context.cache_requirement == CacheRequirement::CacheRequired
        && candidate.cache_state != CacheState::WithinRequiredWindow
    {
        return HardGateEvaluation::rejected(
            HardGate::CacheRequired,
            HardGateReasonCode::CacheRequiredUnavailable,
        );
    }

    HardGateEvaluation::eligible()
}

/// 在没有可复用候选或候选被淘汰后，独立判断当前请求是否允许 Spawn。
///
/// 不接收候选事实，避免坏候选伪造或遗漏同 Agent Type 的全局并发状态。
pub fn evaluate_spawn_hard_gates(context: &HardGateContext) -> HardGateEvaluation {
    if let Some(result) = evaluate_dispatch_prerequisites(context) {
        return result;
    }
    if !is_present(&context.requested_agent_type) {
        return HardGateEvaluation::blocked(
            HardGate::AgentTypeLease,
            HardGateReasonCode::AgentTypeUnknown,
        );
    }
    if context.conflicting_agent_type_lease {
        return HardGateEvaluation::blocked(
            HardGate::AgentTypeLease,
            HardGateReasonCode::AgentTypeLeaseConflict,
        );
    }
    HardGateEvaluation::spawn_allowed()
}

fn evaluate_dispatch_prerequisites(context: &HardGateContext) -> Option<HardGateEvaluation> {
    if !context.agent.active {
        return Some(HardGateEvaluation::blocked(
            HardGate::AgentExecutable,
            HardGateReasonCode::AgentNotActive,
        ));
    }
    if !context.agent.enabled {
        return Some(HardGateEvaluation::blocked(
            HardGate::AgentExecutable,
            HardGateReasonCode::AgentNotEnabled,
        ));
    }
    if !context.agent.model_available {
        return Some(HardGateEvaluation::blocked(
            HardGate::AgentExecutable,
            HardGateReasonCode::ModelUnavailable,
        ));
    }
    if !context.agent.provider_available {
        return Some(HardGateEvaluation::blocked(
            HardGate::AgentExecutable,
            HardGateReasonCode::ProviderUnavailable,
        ));
    }

    if context.exclusions.workspace_excluded {
        return Some(HardGateEvaluation::blocked(
            HardGate::Exclusion,
            HardGateReasonCode::WorkspaceExcluded,
        ));
    }
    if context.exclusions.project_excluded {
        return Some(HardGateEvaluation::blocked(
            HardGate::Exclusion,
            HardGateReasonCode::ProjectExcluded,
        ));
    }
    if context.exclusions.conversation_excluded {
        return Some(HardGateEvaluation::blocked(
            HardGate::Exclusion,
            HardGateReasonCode::ConversationExcluded,
        ));
    }

    if !context.execution.selected_kind.is_dispatchable() {
        return Some(HardGateEvaluation::blocked(
            HardGate::ExecutionCapability,
            HardGateReasonCode::ExecutionKindNotDispatchable,
        ));
    }
    if let Some(reason) = unsupported_or_unknown(
        context.execution.runtime,
        HardGateReasonCode::RuntimeCapabilityUnsupported,
        HardGateReasonCode::RuntimeCapabilityUnknown,
    ) {
        return Some(HardGateEvaluation::blocked(
            HardGate::ExecutionCapability,
            reason,
        ));
    }
    if let Some(reason) = unsupported_or_unknown(
        context.execution.agent_execution,
        HardGateReasonCode::AgentExecutionCapabilityUnsupported,
        HardGateReasonCode::AgentExecutionCapabilityUnknown,
    ) {
        return Some(HardGateEvaluation::blocked(
            HardGate::ExecutionCapability,
            reason,
        ));
    }
    if let Some(reason) = unsupported_or_unknown(
        context.execution.event,
        HardGateReasonCode::EventCapabilityUnsupported,
        HardGateReasonCode::EventCapabilityUnknown,
    ) {
        return Some(HardGateEvaluation::blocked(
            HardGate::ExecutionCapability,
            reason,
        ));
    }
    None
}

fn unsupported_or_unknown(
    capability: Capability,
    unsupported: HardGateReasonCode,
    unknown: HardGateReasonCode,
) -> Option<HardGateReasonCode> {
    match capability {
        Capability::Supported => None,
        Capability::Unsupported => Some(unsupported),
        Capability::Unknown => Some(unknown),
    }
}

fn is_present(value: &str) -> bool {
    !value.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestInput {
        context: HardGateContext,
        candidate: CandidateHardGateInput,
    }

    impl std::ops::Deref for TestInput {
        type Target = HardGateContext;

        fn deref(&self) -> &Self::Target {
            &self.context
        }
    }

    impl std::ops::DerefMut for TestInput {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.context
        }
    }

    fn input() -> TestInput {
        TestInput {
            context: HardGateContext {
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
                    agent_id: "agent-a".to_owned(),
                    runtime_fingerprint: Some("fingerprint-a".to_owned()),
                },
                required_scope: RequiredScope {
                    workspace_scope_key: "workspace-a".to_owned(),
                    parent_thread_id: "parent-a".to_owned(),
                    task_scope_key: "task-a".to_owned(),
                },
                cache_requirement: CacheRequirement::NotRequired,
                requested_agent_type: "implementation".to_owned(),
                conflicting_agent_type_lease: false,
            },
            candidate: CandidateHardGateInput {
                thread_id: "thread-a".to_owned(),
                agent_id: "agent-a".to_owned(),
                runtime_fingerprint: Some("fingerprint-a".to_owned()),
                execution_kind: ExecutionKind::ManagedWorker,
                scope: CandidateScope {
                    workspace_scope_key: "workspace-a".to_owned(),
                    parent_thread_id: "parent-a".to_owned(),
                    task_scope_key: "task-a".to_owned(),
                },
                thread_state: ThreadState::Idle,
                context_health: ContextHealth::Healthy,
                active_claim: false,
                active_lease: false,
                recovery_required: false,
                review_pending: false,
                context_pressure: ContextPressure::Known {
                    observed_percent: 50,
                    limit_percent: 80,
                },
                cache_state: CacheState::OutsideRequiredWindow,
            },
        }
    }

    fn evaluate_hard_gates(input: &TestInput) -> HardGateEvaluation {
        super::evaluate_hard_gates(&input.context, &input.candidate)
    }

    fn assert_blocked(result: HardGateEvaluation, reason: HardGateReasonCode) {
        assert_eq!(result.reason_code, Some(reason));
        assert_eq!(
            result.disposition,
            HardGateDisposition::Blocked {
                fallback: Fallback::DoNotSpawn
            }
        );
    }

    fn assert_rejected(result: HardGateEvaluation, reason: HardGateReasonCode) {
        assert_eq!(result.reason_code, Some(reason));
        assert_eq!(
            result.disposition,
            HardGateDisposition::CandidateRejected {
                fallback: Fallback::EvaluateSpawn
            }
        );
    }

    #[test]
    fn eligible_candidate_passes_all_hard_gates() {
        assert_eq!(
            evaluate_hard_gates(&input()),
            HardGateEvaluation::eligible()
        );
    }

    #[test]
    fn inactive_agent_blocks_spawn() {
        let mut value = input();
        value.agent.active = false;
        assert_blocked(
            evaluate_hard_gates(&value),
            HardGateReasonCode::AgentNotActive,
        );
    }

    #[test]
    fn excluded_workspace_blocks_spawn() {
        let mut value = input();
        value.exclusions.workspace_excluded = true;
        assert_blocked(
            evaluate_hard_gates(&value),
            HardGateReasonCode::WorkspaceExcluded,
        );
    }

    #[test]
    fn unknown_execution_capability_blocks_spawn() {
        let mut value = input();
        value.execution.event = Capability::Unknown;
        assert_blocked(
            evaluate_hard_gates(&value),
            HardGateReasonCode::EventCapabilityUnknown,
        );
    }

    #[test]
    fn observed_external_candidate_never_passes_dispatchable_gate() {
        let mut value = input();
        value.candidate.execution_kind = ExecutionKind::ObservedExternal;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::CandidateExecutionKindNotDispatchable,
        );
    }

    #[test]
    fn observed_external_requested_kind_blocks_spawn() {
        let mut value = input();
        value.execution.selected_kind = ExecutionKind::ObservedExternal;
        assert_blocked(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ExecutionKindNotDispatchable,
        );
    }

    #[test]
    fn execution_kind_compatibility_matrix_is_frozen() {
        for (historical, planned, expected) in [
            (ExecutionKind::NativeChild, ExecutionKind::NativeChild, true),
            (
                ExecutionKind::NativeChild,
                ExecutionKind::ManagedWorker,
                true,
            ),
            (
                ExecutionKind::ManagedWorker,
                ExecutionKind::ManagedWorker,
                true,
            ),
            (
                ExecutionKind::ManagedWorker,
                ExecutionKind::NativeChild,
                false,
            ),
            (
                ExecutionKind::ObservedExternal,
                ExecutionKind::ManagedWorker,
                false,
            ),
            (
                ExecutionKind::NativeChild,
                ExecutionKind::ObservedExternal,
                false,
            ),
        ] {
            assert_eq!(execution_kind_is_compatible(historical, planned), expected);
        }
    }

    #[test]
    fn native_child_can_be_reused_for_managed_worker_attempt() {
        let mut value = input();
        value.candidate.execution_kind = ExecutionKind::NativeChild;
        value.execution.selected_kind = ExecutionKind::ManagedWorker;

        assert_eq!(evaluate_hard_gates(&value), HardGateEvaluation::eligible());
    }

    #[test]
    fn unknown_fingerprint_rejects_candidate_fail_closed() {
        let mut value = input();
        value.candidate.runtime_fingerprint = None;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::RuntimeFingerprintUnknown,
        );
    }

    #[test]
    fn agent_id_mismatch_rejects_candidate() {
        let mut value = input();
        value.candidate.agent_id = "agent-b".to_owned();
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::AgentIdMismatch,
        );
    }

    #[test]
    fn blank_thread_id_rejects_candidate_in_identity_gate() {
        let mut value = input();
        value.candidate.thread_id = "  ".to_owned();
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ThreadIdUnknown,
        );
    }

    #[test]
    fn workspace_scope_mismatch_rejects_candidate() {
        let mut value = input();
        value.candidate.scope.workspace_scope_key = "workspace-b".to_owned();
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::WorkspaceScopeMismatch,
        );
    }

    #[test]
    fn parent_scope_mismatch_rejects_candidate() {
        let mut value = input();
        value.candidate.scope.parent_thread_id = "parent-b".to_owned();
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ParentThreadMismatch,
        );
    }

    #[test]
    fn task_scope_mismatch_rejects_candidate() {
        let mut value = input();
        value.candidate.scope.task_scope_key = "task-b".to_owned();
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::TaskScopeMismatch,
        );
    }

    #[test]
    fn non_idle_thread_rejects_candidate() {
        let mut value = input();
        value.candidate.thread_state = ThreadState::Active;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ThreadNotIdle,
        );
    }

    #[test]
    fn unhealthy_context_rejects_candidate() {
        let mut value = input();
        value.candidate.context_health = ContextHealth::Unhealthy;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ContextUnhealthy,
        );
    }

    #[test]
    fn active_claim_rejects_candidate() {
        let mut value = input();
        value.candidate.active_claim = true;
        assert_rejected(evaluate_hard_gates(&value), HardGateReasonCode::ActiveClaim);
    }

    #[test]
    fn active_lease_rejects_candidate() {
        let mut value = input();
        value.candidate.active_lease = true;
        assert_rejected(evaluate_hard_gates(&value), HardGateReasonCode::ActiveLease);
    }

    #[test]
    fn recovery_required_rejects_candidate() {
        let mut value = input();
        value.candidate.recovery_required = true;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::RecoveryRequired,
        );
    }

    #[test]
    fn pending_review_rejects_candidate() {
        let mut value = input();
        value.candidate.review_pending = true;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ReviewPending,
        );
    }

    #[test]
    fn same_agent_type_active_lease_blocks_spawn() {
        let mut value = input();
        value.conflicting_agent_type_lease = true;
        assert_blocked(
            evaluate_hard_gates(&value),
            HardGateReasonCode::AgentTypeLeaseConflict,
        );
    }

    #[test]
    fn unknown_context_pressure_rejects_candidate_fail_closed() {
        let mut value = input();
        value.candidate.context_pressure = ContextPressure::Unknown;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ContextPressureUnknown,
        );
    }

    #[test]
    fn excessive_context_pressure_rejects_candidate() {
        let mut value = input();
        value.candidate.context_pressure = ContextPressure::Known {
            observed_percent: 81,
            limit_percent: 80,
        };
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::ContextPressureExceeded,
        );
    }

    #[test]
    fn invalid_context_pressure_rejects_candidate_fail_closed() {
        for (observed_percent, limit_percent) in [(200, 80), (80, 250), (200, 250)] {
            let mut value = input();
            value.candidate.context_pressure = ContextPressure::Known {
                observed_percent,
                limit_percent,
            };
            assert_rejected(
                evaluate_hard_gates(&value),
                HardGateReasonCode::ContextPressureInvalid,
            );
        }
    }

    #[test]
    fn context_pressure_boundary_values_are_valid() {
        for (observed_percent, limit_percent) in [(0, 0), (100, 100)] {
            let mut value = input();
            value.candidate.context_pressure = ContextPressure::Known {
                observed_percent,
                limit_percent,
            };
            assert_eq!(evaluate_hard_gates(&value), HardGateEvaluation::eligible());
        }
    }

    #[test]
    fn cache_expiry_is_not_a_default_hard_gate() {
        let value = input();
        assert_eq!(
            evaluate_hard_gates(&value).disposition,
            HardGateDisposition::EligibleForReuse
        );
    }

    #[test]
    fn cache_required_rejects_candidate_without_current_cache() {
        let mut value = input();
        value.cache_requirement = CacheRequirement::CacheRequired;
        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::CacheRequiredUnavailable,
        );
    }

    #[test]
    fn first_failure_priority_is_frozen_across_groups() {
        let mut value = input();
        value.agent.active = false;
        value.exclusions.workspace_excluded = true;
        value.execution.runtime = Capability::Unknown;
        value.candidate.runtime_fingerprint = None;
        value.candidate.scope.workspace_scope_key = "other".to_owned();
        value.candidate.scope.parent_thread_id = "other".to_owned();
        value.candidate.scope.task_scope_key = "other".to_owned();
        value.candidate.thread_state = ThreadState::Active;
        value.candidate.active_claim = true;
        value.conflicting_agent_type_lease = true;
        value.candidate.context_pressure = ContextPressure::Unknown;

        let result = evaluate_hard_gates(&value);
        assert_eq!(result.failed_gate, Some(HardGate::AgentExecutable));
        assert_eq!(result.reason_code, Some(HardGateReasonCode::AgentNotActive));
    }

    #[test]
    fn first_failure_priority_is_frozen_within_occupancy_group() {
        let mut value = input();
        value.candidate.active_lease = true;
        value.candidate.recovery_required = true;
        value.candidate.review_pending = true;

        let result = evaluate_hard_gates(&value);
        assert_eq!(result.failed_gate, Some(HardGate::OccupancyAndReview));
        assert_eq!(result.reason_code, Some(HardGateReasonCode::ActiveLease));
    }

    #[test]
    fn first_failure_priority_is_frozen_within_other_multi_check_groups() {
        let mut agent = input();
        agent.agent.active = false;
        agent.agent.enabled = false;
        agent.agent.model_available = false;
        agent.agent.provider_available = false;
        assert_eq!(
            evaluate_hard_gates(&agent).reason_code,
            Some(HardGateReasonCode::AgentNotActive)
        );

        let mut exclusion = input();
        exclusion.exclusions.workspace_excluded = true;
        exclusion.exclusions.project_excluded = true;
        exclusion.exclusions.conversation_excluded = true;
        assert_eq!(
            evaluate_hard_gates(&exclusion).reason_code,
            Some(HardGateReasonCode::WorkspaceExcluded)
        );

        let mut capability = input();
        capability.execution.selected_kind = ExecutionKind::ObservedExternal;
        capability.execution.runtime = Capability::Unknown;
        capability.execution.agent_execution = Capability::Unknown;
        capability.execution.event = Capability::Unknown;
        assert_eq!(
            evaluate_hard_gates(&capability).reason_code,
            Some(HardGateReasonCode::ExecutionKindNotDispatchable)
        );

        let mut identity = input();
        identity.candidate.thread_id.clear();
        identity.candidate.agent_id.clear();
        identity.candidate.runtime_fingerprint = None;
        assert_eq!(
            evaluate_hard_gates(&identity).reason_code,
            Some(HardGateReasonCode::ThreadIdUnknown)
        );

        let mut thread = input();
        thread.candidate.thread_state = ThreadState::Active;
        thread.candidate.context_health = ContextHealth::Unknown;
        assert_eq!(
            evaluate_hard_gates(&thread).reason_code,
            Some(HardGateReasonCode::ThreadNotIdle)
        );
    }

    #[test]
    fn spawn_gate_blocks_same_type_lease_without_a_candidate() {
        let mut context = input().context;
        context.conflicting_agent_type_lease = true;

        assert_blocked(
            super::evaluate_spawn_hard_gates(&context),
            HardGateReasonCode::AgentTypeLeaseConflict,
        );
    }

    #[test]
    fn candidate_early_rejection_does_not_hide_spawn_lease_block() {
        let mut value = input();
        value.candidate.agent_id = "other-agent".to_owned();
        value.conflicting_agent_type_lease = true;

        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::AgentIdMismatch,
        );
        assert_blocked(
            super::evaluate_spawn_hard_gates(&value.context),
            HardGateReasonCode::AgentTypeLeaseConflict,
        );
    }

    #[test]
    fn bad_candidate_cannot_supply_agent_type_for_spawn() {
        let mut value = input();
        value.candidate.agent_id = "other-agent".to_owned();

        assert_rejected(
            evaluate_hard_gates(&value),
            HardGateReasonCode::AgentIdMismatch,
        );
        assert_eq!(
            super::evaluate_spawn_hard_gates(&value.context),
            HardGateEvaluation::spawn_allowed()
        );
    }

    #[test]
    fn spawn_gate_allows_happy_path() {
        let value = input();
        assert_eq!(
            super::evaluate_spawn_hard_gates(&value.context),
            HardGateEvaluation::spawn_allowed()
        );
    }
}
