#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const TASK_PACKET_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum PermissionPolicy {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
    Inherit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ExecutionKindPolicy {
    NativeChildRequired,
    NativeChildOrManagedWorker,
    ManagedWorkerRequired,
}

impl ExecutionKindPolicy {
    pub(crate) fn allows(self, execution_kind: ExecutionKind) -> bool {
        match self {
            Self::NativeChildRequired => execution_kind == ExecutionKind::NativeChild,
            Self::NativeChildOrManagedWorker => matches!(
                execution_kind,
                ExecutionKind::NativeChild | ExecutionKind::ManagedWorker
            ),
            Self::ManagedWorkerRequired => execution_kind == ExecutionKind::ManagedWorker,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum OutputContract {
    StandardV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReviewPolicy {
    PrimaryRequired,
    PrimaryWithReadOnlyReviewer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ExecutionKind {
    NativeChild,
    ManagedWorker,
    ObservedExternal,
}

impl ExecutionKind {
    pub(crate) fn is_dispatchable(self) -> bool {
        !matches!(self, Self::ObservedExternal)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ThreadReuseState {
    Active,
    HeldForReview,
    RetirePending,
    Retired,
}

impl ThreadReuseState {
    pub(crate) fn is_reusable(self) -> bool {
        self == Self::Active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum RouteAction {
    Reuse,
    Spawn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TaskPacket {
    pub(crate) schema_version: u32,
    pub(crate) job_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) agent_id: String,
    pub(crate) parent_thread_id: String,
    pub(crate) workspace_scope_key: String,
    pub(crate) task_scope_key: String,
    pub(crate) objective: String,
    pub(crate) allowed_scope: Vec<String>,
    pub(crate) constraints: Vec<String>,
    pub(crate) success_criteria: Vec<String>,
    pub(crate) allowed_tools: Vec<String>,
    pub(crate) permission_policy: PermissionPolicy,
    pub(crate) execution_kind_policy: ExecutionKindPolicy,
    pub(crate) context_references: Vec<String>,
    pub(crate) output_contract: OutputContract,
    pub(crate) review_policy: ReviewPolicy,
}

impl TaskPacket {
    /// 校验 TaskPacket 是否满足冻结契约（设计方案 §24.3）。
    /// 按固定顺序检查，首个失败立即返回；只做只读检查，不回写 trim 后的值。
    pub(crate) fn validate(&self) -> Result<(), OrchestrationError> {
        // 1. schema_version 必须与冻结版本一致
        if self.schema_version != TASK_PACKET_SCHEMA_VERSION {
            return Err(task_packet_error(
                OrchestrationErrorCode::TaskPacketFieldInvalid,
                "schema_version 与冻结契约版本不一致",
                "schema_version",
            ));
        }

        // 2. ID 类字段：去除首尾空白后必须非空
        for (field, value) in [
            ("job_id", &self.job_id),
            ("agent_id", &self.agent_id),
            ("parent_thread_id", &self.parent_thread_id),
        ] {
            if value.trim().is_empty() {
                return Err(task_packet_error(
                    OrchestrationErrorCode::TaskPacketFieldRequired,
                    &format!("{field} 不能为空"),
                    field,
                ));
            }
        }

        // 3. workspace_scope_key：去除首尾空白后必须非空
        if self.workspace_scope_key.trim().is_empty() {
            return Err(task_packet_error(
                OrchestrationErrorCode::TaskPacketFieldRequired,
                "workspace_scope_key 不能为空",
                "workspace_scope_key",
            ));
        }

        // 4. idempotency_key：1~128 字符且匹配 ^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$
        if !is_valid_idempotency_key(&self.idempotency_key) {
            return Err(task_packet_error(
                OrchestrationErrorCode::TaskPacketFieldInvalid,
                "idempotency_key 格式非法",
                "idempotency_key",
            ));
        }

        // 5. task_scope_key：匹配 ^[a-z0-9][a-z0-9_-]{0,63}$
        if !is_valid_task_scope_key(&self.task_scope_key) {
            return Err(task_packet_error(
                OrchestrationErrorCode::TaskPacketFieldInvalid,
                "task_scope_key 格式非法",
                "task_scope_key",
            ));
        }

        // 6. objective：去除首尾空白后必须非空
        if self.objective.trim().is_empty() {
            return Err(task_packet_error(
                OrchestrationErrorCode::TaskPacketFieldRequired,
                "objective 不能为空",
                "objective",
            ));
        }

        // 7~11. 数组字段：allowed_scope 与 success_criteria 不允许为空数组，
        // constraints / allowed_tools / context_references 允许为空数组；
        // 任何成员去除首尾空白后不得为空
        validate_string_list(&self.allowed_scope, "allowed_scope", true)?;
        validate_string_list(&self.constraints, "constraints", false)?;
        validate_string_list(&self.success_criteria, "success_criteria", true)?;
        validate_string_list(&self.allowed_tools, "allowed_tools", false)?;
        validate_string_list(&self.context_references, "context_references", false)?;

        Ok(())
    }

    /// 生成 Canonical Form：UTF-8 紧凑 JSON，对象键递归按字典序（byte 序）排列，
    /// 数组顺序与字符串内容保持原样。序列化失败视为规范化失败。
    pub(crate) fn canonical_form(&self) -> Result<String, OrchestrationError> {
        // 先转成 serde_json::Value，再手动递归写出，
        // 不依赖 serde_json Map 的默认排序行为（避免 preserve_order feature 差异）
        let value = serde_json::to_value(self).map_err(|_| task_packet_canonicalization_error())?;
        let mut out = String::new();
        write_canonical_value(&value, &mut out);
        Ok(out)
    }

    /// Canonical Form 字符串的 SHA-256 小写十六进制摘要（固定 64 字符）。
    pub(crate) fn task_packet_hash(&self) -> Result<String, OrchestrationError> {
        let canonical = self.canonical_form()?;
        let digest = Sha256::digest(canonical.as_bytes());
        // 逐字节转两位小写十六进制，保证输出恒为 64 个 [0-9a-f] 字符
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

/// 构造 TaskPacket 字段级校验错误；job_id / attempt_id 由上层补充，这里固定为 None。
fn task_packet_error(
    code: OrchestrationErrorCode,
    message: &str,
    field_path: &str,
) -> OrchestrationError {
    OrchestrationError {
        code,
        message: message.to_owned(),
        field_path: Some(field_path.to_owned()),
        job_id: None,
        attempt_id: None,
    }
}

/// 构造 Canonical Form 序列化失败错误。
fn task_packet_canonicalization_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::TaskPacketCanonicalizationFailed,
        message: "TaskPacket 序列化为规范形式失败".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

/// 匹配 ^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$（即 1~128 字符）。
/// 字符类均为 ASCII，非 ASCII 输入必然不匹配，按字节判断即可。
fn is_valid_idempotency_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    match bytes.first() {
        Some(&first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    bytes.len() <= 128
        && bytes[1..]
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// 匹配 ^[a-z0-9][a-z0-9_-]{0,63}$（即 1~64 字符）。
fn is_valid_task_scope_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    match bytes.first() {
        Some(&first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    bytes.len() <= 64
        && bytes[1..]
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// 校验字符串数组字段：required 为 true 时不允许空数组；任何成员去除首尾空白后不得为空，
/// 失败时 field_path 形如 "allowed_scope[1]"。
fn validate_string_list(
    items: &[String],
    field: &str,
    required: bool,
) -> Result<(), OrchestrationError> {
    if required && items.is_empty() {
        return Err(task_packet_error(
            OrchestrationErrorCode::TaskPacketFieldRequired,
            &format!("{field} 不能为空数组"),
            field,
        ));
    }
    for (index, item) in items.iter().enumerate() {
        if item.trim().is_empty() {
            let indexed_path = format!("{field}[{index}]");
            return Err(task_packet_error(
                OrchestrationErrorCode::TaskPacketFieldInvalid,
                &format!("{indexed_path} 不能为空白"),
                &indexed_path,
            ));
        }
    }
    Ok(())
}

/// 递归写出 Canonical Form：对象键收集后按字典序（byte 序）排序再输出，
/// 紧凑输出无多余空白；数组顺序与字符串内容保持原样；
/// 字符串统一用 serde_json 转义，数字直接紧凑写出（标准十进制）。
fn write_canonical_value(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(String::as_str).collect();
            keys.sort_unstable();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(*key).expect("字符串键序列化不会失败"));
                out.push(':');
                write_canonical_value(map.get(*key).expect("键来自 map 本身，必存在"), out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical_value(item, out);
            }
            out.push(']');
        }
        // 字符串 / 数字 / 布尔 / null：直接紧凑序列化
        _ => out.push_str(&serde_json::to_string(value).expect("标量序列化不会失败")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum JobState {
    Created,
    Routed,
    Claimed,
    Dispatched,
    Running,
    ResultReceived,
    ReviewPending,
    RevisionRequired,
    Approved,
    Completed,
    Waiting,
    Uncertain,
    Blocked,
    Failed,
    Cancelled,
    Rejected,
}

impl JobState {
    pub(crate) fn can_transition_to(self, next: Self) -> bool {
        use JobState::*;

        matches!(
            (self, next),
            (Created, Routed | Waiting | Blocked | Failed | Cancelled)
                | (
                    Routed,
                    Claimed | Waiting | Blocked | Uncertain | Failed | Cancelled
                )
                | (Claimed, Dispatched | Routed | Failed | Cancelled)
                | (
                    Dispatched,
                    Running | ResultReceived | Uncertain | Failed | Cancelled
                )
                | (Running, ResultReceived | Uncertain | Failed | Cancelled)
                | (ResultReceived, ReviewPending)
                | (
                    ReviewPending,
                    Approved | RevisionRequired | Rejected | Cancelled
                )
                | (
                    RevisionRequired,
                    Routed | Waiting | Blocked | Failed | Cancelled
                )
                | (Approved, Completed)
                | (Waiting, Routed | Blocked | Failed | Cancelled)
                | (
                    Uncertain,
                    Routed | Running | ResultReceived | Failed | Cancelled
                )
        )
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Blocked | Self::Failed | Self::Cancelled | Self::Rejected
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum AttemptState {
    Planned,
    Dispatching,
    Accepted,
    Running,
    Succeeded,
    Uncertain,
    Failed,
    Cancelled,
}

impl AttemptState {
    pub(crate) fn can_transition_to(self, next: Self) -> bool {
        use AttemptState::*;

        matches!(
            (self, next),
            (Planned, Dispatching | Failed | Cancelled)
                | (Dispatching, Accepted | Uncertain | Failed | Cancelled)
                | (
                    Accepted,
                    Running | Succeeded | Uncertain | Failed | Cancelled
                )
                | (Running, Succeeded | Uncertain | Failed | Cancelled)
                | (
                    Uncertain,
                    Accepted | Running | Succeeded | Failed | Cancelled
                )
        )
    }

    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OrchestrationJob {
    pub(crate) job_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) task_packet: TaskPacket,
    pub(crate) task_packet_hash: String,
    pub(crate) agent_id: String,
    pub(crate) parent_thread_id: String,
    pub(crate) workspace_scope_key: String,
    pub(crate) task_scope_key: String,
    pub(crate) state: JobState,
    pub(crate) last_error_code: Option<OrchestrationErrorCode>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) terminal_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JobAttempt {
    pub(crate) attempt_id: String,
    pub(crate) job_id: String,
    pub(crate) attempt_no: u32,
    pub(crate) previous_attempt_id: Option<String>,
    pub(crate) schedule_decision_id: String,
    pub(crate) lease_id: String,
    pub(crate) route_action: RouteAction,
    pub(crate) planned_execution_kind: ExecutionKind,
    pub(crate) execution_kind: Option<ExecutionKind>,
    pub(crate) thread_instance_id: Option<String>,
    pub(crate) codex_turn_id: Option<String>,
    pub(crate) state: AttemptState,
    pub(crate) recovery_count: u32,
    pub(crate) last_error_code: Option<OrchestrationErrorCode>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) dispatch_recorded_at: Option<String>,
    pub(crate) accepted_at: Option<String>,
    pub(crate) terminal_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReceiptStage {
    DispatchRecorded,
    TurnAccepted,
    ResultObserved,
    ParentAcknowledged,
}

/// Receipt 的查询态；`UNKNOWN` 只表示尚无持久化阶段，不能写入数据库。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReceiptProgress {
    Unknown,
    DispatchRecorded,
    TurnAccepted,
    ResultObserved,
    ParentAcknowledged,
}

impl From<Option<ReceiptStage>> for ReceiptProgress {
    fn from(stage: Option<ReceiptStage>) -> Self {
        match stage {
            None => Self::Unknown,
            Some(ReceiptStage::DispatchRecorded) => Self::DispatchRecorded,
            Some(ReceiptStage::TurnAccepted) => Self::TurnAccepted,
            Some(ReceiptStage::ResultObserved) => Self::ResultObserved,
            Some(ReceiptStage::ParentAcknowledged) => Self::ParentAcknowledged,
        }
    }
}

impl ReceiptStage {
    pub(crate) fn can_append_after(self, current: Option<Self>) -> bool {
        matches!(
            (current, self),
            (None, Self::DispatchRecorded)
                | (Some(Self::DispatchRecorded), Self::TurnAccepted)
                | (Some(Self::TurnAccepted), Self::ResultObserved)
                | (Some(Self::ResultObserved), Self::ParentAcknowledged)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReceiptEvidenceSource {
    CasTransaction,
    AppServerResponse,
    NativeParentChildEvent,
    NativeStateDb,
    RuntimeEvent,
    RecoveryRead,
    PrimaryReview,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeliveryReceipt {
    pub(crate) receipt_id: String,
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    pub(crate) stage: ReceiptStage,
    pub(crate) execution_kind: Option<ExecutionKind>,
    pub(crate) evidence_source: ReceiptEvidenceSource,
    pub(crate) evidence_ref: String,
    pub(crate) parent_thread_id: String,
    pub(crate) codex_thread_id: Option<String>,
    pub(crate) codex_turn_id: Option<String>,
    pub(crate) schema_profile: Option<String>,
    pub(crate) evidence_at: String,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ReviewOutcome {
    Approve,
    RevisionRequired,
    Reject,
}

impl ReviewOutcome {
    pub(crate) fn job_state(self) -> JobState {
        match self {
            Self::Approve => JobState::Approved,
            Self::RevisionRequired => JobState::RevisionRequired,
            Self::Reject => JobState::Rejected,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ReviewDecision {
    pub(crate) review_id: String,
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    pub(crate) decision: ReviewOutcome,
    pub(crate) reviewer_thread_id: String,
    pub(crate) reason: String,
    pub(crate) evidence_refs: Vec<String>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum IdempotencyOutcome {
    Created,
    ExistingNotDispatched,
    ExistingKnown,
    ExistingUncertain,
    KeyConflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum OrchestrationErrorCode {
    TaskPacketFieldRequired,
    TaskPacketFieldInvalid,
    TaskPacketScopeMismatch,
    TaskPacketCanonicalizationFailed,
    IdempotencyKeyConflict,
    AgentNotExecutable,
    ScopeExcluded,
    ExecutionKindUnsupported,
    RuntimeUnavailable,
    SchemaUnverified,
    PermissionDenied,
    ConcurrencyLimitReached,
    StaleExpectedDecision,
    StaleExpectedCandidate,
    ActiveTurnExists,
    DispatchRejected,
    DispatchOutcomeUnknown,
    ThreadIdMissing,
    TurnIdMissing,
    ThreadIdMismatch,
    TurnIdMismatch,
    NativeParentChildEvidenceMissing,
    ExecutionKindMismatch,
    RecoveryRequired,
    RecoveryLimitReached,
    ResultNotObserved,
    ReviewRequired,
    ReviewDecisionConflict,
    AttemptNotCurrent,
    InvalidStateTransition,
    CancellationUnconfirmed,
    PersistenceError,
    InternalInvariantViolation,
}

impl OrchestrationErrorCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::TaskPacketFieldRequired => "TASK_PACKET_FIELD_REQUIRED",
            Self::TaskPacketFieldInvalid => "TASK_PACKET_FIELD_INVALID",
            Self::TaskPacketScopeMismatch => "TASK_PACKET_SCOPE_MISMATCH",
            Self::TaskPacketCanonicalizationFailed => "TASK_PACKET_CANONICALIZATION_FAILED",
            Self::IdempotencyKeyConflict => "IDEMPOTENCY_KEY_CONFLICT",
            Self::AgentNotExecutable => "AGENT_NOT_EXECUTABLE",
            Self::ScopeExcluded => "SCOPE_EXCLUDED",
            Self::ExecutionKindUnsupported => "EXECUTION_KIND_UNSUPPORTED",
            Self::RuntimeUnavailable => "RUNTIME_UNAVAILABLE",
            Self::SchemaUnverified => "SCHEMA_UNVERIFIED",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::ConcurrencyLimitReached => "CONCURRENCY_LIMIT_REACHED",
            Self::StaleExpectedDecision => "STALE_EXPECTED_DECISION",
            Self::StaleExpectedCandidate => "STALE_EXPECTED_CANDIDATE",
            Self::ActiveTurnExists => "ACTIVE_TURN_EXISTS",
            Self::DispatchRejected => "DISPATCH_REJECTED",
            Self::DispatchOutcomeUnknown => "DISPATCH_OUTCOME_UNKNOWN",
            Self::ThreadIdMissing => "THREAD_ID_MISSING",
            Self::TurnIdMissing => "TURN_ID_MISSING",
            Self::ThreadIdMismatch => "THREAD_ID_MISMATCH",
            Self::TurnIdMismatch => "TURN_ID_MISMATCH",
            Self::NativeParentChildEvidenceMissing => "NATIVE_PARENT_CHILD_EVIDENCE_MISSING",
            Self::ExecutionKindMismatch => "EXECUTION_KIND_MISMATCH",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::RecoveryLimitReached => "RECOVERY_LIMIT_REACHED",
            Self::ResultNotObserved => "RESULT_NOT_OBSERVED",
            Self::ReviewRequired => "REVIEW_REQUIRED",
            Self::ReviewDecisionConflict => "REVIEW_DECISION_CONFLICT",
            Self::AttemptNotCurrent => "ATTEMPT_NOT_CURRENT",
            Self::InvalidStateTransition => "INVALID_STATE_TRANSITION",
            Self::CancellationUnconfirmed => "CANCELLATION_UNCONFIRMED",
            Self::PersistenceError => "PERSISTENCE_ERROR",
            Self::InternalInvariantViolation => "INTERNAL_INVARIANT_VIOLATION",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OrchestrationError {
    pub(crate) code: OrchestrationErrorCode,
    pub(crate) message: String,
    pub(crate) field_path: Option<String>,
    pub(crate) job_id: Option<String>,
    pub(crate) attempt_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::*;

    const JOB_STATES: [JobState; 16] = [
        JobState::Created,
        JobState::Routed,
        JobState::Claimed,
        JobState::Dispatched,
        JobState::Running,
        JobState::ResultReceived,
        JobState::ReviewPending,
        JobState::RevisionRequired,
        JobState::Approved,
        JobState::Completed,
        JobState::Waiting,
        JobState::Uncertain,
        JobState::Blocked,
        JobState::Failed,
        JobState::Cancelled,
        JobState::Rejected,
    ];

    const ATTEMPT_STATES: [AttemptState; 8] = [
        AttemptState::Planned,
        AttemptState::Dispatching,
        AttemptState::Accepted,
        AttemptState::Running,
        AttemptState::Succeeded,
        AttemptState::Uncertain,
        AttemptState::Failed,
        AttemptState::Cancelled,
    ];

    #[test]
    fn task_packet_serialization_freezes_field_and_enum_names() {
        let packet = TaskPacket {
            schema_version: TASK_PACKET_SCHEMA_VERSION,
            job_id: "job-1".to_owned(),
            idempotency_key: "r0-02".to_owned(),
            agent_id: "agent-1".to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            workspace_scope_key: "workspace-1".to_owned(),
            task_scope_key: "r0_02".to_owned(),
            objective: "冻结内部契约".to_owned(),
            allowed_scope: vec!["src-tauri/src/orchestration_contract.rs".to_owned()],
            constraints: vec!["不进入 A-01".to_owned()],
            success_criteria: vec!["状态迁移测试通过".to_owned()],
            allowed_tools: Vec::new(),
            permission_policy: PermissionPolicy::WorkspaceWrite,
            execution_kind_policy: ExecutionKindPolicy::NativeChildRequired,
            context_references: Vec::new(),
            output_contract: OutputContract::StandardV1,
            review_policy: ReviewPolicy::PrimaryRequired,
        };

        let value = serde_json::to_value(packet).unwrap();
        let actual = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let expected = [
            "schema_version",
            "job_id",
            "idempotency_key",
            "agent_id",
            "parent_thread_id",
            "workspace_scope_key",
            "task_scope_key",
            "objective",
            "allowed_scope",
            "constraints",
            "success_criteria",
            "allowed_tools",
            "permission_policy",
            "execution_kind_policy",
            "context_references",
            "output_contract",
            "review_policy",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

        assert_eq!(actual, expected);
        assert_eq!(value["schema_version"], json!(1));
        assert_eq!(value["permission_policy"], json!("WORKSPACE_WRITE"));
        assert_eq!(
            value["execution_kind_policy"],
            json!("NATIVE_CHILD_REQUIRED")
        );
        assert_eq!(value["output_contract"], json!("STANDARD_V1"));
        assert_eq!(value["review_policy"], json!("PRIMARY_REQUIRED"));
    }

    #[test]
    fn job_state_machine_matches_frozen_transitions() {
        use JobState::*;

        let allowed = [
            (Created, Routed),
            (Created, Waiting),
            (Created, Blocked),
            (Created, Failed),
            (Created, Cancelled),
            (Routed, Claimed),
            (Routed, Waiting),
            (Routed, Blocked),
            (Routed, Uncertain),
            (Routed, Failed),
            (Routed, Cancelled),
            (Claimed, Dispatched),
            (Claimed, Routed),
            (Claimed, Failed),
            (Claimed, Cancelled),
            (Dispatched, Running),
            (Dispatched, ResultReceived),
            (Dispatched, Uncertain),
            (Dispatched, Failed),
            (Dispatched, Cancelled),
            (Running, ResultReceived),
            (Running, Uncertain),
            (Running, Failed),
            (Running, Cancelled),
            (ResultReceived, ReviewPending),
            (ReviewPending, Approved),
            (ReviewPending, RevisionRequired),
            (ReviewPending, Rejected),
            (ReviewPending, Cancelled),
            (RevisionRequired, Routed),
            (RevisionRequired, Waiting),
            (RevisionRequired, Blocked),
            (RevisionRequired, Failed),
            (RevisionRequired, Cancelled),
            (Approved, Completed),
            (Waiting, Routed),
            (Waiting, Blocked),
            (Waiting, Failed),
            (Waiting, Cancelled),
            (Uncertain, Routed),
            (Uncertain, Running),
            (Uncertain, ResultReceived),
            (Uncertain, Failed),
            (Uncertain, Cancelled),
        ];

        for from in JOB_STATES {
            for to in JOB_STATES {
                assert_eq!(
                    from.can_transition_to(to),
                    allowed.contains(&(from, to)),
                    "unexpected Job transition: {from:?} -> {to:?}"
                );
            }
        }

        assert!(Completed.is_terminal());
        assert!(Blocked.is_terminal());
        assert!(Failed.is_terminal());
        assert!(Cancelled.is_terminal());
        assert!(Rejected.is_terminal());
        assert!(!Uncertain.is_terminal());
    }

    #[test]
    fn attempt_state_machine_matches_frozen_transitions() {
        use AttemptState::*;

        let allowed = [
            (Planned, Dispatching),
            (Planned, Failed),
            (Planned, Cancelled),
            (Dispatching, Accepted),
            (Dispatching, Uncertain),
            (Dispatching, Failed),
            (Dispatching, Cancelled),
            (Accepted, Running),
            (Accepted, Succeeded),
            (Accepted, Uncertain),
            (Accepted, Failed),
            (Accepted, Cancelled),
            (Running, Succeeded),
            (Running, Uncertain),
            (Running, Failed),
            (Running, Cancelled),
            (Uncertain, Accepted),
            (Uncertain, Running),
            (Uncertain, Succeeded),
            (Uncertain, Failed),
            (Uncertain, Cancelled),
        ];

        for from in ATTEMPT_STATES {
            for to in ATTEMPT_STATES {
                assert_eq!(
                    from.can_transition_to(to),
                    allowed.contains(&(from, to)),
                    "unexpected Attempt transition: {from:?} -> {to:?}"
                );
            }
        }

        assert!(Succeeded.is_terminal());
        assert!(Failed.is_terminal());
        assert!(Cancelled.is_terminal());
        assert!(!Uncertain.is_terminal());
    }

    #[test]
    fn receipt_stages_only_append_in_order() {
        assert!(ReceiptStage::DispatchRecorded.can_append_after(None));
        assert!(ReceiptStage::TurnAccepted.can_append_after(Some(ReceiptStage::DispatchRecorded)));
        assert!(ReceiptStage::ResultObserved.can_append_after(Some(ReceiptStage::TurnAccepted)));
        assert!(
            ReceiptStage::ParentAcknowledged.can_append_after(Some(ReceiptStage::ResultObserved))
        );

        assert!(
            !ReceiptStage::ResultObserved.can_append_after(Some(ReceiptStage::DispatchRecorded))
        );
        assert!(!ReceiptStage::TurnAccepted.can_append_after(Some(ReceiptStage::TurnAccepted)));
        assert!(!ReceiptStage::ParentAcknowledged.can_append_after(None));
    }

    #[test]
    fn execution_identity_and_review_boundaries_are_frozen() {
        assert!(ExecutionKind::NativeChild.is_dispatchable());
        assert!(ExecutionKind::ManagedWorker.is_dispatchable());
        assert!(!ExecutionKind::ObservedExternal.is_dispatchable());
        assert!(!ExecutionKindPolicy::NativeChildRequired.allows(ExecutionKind::ManagedWorker));
        assert!(
            ExecutionKindPolicy::NativeChildOrManagedWorker.allows(ExecutionKind::ManagedWorker)
        );
        assert!(
            !ExecutionKindPolicy::NativeChildOrManagedWorker
                .allows(ExecutionKind::ObservedExternal)
        );
        assert!(ThreadReuseState::Active.is_reusable());
        assert!(!ThreadReuseState::HeldForReview.is_reusable());
        assert_eq!(
            serde_json::to_string(&ThreadReuseState::HeldForReview).unwrap(),
            "\"HELD_FOR_REVIEW\""
        );

        assert_eq!(ReviewOutcome::Approve.job_state(), JobState::Approved);
        assert_eq!(
            ReviewOutcome::RevisionRequired.job_state(),
            JobState::RevisionRequired
        );
        assert_eq!(ReviewOutcome::Reject.job_state(), JobState::Rejected);
    }

    #[test]
    fn stable_result_and_error_codes_use_frozen_names() {
        assert_eq!(
            serde_json::to_string(&IdempotencyOutcome::ExistingUncertain).unwrap(),
            "\"EXISTING_UNCERTAIN\""
        );
        assert_eq!(
            serde_json::to_string(&OrchestrationErrorCode::IdempotencyKeyConflict).unwrap(),
            "\"IDEMPOTENCY_KEY_CONFLICT\""
        );
        assert_eq!(
            serde_json::to_string(&ReceiptEvidenceSource::NativeParentChildEvent).unwrap(),
            "\"NATIVE_PARENT_CHILD_EVENT\""
        );
    }

    /// 复用既有测试 task_packet_serialization_freezes_field_and_enum_names 的合法样例构造方式
    fn sample_packet() -> TaskPacket {
        TaskPacket {
            schema_version: TASK_PACKET_SCHEMA_VERSION,
            job_id: "job-1".to_owned(),
            idempotency_key: "r0-02".to_owned(),
            agent_id: "agent-1".to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            workspace_scope_key: "workspace-1".to_owned(),
            task_scope_key: "r0_02".to_owned(),
            objective: "冻结内部契约".to_owned(),
            allowed_scope: vec!["src-tauri/src/orchestration_contract.rs".to_owned()],
            constraints: vec!["不进入 A-01".to_owned()],
            success_criteria: vec!["状态迁移测试通过".to_owned()],
            allowed_tools: Vec::new(),
            permission_policy: PermissionPolicy::WorkspaceWrite,
            execution_kind_policy: ExecutionKindPolicy::NativeChildRequired,
            context_references: Vec::new(),
            output_contract: OutputContract::StandardV1,
            review_policy: ReviewPolicy::PrimaryRequired,
        }
    }

    #[test]
    fn valid_task_packet_passes_validation_and_hash() {
        let packet = sample_packet();
        packet.validate().expect("合法 packet 应通过校验");

        let canonical = packet.canonical_form().expect("canonical_form 应成功");
        // 对象键按字典序排列：首个键是 agent_id，最后一个键是 workspace_scope_key
        assert!(canonical.starts_with("{\"agent_id\":"));
        assert!(canonical.ends_with("\"workspace_scope_key\":\"workspace-1\"}"));

        let hash = packet.task_packet_hash().expect("task_packet_hash 应成功");
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn blank_objective_is_rejected_as_required() {
        for objective in ["", "   \t "] {
            let mut packet = sample_packet();
            packet.objective = objective.to_owned();
            let err = packet.validate().expect_err("objective 为空应校验失败");
            assert_eq!(err.code, OrchestrationErrorCode::TaskPacketFieldRequired);
            assert_eq!(err.field_path.as_deref(), Some("objective"));
        }
    }

    #[test]
    fn empty_success_criteria_is_rejected_as_required() {
        let mut packet = sample_packet();
        packet.success_criteria = Vec::new();
        let err = packet
            .validate()
            .expect_err("success_criteria 为空数组应校验失败");
        assert_eq!(err.code, OrchestrationErrorCode::TaskPacketFieldRequired);
        assert_eq!(err.field_path.as_deref(), Some("success_criteria"));
    }

    #[test]
    fn invalid_task_scope_key_is_rejected() {
        // 含大写字母 / 下划线开头均不匹配 ^[a-z0-9][a-z0-9_-]{0,63}$
        for key in ["R0_02", "_r0_02"] {
            let mut packet = sample_packet();
            packet.task_scope_key = key.to_owned();
            let err = packet
                .validate()
                .expect_err("task_scope_key 非法应校验失败");
            assert_eq!(err.code, OrchestrationErrorCode::TaskPacketFieldInvalid);
            assert_eq!(err.field_path.as_deref(), Some("task_scope_key"));
        }
    }

    #[test]
    fn idempotency_key_with_leading_dash_is_rejected() {
        let mut packet = sample_packet();
        packet.idempotency_key = "-r0-02".to_owned();
        let err = packet
            .validate()
            .expect_err("idempotency_key 首字符非法应校验失败");
        assert_eq!(err.code, OrchestrationErrorCode::TaskPacketFieldInvalid);
        assert_eq!(err.field_path.as_deref(), Some("idempotency_key"));
    }

    #[test]
    fn blank_allowed_scope_member_is_rejected_with_index() {
        let mut packet = sample_packet();
        packet.allowed_scope = vec!["src-tauri/src/lib.rs".to_owned(), "   ".to_owned()];
        let err = packet
            .validate()
            .expect_err("allowed_scope 成员为空白应校验失败");
        assert_eq!(err.code, OrchestrationErrorCode::TaskPacketFieldInvalid);
        assert_eq!(err.field_path.as_deref(), Some("allowed_scope[1]"));
    }

    #[test]
    fn wrong_schema_version_is_rejected() {
        let mut packet = sample_packet();
        packet.schema_version = 2;
        let err = packet
            .validate()
            .expect_err("schema_version 不一致应校验失败");
        assert_eq!(err.code, OrchestrationErrorCode::TaskPacketFieldInvalid);
        assert_eq!(err.field_path.as_deref(), Some("schema_version"));
    }

    #[test]
    fn canonical_form_is_independent_of_field_order() {
        // 同一内容的两个 JSON 字符串：字段键顺序相反，但字段与值完全一致
        let ordered = r#"{"schema_version":1,"job_id":"job-9","idempotency_key":"r9-01","agent_id":"agent-9","parent_thread_id":"parent-9","workspace_scope_key":"workspace-9","task_scope_key":"r9_01","objective":"验证字段顺序无关性","allowed_scope":["src/a.rs","src/b.rs"],"constraints":["仅限只读"],"success_criteria":["哈希一致"],"allowed_tools":[],"permission_policy":"READ_ONLY","execution_kind_policy":"MANAGED_WORKER_REQUIRED","context_references":[],"output_contract":"STANDARD_V1","review_policy":"PRIMARY_REQUIRED"}"#;
        let reversed = r#"{"review_policy":"PRIMARY_REQUIRED","output_contract":"STANDARD_V1","context_references":[],"execution_kind_policy":"MANAGED_WORKER_REQUIRED","permission_policy":"READ_ONLY","allowed_tools":[],"success_criteria":["哈希一致"],"constraints":["仅限只读"],"allowed_scope":["src/a.rs","src/b.rs"],"objective":"验证字段顺序无关性","task_scope_key":"r9_01","workspace_scope_key":"workspace-9","parent_thread_id":"parent-9","agent_id":"agent-9","idempotency_key":"r9-01","job_id":"job-9","schema_version":1}"#;

        let a: TaskPacket = serde_json::from_str(ordered).expect("顺序 A 应可反序列化");
        let b: TaskPacket = serde_json::from_str(reversed).expect("顺序 B 应可反序列化");

        assert_eq!(
            a.canonical_form().unwrap(),
            b.canonical_form().unwrap(),
            "字段键顺序不同但内容相同的 packet 应有相同 Canonical Form"
        );
        assert_eq!(
            a.task_packet_hash().unwrap(),
            b.task_packet_hash().unwrap(),
            "字段键顺序不同但内容相同的 packet 应有相同 hash"
        );
    }

    #[test]
    fn hash_changes_when_array_order_or_content_changes() {
        let base = sample_packet();
        let base_hash = base.task_packet_hash().unwrap();

        // 数组顺序变化 → hash 变化
        let mut reordered = sample_packet();
        reordered.allowed_scope = vec![
            "src-tauri/src/other.rs".to_owned(),
            "src-tauri/src/orchestration_contract.rs".to_owned(),
        ];
        let mut swapped = sample_packet();
        swapped.allowed_scope = vec![
            "src-tauri/src/orchestration_contract.rs".to_owned(),
            "src-tauri/src/other.rs".to_owned(),
        ];
        assert_ne!(
            reordered.task_packet_hash().unwrap(),
            swapped.task_packet_hash().unwrap(),
            "数组顺序变化应导致 hash 变化"
        );

        // 字符串内容变化 → hash 变化
        let mut modified = sample_packet();
        modified.objective = "冻结内部契约（修订）".to_owned();
        assert_ne!(
            base_hash,
            modified.task_packet_hash().unwrap(),
            "字符串内容变化应导致 hash 变化"
        );
    }
}
