use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::persistence::{PersistenceError, open_database};
use crate::provider::ApiError;

pub(crate) struct AgentService {
    repository: Mutex<SqliteAgentRepository>,
}

impl AgentService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, AgentServiceError> {
        Ok(Self {
            repository: Mutex::new(SqliteAgentRepository::open(database_path)?),
        })
    }

    pub(crate) fn presets(&self) -> Vec<AgentPresetResponse> {
        AGENT_PRESETS
            .iter()
            .map(AgentPresetResponse::from)
            .collect()
    }

    pub(crate) fn list(&self, request: AgentListRequest) -> Result<Vec<AgentSummary>, ApiError> {
        let search = request.search.as_deref().map(str::trim);
        self.repository()?
            .list(search.filter(|value| !value.is_empty()), request.enabled)
            .map(|agents| agents.into_iter().map(AgentSummary::from).collect())
            .map_err(ApiError::from)
    }

    pub(crate) fn get(&self, request: AgentGetRequest) -> Result<AgentDetailResponse, ApiError> {
        let id = parse_uuid(&request.agent_id, "agentId")?;
        self.repository()?
            .find_by_id(&id)
            .map(AgentDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn create(
        &self,
        request: AgentCreateRequest,
    ) -> Result<AgentDetailResponse, ApiError> {
        let pending = NewAgent::try_from(request)?;
        self.repository()?
            .add(&pending)
            .map(AgentDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn update(
        &self,
        request: AgentUpdateRequest,
    ) -> Result<AgentDetailResponse, ApiError> {
        let changes = AgentChanges::try_from(request)?;
        self.repository()?
            .update(&changes)
            .map(AgentDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn set_enabled(
        &self,
        request: AgentSetEnabledRequest,
    ) -> Result<AgentDetailResponse, ApiError> {
        let id = parse_uuid(&request.agent_id, "agentId")?;
        self.repository()?
            .set_enabled(&id, request.enabled)
            .map(AgentDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn set_model_binding(
        &self,
        request: AgentSetModelBindingRequest,
    ) -> Result<AgentBindingResponse, ApiError> {
        let agent_id = parse_uuid(&request.agent_id, "agentId")?;
        let model_id = parse_uuid(&request.model_id, "modelId")?;
        self.repository()?
            .set_model_binding(&agent_id, &model_id)
            .map(AgentBindingResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn remove_model_binding(
        &self,
        request: AgentRemoveModelBindingRequest,
    ) -> Result<AgentDetailResponse, ApiError> {
        let id = parse_uuid(&request.agent_id, "agentId")?;
        self.repository()?
            .remove_model_binding(&id)
            .map(AgentDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn delete(&self, request: AgentDeleteRequest) -> Result<(), ApiError> {
        let id = parse_uuid(&request.agent_id, "agentId")?;
        self.repository()?.delete(&id).map_err(ApiError::from)
    }

    fn repository(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, SqliteAgentRepository>, AgentServiceError> {
        self.repository
            .lock()
            .map_err(|_| AgentServiceError::DatabaseUnavailable)
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            repository: Mutex::new(SqliteAgentRepository::in_memory().unwrap()),
        }
    }
}

struct SqliteAgentRepository {
    connection: Connection,
}

impl SqliteAgentRepository {
    fn open(path: &Path) -> Result<Self, AgentRepositoryError> {
        Ok(Self {
            connection: open_database(path)?,
        })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, AgentRepositoryError> {
        Ok(Self {
            connection: crate::persistence::open_in_memory()?,
        })
    }

    fn add(&mut self, agent: &NewAgent) -> Result<AgentRecord, AgentRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let timestamp = now(&transaction)?;
        transaction.execute(
            "INSERT INTO agents (
                id, agent_key, name, description, instruction, agent_type, enabled,
                sandbox_policy, reasoning_policy, source, managed, minimum_context_window,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?12)",
            params![
                agent.id,
                agent.agent_key,
                agent.name,
                agent.description,
                agent.instruction,
                agent.agent_type,
                agent.enabled,
                agent.sandbox_policy,
                agent.reasoning_policy,
                agent.source,
                agent.minimum_context_window,
                timestamp,
            ],
        )?;
        insert_capabilities(
            &transaction,
            "agent_required_capabilities",
            &agent.id,
            &agent.required_capabilities,
        )?;
        insert_capabilities(
            &transaction,
            "agent_preferred_capabilities",
            &agent.id,
            &agent.preferred_capabilities,
        )?;

        if let Some(model_id) = &agent.model_id {
            let model =
                load_model(&transaction, model_id)?.ok_or(AgentRepositoryError::ModelNotFound)?;
            let compatibility = evaluate_compatibility(
                &agent.required_capabilities,
                &agent.preferred_capabilities,
                agent.minimum_context_window,
                &model,
            );
            if compatibility.status == BindingCompatibilityStatus::Incompatible {
                return Err(AgentRepositoryError::IncompatibleModel);
            }
            insert_binding(&transaction, &agent.id, model_id, &timestamp)?;
        }
        transaction.commit()?;
        self.find_by_id(&agent.id)
    }

    fn update(&mut self, changes: &AgentChanges) -> Result<AgentRecord, AgentRepositoryError> {
        let changed = self.connection.execute(
            "UPDATE agents
             SET name = ?2, description = ?3, instruction = ?4, sandbox_policy = ?5,
                 reasoning_policy = ?6, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![
                changes.id,
                changes.name,
                changes.description,
                changes.instruction,
                changes.sandbox_policy,
                changes.reasoning_policy,
            ],
        )?;
        ensure_changed(changed)?;
        self.find_by_id(&changes.id)
    }

    fn set_enabled(
        &mut self,
        id: &str,
        enabled: bool,
    ) -> Result<AgentRecord, AgentRepositoryError> {
        let changed = self.connection.execute(
            "UPDATE agents
             SET enabled = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![id, enabled],
        )?;
        ensure_changed(changed)?;
        self.find_by_id(id)
    }

    fn set_model_binding(
        &mut self,
        agent_id: &str,
        model_id: &str,
    ) -> Result<AgentRecord, AgentRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let agent = load_agent(&transaction, agent_id)?.ok_or(AgentRepositoryError::NotFound)?;
        let model =
            load_model(&transaction, model_id)?.ok_or(AgentRepositoryError::ModelNotFound)?;
        let compatibility = evaluate_compatibility(
            &agent.required_capabilities,
            &agent.preferred_capabilities,
            agent.minimum_context_window,
            &model,
        );
        if compatibility.status == BindingCompatibilityStatus::Incompatible {
            return Err(AgentRepositoryError::IncompatibleModel);
        }
        let timestamp = now(&transaction)?;
        transaction.execute(
            "INSERT INTO agent_model_bindings (
                id, agent_id, model_id, enabled, priority, source, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 1, 0, 'USER', ?4, ?4)
             ON CONFLICT(agent_id) DO UPDATE SET
                model_id = excluded.model_id,
                enabled = 1,
                source = 'USER',
                updated_at = excluded.updated_at",
            params![Uuid::new_v4().to_string(), agent_id, model_id, timestamp],
        )?;
        transaction.commit()?;
        self.find_by_id(agent_id)
    }

    fn remove_model_binding(&mut self, id: &str) -> Result<AgentRecord, AgentRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM agents WHERE id = ?1)",
            [id],
            |row| row.get::<_, bool>(0),
        )?;
        if !exists {
            return Err(AgentRepositoryError::NotFound);
        }
        transaction.execute("DELETE FROM agent_model_bindings WHERE agent_id = ?1", [id])?;
        transaction.execute(
            "UPDATE agents
             SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
            [id],
        )?;
        transaction.commit()?;
        self.find_by_id(id)
    }

    fn delete(&mut self, id: &str) -> Result<(), AgentRepositoryError> {
        let active = self.connection.query_row(
            "SELECT active_agent_id = ?1 FROM configuration_state WHERE id = 1",
            [id],
            |row| row.get::<_, Option<bool>>(0),
        )?;
        if active == Some(true) {
            return Err(AgentRepositoryError::Active);
        }
        ensure_changed(
            self.connection
                .execute("DELETE FROM agents WHERE id = ?1", [id])?,
        )
    }

    fn find_by_id(&self, id: &str) -> Result<AgentRecord, AgentRepositoryError> {
        load_agent(&self.connection, id)?.ok_or(AgentRepositoryError::NotFound)
    }

    fn list(
        &self,
        search: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Vec<AgentRecord>, AgentRepositoryError> {
        let search = search.map(|value| format!("%{value}%"));
        let enabled = enabled.map(i64::from);
        let ids = {
            let mut statement = self.connection.prepare(
                "SELECT id FROM agents
                 WHERE (?1 IS NULL OR agent_key LIKE ?1 OR name LIKE ?1 OR description LIKE ?1)
                   AND (?2 IS NULL OR enabled = ?2)
                 ORDER BY name COLLATE NOCASE, agent_key COLLATE NOCASE",
            )?;
            statement
                .query_map(params![search, enabled], |row| row.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?
        };
        ids.iter()
            .map(|id| self.find_by_id(id))
            .collect::<Result<Vec<_>, _>>()
    }
}

fn load_agent(
    connection: &Connection,
    id: &str,
) -> Result<Option<AgentRecord>, AgentRepositoryError> {
    let mut agent = connection
        .query_row(
            "SELECT a.id, a.agent_key, a.name, a.description, a.instruction, a.agent_type,
                    a.enabled, a.sandbox_policy, a.reasoning_policy, a.source, a.managed,
                    a.minimum_context_window, a.created_at, a.updated_at, m.id, m.provider_id,
                    p.name, m.model_id, m.display_name, m.enabled, p.enabled, m.lifecycle,
                    m.compatibility_level, m.context_window
             FROM agents a
             LEFT JOIN agent_model_bindings b ON b.agent_id = a.id AND b.enabled = 1
             LEFT JOIN models m ON m.id = b.model_id
             LEFT JOIN providers p ON p.id = m.provider_id
             WHERE a.id = ?1",
            [id],
            map_agent,
        )
        .optional()?;
    if let Some(record) = agent.as_mut() {
        record.required_capabilities =
            load_capabilities(connection, "agent_required_capabilities", &record.id)?;
        record.preferred_capabilities =
            load_capabilities(connection, "agent_preferred_capabilities", &record.id)?;
        if let Some(model) = record.model.as_mut() {
            model.capabilities = load_model_capabilities(connection, &model.reference.id)?;
            record.compatibility = evaluate_compatibility(
                &record.required_capabilities,
                &record.preferred_capabilities,
                record.minimum_context_window,
                model,
            );
        }
        record.availability = availability(record);
    }
    Ok(agent)
}

fn load_model(
    connection: &Connection,
    id: &str,
) -> Result<Option<BindingModelRecord>, AgentRepositoryError> {
    let mut model = connection
        .query_row(
            "SELECT m.id, m.provider_id, p.name, m.model_id, m.display_name, m.enabled,
                    p.enabled, m.lifecycle, m.compatibility_level, m.context_window
             FROM models m
             JOIN providers p ON p.id = m.provider_id
             WHERE m.id = ?1",
            [id],
            map_binding_model,
        )
        .optional()?;
    if let Some(record) = model.as_mut() {
        record.capabilities = load_model_capabilities(connection, &record.reference.id)?;
    }
    Ok(model)
}

fn map_agent(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRecord> {
    let model_id = row.get::<_, Option<String>>(14)?;
    let model = if let Some(id) = model_id {
        Some(BindingModelRecord {
            reference: AgentModelReference {
                id,
                provider_id: row.get::<_, Option<String>>(15)?.unwrap_or_default(),
                provider_name: row.get::<_, Option<String>>(16)?.unwrap_or_default(),
                model_id: row.get::<_, Option<String>>(17)?.unwrap_or_default(),
                display_name: row.get::<_, Option<String>>(18)?.unwrap_or_default(),
            },
            model_enabled: row.get::<_, Option<i64>>(19)?.unwrap_or_default() != 0,
            provider_enabled: row.get::<_, Option<i64>>(20)?.unwrap_or_default() != 0,
            lifecycle: row
                .get::<_, Option<String>>(21)?
                .unwrap_or_else(|| "UNKNOWN".into()),
            compatibility_level: row
                .get::<_, Option<String>>(22)?
                .unwrap_or_else(|| "UNKNOWN".into()),
            context_window: row.get(23)?,
            capabilities: HashMap::new(),
        })
    } else {
        None
    };
    Ok(AgentRecord {
        id: row.get(0)?,
        agent_key: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        instruction: row.get(4)?,
        agent_type: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        sandbox_policy: row.get(7)?,
        reasoning_policy: row.get(8)?,
        source: row.get(9)?,
        managed: row.get::<_, i64>(10)? != 0,
        minimum_context_window: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        required_capabilities: Vec::new(),
        preferred_capabilities: Vec::new(),
        model,
        compatibility: missing_model_compatibility(),
        availability: AgentAvailability::ModelMissing,
    })
}

fn map_binding_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<BindingModelRecord> {
    Ok(BindingModelRecord {
        reference: AgentModelReference {
            id: row.get(0)?,
            provider_id: row.get(1)?,
            provider_name: row.get(2)?,
            model_id: row.get(3)?,
            display_name: row.get(4)?,
        },
        model_enabled: row.get::<_, i64>(5)? != 0,
        provider_enabled: row.get::<_, i64>(6)? != 0,
        lifecycle: row.get(7)?,
        compatibility_level: row.get(8)?,
        context_window: row.get(9)?,
        capabilities: HashMap::new(),
    })
}

fn load_capabilities(
    connection: &Connection,
    table: &str,
    agent_id: &str,
) -> Result<Vec<String>, AgentRepositoryError> {
    let mut statement = connection.prepare(&format!(
        "SELECT capability FROM {table} WHERE agent_id = ?1 ORDER BY capability"
    ))?;
    statement
        .query_map([agent_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AgentRepositoryError::from)
}

fn load_model_capabilities(
    connection: &Connection,
    model_id: &str,
) -> Result<HashMap<String, String>, AgentRepositoryError> {
    let mut statement = connection
        .prepare("SELECT capability, status FROM model_capabilities WHERE model_id = ?1")?;
    statement
        .query_map([model_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(AgentRepositoryError::from)
}

fn insert_capabilities(
    connection: &Connection,
    table: &str,
    agent_id: &str,
    capabilities: &[String],
) -> rusqlite::Result<()> {
    for capability in capabilities {
        connection.execute(
            &format!("INSERT INTO {table} (agent_id, capability) VALUES (?1, ?2)"),
            params![agent_id, capability],
        )?;
    }
    Ok(())
}

fn insert_binding(
    connection: &Connection,
    agent_id: &str,
    model_id: &str,
    timestamp: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO agent_model_bindings (
            id, agent_id, model_id, enabled, priority, source, created_at, updated_at
         ) VALUES (?1, ?2, ?3, 1, 0, 'USER', ?4, ?4)",
        params![Uuid::new_v4().to_string(), agent_id, model_id, timestamp],
    )?;
    Ok(())
}

fn now(connection: &Connection) -> rusqlite::Result<String> {
    connection.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
        row.get(0)
    })
}

fn ensure_changed(changed: usize) -> Result<(), AgentRepositoryError> {
    (changed > 0)
        .then_some(())
        .ok_or(AgentRepositoryError::NotFound)
}

fn evaluate_compatibility(
    required: &[String],
    preferred: &[String],
    minimum_context_window: Option<i64>,
    model: &BindingModelRecord,
) -> AgentBindingCompatibility {
    let mut issues = Vec::new();
    let mut incompatible = false;
    let mut unknown = false;
    let mut warning = false;

    let mut issue = |code: &str, severity: CompatibilityIssueSeverity, message: String| {
        match severity {
            CompatibilityIssueSeverity::Error => incompatible = true,
            CompatibilityIssueSeverity::Warning => warning = true,
        }
        issues.push(CompatibilityIssueResponse {
            code: code.to_owned(),
            severity,
            message,
            source: "CAS_RULE".to_owned(),
        });
    };

    if !model.provider_enabled {
        issue(
            "PROVIDER_DISABLED",
            CompatibilityIssueSeverity::Error,
            "Model 所属 Provider 已停用。".to_owned(),
        );
    }
    if !model.model_enabled {
        issue(
            "MODEL_DISABLED",
            CompatibilityIssueSeverity::Error,
            "Model 已停用。".to_owned(),
        );
    }
    match model.compatibility_level.as_str() {
        "UNSUPPORTED" | "GATEWAY_REQUIRED" => issue(
            "MODEL_CODEX_INCOMPATIBLE",
            CompatibilityIssueSeverity::Error,
            "Model 不支持当前 Codex 直连方式。".to_owned(),
        ),
        "UNKNOWN" => {
            unknown = true;
            issue(
                "MODEL_COMPATIBILITY_UNKNOWN",
                CompatibilityIssueSeverity::Warning,
                "Model 的 Codex 兼容性尚未确认。".to_owned(),
            );
        }
        _ => {}
    }
    match model.lifecycle.as_str() {
        "DEPRECATED" | "PREVIEW" => issue(
            "MODEL_LIFECYCLE_WARNING",
            CompatibilityIssueSeverity::Warning,
            format!("Model lifecycle 为 {}。", model.lifecycle),
        ),
        "UNKNOWN" => unknown = true,
        _ => {}
    }

    for capability in required {
        match model.capabilities.get(capability).map(String::as_str) {
            Some("SUPPORTED") => {}
            Some("UNSUPPORTED") => issue(
                "REQUIRED_CAPABILITY_UNSUPPORTED",
                CompatibilityIssueSeverity::Error,
                format!("缺少必需能力：{capability}。"),
            ),
            _ => {
                unknown = true;
                issue(
                    "REQUIRED_CAPABILITY_UNKNOWN",
                    CompatibilityIssueSeverity::Warning,
                    format!("必需能力尚未确认：{capability}。"),
                );
            }
        }
    }
    for capability in preferred {
        if !matches!(
            model.capabilities.get(capability).map(String::as_str),
            Some("SUPPORTED")
        ) {
            issue(
                "PREFERRED_CAPABILITY_MISSING",
                CompatibilityIssueSeverity::Warning,
                format!("推荐能力不可用或未知：{capability}。"),
            );
        }
    }
    if let Some(minimum) = minimum_context_window {
        match model.context_window {
            Some(actual) if actual < minimum => issue(
                "CONTEXT_WINDOW_TOO_SMALL",
                CompatibilityIssueSeverity::Error,
                format!("Context Window {actual} 小于要求的 {minimum}。"),
            ),
            None => {
                unknown = true;
                issue(
                    "CONTEXT_WINDOW_UNKNOWN",
                    CompatibilityIssueSeverity::Warning,
                    "Model Context Window 未知。".to_owned(),
                );
            }
            _ => {}
        }
    }

    let status = if incompatible {
        BindingCompatibilityStatus::Incompatible
    } else if unknown {
        BindingCompatibilityStatus::Unknown
    } else if warning {
        BindingCompatibilityStatus::Warning
    } else {
        BindingCompatibilityStatus::Compatible
    };
    AgentBindingCompatibility { status, issues }
}

fn missing_model_compatibility() -> AgentBindingCompatibility {
    AgentBindingCompatibility {
        status: BindingCompatibilityStatus::Unknown,
        issues: vec![CompatibilityIssueResponse {
            code: "MODEL_MISSING".to_owned(),
            severity: CompatibilityIssueSeverity::Warning,
            message: "Agent 尚未绑定 Model。".to_owned(),
            source: "CAS_RULE".to_owned(),
        }],
    }
}

fn availability(agent: &AgentRecord) -> AgentAvailability {
    let Some(model) = &agent.model else {
        return AgentAvailability::ModelMissing;
    };
    if !model.provider_enabled {
        return AgentAvailability::ProviderUnavailable;
    }
    if !model.model_enabled {
        return AgentAvailability::InvalidConfiguration;
    }
    if agent.compatibility.status == BindingCompatibilityStatus::Incompatible {
        return AgentAvailability::IncompatibleModel;
    }
    AgentAvailability::Ready
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentGetRequest {
    agent_id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentListRequest {
    search: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentCreateRequest {
    agent_key: String,
    name: String,
    description: String,
    instruction: String,
    template_key: Option<String>,
    enabled: bool,
    sandbox_policy: SandboxPolicy,
    reasoning_policy: ReasoningPolicy,
    model_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentUpdateRequest {
    agent_id: String,
    name: String,
    description: String,
    instruction: String,
    sandbox_policy: SandboxPolicy,
    reasoning_policy: ReasoningPolicy,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSetEnabledRequest {
    agent_id: String,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSetModelBindingRequest {
    agent_id: String,
    model_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentRemoveModelBindingRequest {
    agent_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentDeleteRequest {
    agent_id: String,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum SandboxPolicy {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
    Inherit,
}

impl SandboxPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "READ_ONLY",
            Self::WorkspaceWrite => "WORKSPACE_WRITE",
            Self::DangerFullAccess => "DANGER_FULL_ACCESS",
            Self::Inherit => "INHERIT",
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ReasoningPolicy {
    Inherit,
    Low,
    Medium,
    High,
    ModelDefault,
}

impl ReasoningPolicy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "INHERIT",
            Self::Low => "LOW",
            Self::Medium => "MEDIUM",
            Self::High => "HIGH",
            Self::ModelDefault => "MODEL_DEFAULT",
        }
    }
}

struct NewAgent {
    id: String,
    agent_key: String,
    name: String,
    description: String,
    instruction: String,
    agent_type: &'static str,
    enabled: bool,
    sandbox_policy: &'static str,
    reasoning_policy: &'static str,
    source: &'static str,
    minimum_context_window: Option<i64>,
    required_capabilities: Vec<String>,
    preferred_capabilities: Vec<String>,
    model_id: Option<String>,
}

impl TryFrom<AgentCreateRequest> for NewAgent {
    type Error = AgentServiceError;

    fn try_from(request: AgentCreateRequest) -> Result<Self, Self::Error> {
        let preset = request
            .template_key
            .as_deref()
            .map(find_preset)
            .transpose()?
            .flatten();
        let agent_key = value_or_default(request.agent_key, preset.map(|value| value.key));
        validate_agent_key(&agent_key)?;
        let name = value_or_default(request.name, preset.map(|value| value.name));
        validate_text(&name, "name", 160)?;
        let description =
            value_or_default(request.description, preset.map(|value| value.description));
        validate_text(&description, "description", 2_000)?;
        let instruction =
            value_or_default(request.instruction, preset.map(|value| value.instruction));
        validate_text(&instruction, "instruction", 100_000)?;
        let model_id = request
            .model_id
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_uuid(&value, "modelId"))
            .transpose()?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            agent_key,
            name,
            description,
            instruction,
            agent_type: if preset.is_some() { "PRESET" } else { "CUSTOM" },
            enabled: request.enabled,
            sandbox_policy: request.sandbox_policy.as_str(),
            reasoning_policy: request.reasoning_policy.as_str(),
            source: if preset.is_some() { "CAS" } else { "USER" },
            minimum_context_window: preset.and_then(|value| value.minimum_context_window),
            required_capabilities: preset
                .map(|value| strings(value.required_capabilities))
                .unwrap_or_default(),
            preferred_capabilities: preset
                .map(|value| strings(value.preferred_capabilities))
                .unwrap_or_default(),
            model_id,
        })
    }
}

struct AgentChanges {
    id: String,
    name: String,
    description: String,
    instruction: String,
    sandbox_policy: &'static str,
    reasoning_policy: &'static str,
}

impl TryFrom<AgentUpdateRequest> for AgentChanges {
    type Error = AgentServiceError;

    fn try_from(request: AgentUpdateRequest) -> Result<Self, Self::Error> {
        let id = parse_uuid(&request.agent_id, "agentId")?;
        validate_text(&request.name, "name", 160)?;
        validate_text(&request.description, "description", 2_000)?;
        validate_text(&request.instruction, "instruction", 100_000)?;
        Ok(Self {
            id,
            name: request.name.trim().to_owned(),
            description: request.description.trim().to_owned(),
            instruction: request.instruction.trim().to_owned(),
            sandbox_policy: request.sandbox_policy.as_str(),
            reasoning_policy: request.reasoning_policy.as_str(),
        })
    }
}

fn value_or_default(value: String, default: Option<&str>) -> String {
    if value.trim().is_empty() {
        default.unwrap_or_default().to_owned()
    } else {
        value.trim().to_owned()
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn parse_uuid(value: &str, field: &'static str) -> Result<String, AgentServiceError> {
    Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| AgentServiceError::InvalidField(field))
}

fn validate_agent_key(value: &str) -> Result<(), AgentServiceError> {
    let mut bytes = value.bytes();
    let valid = !value.is_empty()
        && value.len() <= 64
        && bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    valid
        .then_some(())
        .ok_or(AgentServiceError::InvalidField("agentKey"))
}

fn validate_text(
    value: &str,
    field: &'static str,
    max_length: usize,
) -> Result<(), AgentServiceError> {
    (!value.trim().is_empty() && value.len() <= max_length)
        .then_some(())
        .ok_or(AgentServiceError::InvalidField(field))
}

struct AgentPresetDefinition {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    instruction: &'static str,
    sandbox_policy: &'static str,
    reasoning_policy: &'static str,
    required_capabilities: &'static [&'static str],
    preferred_capabilities: &'static [&'static str],
    minimum_context_window: Option<i64>,
}

const COMMON_REQUIRED: &[&str] = &["TOOL_CALLING", "CODEX_MULTI_AGENT"];
const COMMON_PREFERRED: &[&str] = &["PARALLEL_TOOL_CALLING"];
const AGENT_PRESETS: &[AgentPresetDefinition] = &[
    AgentPresetDefinition {
        key: "executor",
        name: "Executor",
        description: "在技术方向明确后负责代码实现、修改与实现级修复。",
        instruction: "你是实现执行 Agent。严格按已确定的范围修改代码，运行必要验证，并报告结果与未完成事项。",
        sandbox_policy: "WORKSPACE_WRITE",
        reasoning_policy: "HIGH",
        required_capabilities: COMMON_REQUIRED,
        preferred_capabilities: COMMON_PREFERRED,
        minimum_context_window: None,
    },
    AgentPresetDefinition {
        key: "explorer",
        name: "Explorer",
        description: "负责代码库探索、信息检索与上下文收集。",
        instruction: "你是代码库探索 Agent。只调查与汇总证据，不修改项目文件；明确区分事实、推断与未知项。",
        sandbox_policy: "READ_ONLY",
        reasoning_policy: "MEDIUM",
        required_capabilities: COMMON_REQUIRED,
        preferred_capabilities: COMMON_PREFERRED,
        minimum_context_window: None,
    },
    AgentPresetDefinition {
        key: "reviewer",
        name: "Reviewer",
        description: "负责独立检查实现的正确性、风险与遗漏。",
        instruction: "你是独立审查 Agent。优先报告可复现的正确性、安全性和回归问题，并提供精确文件位置。",
        sandbox_policy: "READ_ONLY",
        reasoning_policy: "HIGH",
        required_capabilities: COMMON_REQUIRED,
        preferred_capabilities: COMMON_PREFERRED,
        minimum_context_window: None,
    },
    AgentPresetDefinition {
        key: "tester",
        name: "Tester",
        description: "负责测试设计、执行与实现验证。",
        instruction: "你是测试 Agent。用最小测试复现问题或验证需求，避免修改与测试无关的实现。",
        sandbox_policy: "WORKSPACE_WRITE",
        reasoning_policy: "MEDIUM",
        required_capabilities: COMMON_REQUIRED,
        preferred_capabilities: COMMON_PREFERRED,
        minimum_context_window: None,
    },
];

fn find_preset(key: &str) -> Result<Option<&'static AgentPresetDefinition>, AgentServiceError> {
    AGENT_PRESETS
        .iter()
        .find(|preset| preset.key == key)
        .map(Some)
        .ok_or(AgentServiceError::PresetNotFound)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentPresetResponse {
    key: &'static str,
    name: &'static str,
    description: &'static str,
    default_sandbox_policy: &'static str,
    default_reasoning_policy: &'static str,
    required_capabilities: &'static [&'static str],
}

impl From<&'static AgentPresetDefinition> for AgentPresetResponse {
    fn from(preset: &'static AgentPresetDefinition) -> Self {
        Self {
            key: preset.key,
            name: preset.name,
            description: preset.description,
            default_sandbox_policy: preset.sandbox_policy,
            default_reasoning_policy: preset.reasoning_policy,
            required_capabilities: preset.required_capabilities,
        }
    }
}

struct AgentRecord {
    id: String,
    agent_key: String,
    name: String,
    description: String,
    instruction: String,
    agent_type: String,
    enabled: bool,
    sandbox_policy: String,
    reasoning_policy: String,
    source: String,
    managed: bool,
    minimum_context_window: Option<i64>,
    created_at: String,
    updated_at: String,
    required_capabilities: Vec<String>,
    preferred_capabilities: Vec<String>,
    model: Option<BindingModelRecord>,
    compatibility: AgentBindingCompatibility,
    availability: AgentAvailability,
}

struct BindingModelRecord {
    reference: AgentModelReference,
    model_enabled: bool,
    provider_enabled: bool,
    lifecycle: String,
    compatibility_level: String,
    context_window: Option<i64>,
    capabilities: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentModelReference {
    id: String,
    provider_id: String,
    provider_name: String,
    model_id: String,
    display_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentSummary {
    id: String,
    agent_key: String,
    name: String,
    description: String,
    enabled: bool,
    model: Option<AgentModelReference>,
    availability: AgentAvailability,
    reasoning_policy: String,
}

impl From<AgentRecord> for AgentSummary {
    fn from(agent: AgentRecord) -> Self {
        Self {
            id: agent.id,
            agent_key: agent.agent_key,
            name: agent.name,
            description: agent.description,
            enabled: agent.enabled,
            model: agent.model.map(|model| model.reference),
            availability: agent.availability,
            reasoning_policy: agent.reasoning_policy,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentDetailResponse {
    id: String,
    agent_key: String,
    name: String,
    description: String,
    instruction: String,
    agent_type: String,
    enabled: bool,
    sandbox_policy: String,
    reasoning_policy: String,
    required_capabilities: Vec<String>,
    preferred_capabilities: Vec<String>,
    model_binding: Option<AgentModelReference>,
    compatibility: AgentBindingCompatibility,
    source: String,
    managed: bool,
    created_at: String,
    updated_at: String,
}

impl From<AgentRecord> for AgentDetailResponse {
    fn from(agent: AgentRecord) -> Self {
        Self {
            id: agent.id,
            agent_key: agent.agent_key,
            name: agent.name,
            description: agent.description,
            instruction: agent.instruction,
            agent_type: agent.agent_type,
            enabled: agent.enabled,
            sandbox_policy: agent.sandbox_policy,
            reasoning_policy: agent.reasoning_policy,
            required_capabilities: agent.required_capabilities,
            preferred_capabilities: agent.preferred_capabilities,
            model_binding: agent.model.map(|model| model.reference),
            compatibility: agent.compatibility,
            source: agent.source,
            managed: agent.managed,
            created_at: agent.created_at,
            updated_at: agent.updated_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentBindingResponse {
    agent_id: String,
    model: AgentModelReference,
    compatibility: AgentBindingCompatibility,
}

impl From<AgentRecord> for AgentBindingResponse {
    fn from(agent: AgentRecord) -> Self {
        Self {
            agent_id: agent.id,
            model: agent
                .model
                .expect("saved binding must have a model")
                .reference,
            compatibility: agent.compatibility,
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum AgentAvailability {
    Ready,
    ModelMissing,
    ProviderUnavailable,
    IncompatibleModel,
    InvalidConfiguration,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentBindingCompatibility {
    status: BindingCompatibilityStatus,
    issues: Vec<CompatibilityIssueResponse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum BindingCompatibilityStatus {
    Compatible,
    Warning,
    Incompatible,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompatibilityIssueResponse {
    code: String,
    severity: CompatibilityIssueSeverity,
    message: String,
    source: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CompatibilityIssueSeverity {
    Warning,
    Error,
}

#[derive(Debug)]
pub(crate) enum AgentServiceError {
    InvalidField(&'static str),
    PresetNotFound,
    Repository(AgentRepositoryError),
    DatabaseUnavailable,
}

impl fmt::Display for AgentServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid agent field: {field}"),
            Self::PresetNotFound => formatter.write_str("agent preset not found"),
            Self::Repository(error) => write!(formatter, "agent repository failed: {error}"),
            Self::DatabaseUnavailable => formatter.write_str("database unavailable"),
        }
    }
}

impl std::error::Error for AgentServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            _ => None,
        }
    }
}

impl From<AgentRepositoryError> for AgentServiceError {
    fn from(error: AgentRepositoryError) -> Self {
        Self::Repository(error)
    }
}

#[derive(Debug)]
pub(crate) enum AgentRepositoryError {
    NotFound,
    ModelNotFound,
    IncompatibleModel,
    Active,
    Conflict,
    Persistence(PersistenceError),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for AgentRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("agent not found"),
            Self::ModelNotFound => formatter.write_str("model not found"),
            Self::IncompatibleModel => formatter.write_str("model incompatible"),
            Self::Active => formatter.write_str("agent is active"),
            Self::Conflict => formatter.write_str("agent key conflict"),
            Self::Persistence(error) => write!(formatter, "persistence failed: {error}"),
            Self::Sqlite(_) => formatter.write_str("sqlite operation failed"),
        }
    }
}

impl std::error::Error for AgentRepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PersistenceError> for AgentRepositoryError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

impl From<rusqlite::Error> for AgentRepositoryError {
    fn from(error: rusqlite::Error) -> Self {
        match &error {
            rusqlite::Error::SqliteFailure(inner, _)
                if inner.code == ErrorCode::ConstraintViolation =>
            {
                Self::Conflict
            }
            _ => Self::Sqlite(error),
        }
    }
}

impl From<AgentServiceError> for ApiError {
    fn from(error: AgentServiceError) -> Self {
        let mut details = None;
        let (code, message, retryable) = match error {
            AgentServiceError::InvalidField(field) => {
                details = Some(BTreeMap::from([("field", field.to_owned())]));
                ("VALIDATION_ERROR", "Agent 字段无效。", false)
            }
            AgentServiceError::PresetNotFound => {
                ("AGENT_PRESET_NOT_FOUND", "Agent 模板不存在。", false)
            }
            AgentServiceError::Repository(AgentRepositoryError::NotFound) => {
                ("AGENT_NOT_FOUND", "Agent 不存在。", false)
            }
            AgentServiceError::Repository(AgentRepositoryError::ModelNotFound) => {
                ("MODEL_NOT_FOUND", "Model 不存在。", false)
            }
            AgentServiceError::Repository(AgentRepositoryError::IncompatibleModel) => {
                ("MODEL_INCOMPATIBLE", "Model 与 Agent 明确不兼容。", false)
            }
            AgentServiceError::Repository(AgentRepositoryError::Active) => (
                "AGENT_ACTIVE",
                "当前正在使用该 Agent，请先在概览切换运行模式。",
                false,
            ),
            AgentServiceError::Repository(AgentRepositoryError::Conflict) => {
                details = Some(BTreeMap::from([("field", "agentKey".to_owned())]));
                ("AGENT_NAME_CONFLICT", "Agent Key 已存在。", false)
            }
            AgentServiceError::Repository(AgentRepositoryError::Persistence(
                PersistenceError::SchemaTooNew,
            )) => (
                "DATABASE_SCHEMA_TOO_NEW",
                "数据库版本高于当前应用支持版本。",
                false,
            ),
            AgentServiceError::DatabaseUnavailable
            | AgentServiceError::Repository(AgentRepositoryError::Persistence(
                PersistenceError::Unavailable,
            )) => ("DATABASE_UNAVAILABLE", "CAS 数据库当前不可用。", true),
            AgentServiceError::Repository(_) => {
                ("DATABASE_OPERATION_FAILED", "Agent 数据保存失败。", true)
            }
        };
        ApiError::new(code, message, retryable, details)
    }
}

impl From<AgentRepositoryError> for ApiError {
    fn from(error: AgentRepositoryError) -> Self {
        AgentServiceError::Repository(error).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(template_key: Option<&str>, model_id: Option<String>) -> AgentCreateRequest {
        AgentCreateRequest {
            agent_key: String::new(),
            name: String::new(),
            description: String::new(),
            instruction: String::new(),
            template_key: template_key.map(str::to_owned),
            enabled: true,
            sandbox_policy: SandboxPolicy::WorkspaceWrite,
            reasoning_policy: ReasoningPolicy::High,
            model_id,
        }
    }

    fn insert_model(
        service: &AgentService,
        compatibility: &str,
        with_capabilities: bool,
    ) -> String {
        let model_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let repository = service.repository().unwrap();
        repository
            .connection
            .execute(
            "INSERT INTO providers (
                id, provider_key, name, provider_type, base_url, protocol, auth_type, enabled, source,
                created_at, updated_at
             ) VALUES (?1, ?2, 'Provider', 'CUSTOM', 'https://example.com', 'RESPONSES',
                       'BEARER_TOKEN', 1,
                       'USER', '2026-01-01', '2026-01-01')",
                params![provider_id, format!("provider-{}", &provider_id[..8])],
            )
            .unwrap();
        repository
            .connection
            .execute(
                "INSERT INTO models (
                id, provider_id, model_id, display_name, enabled, source, lifecycle,
                compatibility_level, compatibility_source, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'Model', 1, 'USER', 'ACTIVE', ?4, 'TEST',
                       '2026-01-01', '2026-01-01')",
                params![
                    model_id,
                    provider_id,
                    format!("model-{}", &model_id[..8]),
                    compatibility
                ],
            )
            .unwrap();
        if with_capabilities {
            for capability in COMMON_REQUIRED.iter().chain(COMMON_PREFERRED) {
                repository
                    .connection
                    .execute(
                        "INSERT INTO model_capabilities (
                        model_id, capability, status, source, confidence
                     ) VALUES (?1, ?2, 'SUPPORTED', 'TEST', 'AUTHORITATIVE')",
                        params![model_id, capability],
                    )
                    .unwrap();
            }
        }
        model_id
    }

    #[test]
    fn exposes_four_agent_presets() {
        let service = AgentService::in_memory();
        assert_eq!(service.presets().len(), 4);
    }

    #[test]
    fn preset_create_and_initial_binding_are_atomic() {
        let service = AgentService::in_memory();
        let model_id = insert_model(&service, "NATIVE", true);
        let agent = service
            .create(request(Some("executor"), Some(model_id)))
            .unwrap();
        assert_eq!(agent.agent_key, "executor");
        assert_eq!(
            agent.required_capabilities,
            vec!["CODEX_MULTI_AGENT".to_owned(), "TOOL_CALLING".to_owned()]
        );
        assert!(agent.model_binding.is_some());
        assert_eq!(
            agent.compatibility.status,
            BindingCompatibilityStatus::Compatible
        );
    }

    #[test]
    fn unknown_model_is_allowed_but_explicitly_marked() {
        let service = AgentService::in_memory();
        let model_id = insert_model(&service, "UNKNOWN", false);
        let agent = service
            .create(request(Some("executor"), Some(model_id)))
            .unwrap();
        assert_eq!(
            agent.compatibility.status,
            BindingCompatibilityStatus::Unknown
        );
    }

    #[test]
    fn incompatible_initial_binding_rolls_back_agent() {
        let service = AgentService::in_memory();
        let model_id = insert_model(&service, "UNSUPPORTED", false);
        let error = service
            .create(request(Some("executor"), Some(model_id)))
            .unwrap_err();
        assert_eq!(error.code(), "MODEL_INCOMPATIBLE");
        assert!(
            service
                .list(AgentListRequest::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn active_agent_cannot_be_deleted() {
        let service = AgentService::in_memory();
        let agent = service.create(request(Some("executor"), None)).unwrap();
        let mut repository = service.repository().unwrap();
        repository
            .connection
            .execute(
                "UPDATE configuration_state SET active_agent_id = ?1 WHERE id = 1",
                [&agent.id],
            )
            .unwrap();

        assert!(matches!(
            repository.delete(&agent.id),
            Err(AgentRepositoryError::Active)
        ));
    }
}
