use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
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

    pub(crate) fn set_agent_instance_scope(
        &self,
        request: AgentThreadInstanceScopeRequest,
    ) -> Result<AgentThreadInstanceResponse, ApiError> {
        let thread_id = validate_runtime_key(&request.thread_id, "threadId")?;
        let scope_key = request
            .scope_key
            .as_deref()
            .map(normalize_scope_key)
            .transpose()?;
        self.repository()?
            .set_agent_instance_scope(&thread_id, scope_key.as_deref())
            .map_err(ApiError::from)
    }

    pub(crate) fn recommend_agent_instance(
        &self,
        request: AgentThreadInstanceRecommendRequest,
    ) -> Result<AgentThreadInstanceRecommendation, ApiError> {
        let agent_id = validate_runtime_key(&request.agent_id, "agentId")?;
        let scope_key = normalize_scope_key(&request.scope_key)?;
        let repository = self.repository()?;
        let profile = repository.scheduling_profile(&agent_id)?;
        let candidates = repository.scope_candidates(&agent_id, &scope_key)?;
        Ok(recommend_instance(scope_key, candidates, profile))
    }

    pub(crate) fn prepare_agent_execution(
        &self,
        request: AgentThreadExecutionRequest,
    ) -> Result<AgentThreadExecutionPlan, ApiError> {
        let agent_id = validate_runtime_key(&request.agent_id, "agentId")?;
        let scope_key = normalize_scope_key(&request.scope_key)?;
        if !matches!(request.expected_decision.as_str(), "REUSE" | "SPAWN") {
            return Err(UsageServiceError::InvalidField("expectedDecision").into());
        }
        let repository = self.repository()?;
        let scheduling_profile = repository.scheduling_profile(&agent_id)?;
        let candidates = repository.scope_candidates(&agent_id, &scope_key)?;
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
            scope_key,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceScopeRequest {
    thread_id: String,
    scope_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceRecommendRequest {
    agent_id: String,
    scope_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadExecutionRequest {
    agent_id: String,
    scope_key: String,
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
    scope_key: String,
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
    pub(crate) scope_key: String,
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
    pub(crate) provider_key: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadInstanceResponse {
    id: String,
    agent_id: Option<String>,
    agent_name_snapshot: Option<String>,
    codex_thread_id: String,
    parent_thread_id: Option<String>,
    scope_key: Option<String>,
    status: String,
    input_tokens: i64,
    cached_input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    context_window: Option<i64>,
    created_at: String,
    last_used_at: String,
    closed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentThreadCandidate {
    instance: AgentThreadInstanceResponse,
    age_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AgentSchedulingProfile {
    reuse_strategy: String,
    cache_support: String,
    cache_retention_type: String,
    cache_retention_hint_seconds: Option<i64>,
    agent_cache_retention_override_seconds: Option<i64>,
}

impl Default for AgentSchedulingProfile {
    fn default() -> Self {
        Self {
            reuse_strategy: "AUTO".to_owned(),
            cache_support: "UNKNOWN".to_owned(),
            cache_retention_type: "UNKNOWN".to_owned(),
            cache_retention_hint_seconds: None,
            agent_cache_retention_override_seconds: None,
        }
    }
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

        sync_agent_thread_instance(&transaction, &record)?;
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
                    total_tokens, context_window, created_at, last_used_at, closed_at
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

    fn set_agent_instance_scope(
        &self,
        thread_id: &str,
        scope_key: Option<&str>,
    ) -> Result<AgentThreadInstanceResponse, UsageRepositoryError> {
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
                        total_tokens, context_window, created_at, last_used_at, closed_at
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
    ) -> Result<Vec<AgentThreadCandidate>, UsageRepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                    scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                    total_tokens, context_window, created_at, last_used_at, closed_at,
                    COALESCE(
                        CAST(MAX(0, (julianday('now') - julianday(last_used_at)) * 86400) AS INTEGER),
                        0
                    )
             FROM agent_thread_instances
             WHERE agent_id = ?1 AND scope_key = ?2
             ORDER BY last_used_at DESC, codex_thread_id ASC",
        )?;
        statement
            .query_map(params![agent_id, scope_key], |row| {
                Ok(AgentThreadCandidate {
                    instance: map_agent_thread_instance(row)?,
                    age_seconds: row.get(15)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(UsageRepositoryError::from)
    }

    fn scheduling_profile(
        &self,
        agent_id: &str,
    ) -> Result<AgentSchedulingProfile, UsageRepositoryError> {
        Ok(self
            .connection
            .query_row(
                "SELECT a.reuse_strategy, a.cache_retention_override_seconds,
                        COALESCE(p.cache_support, 'UNKNOWN'),
                        COALESCE(p.cache_retention_type, 'UNKNOWN'),
                        p.cache_retention_hint_seconds
                 FROM agents a
                 LEFT JOIN agent_model_bindings b ON b.agent_id = a.id AND b.enabled = 1
                 LEFT JOIN models m ON m.id = b.model_id
                 LEFT JOIN providers p ON p.id = m.provider_id
                 WHERE a.id = ?1",
                [agent_id],
                |row| {
                    Ok(AgentSchedulingProfile {
                        reuse_strategy: row.get(0)?,
                        agent_cache_retention_override_seconds: row.get(1)?,
                        cache_support: row.get(2)?,
                        cache_retention_type: row.get(3)?,
                        cache_retention_hint_seconds: row.get(4)?,
                    })
                },
            )
            .optional()?
            .unwrap_or_default())
    }

    fn agent_runtime_profile(
        &self,
        agent_id: &str,
    ) -> Result<Option<AgentRuntimeProfile>, UsageRepositoryError> {
        self.connection
            .query_row(
                "SELECT a.id, a.agent_key, a.name, a.instruction, a.sandbox_policy,
                        a.reasoning_policy, m.model_id, p.provider_key
                 FROM active_agent_bindings active
                 JOIN agents a ON a.id = active.agent_id AND a.enabled = 1
                 JOIN agent_model_bindings binding
                   ON binding.agent_id = a.id AND binding.enabled = 1
                 JOIN models m ON m.id = binding.model_id AND m.enabled = 1
                 JOIN providers p ON p.id = m.provider_id AND p.enabled = 1
                 WHERE a.id = ?1",
                [agent_id],
                |row| {
                    Ok(AgentRuntimeProfile {
                        agent_id: row.get(0)?,
                        agent_key: row.get(1)?,
                        agent_name: row.get(2)?,
                        instruction: row.get(3)?,
                        sandbox_policy: row.get(4)?,
                        reasoning_policy: row.get(5)?,
                        model_slug: row.get(6)?,
                        provider_key: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(UsageRepositoryError::from)
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
                total_tokens, context_window, created_at, last_used_at, closed_at
             ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5, ?6, 0, 0, 0, 0, NULL,
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), NULL
             )
             ON CONFLICT(codex_thread_id) DO UPDATE SET
                scope_key = excluded.scope_key,
                status = excluded.status,
                last_used_at = excluded.last_used_at",
            params![
                Uuid::new_v4().to_string(),
                profile.agent_id,
                profile.agent_name,
                thread_id,
                scope_key,
                status,
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
        scope_key: row.get(5)?,
        status: row.get(6)?,
        input_tokens: row.get(7)?,
        cached_input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
        total_tokens: row.get(10)?,
        context_window: row.get(11)?,
        created_at: row.get(12)?,
        last_used_at: row.get(13)?,
        closed_at: row.get(14)?,
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
            total_tokens, context_window, created_at, last_used_at, closed_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL
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
            context_window = COALESCE(
                agent_thread_instances.context_window,
                excluded.context_window
            ),
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
            record.model_context_window,
            record.started_at,
            record.updated_at,
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
    record.model_context_window = record
        .model_context_window
        .or(snapshot.model_context_window);
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

fn normalize_scope_key(value: &str) -> Result<String, UsageServiceError> {
    let value = value.trim().replace('\\', "/").to_ascii_lowercase();
    let valid = !value.is_empty()
        && value.len() <= 200
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-' | b':')
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && !value.contains("//");
    valid
        .then_some(value)
        .ok_or(UsageServiceError::InvalidField("scopeKey"))
}

fn recommend_instance(
    scope_key: String,
    candidates: Vec<AgentThreadCandidate>,
    profile: AgentSchedulingProfile,
) -> AgentThreadInstanceRecommendation {
    if let Some(candidate) = candidates.iter().find(|candidate| {
        candidate.instance.status == "IDLE"
            && context_pressure_percent(&candidate.instance).map_or(true, |percent| {
                percent < context_pressure_limit(&profile, candidate.age_seconds)
            })
    }) {
        return recommendation(
            "REUSE",
            "EXACT_SCOPE_IDLE",
            "存在同一 Agent、Scope 完全一致且符合当前复用偏好的空闲 Thread。",
            scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.instance.status == "IDLE")
    {
        let base_limit = base_context_pressure_limit(&profile.reuse_strategy);
        let limit = context_pressure_limit(&profile, candidate.age_seconds);
        let pressure = context_pressure_percent(&candidate.instance);
        let cache_adjusted = limit < base_limit && pressure.is_some_and(|value| value < base_limit);
        return recommendation(
            "SPAWN",
            if cache_adjusted {
                "CACHE_HINT_PRESSURE"
            } else {
                "CONTEXT_PRESSURE"
            },
            if cache_adjusted {
                "Provider 缓存提示已降低复用倾向，当前 Context 压力建议新建 Thread。"
            } else {
                "同 Scope 的空闲 Thread 累计输入已达到当前策略阈值，建议新建。"
            },
            scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates.first() {
        return recommendation(
            "SPAWN",
            "NO_HEALTHY_IDLE_THREAD",
            "同 Scope Thread 当前不可安全复用，建议新建。",
            scope_key,
            Some(candidate),
            &profile,
        );
    }
    recommendation(
        "SPAWN",
        "NO_SCOPE_MATCH",
        "没有同一 Agent 且 Scope 完全一致的 Thread，建议新建。",
        scope_key,
        None,
        &profile,
    )
}

fn recommendation(
    decision: &'static str,
    reason_code: &'static str,
    message: &'static str,
    scope_key: String,
    candidate: Option<&AgentThreadCandidate>,
    profile: &AgentSchedulingProfile,
) -> AgentThreadInstanceRecommendation {
    let age_seconds = candidate.map(|candidate| candidate.age_seconds);
    let (cache_retention_hint_seconds, cache_retention_source) = effective_cache_retention(profile);
    AgentThreadInstanceRecommendation {
        decision,
        reason_code,
        message,
        scope_key,
        candidate_instance_id: candidate.map(|candidate| candidate.instance.id.clone()),
        candidate_thread_id: candidate.map(|candidate| candidate.instance.codex_thread_id.clone()),
        context_pressure_percent: candidate
            .and_then(|candidate| context_pressure_percent(&candidate.instance)),
        context_pressure_limit_percent: context_pressure_limit(
            profile,
            age_seconds.unwrap_or_default(),
        ),
        reuse_strategy: profile.reuse_strategy.clone(),
        cache_support: profile.cache_support.clone(),
        cache_retention_type: profile.cache_retention_type.clone(),
        cache_retention_hint_seconds,
        cache_retention_source,
        cache_hint: cache_hint(profile, age_seconds),
        candidate_age_seconds: age_seconds,
    }
}

fn base_context_pressure_limit(strategy: &str) -> i64 {
    match strategy {
        "HOT" => 90,
        "COLD" => 60,
        _ => 80,
    }
}

fn context_pressure_limit(profile: &AgentSchedulingProfile, age_seconds: i64) -> i64 {
    let base = base_context_pressure_limit(&profile.reuse_strategy);
    if profile.reuse_strategy == "HOT" {
        return base;
    }
    let (retention, _) = effective_cache_retention(profile);
    let cache_penalty = profile.cache_support == "UNSUPPORTED"
        || retention.is_some_and(|retention| age_seconds > retention);
    if cache_penalty { base - 10 } else { base }
}

fn cache_hint(profile: &AgentSchedulingProfile, age_seconds: Option<i64>) -> &'static str {
    if profile.cache_support == "UNSUPPORTED" {
        return "UNSUPPORTED";
    }
    match (effective_cache_retention(profile).0, age_seconds) {
        (Some(retention), Some(age)) if age <= retention => "WITHIN_RETENTION_HINT",
        (Some(_), Some(_)) => "OUTSIDE_RETENTION_HINT",
        (None, _) if profile.cache_support == "SUPPORTED" => "SUPPORTED_NO_RETENTION_HINT",
        _ => "UNKNOWN",
    }
}

fn effective_cache_retention(profile: &AgentSchedulingProfile) -> (Option<i64>, &'static str) {
    if profile.cache_support == "UNSUPPORTED" {
        return (None, "PROVIDER");
    }
    match (
        profile.agent_cache_retention_override_seconds,
        profile.cache_retention_hint_seconds,
    ) {
        (Some(agent), Some(provider)) if agent <= provider => (Some(agent), "AGENT_OVERRIDE"),
        (Some(_), Some(provider)) => (Some(provider), "PROVIDER"),
        (Some(agent), None) => (Some(agent), "AGENT_OVERRIDE"),
        (None, Some(provider)) => (Some(provider), "PROVIDER"),
        (None, None) => (None, "NONE"),
    }
}

fn context_pressure_percent(instance: &AgentThreadInstanceResponse) -> Option<i64> {
    instance.context_window.map(|window| {
        instance
            .input_tokens
            .saturating_mul(100)
            .checked_div(window)
            .unwrap_or(100)
            .clamp(0, 100)
    })
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
    fn exact_scope_idle_thread_is_reused_and_other_scope_spawns() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(100, "FINAL")).unwrap();
        let updated = service
            .set_agent_instance_scope(AgentThreadInstanceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                scope_key: Some("Order\\Refund".to_owned()),
            })
            .unwrap();
        assert_eq!(updated.scope_key.as_deref(), Some("order/refund"));

        let reuse = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                scope_key: "order/refund".to_owned(),
            })
            .unwrap();
        assert_eq!(reuse.decision, "REUSE");
        assert_eq!(reuse.candidate_thread_id.as_deref(), Some("thread-child-1"));
        assert_eq!(reuse.reuse_strategy, "AUTO");
        assert_eq!(reuse.context_pressure_limit_percent, 80);

        let spawn = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                scope_key: "auth/oauth2".to_owned(),
            })
            .unwrap();
        assert_eq!(spawn.decision, "SPAWN");
        assert_eq!(spawn.reason_code, "NO_SCOPE_MATCH");
    }

    #[test]
    fn execution_rejects_a_stale_reuse_decision() {
        let service = UsageService::in_memory();
        service.upsert_snapshot(snapshot(100, "FINAL")).unwrap();
        service
            .set_agent_instance_scope(AgentThreadInstanceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                scope_key: Some("order/refund".to_owned()),
            })
            .unwrap();

        let error = service
            .prepare_agent_execution(AgentThreadExecutionRequest {
                agent_id: "agent-1".to_owned(),
                scope_key: "order/refund".to_owned(),
                cwd: "C:\\workspace\\project".to_owned(),
                input: "执行任务".to_owned(),
                expected_decision: "SPAWN".to_owned(),
                expected_candidate_thread_id: None,
            })
            .unwrap_err();

        assert_eq!(error.code(), "AGENT_THREAD_DECISION_CHANGED");
    }

    #[test]
    fn context_pressure_prevents_reuse() {
        let service = UsageService::in_memory();
        let mut pressured = snapshot(100, "FINAL");
        pressured.model_context_window = Some(100);
        service.upsert_snapshot(pressured).unwrap();
        service
            .set_agent_instance_scope(AgentThreadInstanceScopeRequest {
                thread_id: "thread-child-1".to_owned(),
                scope_key: Some("order/refund".to_owned()),
            })
            .unwrap();

        let decision = service
            .recommend_agent_instance(AgentThreadInstanceRecommendRequest {
                agent_id: "agent-1".to_owned(),
                scope_key: "order/refund".to_owned(),
            })
            .unwrap();
        assert_eq!(decision.decision, "SPAWN");
        assert_eq!(decision.reason_code, "CONTEXT_PRESSURE");
        assert_eq!(decision.context_pressure_percent, Some(80));
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
