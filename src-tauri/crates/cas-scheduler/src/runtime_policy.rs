//! Runtime 委派准入的确定性纯函数。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDecision {
    /// 当前调用方可继续；是否允许创建委派占用仍以 `reservation_allowed` 为准。
    Allow,
    /// Primary 可继续进入底层 Sandbox/Approval 流程，但必须持久化并展示警告；
    /// 不得创建委派占用或授予工具权限。
    Warn,
    /// Strict Stop：当前调用方不得继续，也不得创建委派占用。
    Deny,
}

impl AdmissionDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "ALLOW",
            Self::Warn => "WARN",
            Self::Deny => "DENY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePolicyMode {
    Default,
    Orchestration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchestrationFailurePolicy {
    StrictStop,
    PrimaryFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimePolicyInput {
    pub mode: RuntimePolicyMode,
    pub delegation_required: bool,
    pub failure_policy: OrchestrationFailurePolicy,
    pub workspace_excluded: bool,
    pub project_excluded: bool,
    pub conversation_excluded: bool,
    pub capability_supported: bool,
    pub runtime_healthy: bool,
    pub permission_allowed: bool,
    pub scope_allowed: bool,
    pub schedule_certain: bool,
    pub lease_certain: bool,
    pub receipt_certain: bool,
    pub schema_verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimePolicyOutcome {
    pub decision: AdmissionDecision,
    pub reason_code: RuntimePolicyReasonCode,
    /// 创建 Reuse Claim 或 Spawn Reservation 前必须检查的唯一授权位。
    pub reservation_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimePolicyReasonCode {
    DefaultMode,
    WorkspaceExcluded,
    ProjectExcluded,
    ConversationExcluded,
    DelegationNotRequired,
    CapabilityUnsupported,
    RuntimeUnhealthy,
    PermissionDenied,
    ScopeDenied,
    ScheduleUncertain,
    LeaseUncertain,
    ReceiptUncertain,
    SchemaUnverified,
    AdmissionAllowed,
}

impl RuntimePolicyReasonCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefaultMode => "DEFAULT_MODE",
            Self::WorkspaceExcluded => "WORKSPACE_EXCLUDED",
            Self::ProjectExcluded => "PROJECT_EXCLUDED",
            Self::ConversationExcluded => "CONVERSATION_EXCLUDED",
            Self::DelegationNotRequired => "DELEGATION_NOT_REQUIRED",
            Self::CapabilityUnsupported => "CAPABILITY_UNSUPPORTED",
            Self::RuntimeUnhealthy => "RUNTIME_UNHEALTHY",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::ScopeDenied => "SCOPE_DENIED",
            Self::ScheduleUncertain => "SCHEDULE_UNCERTAIN",
            Self::LeaseUncertain => "LEASE_UNCERTAIN",
            Self::ReceiptUncertain => "RECEIPT_UNCERTAIN",
            Self::SchemaUnverified => "SCHEMA_UNVERIFIED",
            Self::AdmissionAllowed => "ADMISSION_ALLOWED",
        }
    }
}

/// 固定首因顺序：Default、三层排除、非必须委派、能力、Runtime、权限、Scope、
/// Schedule、Lease、Receipt、Schema。函数不读取数据库、环境、文件或 AGENTS 文本。
pub fn evaluate_runtime_policy(input: &RuntimePolicyInput) -> RuntimePolicyOutcome {
    if input.mode == RuntimePolicyMode::Default {
        return allow(RuntimePolicyReasonCode::DefaultMode, false);
    }
    if input.workspace_excluded {
        return allow(RuntimePolicyReasonCode::WorkspaceExcluded, false);
    }
    if input.project_excluded {
        return allow(RuntimePolicyReasonCode::ProjectExcluded, false);
    }
    if input.conversation_excluded {
        return allow(RuntimePolicyReasonCode::ConversationExcluded, false);
    }
    if !input.delegation_required {
        return allow(RuntimePolicyReasonCode::DelegationNotRequired, false);
    }
    if !input.capability_supported {
        return safety_failure(input, RuntimePolicyReasonCode::CapabilityUnsupported);
    }
    if !input.runtime_healthy {
        return safety_failure(input, RuntimePolicyReasonCode::RuntimeUnhealthy);
    }
    if !input.permission_allowed {
        return safety_failure(input, RuntimePolicyReasonCode::PermissionDenied);
    }
    if !input.scope_allowed {
        return safety_failure(input, RuntimePolicyReasonCode::ScopeDenied);
    }
    if !input.schedule_certain {
        return safety_failure(input, RuntimePolicyReasonCode::ScheduleUncertain);
    }
    if !input.lease_certain {
        return safety_failure(input, RuntimePolicyReasonCode::LeaseUncertain);
    }
    if !input.receipt_certain {
        return safety_failure(input, RuntimePolicyReasonCode::ReceiptUncertain);
    }
    if !input.schema_verified {
        return safety_failure(input, RuntimePolicyReasonCode::SchemaUnverified);
    }
    allow(RuntimePolicyReasonCode::AdmissionAllowed, true)
}

fn allow(reason_code: RuntimePolicyReasonCode, reservation_allowed: bool) -> RuntimePolicyOutcome {
    RuntimePolicyOutcome {
        decision: AdmissionDecision::Allow,
        reason_code,
        reservation_allowed,
    }
}

fn safety_failure(
    input: &RuntimePolicyInput,
    reason_code: RuntimePolicyReasonCode,
) -> RuntimePolicyOutcome {
    RuntimePolicyOutcome {
        decision: match input.failure_policy {
            OrchestrationFailurePolicy::StrictStop => AdmissionDecision::Deny,
            OrchestrationFailurePolicy::PrimaryFallback => AdmissionDecision::Warn,
        },
        reason_code,
        reservation_allowed: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> RuntimePolicyInput {
        RuntimePolicyInput {
            mode: RuntimePolicyMode::Orchestration,
            delegation_required: true,
            failure_policy: OrchestrationFailurePolicy::StrictStop,
            workspace_excluded: false,
            project_excluded: false,
            conversation_excluded: false,
            capability_supported: true,
            runtime_healthy: true,
            permission_allowed: true,
            scope_allowed: true,
            schedule_certain: true,
            lease_certain: true,
            receipt_certain: true,
            schema_verified: true,
        }
    }

    #[test]
    fn admission_decision_strings_are_stable() {
        assert_eq!(AdmissionDecision::Allow.as_str(), "ALLOW");
        assert_eq!(AdmissionDecision::Warn.as_str(), "WARN");
        assert_eq!(AdmissionDecision::Deny.as_str(), "DENY");
    }

    #[test]
    fn reason_code_strings_are_stable() {
        let cases = [
            (RuntimePolicyReasonCode::DefaultMode, "DEFAULT_MODE"),
            (
                RuntimePolicyReasonCode::WorkspaceExcluded,
                "WORKSPACE_EXCLUDED",
            ),
            (RuntimePolicyReasonCode::ProjectExcluded, "PROJECT_EXCLUDED"),
            (
                RuntimePolicyReasonCode::ConversationExcluded,
                "CONVERSATION_EXCLUDED",
            ),
            (
                RuntimePolicyReasonCode::DelegationNotRequired,
                "DELEGATION_NOT_REQUIRED",
            ),
            (
                RuntimePolicyReasonCode::CapabilityUnsupported,
                "CAPABILITY_UNSUPPORTED",
            ),
            (
                RuntimePolicyReasonCode::RuntimeUnhealthy,
                "RUNTIME_UNHEALTHY",
            ),
            (
                RuntimePolicyReasonCode::PermissionDenied,
                "PERMISSION_DENIED",
            ),
            (RuntimePolicyReasonCode::ScopeDenied, "SCOPE_DENIED"),
            (
                RuntimePolicyReasonCode::ScheduleUncertain,
                "SCHEDULE_UNCERTAIN",
            ),
            (RuntimePolicyReasonCode::LeaseUncertain, "LEASE_UNCERTAIN"),
            (
                RuntimePolicyReasonCode::ReceiptUncertain,
                "RECEIPT_UNCERTAIN",
            ),
            (
                RuntimePolicyReasonCode::SchemaUnverified,
                "SCHEMA_UNVERIFIED",
            ),
            (
                RuntimePolicyReasonCode::AdmissionAllowed,
                "ADMISSION_ALLOWED",
            ),
        ];

        for (reason, value) in cases {
            assert_eq!(reason.as_str(), value);
        }
    }

    #[test]
    fn evaluates_every_branch_with_strict_stop() {
        struct Case {
            name: &'static str,
            mutate: fn(&mut RuntimePolicyInput),
            decision: AdmissionDecision,
            reason_code: RuntimePolicyReasonCode,
            reservation_allowed: bool,
        }

        let cases = [
            Case {
                name: "default",
                mutate: |value| value.mode = RuntimePolicyMode::Default,
                decision: AdmissionDecision::Allow,
                reason_code: RuntimePolicyReasonCode::DefaultMode,
                reservation_allowed: false,
            },
            Case {
                name: "workspace exclusion",
                mutate: |value| value.workspace_excluded = true,
                decision: AdmissionDecision::Allow,
                reason_code: RuntimePolicyReasonCode::WorkspaceExcluded,
                reservation_allowed: false,
            },
            Case {
                name: "project exclusion",
                mutate: |value| value.project_excluded = true,
                decision: AdmissionDecision::Allow,
                reason_code: RuntimePolicyReasonCode::ProjectExcluded,
                reservation_allowed: false,
            },
            Case {
                name: "conversation exclusion",
                mutate: |value| value.conversation_excluded = true,
                decision: AdmissionDecision::Allow,
                reason_code: RuntimePolicyReasonCode::ConversationExcluded,
                reservation_allowed: false,
            },
            Case {
                name: "delegation optional",
                mutate: |value| value.delegation_required = false,
                decision: AdmissionDecision::Allow,
                reason_code: RuntimePolicyReasonCode::DelegationNotRequired,
                reservation_allowed: false,
            },
            Case {
                name: "capability unsupported",
                mutate: |value| value.capability_supported = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::CapabilityUnsupported,
                reservation_allowed: false,
            },
            Case {
                name: "runtime unhealthy",
                mutate: |value| value.runtime_healthy = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::RuntimeUnhealthy,
                reservation_allowed: false,
            },
            Case {
                name: "permission denied",
                mutate: |value| value.permission_allowed = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::PermissionDenied,
                reservation_allowed: false,
            },
            Case {
                name: "scope denied",
                mutate: |value| value.scope_allowed = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::ScopeDenied,
                reservation_allowed: false,
            },
            Case {
                name: "schedule uncertain",
                mutate: |value| value.schedule_certain = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::ScheduleUncertain,
                reservation_allowed: false,
            },
            Case {
                name: "lease uncertain",
                mutate: |value| value.lease_certain = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::LeaseUncertain,
                reservation_allowed: false,
            },
            Case {
                name: "receipt uncertain",
                mutate: |value| value.receipt_certain = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::ReceiptUncertain,
                reservation_allowed: false,
            },
            Case {
                name: "schema unverified",
                mutate: |value| value.schema_verified = false,
                decision: AdmissionDecision::Deny,
                reason_code: RuntimePolicyReasonCode::SchemaUnverified,
                reservation_allowed: false,
            },
            Case {
                name: "all safe",
                mutate: |_| {},
                decision: AdmissionDecision::Allow,
                reason_code: RuntimePolicyReasonCode::AdmissionAllowed,
                reservation_allowed: true,
            },
        ];

        for case in cases {
            let mut value = input();
            (case.mutate)(&mut value);
            assert_eq!(
                evaluate_runtime_policy(&value),
                RuntimePolicyOutcome {
                    decision: case.decision,
                    reason_code: case.reason_code,
                    reservation_allowed: case.reservation_allowed,
                },
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn primary_fallback_warns_for_each_safety_failure() {
        let failures: [fn(&mut RuntimePolicyInput); 9] = [
            |value| value.capability_supported = false,
            |value| value.runtime_healthy = false,
            |value| value.permission_allowed = false,
            |value| value.scope_allowed = false,
            |value| value.schedule_certain = false,
            |value| value.lease_certain = false,
            |value| value.receipt_certain = false,
            |value| value.schema_verified = false,
            |value| {
                value.capability_supported = false;
                value.schema_verified = false;
            },
        ];

        for failure in failures {
            let mut value = input();
            value.failure_policy = OrchestrationFailurePolicy::PrimaryFallback;
            failure(&mut value);
            let result = evaluate_runtime_policy(&value);
            assert_eq!(result.decision, AdmissionDecision::Warn);
            assert!(!result.reservation_allowed);
        }
    }

    #[test]
    fn priority_is_frozen_and_exclusions_never_reserve() {
        let mut value = input();
        value.mode = RuntimePolicyMode::Default;
        value.workspace_excluded = true;
        value.capability_supported = false;
        assert_eq!(
            evaluate_runtime_policy(&value).reason_code,
            RuntimePolicyReasonCode::DefaultMode
        );

        let mut value = input();
        value.workspace_excluded = true;
        value.project_excluded = true;
        value.conversation_excluded = true;
        value.capability_supported = false;
        let result = evaluate_runtime_policy(&value);
        assert_eq!(
            result.reason_code,
            RuntimePolicyReasonCode::WorkspaceExcluded
        );
        assert!(!result.reservation_allowed);

        for mutate in [
            |value: &mut RuntimePolicyInput| value.workspace_excluded = true,
            |value: &mut RuntimePolicyInput| value.project_excluded = true,
            |value: &mut RuntimePolicyInput| value.conversation_excluded = true,
        ] {
            let mut value = input();
            mutate(&mut value);
            assert!(!evaluate_runtime_policy(&value).reservation_allowed);
        }
    }

    #[test]
    fn priority_is_frozen_across_all_policy_groups() {
        let mut value = input();
        value.mode = RuntimePolicyMode::Default;
        value.workspace_excluded = true;
        value.project_excluded = true;
        value.conversation_excluded = true;
        value.delegation_required = false;
        value.capability_supported = false;
        value.runtime_healthy = false;
        value.permission_allowed = false;
        value.scope_allowed = false;
        value.schedule_certain = false;
        value.lease_certain = false;
        value.receipt_certain = false;
        value.schema_verified = false;

        let steps: [(RuntimePolicyReasonCode, fn(&mut RuntimePolicyInput)); 14] = [
            (RuntimePolicyReasonCode::DefaultMode, |value| {
                value.mode = RuntimePolicyMode::Orchestration
            }),
            (RuntimePolicyReasonCode::WorkspaceExcluded, |value| {
                value.workspace_excluded = false
            }),
            (RuntimePolicyReasonCode::ProjectExcluded, |value| {
                value.project_excluded = false
            }),
            (RuntimePolicyReasonCode::ConversationExcluded, |value| {
                value.conversation_excluded = false
            }),
            (RuntimePolicyReasonCode::DelegationNotRequired, |value| {
                value.delegation_required = true
            }),
            (RuntimePolicyReasonCode::CapabilityUnsupported, |value| {
                value.capability_supported = true
            }),
            (RuntimePolicyReasonCode::RuntimeUnhealthy, |value| {
                value.runtime_healthy = true
            }),
            (RuntimePolicyReasonCode::PermissionDenied, |value| {
                value.permission_allowed = true
            }),
            (RuntimePolicyReasonCode::ScopeDenied, |value| {
                value.scope_allowed = true
            }),
            (RuntimePolicyReasonCode::ScheduleUncertain, |value| {
                value.schedule_certain = true
            }),
            (RuntimePolicyReasonCode::LeaseUncertain, |value| {
                value.lease_certain = true
            }),
            (RuntimePolicyReasonCode::ReceiptUncertain, |value| {
                value.receipt_certain = true
            }),
            (RuntimePolicyReasonCode::SchemaUnverified, |value| {
                value.schema_verified = true
            }),
            (RuntimePolicyReasonCode::AdmissionAllowed, |_| {}),
        ];

        for (reason, clear_current_failure) in steps {
            assert_eq!(evaluate_runtime_policy(&value).reason_code, reason);
            clear_current_failure(&mut value);
        }
    }

    #[test]
    fn same_input_always_has_same_result() {
        let value = input();
        let first = evaluate_runtime_policy(&value);
        for _ in 0..10 {
            assert_eq!(evaluate_runtime_policy(&value), first);
        }
    }
}
