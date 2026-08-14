use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use cas_native_lifecycle::{
    ThreadState as NativeThreadState, rollout_state, thread_state_from_rollout,
};
use cas_scheduler::{
    Candidate as AgentThreadCandidate, Profile as AgentSchedulingProfile,
    normalize_workspace_scope_key as canonical_workspace_scope_key, recommend as schedule_instance,
    runtime_fingerprint as shared_runtime_fingerprint,
};
#[cfg(test)]
use cas_scheduler::{cache_hint, context_pressure_limit, effective_cache_retention};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::persistence::{PersistenceError, open_database};
use crate::provider::ApiError;

pub(crate) struct UsageService {
    repository: Mutex<SqliteUsageRepository>,
}

impl UsageService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, UsageServiceError> {
        Ok(Self {
            repository: Mutex::new(SqliteUsageRepository::open(database_path)?),
        })
    }

    pub(crate) fn summary(
        &self,
        request: UsageQueryRequest,
    ) -> Result<UsageSummaryResponse, ApiError> {
        request.validate()?;
        self.repository()?.summary(&request).map_err(ApiError::from)
    }

    pub(crate) fn list(
        &self,
        request: UsageListRequest,
    ) -> Result<Vec<UsageRecordResponse>, ApiError> {
        request.query.validate()?;
        let limit = request.limit.unwrap_or(100);
        if !(1..=200).contains(&limit) {
            return Err(UsageServiceError::InvalidField("limit").into());
        }
        self.repository()?
            .list(&request.query, limit)
            .map(|records| records.into_iter().map(UsageRecordResponse::from).collect())
            .map_err(ApiError::from)
    }

    pub(crate) fn list_agent_instances(
        &self,
        request: AgentThreadInstanceListRequest,
    ) -> Result<Vec<AgentThreadInstanceResponse>, ApiError> {
        if request
            .agent_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(UsageServiceError::InvalidField("agentId").into());
        }
        let limit = request.limit.unwrap_or(50);
        if !(1..=200).contains(&limit) {
            return Err(UsageServiceError::InvalidField("limit").into());
        }
        self.repository()?
            .list_agent_instances(request.agent_id.as_deref(), limit)
            .map_err(ApiError::from)
    }

    pub(crate) fn sync_native_subagents(
        &self,
        codex_home: &Path,
    ) -> Result<NativeSubagentSyncResponse, ApiError> {
        let Some(state_path) = find_codex_state_database(codex_home) else {
            return Ok(NativeSubagentSyncResponse::unavailable(
                "未找到 Codex state_*.sqlite；尚不能同步 Primary 原生子 Agent。",
            ));
        };
        let source_path = state_path.to_string_lossy().into_owned();
        let source = match Connection::open_with_flags(
            &state_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(source) => source,
            Err(_) => {
                return Ok(NativeSubagentSyncResponse::unavailable_with_path(
                    source_path,
                    "Codex 状态库当前不可读；稍后刷新可重试。",
                ));
            }
        };
        if !native_state_schema_supported(&source) {
            return Ok(NativeSubagentSyncResponse::incompatible(
                source_path,
                "当前 Codex 状态库结构与 CAS 适配器不兼容；同步已安全停止。",
            ));
        }
        let records = match load_native_subagent_records(&source) {
            Ok(records) => records,
            Err(_) => {
                return Ok(NativeSubagentSyncResponse::incompatible(
                    source_path,
                    "读取 Codex 原生子 Agent 数据失败；同步已安全停止。",
                ));
            }
        };
        let discovered_count = records.len();
        let mut synced_count = 0;
        let mut repository = self.repository()?;
        let transaction = repository
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(UsageServiceError::from)?;
        for record in records {
            let Some((agent_id, agent_name, _configured_context_window)) =
                resolve_native_agent(&transaction, &record).map_err(UsageServiceError::from)?
            else {
                continue;
            };
            upsert_native_agent_instance(&transaction, &record, &agent_id, &agent_name)
                .map_err(UsageServiceError::from)?;
            synced_count += 1;
        }
        transaction.commit().map_err(UsageServiceError::from)?;
        Ok(NativeSubagentSyncResponse {
            capability: NativeSubagentSyncCapability::Supported,
            source_path: Some(source_path),
            discovered_count,
            synced_count,
            unmapped_count: discovered_count.saturating_sub(synced_count),
            message: "已从 Codex 本地状态同步 Primary 原生子 Agent；当前只能可靠取得总 Token，Input / Cached / Output 明细仍以 Runtime Bridge 事件为准。".to_owned(),
        })
    }

    pub(crate) fn set_agent_instance_workspace_scope(
        &self,
        request: AgentThreadInstanceWorkspaceScopeRequest,
    ) -> Result<AgentThreadInstanceResponse, ApiError> {
        let thread_id = validate_runtime_key(&request.thread_id, "threadId")?;
        let scope_key = request
            .workspace_scope_key
            .as_deref()
            .map(normalize_workspace_scope_key)
            .transpose()?;
        self.repository()?
            .set_agent_instance_workspace_scope(&thread_id, scope_key.as_deref())
            .map_err(ApiError::from)
    }

    pub(crate) fn recommend_agent_instance(
        &self,
        request: AgentThreadInstanceRecommendRequest,
    ) -> Result<AgentThreadInstanceRecommendation, ApiError> {
        let agent_id = validate_runtime_key(&request.agent_id, "agentId")?;
        let scope_key = normalize_workspace_scope_key(&request.workspace_scope_key)?;
        let parent_thread_id = request
            .parent_thread_id
            .as_deref()
            .map(|value| validate_runtime_key(value, "parentThreadId"))
            .transpose()?;
        let repository = self.repository()?;
        let profile = repository.scheduling_profile(&agent_id)?;
        let candidates =
            repository.scope_candidates(&agent_id, &scope_key, parent_thread_id.as_deref())?;
        Ok(recommend_instance(scope_key, candidates, profile))
    }

    pub(crate) fn prepare_agent_execution(
        &self,
        request: AgentThreadExecutionRequest,
    ) -> Result<AgentThreadExecutionPlan, ApiError> {
        let agent_id = validate_runtime_key(&request.agent_id, "agentId")?;
        let scope_key = normalize_workspace_scope_key(&request.workspace_scope_key)?;
        let cwd_scope = canonical_workspace_scope_key(&request.cwd)
            .ok_or(UsageServiceError::InvalidField("cwd"))?;
        if cwd_scope != scope_key {
            return Err(UsageServiceError::InvalidField("cwd").into());
        }
        if !matches!(request.expected_decision.as_str(), "REUSE" | "SPAWN") {
            return Err(UsageServiceError::InvalidField("expectedDecision").into());
        }
        let repository = self.repository()?;
        let scheduling_profile = repository.scheduling_profile(&agent_id)?;
        let candidates = repository.scope_candidates(&agent_id, &scope_key, None)?;
        let recommendation = recommend_instance(scope_key.clone(), candidates, scheduling_profile);
        if recommendation.decision != request.expected_decision
            || recommendation.candidate_thread_id != request.expected_candidate_thread_id
        {
            return Err(UsageServiceError::DecisionChanged.into());
        }
        let profile = repository
            .agent_runtime_profile(&agent_id)?
            .ok_or(UsageServiceError::AgentRuntimeUnavailable)?;
        Ok(AgentThreadExecutionPlan {
            profile,
            recommendation,
            cwd: request.cwd,
            input: request.input,
            workspace_scope_key: scope_key,
        })
    }

    pub(crate) fn register_agent_execution_thread(
        &self,
        profile: &AgentRuntimeProfile,
        thread_id: &str,
        scope_key: &str,
    ) -> Result<(), UsageServiceError> {
        self.repository()?
            .register_agent_execution_thread(profile, thread_id, scope_key, "IDLE")
    }

    pub(crate) fn mark_agent_execution_running(
        &self,
        thread_id: &str,
    ) -> Result<(), UsageServiceError> {
        self.repository()?
            .set_agent_execution_status(thread_id, "RUNNING")
    }

    pub(crate) fn mark_agent_execution_recovery_required(
        &self,
        thread_id: &str,
    ) -> Result<(), UsageServiceError> {
        self.repository()?
            .set_agent_execution_status(thread_id, "RECOVERY_REQUIRED")
    }

    pub(crate) fn mark_agent_execution_idle_if_known(
        &self,
        thread_id: &str,
    ) -> Result<(), UsageServiceError> {
        self.repository()?
            .set_agent_execution_status_if_known(thread_id, "IDLE")
    }

    pub(crate) fn upsert_snapshot(
        &self,
        snapshot: UsageSnapshot,
    ) -> Result<UsageUpsertResult, UsageServiceError> {
        snapshot.validate()?;
        self.repository()?.upsert_snapshot(snapshot)
    }

    pub(crate) fn current_timestamp(&self) -> Result<String, UsageServiceError> {
        self.repository()?.current_timestamp().map_err(Into::into)
    }

    pub(crate) fn resolve_attribution(
        &self,
        thread_id: Option<&str>,
        agent_key: Option<&str>,
        model_slug: Option<&str>,
    ) -> Result<Option<UsageAttribution>, UsageServiceError> {
        self.repository()?
            .resolve_attribution(thread_id, agent_key, model_slug)
            .map_err(Into::into)
    }

    fn repository(&self) -> Result<MutexGuard<'_, SqliteUsageRepository>, UsageServiceError> {
        self.repository
            .lock()
            .map_err(|_| UsageServiceError::DatabaseUnavailable)
    }

    #[cfg(test)]
    pub(crate) fn in_memory() -> Self {
        Self {
            repository: Mutex::new(SqliteUsageRepository::in_memory().unwrap()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsageAttribution {
    pub(crate) agent_id: String,
    pub(crate) agent_name: String,
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) model_id: String,
    pub(crate) model_name: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageQueryRequest {
    agent_id: Option<String>,
    provider_id: Option<String>,
    model_id: Option<String>,
    codex_session_id: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

impl UsageQueryRequest {
    fn validate(&self) -> Result<(), UsageServiceError> {
        for (field, value) in [
            ("agentId", self.agent_id.as_deref()),
            ("providerId", self.provider_id.as_deref()),
            ("modelId", self.model_id.as_deref()),
            ("codexSessionId", self.codex_session_id.as_deref()),
            ("from", self.from.as_deref()),
            ("to", self.to.as_deref()),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(UsageServiceError::InvalidField(field));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageListRequest {
    #[serde(flatten)]
    query: UsageQueryRequest,
    limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceListRequest {
    agent_id: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum NativeSubagentSyncCapability {
    Supported,
    Unavailable,
    Incompatible,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeSubagentSyncResponse {
    capability: NativeSubagentSyncCapability,
    source_path: Option<String>,
    discovered_count: usize,
    synced_count: usize,
    unmapped_count: usize,
    message: String,
}

impl NativeSubagentSyncResponse {
    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::unavailable_with_path(None::<String>, message)
    }

    fn unavailable_with_path(
        source_path: impl Into<Option<String>>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            capability: NativeSubagentSyncCapability::Unavailable,
            source_path: source_path.into(),
            discovered_count: 0,
            synced_count: 0,
            unmapped_count: 0,
            message: message.into(),
        }
    }

    fn incompatible(source_path: String, message: impl Into<String>) -> Self {
        Self {
            capability: NativeSubagentSyncCapability::Incompatible,
            source_path: Some(source_path),
            discovered_count: 0,
            synced_count: 0,
            unmapped_count: 0,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceListResponse {
    items: Vec<AgentThreadInstanceResponse>,
    sync: NativeSubagentSyncResponse,
}

impl AgentThreadInstanceListResponse {
    pub(crate) fn new(
        items: Vec<AgentThreadInstanceResponse>,
        sync: NativeSubagentSyncResponse,
    ) -> Self {
        Self { items, sync }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceWorkspaceScopeRequest {
    thread_id: String,
    workspace_scope_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceRecommendRequest {
    agent_id: String,
    workspace_scope_key: String,
    parent_thread_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadExecutionRequest {
    agent_id: String,
    workspace_scope_key: String,
    cwd: String,
    input: String,
    expected_decision: String,
    expected_candidate_thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceRecommendation {
    pub(crate) decision: &'static str,
    pub(crate) reason_code: &'static str,
    message: &'static str,
    workspace_scope_key: String,
    candidate_instance_id: Option<String>,
    pub(crate) candidate_thread_id: Option<String>,
    context_pressure_percent: Option<i64>,
    context_pressure_limit_percent: i64,
    reuse_strategy: String,
    cache_support: String,
    cache_retention_type: String,
    cache_retention_hint_seconds: Option<i64>,
    cache_retention_source: &'static str,
    cache_hint: &'static str,
    candidate_age_seconds: Option<i64>,
}

#[derive(Debug, Clone)]
pub(crate) struct AgentThreadExecutionPlan {
    pub(crate) profile: AgentRuntimeProfile,
    pub(crate) recommendation: AgentThreadInstanceRecommendation,
    pub(crate) cwd: String,
    pub(crate) input: String,
    pub(crate) workspace_scope_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentRuntimeProfile {
    pub(crate) agent_id: String,
    pub(crate) agent_key: String,
    pub(crate) agent_name: String,
    pub(crate) instruction: String,
    pub(crate) sandbox_policy: String,
    pub(crate) reasoning_policy: String,
    pub(crate) model_slug: String,
    pub(crate) model_provider: Option<String>,
    pub(crate) runtime_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceResponse {
    id: String,
    agent_id: Option<String>,
    agent_name_snapshot: Option<String>,
    codex_thread_id: String,
    parent_thread_id: Option<String>,
    workspace_scope_key: Option<String>,
    status: String,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    current_context_tokens: Option<i64>,
    context_window: Option<i64>,
    runtime_fingerprint: Option<String>,
    created_at: String,
    last_used_at: String,
    closed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageSummaryResponse {
    record_count: i64,
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageRecordResponse {
    id: String,
    codex_session_id: String,
    codex_thread_id: String,
    parent_thread_id: Option<String>,
    agent_id: Option<String>,
    agent_name_snapshot: Option<String>,
    provider_id: Option<String>,
    provider_name_snapshot: Option<String>,
    model_id: Option<String>,
    model_name_snapshot: Option<String>,
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
    model_context_window: Option<i64>,
    usage_status: String,
    source: String,
    started_at: String,
    completed_at: Option<String>,
    updated_at: String,
}

impl From<UsageRecord> for UsageRecordResponse {
    fn from(record: UsageRecord) -> Self {
        Self {
            id: record.id,
            codex_session_id: record.codex_session_id,
            codex_thread_id: record.codex_thread_id,
            parent_thread_id: record.parent_thread_id,
            agent_id: record.agent_id,
            agent_name_snapshot: record.agent_name_snapshot,
            provider_id: record.provider_id,
            provider_name_snapshot: record.provider_name_snapshot,
            model_id: record.model_id,
            model_name_snapshot: record.model_name_snapshot,
            input_tokens: record.input_tokens,
            cached_input_tokens: record.cached_input_tokens,
            cache_write_input_tokens: record.cache_write_input_tokens,
            output_tokens: record.output_tokens,
            reasoning_output_tokens: record.reasoning_output_tokens,
            total_tokens: record.total_tokens,
            model_context_window: record.model_context_window,
            usage_status: record.usage_status,
            source: record.source,
            started_at: record.started_at,
            completed_at: record.completed_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UsageSnapshot {
    pub(crate) codex_session_id: String,
    pub(crate) codex_thread_id: String,
    pub(crate) parent_thread_id: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) agent_name_snapshot: Option<String>,
    pub(crate) provider_id: Option<String>,
    pub(crate) provider_name_snapshot: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) model_name_snapshot: Option<String>,
    pub(crate) input_tokens: i64,
    pub(crate) cached_input_tokens: i64,
    pub(crate) cache_write_input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) reasoning_output_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) current_context_tokens: Option<i64>,
    pub(crate) model_context_window: Option<i64>,
    pub(crate) usage_status: String,
    pub(crate) source: String,
    pub(crate) started_at: String,
    pub(crate) completed_at: Option<String>,
    pub(crate) updated_at: String,
}

impl UsageSnapshot {
    fn validate(&self) -> Result<(), UsageServiceError> {
        for (field, value) in [
            ("codexSessionId", self.codex_session_id.as_str()),
            ("codexThreadId", self.codex_thread_id.as_str()),
            ("usageStatus", self.usage_status.as_str()),
            ("source", self.source.as_str()),
            ("startedAt", self.started_at.as_str()),
            ("updatedAt", self.updated_at.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(UsageServiceError::InvalidField(field));
            }
        }
        for (field, value) in [
            ("inputTokens", self.input_tokens),
            ("cachedInputTokens", self.cached_input_tokens),
            ("cacheWriteInputTokens", self.cache_write_input_tokens),
            ("outputTokens", self.output_tokens),
            ("reasoningOutputTokens", self.reasoning_output_tokens),
            ("totalTokens", self.total_tokens),
        ] {
            if value < 0 {
                return Err(UsageServiceError::InvalidField(field));
            }
        }
        if self.model_context_window.is_some_and(|value| value <= 0) {
            return Err(UsageServiceError::InvalidField("modelContextWindow"));
        }
        if self.current_context_tokens.is_some_and(|value| value < 0) {
            return Err(UsageServiceError::InvalidField("currentContextTokens"));
        }
        if !matches!(
            self.usage_status.as_str(),
            "LIVE" | "FINAL" | "PARTIAL" | "UNKNOWN"
        ) {
            return Err(UsageServiceError::InvalidField("usageStatus"));
        }
        if !matches!(
            self.source.as_str(),
            "CODEX_APP_SERVER" | "CODEX_EXEC_JSONL" | "RESPONSES_PROXY"
        ) {
            return Err(UsageServiceError::InvalidField("source"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UsageUpsertResult {
    outcome: UsageUpsertOutcome,
    record: UsageRecordResponse,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum UsageUpsertOutcome {
    Inserted,
    Updated,
    Unchanged,
    StaleIgnored,
}

struct SqliteUsageRepository {
    connection: Connection,
}

impl SqliteUsageRepository {
    fn open(database_path: &Path) -> Result<Self, UsageRepositoryError> {
        Ok(Self {
            connection: open_database(database_path)?,
        })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, UsageRepositoryError> {
        Ok(Self {
            connection: crate::persistence::open_in_memory()?,
        })
    }

    fn upsert_snapshot(
        &mut self,
        snapshot: UsageSnapshot,
    ) -> Result<UsageUpsertResult, UsageServiceError> {
        let current_context_tokens = snapshot.current_context_tokens;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing = find_by_thread(&transaction, &snapshot.codex_thread_id)?;

        let (outcome, record) = match existing {
            None => {
                let record = UsageRecord::from_snapshot(snapshot);
                insert_record(&transaction, &record)?;
                (UsageUpsertOutcome::Inserted, record)
            }
            Some(existing) => {
                if existing.codex_session_id != snapshot.codex_session_id
                    || option_conflicts(
                        existing.parent_thread_id.as_deref(),
                        snapshot.parent_thread_id.as_deref(),
                    )
                {
                    return Err(UsageServiceError::ThreadIdentityConflict);
                }
                if snapshot_is_stale(&existing, &snapshot) {
                    (UsageUpsertOutcome::StaleIgnored, existing)
                } else {
                    let merged = merge_snapshot(existing.clone(), snapshot);
                    if merged == existing {
                        (UsageUpsertOutcome::Unchanged, existing)
                    } else {
                        update_record(&transaction, &merged)?;
                        (UsageUpsertOutcome::Updated, merged)
                    }
                }
            }
        };

        sync_agent_thread_instance(
            &transaction,
            &record,
            current_context_tokens,
            outcome != UsageUpsertOutcome::StaleIgnored,
        )?;
        transaction.commit()?;
        Ok(UsageUpsertResult {
            outcome,
            record: UsageRecordResponse::from(record),
        })
    }

    fn current_timestamp(&self) -> Result<String, UsageRepositoryError> {
        self.connection
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get(0)
            })
            .map_err(UsageRepositoryError::from)
    }

    fn resolve_attribution(
        &self,
        thread_id: Option<&str>,
        agent_key: Option<&str>,
        model_slug: Option<&str>,
    ) -> Result<Option<UsageAttribution>, UsageRepositoryError> {
        if let Some(thread_id) = thread_id {
            let attribution = self
                .connection
                .query_row(
                    "SELECT a.id, a.name, p.id, p.name, m.id, m.display_name
                     FROM agent_thread_instances instance
                     JOIN agents a ON a.id = instance.agent_id
                     JOIN agent_model_bindings binding
                       ON binding.agent_id = a.id AND binding.enabled = 1
                     JOIN models m ON m.id = binding.model_id
                     JOIN providers p ON p.id = m.provider_id
                     WHERE instance.codex_thread_id = ?1",
                    [thread_id],
                    map_usage_attribution,
                )
                .optional()?;
            if attribution.is_some() {
                return Ok(attribution);
            }
        }
        if agent_key.is_none() && model_slug.is_none() {
            return Ok(None);
        }
        let mut statement = self.connection.prepare(
            "SELECT a.id, a.name, p.id, p.name, m.id, m.display_name
             FROM active_agent_bindings active
             JOIN agents a ON a.id = active.agent_id
             JOIN agent_model_bindings binding
               ON binding.agent_id = a.id AND binding.enabled = 1
             JOIN models m ON m.id = binding.model_id
             JOIN providers p ON p.id = m.provider_id
             WHERE (?1 IS NOT NULL AND a.agent_key = ?1)
                OR (?1 IS NULL AND ?2 IS NOT NULL AND m.model_id = ?2)
             ORDER BY a.agent_key
             LIMIT 2",
        )?;
        let rows = statement
            .query_map(params![agent_key, model_slug], map_usage_attribution)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok((rows.len() == 1).then(|| rows[0].clone()))
    }

    fn summary(
        &self,
        request: &UsageQueryRequest,
    ) -> Result<UsageSummaryResponse, UsageRepositoryError> {
        self.connection
            .query_row(
                "SELECT COUNT(*),
                        COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(cached_input_tokens), 0),
                        COALESCE(SUM(cache_write_input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(reasoning_output_tokens), 0),
                        COALESCE(SUM(total_tokens), 0)
                 FROM token_usage_records
                 WHERE (?1 IS NULL OR agent_id = ?1)
                   AND (?2 IS NULL OR provider_id = ?2)
                   AND (?3 IS NULL OR model_id = ?3)
                   AND (?4 IS NULL OR codex_session_id = ?4)
                   AND (?5 IS NULL OR julianday(started_at) >= julianday(?5))
                   AND (?6 IS NULL OR julianday(started_at) < julianday(?6))",
                params![
                    request.agent_id.as_deref(),
                    request.provider_id.as_deref(),
                    request.model_id.as_deref(),
                    request.codex_session_id.as_deref(),
                    request.from.as_deref(),
                    request.to.as_deref(),
                ],
                |row| {
                    Ok(UsageSummaryResponse {
                        record_count: row.get(0)?,
                        input_tokens: row.get(1)?,
                        cached_input_tokens: row.get(2)?,
                        cache_write_input_tokens: row.get(3)?,
                        output_tokens: row.get(4)?,
                        reasoning_output_tokens: row.get(5)?,
                        total_tokens: row.get(6)?,
                    })
                },
            )
            .map_err(UsageRepositoryError::from)
    }

    fn list(
        &self,
        request: &UsageQueryRequest,
        limit: u32,
    ) -> Result<Vec<UsageRecord>, UsageRepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, codex_session_id, codex_thread_id, parent_thread_id,
                    agent_id, agent_name_snapshot, provider_id, provider_name_snapshot,
                    model_id, model_name_snapshot, input_tokens, cached_input_tokens,
                    cache_write_input_tokens, output_tokens, reasoning_output_tokens,
                    total_tokens, model_context_window, usage_status, source,
                    started_at, completed_at, updated_at
             FROM token_usage_records
             WHERE (?1 IS NULL OR agent_id = ?1)
               AND (?2 IS NULL OR provider_id = ?2)
               AND (?3 IS NULL OR model_id = ?3)
               AND (?4 IS NULL OR codex_session_id = ?4)
               AND (?5 IS NULL OR julianday(started_at) >= julianday(?5))
               AND (?6 IS NULL OR julianday(started_at) < julianday(?6))
             ORDER BY updated_at DESC, codex_thread_id ASC
             LIMIT ?7",
        )?;
        statement
            .query_map(
                params![
                    request.agent_id.as_deref(),
                    request.provider_id.as_deref(),
                    request.model_id.as_deref(),
                    request.codex_session_id.as_deref(),
                    request.from.as_deref(),
                    request.to.as_deref(),
                    limit,
                ],
                map_usage_record,
            )?
            .collect::<Result<Vec<_>, _>>()
            .map_err(UsageRepositoryError::from)
    }

    fn list_agent_instances(
        &self,
        agent_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AgentThreadInstanceResponse>, UsageRepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                    scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at, closed_at
             FROM agent_thread_instances
             WHERE (?1 IS NULL OR agent_id = ?1)
             ORDER BY last_used_at DESC, codex_thread_id ASC
             LIMIT ?2",
        )?;
        statement
            .query_map(params![agent_id, limit], map_agent_thread_instance)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(UsageRepositoryError::from)
    }

    fn set_agent_instance_workspace_scope(
        &self,
        thread_id: &str,
        scope_key: Option<&str>,
    ) -> Result<AgentThreadInstanceResponse, UsageRepositoryError> {
        // `scope_key` is the retained SQLite storage column for Workspace Scope.
        let changed = self.connection.execute(
            "UPDATE agent_thread_instances
             SET scope_key = ?2
             WHERE codex_thread_id = ?1",
            params![thread_id, scope_key],
        )?;
        if changed == 0 {
            return Err(UsageRepositoryError::AgentInstanceNotFound);
        }
        self.connection
            .query_row(
                "SELECT id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                        scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at, closed_at
                 FROM agent_thread_instances
                 WHERE codex_thread_id = ?1",
                [thread_id],
                map_agent_thread_instance,
            )
            .map_err(UsageRepositoryError::from)
    }

    fn scope_candidates(
        &self,
        agent_id: &str,
        scope_key: &str,
        parent_thread_id: Option<&str>,
    ) -> Result<Vec<AgentThreadCandidate>, UsageRepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                    scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at, closed_at,
                    COALESCE(
                        CAST(MAX(0, (julianday('now') - julianday(last_used_at)) * 86400) AS INTEGER),
                        0
                    )
             FROM agent_thread_instances
             WHERE agent_id = ?1
               AND scope_key = ?2
               AND (?3 IS NULL OR parent_thread_id = ?3)
             ORDER BY last_used_at DESC, codex_thread_id ASC",
        )?;
        statement
            .query_map(params![agent_id, scope_key, parent_thread_id], |row| {
                let instance = map_agent_thread_instance(row)?;
                Ok(AgentThreadCandidate {
                    instance_id: instance.id,
                    thread_id: instance.codex_thread_id,
                    status: instance.status,
                    input_tokens: instance.input_tokens,
                    cached_input_tokens: instance.cached_input_tokens,
                    output_tokens: instance.output_tokens,
                    total_tokens: instance.total_tokens,
                    current_context_tokens: instance.current_context_tokens,
                    context_window: instance.context_window,
                    runtime_fingerprint: instance.runtime_fingerprint,
                    age_seconds: row.get(17)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(UsageRepositoryError::from)
    }

    fn scheduling_profile(
        &self,
        agent_id: &str,
    ) -> Result<AgentSchedulingProfile, UsageRepositoryError> {
        let profile = self
            .connection
            .query_row(
                "SELECT a.reuse_strategy, a.cache_retention_override_seconds,
                        COALESCE(p.cache_support, 'UNKNOWN'),
                        COALESCE(p.cache_retention_type, 'UNKNOWN'),
                        p.cache_retention_hint_seconds, a.reasoning_policy, a.sandbox_policy,
                        a.instruction, m.id, m.model_id, p.provider_key, p.preset_id,
                        p.base_url, p.protocol, p.custom_headers_json
                 FROM agents a
                 LEFT JOIN agent_model_bindings b ON b.agent_id = a.id AND b.enabled = 1
                 LEFT JOIN models m ON m.id = b.model_id
                 LEFT JOIN providers p ON p.id = m.provider_id
                 WHERE a.id = ?1",
                [agent_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, Option<String>>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, Option<String>>(14)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            reuse_strategy,
            agent_cache_retention_override_seconds,
            cache_support,
            cache_retention_type,
            cache_retention_hint_seconds,
            reasoning_policy,
            sandbox_policy,
            instruction,
            model_id,
            model_slug,
            provider_key,
            preset_id,
            base_url,
            protocol,
            custom_headers_json,
        )) = profile
        else {
            return Ok(AgentSchedulingProfile::default());
        };
        Ok(AgentSchedulingProfile {
            reuse_strategy,
            agent_cache_retention_override_seconds,
            cache_support,
            cache_retention_type,
            cache_retention_hint_seconds,
            runtime_fingerprint: Some(self.runtime_fingerprint(
                agent_id,
                &model_id,
                &provider_key,
                preset_id.as_deref(),
                &base_url,
                &protocol,
                custom_headers_json.as_deref(),
                &model_slug,
                &reasoning_policy,
                &sandbox_policy,
                &instruction,
            )?),
        })
    }

    fn agent_runtime_profile(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentRuntimeProfile>, UsageRepositoryError> {
        let profile = self
            .connection
            .query_row(
                "SELECT a.id, a.agent_key, a.name, a.instruction, a.sandbox_policy,
                        a.reasoning_policy, m.id, m.model_id, p.provider_key, p.preset_id,
                        p.base_url, p.protocol, p.custom_headers_json
                 FROM active_agent_bindings active
                 JOIN agents a ON a.id = active.agent_id AND a.enabled = 1
                 JOIN agent_model_bindings binding
                   ON binding.agent_id = a.id AND binding.enabled = 1
                 JOIN models m ON m.id = binding.model_id AND m.enabled = 1
                 JOIN providers p ON p.id = m.provider_id AND p.enabled = 1
                 WHERE a.id = ?1",
                [agent_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, Option<String>>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            agent_id,
            agent_key,
            agent_name,
            instruction,
            sandbox_policy,
            reasoning_policy,
            model_id,
            model_slug,
            provider_key,
            preset_id,
            base_url,
            protocol,
            custom_headers_json,
        )) = profile
        else {
            return Ok(None);
        };
        let model_provider =
            (preset_id.as_deref() != Some("codex-native")).then(|| format!("cas_{provider_key}"));
        Ok(Some(AgentRuntimeProfile {
            runtime_fingerprint: self.runtime_fingerprint(
                &agent_id,
                &model_id,
                &provider_key,
                preset_id.as_deref(),
                &base_url,
                &protocol,
                custom_headers_json.as_deref(),
                &model_slug,
                &reasoning_policy,
                &sandbox_policy,
                &instruction,
            )?,
            agent_id,
            agent_key,
            agent_name,
            instruction,
            sandbox_policy,
            reasoning_policy,
            model_slug,
            model_provider,
        }))
    }

    fn runtime_fingerprint(
        &self,
        agent_id: &str,
        model_id: &str,
        provider_key: &str,
        preset_id: Option<&str>,
        base_url: &str,
        protocol: &str,
        custom_headers_json: Option<&str>,
        model_slug: &str,
        reasoning_policy: &str,
        sandbox_policy: &str,
        instruction: &str,
    ) -> Result<String, UsageRepositoryError> {
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
            ("reasoning", vec![reasoning_policy.to_owned()]),
            ("sandbox", vec![sandbox_policy.to_owned()]),
            ("instruction", vec![instruction.to_owned()]),
            (
                "required_capabilities",
                fingerprint_values(
                    &self.connection,
                    "SELECT capability FROM agent_required_capabilities WHERE agent_id = ?1",
                    agent_id,
                )?,
            ),
            (
                "preferred_capabilities",
                fingerprint_values(
                    &self.connection,
                    "SELECT capability FROM agent_preferred_capabilities WHERE agent_id = ?1",
                    agent_id,
                )?,
            ),
            (
                "model_capabilities",
                fingerprint_values(
                    &self.connection,
                    "SELECT capability || '=' || status
                     FROM model_capabilities WHERE model_id = ?1",
                    model_id,
                )?,
            ),
        ]))
    }

    fn register_agent_execution_thread(
        &self,
        profile: &AgentRuntimeProfile,
        thread_id: &str,
        scope_key: &str,
        status: &str,
    ) -> Result<(), UsageServiceError> {
        let existing_agent_id = self
            .connection
            .query_row(
                "SELECT agent_id FROM agent_thread_instances WHERE codex_thread_id = ?1",
                [thread_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        if existing_agent_id
            .as_deref()
            .is_some_and(|existing| existing != profile.agent_id)
        {
            return Err(UsageServiceError::ThreadIdentityConflict);
        }
        self.connection.execute(
            "INSERT INTO agent_thread_instances (
                id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                total_tokens, current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at, closed_at
             ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5, ?6, 0, 0, 0, 0, NULL, NULL, ?7,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL
             )
             ON CONFLICT(codex_thread_id) DO UPDATE SET
                scope_key = excluded.scope_key,
                status = excluded.status,
                runtime_fingerprint = COALESCE(agent_thread_instances.runtime_fingerprint, excluded.runtime_fingerprint),
                last_used_at = excluded.last_used_at",
            params![
                Uuid::new_v4().to_string(),
                profile.agent_id,
                profile.agent_name,
                thread_id,
                scope_key,
                status,
                profile.runtime_fingerprint,
            ],
        )?;
        Ok(())
    }

    fn set_agent_execution_status(
        &self,
        thread_id: &str,
        status: &str,
    ) -> Result<(), UsageServiceError> {
        let changed = self.connection.execute(
            "UPDATE agent_thread_instances
             SET status = ?2, last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE codex_thread_id = ?1",
            params![thread_id, status],
        )?;
        if changed == 0 {
            return Err(UsageRepositoryError::AgentInstanceNotFound.into());
        }
        Ok(())
    }

    fn set_agent_execution_status_if_known(
        &self,
        thread_id: &str,
        status: &str,
    ) -> Result<(), UsageServiceError> {
        self.connection.execute(
            "UPDATE agent_thread_instances
             SET status = ?2, last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE codex_thread_id = ?1",
            params![thread_id, status],
        )?;
        Ok(())
    }
}

#[derive(Debug)]
struct NativeSubagentRecord {
    thread_id: String,
    parent_thread_id: String,
    agent_role: Option<String>,
    model_provider: String,
    model_slug: Option<String>,
    status: &'static str,
    total_tokens: i64,
    current_context_tokens: Option<i64>,
    context_window: Option<i64>,
    scope_key: Option<String>,
    created_at: String,
    updated_at: String,
}

fn find_codex_state_database(codex_home: &Path) -> Option<PathBuf> {
    fs::read_dir(codex_home)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u64>()
                .ok()?;
            path.is_file().then_some((version, path))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

fn native_state_schema_supported(connection: &Connection) -> bool {
    let required = [
        (
            "threads",
            [
                "id",
                "agent_role",
                "model_provider",
                "model",
                "tokens_used",
                "cwd",
                "rollout_path",
                "created_at",
                "updated_at",
            ]
            .as_slice(),
        ),
        (
            "thread_spawn_edges",
            ["parent_thread_id", "child_thread_id", "status"].as_slice(),
        ),
    ];
    required.iter().all(|(table, required_columns)| {
        table_columns(connection, table).is_ok_and(|columns| {
            required_columns
                .iter()
                .all(|column| columns.contains(*column))
        })
    })
}

fn table_columns(
    connection: &Connection,
    table: &str,
) -> Result<BTreeSet<String>, rusqlite::Error> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    statement
        .query_map([], |row| row.get(1))?
        .collect::<Result<BTreeSet<_>, _>>()
}

fn load_native_subagent_records(
    connection: &Connection,
) -> Result<Vec<NativeSubagentRecord>, rusqlite::Error> {
    let mut statement = connection.prepare(
        "SELECT child.id, edge.parent_thread_id, child.agent_role,
                child.model_provider, child.model, child.tokens_used, edge.status,
                child.cwd, child.rollout_path,
                strftime('%Y-%m-%dT%H:%M:%fZ', child.created_at, 'unixepoch'),
                strftime('%Y-%m-%dT%H:%M:%fZ', child.updated_at, 'unixepoch')
         FROM thread_spawn_edges edge
         JOIN threads child ON child.id = edge.child_thread_id
         ORDER BY child.updated_at DESC",
    )?;
    statement
        .query_map([], |row| {
            let thread_id = row.get::<_, String>(0)?;
            let rollout = rollout_state(Path::new(&row.get::<_, String>(8)?)).ok();
            let status =
                match thread_state_from_rollout(&row.get::<_, String>(6)?, rollout.as_ref()) {
                    NativeThreadState::Closed => "CLOSED",
                    NativeThreadState::Idle => "IDLE",
                    NativeThreadState::Running => "RUNNING",
                    NativeThreadState::Unknown => "UNKNOWN",
                };
            let cwd = row.get::<_, String>(7)?;
            Ok(NativeSubagentRecord {
                thread_id,
                parent_thread_id: row.get(1)?,
                agent_role: row.get(2)?,
                model_provider: row.get(3)?,
                model_slug: row.get(4)?,
                status,
                total_tokens: row.get::<_, i64>(5)?.max(0),
                current_context_tokens: rollout.and_then(|state| state.current_context_tokens),
                context_window: rollout.and_then(|state| state.model_context_window),
                scope_key: normalize_native_scope(&cwd),
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })?
        .collect()
}

fn normalize_native_scope(cwd: &str) -> Option<String> {
    normalize_workspace_scope_key(cwd).ok()
}

fn resolve_native_agent(
    transaction: &Transaction<'_>,
    record: &NativeSubagentRecord,
) -> Result<Option<(String, String, Option<i64>)>, rusqlite::Error> {
    let mut statement = transaction.prepare(
        "SELECT a.id, a.name, m.context_window,
                CASE
                    WHEN ?2 IS NOT NULL
                         AND m.model_id = ?2
                         AND (
                            ?3 = 'cas_' || p.provider_key
                            OR (
                                p.preset_id = 'codex-native'
                                AND ?3 IN ('openai', 'chatgpt')
                            )
                         ) THEN 0
                    WHEN active.role_key = ?1 THEN 1
                    WHEN a.agent_key = ?1 THEN 2
                    ELSE 3
                END AS match_priority
         FROM agents a
         JOIN agent_model_bindings binding
           ON binding.agent_id = a.id AND binding.enabled = 1
         JOIN models m ON m.id = binding.model_id AND m.enabled = 1
         JOIN providers p ON p.id = m.provider_id AND p.enabled = 1
         LEFT JOIN active_agent_bindings active ON active.agent_id = a.id
         WHERE a.enabled = 1
           AND (
               (?1 IS NOT NULL AND (
                    active.role_key = ?1
                    OR a.agent_key = ?1
                    OR a.role_key = ?1
               ))
            OR (
                ?2 IS NOT NULL
                AND m.model_id = ?2
                AND (
                    ?3 = 'cas_' || p.provider_key
                    OR (
                        p.preset_id = 'codex-native'
                        AND ?3 IN ('openai', 'chatgpt')
                    )
                )
            )
           )
         ORDER BY match_priority, a.id
         LIMIT 2",
    )?;
    let matches = statement
        .query_map(
            params![
                record.agent_role.as_deref(),
                record.model_slug.as_deref(),
                record.model_provider
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = matches.first() else {
        return Ok(None);
    };
    if matches
        .get(1)
        .is_some_and(|second| second.3 == first.3 && second.0 != first.0)
    {
        return Ok(None);
    }
    Ok(Some((first.0.clone(), first.1.clone(), first.2)))
}

fn upsert_native_agent_instance(
    transaction: &Transaction<'_>,
    record: &NativeSubagentRecord,
    agent_id: &str,
    agent_name: &str,
) -> Result<(), rusqlite::Error> {
    transaction.execute(
        "INSERT INTO agent_thread_instances (
            id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
            scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at, closed_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, 0, 0, ?8, ?9, ?10, ?11, ?12, ?13,
            CASE WHEN ?7 = 'CLOSED' THEN ?13 ELSE NULL END
         )
         ON CONFLICT(codex_thread_id) DO UPDATE SET
            agent_id = excluded.agent_id,
            agent_name_snapshot = excluded.agent_name_snapshot,
            parent_thread_id = excluded.parent_thread_id,
            scope_key = COALESCE(agent_thread_instances.scope_key, excluded.scope_key),
            status = excluded.status,
            total_tokens = MAX(agent_thread_instances.total_tokens, excluded.total_tokens),
            current_context_tokens = excluded.current_context_tokens,
            context_window = excluded.context_window,
            runtime_fingerprint = COALESCE(
                agent_thread_instances.runtime_fingerprint,
                excluded.runtime_fingerprint
            ),
            created_at = MIN(agent_thread_instances.created_at, excluded.created_at),
            last_used_at = MAX(agent_thread_instances.last_used_at, excluded.last_used_at),
            closed_at = excluded.closed_at
         WHERE agent_thread_instances.agent_id IS NOT excluded.agent_id
            OR agent_thread_instances.agent_name_snapshot IS NOT excluded.agent_name_snapshot
            OR agent_thread_instances.parent_thread_id IS NOT excluded.parent_thread_id
            OR (
                agent_thread_instances.scope_key IS NULL
                AND excluded.scope_key IS NOT NULL
            )
            OR agent_thread_instances.status IS NOT excluded.status
            OR excluded.total_tokens > agent_thread_instances.total_tokens
            OR agent_thread_instances.current_context_tokens IS NOT excluded.current_context_tokens
            OR agent_thread_instances.context_window IS NOT excluded.context_window
            OR excluded.created_at < agent_thread_instances.created_at
            OR excluded.last_used_at > agent_thread_instances.last_used_at
            OR agent_thread_instances.closed_at IS NOT excluded.closed_at",
        params![
            format!("native-{}", record.thread_id),
            agent_id,
            agent_name,
            record.thread_id,
            record.parent_thread_id,
            record.scope_key.as_deref(),
            record.status,
            record.total_tokens,
            record.current_context_tokens,
            record.context_window,
            Option::<String>::None,
            record.created_at,
            record.updated_at,
        ],
    )?;
    Ok(())
}

fn map_usage_attribution(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageAttribution> {
    Ok(UsageAttribution {
        agent_id: row.get(0)?,
        agent_name: row.get(1)?,
        provider_id: row.get(2)?,
        provider_name: row.get(3)?,
        model_id: row.get(4)?,
        model_name: row.get(5)?,
    })
}

fn map_agent_thread_instance(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AgentThreadInstanceResponse> {
    Ok(AgentThreadInstanceResponse {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        agent_name_snapshot: row.get(2)?,
        codex_thread_id: row.get(3)?,
        parent_thread_id: row.get(4)?,
        workspace_scope_key: row.get(5)?,
        status: row.get(6)?,
        input_tokens: row.get(7)?,
        cached_input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
        total_tokens: row.get(10)?,
        current_context_tokens: row.get(11)?,
        context_window: row.get(12)?,
        runtime_fingerprint: row.get(13)?,
        created_at: row.get(14)?,
        last_used_at: row.get(15)?,
        closed_at: row.get(16)?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UsageRecord {
    id: String,
    codex_session_id: String,
    codex_thread_id: String,
    parent_thread_id: Option<String>,
    agent_id: Option<String>,
    agent_name_snapshot: Option<String>,
    provider_id: Option<String>,
    provider_name_snapshot: Option<String>,
    model_id: Option<String>,
    model_name_snapshot: Option<String>,
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
    current_context_tokens: Option<i64>,
    model_context_window: Option<i64>,
    usage_status: String,
    source: String,
    started_at: String,
    completed_at: Option<String>,
    updated_at: String,
}

impl UsageRecord {
    fn from_snapshot(snapshot: UsageSnapshot) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            codex_session_id: snapshot.codex_session_id,
            codex_thread_id: snapshot.codex_thread_id,
            parent_thread_id: snapshot.parent_thread_id,
            agent_id: snapshot.agent_id,
            agent_name_snapshot: snapshot.agent_name_snapshot,
            provider_id: snapshot.provider_id,
            provider_name_snapshot: snapshot.provider_name_snapshot,
            model_id: snapshot.model_id,
            model_name_snapshot: snapshot.model_name_snapshot,
            input_tokens: snapshot.input_tokens,
            cached_input_tokens: snapshot.cached_input_tokens,
            cache_write_input_tokens: snapshot.cache_write_input_tokens,
            output_tokens: snapshot.output_tokens,
            reasoning_output_tokens: snapshot.reasoning_output_tokens,
            total_tokens: snapshot.total_tokens,
            current_context_tokens: snapshot.current_context_tokens,
            model_context_window: snapshot.model_context_window,
            usage_status: snapshot.usage_status,
            source: snapshot.source,
            started_at: snapshot.started_at,
            completed_at: snapshot.completed_at,
            updated_at: snapshot.updated_at,
        }
    }
}

fn find_by_thread(
    transaction: &Transaction<'_>,
    thread_id: &str,
) -> Result<Option<UsageRecord>, UsageRepositoryError> {
    transaction
        .query_row(
            "SELECT id, codex_session_id, codex_thread_id, parent_thread_id,
                    agent_id, agent_name_snapshot, provider_id, provider_name_snapshot,
                    model_id, model_name_snapshot, input_tokens, cached_input_tokens,
                    cache_write_input_tokens, output_tokens, reasoning_output_tokens,
                    total_tokens, model_context_window, usage_status, source,
                    started_at, completed_at, updated_at
             FROM token_usage_records
             WHERE codex_thread_id = ?1",
            [thread_id],
            map_usage_record,
        )
        .optional()
        .map_err(UsageRepositoryError::from)
}

fn map_usage_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRecord> {
    Ok(UsageRecord {
        id: row.get(0)?,
        codex_session_id: row.get(1)?,
        codex_thread_id: row.get(2)?,
        parent_thread_id: row.get(3)?,
        agent_id: row.get(4)?,
        agent_name_snapshot: row.get(5)?,
        provider_id: row.get(6)?,
        provider_name_snapshot: row.get(7)?,
        model_id: row.get(8)?,
        model_name_snapshot: row.get(9)?,
        input_tokens: row.get(10)?,
        cached_input_tokens: row.get(11)?,
        cache_write_input_tokens: row.get(12)?,
        output_tokens: row.get(13)?,
        reasoning_output_tokens: row.get(14)?,
        total_tokens: row.get(15)?,
        current_context_tokens: None,
        model_context_window: row.get(16)?,
        usage_status: row.get(17)?,
        source: row.get(18)?,
        started_at: row.get(19)?,
        completed_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

fn insert_record(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
) -> Result<(), UsageRepositoryError> {
    transaction.execute(
        "INSERT INTO token_usage_records (
            id, codex_session_id, codex_thread_id, parent_thread_id,
            agent_id, agent_name_snapshot, provider_id, provider_name_snapshot,
            model_id, model_name_snapshot, input_tokens, cached_input_tokens,
            cache_write_input_tokens, output_tokens, reasoning_output_tokens,
            total_tokens, model_context_window, usage_status, source,
            started_at, completed_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22
         )",
        usage_record_params(record),
    )?;
    Ok(())
}

fn update_record(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
) -> Result<(), UsageRepositoryError> {
    transaction.execute(
        "UPDATE token_usage_records
         SET codex_session_id = ?2,
             codex_thread_id = ?3,
             parent_thread_id = ?4,
             agent_id = ?5,
             agent_name_snapshot = ?6,
             provider_id = ?7,
             provider_name_snapshot = ?8,
             model_id = ?9,
             model_name_snapshot = ?10,
             input_tokens = ?11,
             cached_input_tokens = ?12,
             cache_write_input_tokens = ?13,
             output_tokens = ?14,
             reasoning_output_tokens = ?15,
             total_tokens = ?16,
             model_context_window = ?17,
             usage_status = ?18,
             source = ?19,
             started_at = ?20,
             completed_at = ?21,
             updated_at = ?22
         WHERE id = ?1",
        usage_record_params(record),
    )?;
    Ok(())
}

fn usage_record_params(record: &UsageRecord) -> [&dyn rusqlite::ToSql; 22] {
    [
        &record.id,
        &record.codex_session_id,
        &record.codex_thread_id,
        &record.parent_thread_id,
        &record.agent_id,
        &record.agent_name_snapshot,
        &record.provider_id,
        &record.provider_name_snapshot,
        &record.model_id,
        &record.model_name_snapshot,
        &record.input_tokens,
        &record.cached_input_tokens,
        &record.cache_write_input_tokens,
        &record.output_tokens,
        &record.reasoning_output_tokens,
        &record.total_tokens,
        &record.model_context_window,
        &record.usage_status,
        &record.source,
        &record.started_at,
        &record.completed_at,
        &record.updated_at,
    ]
}

fn sync_agent_thread_instance(
    transaction: &Transaction<'_>,
    record: &UsageRecord,
    current_context_tokens: Option<i64>,
    current_context_is_fresh: bool,
) -> Result<(), UsageRepositoryError> {
    if record.agent_id.is_none() {
        return Ok(());
    }
    let status = match record.usage_status.as_str() {
        "LIVE" => "RUNNING",
        "FINAL" => "IDLE",
        "PARTIAL" => "RECOVERY_REQUIRED",
        _ => "UNKNOWN",
    };
    transaction.execute(
        "INSERT INTO agent_thread_instances (
            id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
            scope_key, status, input_tokens, cached_input_tokens, output_tokens,
            total_tokens, current_context_tokens, context_window, created_at, last_used_at, closed_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, NULL
         )
         ON CONFLICT(codex_thread_id) DO UPDATE SET
            agent_id = COALESCE(agent_thread_instances.agent_id, excluded.agent_id),
            agent_name_snapshot = COALESCE(
                excluded.agent_name_snapshot,
                agent_thread_instances.agent_name_snapshot
            ),
            parent_thread_id = COALESCE(
                agent_thread_instances.parent_thread_id,
                excluded.parent_thread_id
            ),
            status = excluded.status,
            input_tokens = MAX(agent_thread_instances.input_tokens, excluded.input_tokens),
            cached_input_tokens = MAX(
                agent_thread_instances.cached_input_tokens,
                excluded.cached_input_tokens
            ),
            output_tokens = MAX(agent_thread_instances.output_tokens, excluded.output_tokens),
            total_tokens = MAX(agent_thread_instances.total_tokens, excluded.total_tokens),
            current_context_tokens = CASE
                WHEN ?15 THEN excluded.current_context_tokens
                ELSE agent_thread_instances.current_context_tokens
            END,
            context_window = excluded.context_window,
            last_used_at = MAX(agent_thread_instances.last_used_at, excluded.last_used_at)",
        params![
            format!("thread-{}", record.id),
            record.agent_id,
            record.agent_name_snapshot,
            record.codex_thread_id,
            record.parent_thread_id,
            status,
            record.input_tokens,
            record.cached_input_tokens,
            record.output_tokens,
            record.total_tokens,
            current_context_tokens,
            record.model_context_window,
            record.started_at,
            record.updated_at,
            current_context_is_fresh,
        ],
    )?;
    Ok(())
}

fn snapshot_is_stale(record: &UsageRecord, snapshot: &UsageSnapshot) -> bool {
    snapshot.input_tokens < record.input_tokens
        || snapshot.cached_input_tokens < record.cached_input_tokens
        || snapshot.cache_write_input_tokens < record.cache_write_input_tokens
        || snapshot.output_tokens < record.output_tokens
        || snapshot.reasoning_output_tokens < record.reasoning_output_tokens
        || snapshot.total_tokens < record.total_tokens
}

fn option_conflicts(current: Option<&str>, incoming: Option<&str>) -> bool {
    matches!((current, incoming), (Some(current), Some(incoming)) if current != incoming)
}

fn merge_snapshot(mut record: UsageRecord, snapshot: UsageSnapshot) -> UsageRecord {
    let usage_advanced = snapshot.input_tokens > record.input_tokens
        || snapshot.cached_input_tokens > record.cached_input_tokens
        || snapshot.cache_write_input_tokens > record.cache_write_input_tokens
        || snapshot.output_tokens > record.output_tokens
        || snapshot.reasoning_output_tokens > record.reasoning_output_tokens
        || snapshot.total_tokens > record.total_tokens;
    let keep_final_status =
        record.usage_status == "FINAL" && snapshot.usage_status != "FINAL" && !usage_advanced;
    record.parent_thread_id = record.parent_thread_id.or(snapshot.parent_thread_id);
    record.agent_id = record.agent_id.or(snapshot.agent_id);
    record.agent_name_snapshot = record.agent_name_snapshot.or(snapshot.agent_name_snapshot);
    record.provider_id = record.provider_id.or(snapshot.provider_id);
    record.provider_name_snapshot = record
        .provider_name_snapshot
        .or(snapshot.provider_name_snapshot);
    record.model_id = record.model_id.or(snapshot.model_id);
    record.model_name_snapshot = record.model_name_snapshot.or(snapshot.model_name_snapshot);
    record.input_tokens = snapshot.input_tokens;
    record.cached_input_tokens = snapshot.cached_input_tokens;
    record.cache_write_input_tokens = snapshot.cache_write_input_tokens;
    record.output_tokens = snapshot.output_tokens;
    record.reasoning_output_tokens = snapshot.reasoning_output_tokens;
    record.total_tokens = snapshot.total_tokens;
    record.model_context_window = snapshot
        .model_context_window
        .or(record.model_context_window);
    if !keep_final_status {
        record.usage_status = snapshot.usage_status;
        record.completed_at = snapshot.completed_at;
    }
    record.updated_at = snapshot.updated_at;
    record
}

fn validate_runtime_key(value: &str, field: &'static str) -> Result<String, UsageServiceError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(UsageServiceError::InvalidField(field));
    }
    Ok(value.to_owned())
}

fn normalize_workspace_scope_key(value: &str) -> Result<String, UsageServiceError> {
    canonical_workspace_scope_key(value).ok_or(UsageServiceError::InvalidField("workspaceScopeKey"))
}

fn fingerprint_values(
    connection: &Connection,
    sql: &str,
    parameter: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    connection
        .prepare(sql)?
        .query_map([parameter], |row| row.get(0))?
        .collect()
}

fn recommend_instance(
    scope_key: String,
    candidates: Vec<AgentThreadCandidate>,
    profile: AgentSchedulingProfile,
) -> AgentThreadInstanceRecommendation {
    let recommendation = schedule_instance(scope_key, candidates, profile);
    AgentThreadInstanceRecommendation {
        decision: recommendation.decision,
        reason_code: recommendation.reason_code,
        message: recommendation.message,
        workspace_scope_key: recommendation.workspace_scope_key,
        candidate_instance_id: recommendation.candidate_instance_id,
        candidate_thread_id: recommendation.candidate_thread_id,
        context_pressure_percent: recommendation.context_pressure_percent,
        context_pressure_limit_percent: recommendation.context_pressure_limit_percent,
        reuse_strategy: recommendation.reuse_strategy,
        cache_support: recommendation.cache_support,
        cache_retention_type: recommendation.cache_retention_type,
        cache_retention_hint_seconds: recommendation.cache_retention_hint_seconds,
        cache_retention_source: recommendation.cache_retention_source,
        cache_hint: recommendation.cache_hint,
        candidate_age_seconds: recommendation.candidate_age_seconds,
    }
}

impl From<UsageServiceError> for ApiError {
    fn from(error: UsageServiceError) -> Self {
        let mut details = None;
        let (code, message, retryable) = match error {
            UsageServiceError::InvalidField(field) => {
                details = Some(BTreeMap::from([("field", field.to_owned())]));
                ("VALIDATION_ERROR", "Token Usage 查询字段无效。", false)
            }
            UsageServiceError::ThreadIdentityConflict => (
                "USAGE_THREAD_IDENTITY_CONFLICT",
                "Token Usage 线程归属发生冲突。",
                false,
            ),
            UsageServiceError::DecisionChanged => (
                "AGENT_THREAD_DECISION_CHANGED",
                "Thread 状态已变化，请重新评估后再执行。",
                false,
            ),
            UsageServiceError::AgentRuntimeUnavailable => (
                "AGENT_RUNTIME_UNAVAILABLE",
                "该 Agent 当前未启用，或其 Provider / Model 不可运行。",
                false,
            ),
            UsageServiceError::Repository(UsageRepositoryError::AgentInstanceNotFound) => (
                "AGENT_THREAD_INSTANCE_NOT_FOUND",
                "未找到指定的子 Agent Thread 实例。",
                false,
            ),
            UsageServiceError::DatabaseUnavailable
            | UsageServiceError::Repository(UsageRepositoryError::Persistence(
                PersistenceError::Unavailable,
            )) => ("DATABASE_UNAVAILABLE", "CAS 数据库当前不可用。", true),
            UsageServiceError::Repository(UsageRepositoryError::Persistence(
                PersistenceError::SchemaTooNew,
            )) => (
                "DATABASE_SCHEMA_TOO_NEW",
                "数据库版本高于当前应用支持版本。",
                false,
            ),
            UsageServiceError::Repository(_) => (
                "USAGE_DATABASE_OPERATION_FAILED",
                "Token Usage 数据操作失败。",
                true,
            ),
        };
        Self::new(code, message, retryable, details)
    }
}

impl From<UsageRepositoryError> for ApiError {
    fn from(error: UsageRepositoryError) -> Self {
        UsageServiceError::Repository(error).into()
    }
}

#[derive(Debug)]
pub(crate) enum UsageServiceError {
    InvalidField(&'static str),
    ThreadIdentityConflict,
    DecisionChanged,
    AgentRuntimeUnavailable,
    Repository(UsageRepositoryError),
    DatabaseUnavailable,
}

impl fmt::Display for UsageServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid usage field: {field}"),
            Self::ThreadIdentityConflict => formatter.write_str("usage thread identity conflict"),
            Self::DecisionChanged => formatter.write_str("agent thread decision changed"),
            Self::AgentRuntimeUnavailable => formatter.write_str("agent runtime unavailable"),
            Self::Repository(error) => write!(formatter, "usage repository failed: {error}"),
            Self::DatabaseUnavailable => formatter.write_str("database unavailable"),
        }
    }
}

impl std::error::Error for UsageServiceError {}

impl From<UsageRepositoryError> for UsageServiceError {
    fn from(error: UsageRepositoryError) -> Self {
        Self::Repository(error)
    }
}

impl From<PersistenceError> for UsageServiceError {
    fn from(error: PersistenceError) -> Self {
        Self::Repository(UsageRepositoryError::Persistence(error))
    }
}

impl From<rusqlite::Error> for UsageServiceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Repository(UsageRepositoryError::Sqlite(error))
    }
}

#[derive(Debug)]
pub(crate) enum UsageRepositoryError {
    AgentInstanceNotFound,
    Persistence(PersistenceError),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for UsageRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AgentInstanceNotFound => formatter.write_str("agent thread instance not found"),
            Self::Persistence(error) => write!(formatter, "persistence failed: {error}"),
            Self::Sqlite(_) => formatter.write_str("sqlite operation failed"),
        }
    }
}

impl std::error::Error for UsageRepositoryError {}

impl From<PersistenceError> for UsageRepositoryError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

impl From<rusqlite::Error> for UsageRepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_subagent_state_sync_maps_primary_child_and_total_tokens() {
        let root =
            std::env::temp_dir().join(format!("cas-native-subagent-sync-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source_path = root.join("state_7.sqlite");
        let source = Connection::open(&source_path).unwrap();
        source
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    agent_role TEXT,
                    model_provider TEXT NOT NULL,
                    model TEXT,
                    tokens_used INTEGER NOT NULL,
                    cwd TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL PRIMARY KEY,
                    status TEXT NOT NULL
                 );",
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO threads (
                    id, rollout_path, agent_role, model_provider, model, tokens_used, cwd,
                    created_at, updated_at
                 ) VALUES (
                    'thread-child-native', ?1, 'executor', 'cas_deepseek',
                    'deepseek-v4-flash', 12345, ?2, 1786600000, 1786600300
                 )",
                [
                    root.join("rollout.jsonl").to_string_lossy().as_ref(),
                    r"\\?\C:\workspace\project",
                ],
            )
            .unwrap();
        source
            .execute(
                "INSERT INTO thread_spawn_edges (
                    parent_thread_id, child_thread_id, status
                 ) VALUES ('thread-primary', 'thread-child-native', 'open')",
                [],
            )
            .unwrap();
        drop(source);
        fs::write(
            root.join("rollout.jsonl"),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":12345},\"model_context_window\":128000}}}\n",
        )
        .unwrap();
        fs::create_dir(root.join("thread-writer-locks")).unwrap();
        fs::write(
            root.join("thread-writer-locks/thread-child-native.lock"),
            "",
        )
        .unwrap();

        let service = UsageService::in_memory();
        seed_agent(&service);
        let sync = service.sync_native_subagents(&root).unwrap();
        assert_eq!(sync.capability, NativeSubagentSyncCapability::Supported);
        assert_eq!(sync.discovered_count, 1);
        assert_eq!(sync.synced_count, 1);
        assert_eq!(sync.unmapped_count, 0);

        let instances = service
            .list_agent_instances(AgentThreadInstanceListRequest::default())
            .unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].codex_thread_id, "thread-child-native");
        assert_eq!(
            instances[0].parent_thread_id.as_deref(),
            Some("thread-primary")
        );
        assert_eq!(instances[0].agent_id.as_deref(), Some("agent-1"));
        assert_eq!(instances[0].status, "IDLE");
        assert_eq!(instances[0].total_tokens, 12345);
        assert_eq!(instances[0].context_window, Some(128_000));
        assert_eq!(
            instances[0].workspace_scope_key.as_deref(),
            Some("c:/workspace/project")
        );
        let second_sync = service.sync_native_subagents(&root).unwrap();
        assert_eq!(second_sync.synced_count, 1);
        assert_eq!(
            service
                .list_agent_instances(AgentThreadInstanceListRequest::default())
                .unwrap()
                .len(),
            1
        );

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn native_subagent_sync_stops_on_unknown_state_schema() {
        let root = std::env::temp_dir().join(format!("cas-native-schema-sync-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let source = Connection::open(root.join("state_99.sqlite")).unwrap();
        source
            .execute("CREATE TABLE threads (id TEXT PRIMARY KEY)", [])
            .unwrap();
        drop(source);

        let service = UsageService::in_memory();
        let sync = service.sync_native_subagents(&root).unwrap();
        assert_eq!(sync.capability, NativeSubagentSyncCapability::Incompatible);
        assert_eq!(sync.synced_count, 0);

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cumulative_snapshots_are_idempotent_and_ignore_rollbacks() {
        let service = UsageService::in_memory();

        let inserted = service.upsert_snapshot(snapshot(100, "LIVE")).unwrap();
        assert_eq!(inserted.outcome, UsageUpsertOutcome::Inserted);

        let unchanged = service.upsert_snapshot(snapshot(100, "LIVE")).unwrap();
        assert_eq!(unchanged.outcome, UsageUpsertOutcome::Unchanged);

        let updated = service.upsert_snapshot(snapshot(160, "FINAL")).unwrap();
        assert_eq!(updated.outcome, UsageUpsertOutcome::Updated);
        assert_eq!(updated.record.total_tokens, 160);
        assert_eq!(updated.record.usage_status, "FINAL");
        let idle = service
            .list_agent_instances(AgentThreadInstanceListRequest::default())
            .unwrap();
        assert_eq!(idle.len(), 1);
        assert_eq!(idle[0].status, "IDLE");

        let stale = service.upsert_snapshot(snapshot(120, "LIVE")).unwrap();
        assert_eq!(stale.outcome, UsageUpsertOutcome::StaleIgnored);
        assert_eq!(stale.record.total_tokens, 160);
        assert_eq!(stale.record.usage_status, "FINAL");

        let resumed = service.upsert_snapshot(snapshot(200, "LIVE")).unwrap();
        assert_eq!(resumed.outcome, UsageUpsertOutcome::Updated);
        assert_eq!(resumed.record.usage_status, "LIVE");
        assert_eq!(resumed.record.completed_at, None);

        let summary = service.summary(UsageQueryRequest::default()).unwrap();
        assert_eq!(summary.record_count, 1);
        assert_eq!(summary.total_tokens, 200);
        let running = service
            .list_agent_instances(AgentThreadInstanceListRequest::default())
            .unwrap();
        assert_eq!(running[0].status, "RUNNING");
        assert_eq!(running[0].total_tokens, 200);
        assert_eq!(running[0].codex_thread_id, "thread-child-1");
    }

    #[test]
    fn partial_subagent_thread_requires_recovery() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(80, "PARTIAL")).unwrap();

        let instances = service
            .list_agent_instances(AgentThreadInstanceListRequest {
                agent_id: Some("agent-1".to_owned()),
                limit: Some(10),
            })
            .unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].status, "RECOVERY_REQUIRED");
        assert_eq!(
            instances[0].parent_thread_id.as_deref(),
            Some("thread-root-1")
        );
    }

    #[test]
    fn exact_workspace_scope_idle_thread_spawns_for_unknown_fingerprint_and_other_workspace() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(100, "FINAL")).unwrap();
        let updated = service
            .set_agent_instance_workspace_scope(AgentThreadInstanceWorkspaceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                workspace_scope_key: Some("C:\\Workspace\\Project".to_owned()),
            })
            .unwrap();
        assert_eq!(
            updated.workspace_scope_key.as_deref(),
            Some("c:/workspace/project")
        );

        let reuse = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/project".to_owned(),
                parent_thread_id: None,
            })
            .unwrap();
        assert_eq!(reuse.decision, "SPAWN");
        assert_eq!(reuse.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        assert_eq!(reuse.reuse_strategy, "AUTO");
        assert_eq!(reuse.context_pressure_limit_percent, 80);

        let other_primary = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/project".to_owned(),
                parent_thread_id: Some("thread-root-other".to_owned()),
            })
            .unwrap();
        assert_eq!(other_primary.decision, "SPAWN");
        assert_eq!(other_primary.reason_code, "NO_WORKSPACE_SCOPE_MATCH");

        let spawn = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/other".to_owned(),
                parent_thread_id: None,
            })
            .unwrap();
        assert_eq!(spawn.decision, "SPAWN");
        assert_eq!(spawn.reason_code, "NO_WORKSPACE_SCOPE_MATCH");
    }

    #[test]
    fn canonical_unix_and_unc_workspace_scopes_round_trip_through_the_api() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(100, "FINAL")).unwrap();
        for workspace_scope_key in ["root/work/Foo", "unc/server/share/project"] {
            let updated = service
                .set_agent_instance_workspace_scope(AgentThreadInstanceWorkspaceScopeRequest {
                    thread_id: "thread-child-1".to_owned(),
                    workspace_scope_key: Some(workspace_scope_key.to_owned()),
                })
                .unwrap();
            assert_eq!(
                updated.workspace_scope_key.as_deref(),
                Some(workspace_scope_key)
            );
        }
    }

    #[test]
    fn runtime_fingerprint_uses_runtime_provider_and_capability_rows() {
        let service = UsageService::in_memory();
        seed_agent(&service);
        let repository = service.repository().unwrap();
        let fingerprint = || {
            repository
                .scheduling_profile("agent-1")
                .unwrap()
                .runtime_fingerprint
                .unwrap()
        };

        let baseline = fingerprint();
        repository
            .connection
            .execute(
                "UPDATE providers SET base_url = 'https://api.changed.example/v1'
                 WHERE id = 'provider-deepseek'",
                [],
            )
            .unwrap();
        let base_url_changed = fingerprint();
        assert_ne!(baseline, base_url_changed);

        repository
            .connection
            .execute(
                "UPDATE providers SET custom_headers_json = '{\"x-cas\":\"one\"}'
                 WHERE id = 'provider-deepseek'",
                [],
            )
            .unwrap();
        let headers_changed = fingerprint();
        assert_ne!(base_url_changed, headers_changed);

        repository
            .connection
            .execute(
                "INSERT INTO agent_required_capabilities VALUES ('agent-1', 'shell')",
                [],
            )
            .unwrap();
        let required_changed = fingerprint();
        assert_ne!(headers_changed, required_changed);

        repository
            .connection
            .execute(
                "INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'browser')",
                [],
            )
            .unwrap();
        let preferred_changed = fingerprint();
        assert_ne!(required_changed, preferred_changed);

        repository
            .connection
            .execute(
                "INSERT INTO model_capabilities (
                    model_id, capability, status, source, confidence
                 ) VALUES (
                    'model-deepseek-v4-flash', 'tool_calls', 'SUPPORTED', 'CAS', 'HIGH'
                 )",
                [],
            )
            .unwrap();
        let model_capabilities_changed = fingerprint();
        assert_ne!(preferred_changed, model_capabilities_changed);

        repository
            .connection
            .execute_batch(
                "DELETE FROM agent_required_capabilities;
                 DELETE FROM agent_preferred_capabilities;
                 DELETE FROM model_capabilities;
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'alpha');
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'beta');
                 INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'gamma');
                 INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'delta');
                 INSERT INTO model_capabilities (
                    model_id, capability, status, source, confidence
                 ) VALUES (
                    'model-deepseek-v4-flash', 'images', 'SUPPORTED', 'CAS', 'HIGH'
                 );
                 INSERT INTO model_capabilities (
                    model_id, capability, status, source, confidence
                 ) VALUES (
                    'model-deepseek-v4-flash', 'shell', 'UNKNOWN', 'CAS', 'HIGH'
                 );",
            )
            .unwrap();
        let ordered = fingerprint();
        repository
            .connection
            .execute_batch(
                "DELETE FROM agent_required_capabilities;
                 DELETE FROM agent_preferred_capabilities;
                 DELETE FROM model_capabilities;
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'beta');
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'alpha');
                 INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'delta');
                 INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'gamma');
                 INSERT INTO model_capabilities (
                    model_id, capability, status, source, confidence
                 ) VALUES (
                    'model-deepseek-v4-flash', 'shell', 'UNKNOWN', 'CAS', 'HIGH'
                 );
                 INSERT INTO model_capabilities (
                    model_id, capability, status, source, confidence
                 ) VALUES (
                    'model-deepseek-v4-flash', 'images', 'SUPPORTED', 'CAS', 'HIGH'
                 );",
            )
            .unwrap();
        assert_eq!(ordered, fingerprint());
    }

    #[test]
    fn execution_rejects_a_stale_reuse_decision() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(100, "FINAL")).unwrap();
        service
            .set_agent_instance_workspace_scope(AgentThreadInstanceWorkspaceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                workspace_scope_key: Some("c:/workspace/project".to_owned()),
            })
            .unwrap();

        let error = service
            .prepare_agent_execution(AgentThreadExecutionRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/project".to_owned(),
                cwd: "C:\\workspace\\project".to_owned(),
                input: "执行任务".to_owned(),
                expected_decision: "SPAWN".to_owned(),
                expected_candidate_thread_id: None,
            })
            .unwrap_err();

        assert_eq!(error.code(), "AGENT_THREAD_DECISION_CHANGED");
    }

    #[test]
    fn execution_requires_cwd_to_match_workspace_scope() {
        let service = UsageService::in_memory();
        seed_agent(&service);
        service
            .repository()
            .unwrap()
            .connection
            .execute(
                "INSERT INTO active_agent_bindings (
                    role_key, agent_id, created_at, updated_at
                 ) VALUES ('executor', 'agent-1', '2026-08-11T10:00:00Z', '2026-08-11T10:00:00Z')",
                [],
            )
            .unwrap();
        let mismatch = service
            .prepare_agent_execution(AgentThreadExecutionRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/project".to_owned(),
                cwd: "C:\\workspace\\other".to_owned(),
                input: "执行任务".to_owned(),
                expected_decision: "SPAWN".to_owned(),
                expected_candidate_thread_id: None,
            })
            .unwrap_err();
        assert_eq!(mismatch.code(), "VALIDATION_ERROR");

        for (workspace_scope_key, cwd) in [
            ("C:\\Workspace\\Project", "c:/workspace/project"),
            ("/work/Foo", "/work/Foo"),
        ] {
            assert!(
                service
                    .prepare_agent_execution(AgentThreadExecutionRequest {
                        agent_id: "agent-1".to_owned(),
                        workspace_scope_key: workspace_scope_key.to_owned(),
                        cwd: cwd.to_owned(),
                        input: "执行任务".to_owned(),
                        expected_decision: "SPAWN".to_owned(),
                        expected_candidate_thread_id: None,
                    })
                    .is_ok()
            );
        }
        let posix_case_mismatch = service
            .prepare_agent_execution(AgentThreadExecutionRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "/work/Foo".to_owned(),
                cwd: "/work/foo".to_owned(),
                input: "执行任务".to_owned(),
                expected_decision: "SPAWN".to_owned(),
                expected_candidate_thread_id: None,
            })
            .unwrap_err();
        assert_eq!(posix_case_mismatch.code(), "VALIDATION_ERROR");
    }

    #[test]
    fn context_pressure_prevents_reuse() {
        let service = UsageService::in_memory();
        let mut pressured = snapshot(100, "FINAL");
        pressured.current_context_tokens = Some(100);
        pressured.model_context_window = Some(100);
        service.upsert_snapshot(pressured).unwrap();
        service
            .set_agent_instance_workspace_scope(AgentThreadInstanceWorkspaceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                workspace_scope_key: Some("c:/workspace/project".to_owned()),
            })
            .unwrap();

        let decision = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/project".to_owned(),
                parent_thread_id: None,
            })
            .unwrap();
        assert_eq!(decision.decision, "SPAWN");
        assert_eq!(decision.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        assert_eq!(decision.context_pressure_percent, Some(100));
    }

    #[test]
    fn missing_current_context_clears_old_value_and_prevents_reuse() {
        let service = UsageService::in_memory();
        let mut initial = snapshot(1_667_247, "FINAL");
        initial.current_context_tokens = Some(50_000);
        initial.model_context_window = Some(258_400);
        service.upsert_snapshot(initial).unwrap();

        let mut unknown = snapshot(1_667_248, "FINAL");
        unknown.current_context_tokens = None;
        unknown.model_context_window = None;
        service.upsert_snapshot(unknown).unwrap();
        service
            .set_agent_instance_workspace_scope(AgentThreadInstanceWorkspaceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                workspace_scope_key: Some("c:/workspace/project".to_owned()),
            })
            .unwrap();

        let decision = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                workspace_scope_key: "c:/workspace/project".to_owned(),
                parent_thread_id: None,
            })
            .unwrap();
        assert_eq!(decision.decision, "SPAWN");
        assert_eq!(decision.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
    }

    #[test]
    fn runtime_context_window_replaces_static_model_metadata() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(100, "LIVE")).unwrap();

        let mut runtime = snapshot(200, "FINAL");
        runtime.model_context_window = Some(258_400);
        service.upsert_snapshot(runtime).unwrap();

        let records = service
            .list(UsageListRequest {
                query: UsageQueryRequest::default(),
                limit: Some(10),
            })
            .unwrap();
        let instances = service
            .list_agent_instances(AgentThreadInstanceListRequest::default())
            .unwrap();
        assert_eq!(records[0].model_context_window, Some(258_400));
        assert_eq!(instances[0].context_window, Some(258_400));
    }

    #[test]
    fn reuse_strategy_and_cache_profile_adjust_soft_context_limit() {
        let mut profile = AgentSchedulingProfile {
            reuse_strategy: "HOT".to_owned(),
            ..AgentSchedulingProfile::default()
        };
        assert_eq!(context_pressure_limit(&profile, 10_000), 90);

        profile.reuse_strategy = "COLD".to_owned();
        assert_eq!(context_pressure_limit(&profile, 10_000), 60);

        profile.cache_support = "SUPPORTED".to_owned();
        profile.cache_retention_hint_seconds = Some(300);
        assert_eq!(context_pressure_limit(&profile, 301), 50);
        assert_eq!(cache_hint(&profile, Some(301)), "OUTSIDE_RETENTION_HINT");

        profile.agent_cache_retention_override_seconds = Some(120);
        assert_eq!(
            effective_cache_retention(&profile),
            (Some(120), "AGENT_OVERRIDE")
        );
        assert_eq!(context_pressure_limit(&profile, 121), 50);

        profile.agent_cache_retention_override_seconds = Some(600);
        assert_eq!(effective_cache_retention(&profile), (Some(300), "PROVIDER"));
    }

    #[test]
    fn attribution_is_backfilled_once_and_can_be_queried() {
        let service = UsageService::in_memory();
        let mut initial = snapshot(10, "LIVE");
        initial.agent_id = None;
        initial.agent_name_snapshot = None;
        service.upsert_snapshot(initial).unwrap();

        let mut attributed = snapshot(20, "FINAL");
        attributed.agent_name_snapshot = Some("执行 Agent".to_owned());
        service.upsert_snapshot(attributed).unwrap();

        let records = service
            .list(UsageListRequest {
                query: UsageQueryRequest {
                    agent_id: Some("agent-1".to_owned()),
                    ..UsageQueryRequest::default()
                },
                limit: Some(10),
            })
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].agent_name_snapshot.as_deref(),
            Some("执行 Agent")
        );
        assert_eq!(
            records[0].provider_name_snapshot.as_deref(),
            Some("DeepSeek")
        );
    }

    #[test]
    fn conflicting_thread_ownership_is_rejected() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(10, "LIVE")).unwrap();

        let mut conflicting = snapshot(20, "LIVE");
        conflicting.codex_session_id = "session-other".to_owned();
        assert!(matches!(
            service.upsert_snapshot(conflicting),
            Err(UsageServiceError::ThreadIdentityConflict)
        ));
    }

    fn seed_agent(service: &UsageService) {
        let repository = service.repository().unwrap();
        let timestamp = "2026-08-11T10:00:00Z";
        repository
            .connection
            .execute(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, created_at, updated_at
                 ) VALUES (
                    'provider-deepseek', 'deepseek', 'DeepSeek', 'PRESET',
                    'https://api.deepseek.com/', 'RESPONSES', 'BEARER_TOKEN',
                    1, 'BUILT_IN', ?1, ?1
                 )",
                [timestamp],
            )
            .unwrap();
        repository
            .connection
            .execute(
                "INSERT INTO models (
                    id, provider_id, model_id, display_name, enabled, source,
                    lifecycle, compatibility_level, compatibility_source, context_window,
                    created_at, updated_at
                 ) VALUES (
                    'model-deepseek-v4-flash', 'provider-deepseek', 'deepseek-v4-flash',
                    'DeepSeek V4 Flash', 1, 'PRESET', 'ACTIVE', 'NATIVE',
                    'CAS_BUILT_IN', 1000000, ?1, ?1
                 )",
                [timestamp],
            )
            .unwrap();
        repository
            .connection
            .execute(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    orchestration_phase, created_at, updated_at
                 ) VALUES (
                    'agent-1', 'executor', 'Executor', '执行任务', '执行任务。',
                    'PRESET', 1, 'WORKSPACE_WRITE', 'HIGH', 'CAS', 1,
                    'executor', 'EXECUTION', ?1, ?1
                 )",
                [timestamp],
            )
            .unwrap();
        repository
            .connection
            .execute(
                "INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, enabled, priority, source, created_at, updated_at
                 ) VALUES (
                    'binding-1', 'agent-1', 'model-deepseek-v4-flash',
                    1, 0, 'CAS', ?1, ?1
                 )",
                [timestamp],
            )
            .unwrap();
    }

    fn snapshot(total_tokens: i64, status: &str) -> UsageSnapshot {
        UsageSnapshot {
            codex_session_id: "session-1".to_owned(),
            codex_thread_id: "thread-child-1".to_owned(),
            parent_thread_id: Some("thread-root-1".to_owned()),
            agent_id: Some("agent-1".to_owned()),
            agent_name_snapshot: Some("Executor".to_owned()),
            provider_id: Some("provider-deepseek".to_owned()),
            provider_name_snapshot: Some("DeepSeek".to_owned()),
            model_id: Some("model-deepseek-v4-flash".to_owned()),
            model_name_snapshot: Some("DeepSeek V4 Flash".to_owned()),
            input_tokens: (total_tokens - 20).max(0),
            cached_input_tokens: (total_tokens - 60).max(0),
            cache_write_input_tokens: 0,
            output_tokens: total_tokens.min(20),
            reasoning_output_tokens: total_tokens.min(5),
            total_tokens,
            current_context_tokens: Some(total_tokens),
            model_context_window: Some(1_000_000),
            usage_status: status.to_owned(),
            source: "CODEX_APP_SERVER".to_owned(),
            started_at: "2026-08-11T10:00:00Z".to_owned(),
            completed_at: (status == "FINAL").then(|| "2026-08-11T10:01:00Z".to_owned()),
            updated_at: if status == "FINAL" {
                "2026-08-11T10:01:00Z".to_owned()
            } else {
                "2026-08-11T10:00:30Z".to_owned()
            },
        }
    }
}
