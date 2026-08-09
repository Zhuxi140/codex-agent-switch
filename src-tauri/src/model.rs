use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Mutex;

use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::persistence::{PersistenceError, open_database};
use crate::provider::ApiError;

const DEEPSEEK_V4_FLASH: &str =
    include_str!("../resources/model-definitions/deepseek-v4-flash.json");

pub(crate) struct ModelService {
    repository: Mutex<SqliteModelRepository>,
}

impl ModelService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, ModelServiceError> {
        Ok(Self {
            repository: Mutex::new(SqliteModelRepository::open(database_path)?),
        })
    }

    pub(crate) fn list(&self, request: ModelListRequest) -> Result<Vec<ModelSummary>, ApiError> {
        let provider_id = request
            .provider_id
            .as_deref()
            .map(|value| parse_uuid(value, "providerId"))
            .transpose()?;
        let search = request.search.as_deref().map(str::trim);
        let search = search.filter(|value| !value.is_empty());
        self.repository()?
            .list(
                search,
                provider_id.as_deref(),
                request.enabled,
                request
                    .compatibility
                    .as_ref()
                    .map(CompatibilityLevel::as_str),
            )
            .map(|models| models.into_iter().map(ModelSummary::from).collect())
            .map_err(ApiError::from)
    }

    pub(crate) fn get(&self, request: ModelGetRequest) -> Result<ModelDetailResponse, ApiError> {
        let id = parse_uuid(&request.model_id, "modelId")?;
        self.repository()?
            .find_by_id(&id)
            .map(ModelDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn add(&self, request: ModelAddRequest) -> Result<ModelDetailResponse, ApiError> {
        let pending = NewUserModel::try_from(request)?;
        self.repository()?
            .add(&pending)
            .map(ModelDetailResponse::from)
            .map_err(ApiError::from)
    }

    fn repository(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, SqliteModelRepository>, ModelServiceError> {
        self.repository
            .lock()
            .map_err(|_| ModelServiceError::DatabaseUnavailable)
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            repository: Mutex::new(SqliteModelRepository::in_memory().unwrap()),
        }
    }
}

struct SqliteModelRepository {
    connection: Connection,
}

impl SqliteModelRepository {
    fn open(path: &Path) -> Result<Self, ModelRepositoryError> {
        Ok(Self {
            connection: open_database(path)?,
        })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, ModelRepositoryError> {
        Ok(Self {
            connection: crate::persistence::open_in_memory()?,
        })
    }

    fn add(&mut self, model: &NewUserModel) -> Result<ModelRecord, ModelRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let provider_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM providers WHERE id = ?1)",
            [&model.provider_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !provider_exists {
            return Err(ModelRepositoryError::ProviderNotFound);
        }
        let timestamp =
            transaction.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get::<_, String>(0)
            })?;
        transaction.execute(
            "INSERT INTO models (
                id, provider_id, model_id, display_name, enabled, source, lifecycle,
                compatibility_level, compatibility_source, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, 'USER', 'UNKNOWN', 'UNKNOWN', 'UNKNOWN', ?5, ?5)",
            params![
                model.id,
                model.provider_id,
                model.model_id,
                model.display_name,
                timestamp,
            ],
        )?;
        transaction.commit()?;
        self.find_by_id(&model.id)
    }

    fn find_by_id(&self, id: &str) -> Result<ModelRecord, ModelRepositoryError> {
        let mut model = self
            .connection
            .query_row(
                &format!("{} WHERE m.id = ?1", model_select()),
                [id],
                map_model,
            )
            .optional()?
            .ok_or(ModelRepositoryError::NotFound)?;
        model.reasoning_efforts = self.reasoning_efforts(id)?;
        model.capabilities = self.capabilities(id)?;
        Ok(model)
    }

    fn list(
        &self,
        search: Option<&str>,
        provider_id: Option<&str>,
        enabled: Option<bool>,
        compatibility: Option<&str>,
    ) -> Result<Vec<ModelRecord>, ModelRepositoryError> {
        let search = search.map(|value| format!("%{value}%"));
        let enabled = enabled.map(i64::from);
        let mut statement = self.connection.prepare(&format!(
            "{} WHERE (?1 IS NULL OR m.model_id LIKE ?1 OR m.display_name LIKE ?1 OR p.name LIKE ?1)
             AND (?2 IS NULL OR m.provider_id = ?2)
             AND (?3 IS NULL OR m.enabled = ?3)
             AND (?4 IS NULL OR m.compatibility_level = ?4)
             ORDER BY p.name COLLATE NOCASE, m.display_name COLLATE NOCASE, m.id",
            model_select()
        ))?;
        let rows = statement.query_map(
            params![search, provider_id, enabled, compatibility],
            map_model,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ModelRepositoryError::from)
    }

    fn reasoning_efforts(&self, id: &str) -> Result<Vec<String>, ModelRepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT effort FROM model_reasoning_efforts WHERE model_id = ?1 ORDER BY ordinal, effort",
        )?;
        let rows = statement.query_map([id], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ModelRepositoryError::from)
    }

    fn capabilities(&self, id: &str) -> Result<Vec<CapabilityRecord>, ModelRepositoryError> {
        let mut statement = self.connection.prepare(
            "SELECT capability, status, source, confidence, verified_at
             FROM model_capabilities WHERE model_id = ?1 ORDER BY capability",
        )?;
        let rows = statement.query_map([id], |row| {
            Ok(CapabilityRecord {
                capability: row.get(0)?,
                status: row.get(1)?,
                source: row.get(2)?,
                confidence: row.get(3)?,
                verified_at: row.get(4)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(ModelRepositoryError::from)
    }
}

fn model_select() -> &'static str {
    "SELECT m.id, m.provider_id, p.name, m.model_id, m.display_name, m.enabled,
            m.lifecycle, m.compatibility_level, m.context_window, m.max_output_tokens,
            m.reasoning_supported, m.default_reasoning, m.compatibility_source,
            m.minimum_codex_version, m.compatibility_verified_at, m.created_at, m.updated_at
     FROM models m
     JOIN providers p ON p.id = m.provider_id"
}

fn map_model(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRecord> {
    Ok(ModelRecord {
        id: row.get(0)?,
        provider_id: row.get(1)?,
        provider_name: row.get(2)?,
        model_id: row.get(3)?,
        display_name: row.get(4)?,
        enabled: row.get::<_, i64>(5)? != 0,
        lifecycle: row.get(6)?,
        compatibility_level: row.get(7)?,
        context_window: row.get(8)?,
        max_output_tokens: row.get(9)?,
        reasoning_supported: row.get::<_, Option<i64>>(10)?.map(|value| value != 0),
        default_reasoning: row.get(11)?,
        compatibility_source: row.get(12)?,
        minimum_codex_version: row.get(13)?,
        compatibility_verified_at: row.get(14)?,
        created_at: row.get(15)?,
        updated_at: row.get(16)?,
        reasoning_efforts: Vec::new(),
        capabilities: Vec::new(),
    })
}

pub(crate) fn initial_models_for_preset(
    preset_id: &str,
) -> Result<Vec<NewPresetModel>, CatalogError> {
    if preset_id != "deepseek" {
        return Err(CatalogError::UnknownPreset);
    }
    let definition: BuiltInModelDefinition =
        serde_json::from_str(DEEPSEEK_V4_FLASH).map_err(|_| CatalogError::InvalidResource)?;
    if definition.schema_version != 1 || definition.provider_preset_id != preset_id {
        return Err(CatalogError::InvalidResource);
    }
    Ok(vec![definition.into()])
}

pub(crate) fn insert_preset_model(
    transaction: &Transaction<'_>,
    provider_id: &str,
    model: &NewPresetModel,
    timestamp: &str,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO models (
            id, provider_id, model_id, display_name, enabled, source, lifecycle,
            compatibility_level, compatibility_source, minimum_codex_version,
            compatibility_verified_at, context_window, max_output_tokens,
            reasoning_supported, default_reasoning, metadata_source, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, 1, 'PRESET', ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                   'CAS_BUILT_IN', ?14, ?14)",
        params![
            model.id,
            provider_id,
            model.model_id,
            model.display_name,
            model.lifecycle,
            model.compatibility_level,
            model.compatibility_source,
            model.minimum_codex_version,
            model.compatibility_verified_at,
            model.context_window,
            model.max_output_tokens,
            model.reasoning_supported,
            model.default_reasoning,
            timestamp,
        ],
    )?;
    for (ordinal, effort) in model.reasoning_efforts.iter().enumerate() {
        transaction.execute(
            "INSERT INTO model_reasoning_efforts (model_id, effort, ordinal) VALUES (?1, ?2, ?3)",
            params![model.id, effort, ordinal as i64],
        )?;
    }
    for capability in &model.capabilities {
        transaction.execute(
            "INSERT INTO model_capabilities (
                model_id, capability, status, source, confidence, verified_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                model.id,
                capability.name,
                capability.status,
                capability.source,
                capability.confidence,
                model.compatibility_verified_at,
            ],
        )?;
    }
    Ok(())
}

pub(crate) struct NewPresetModel {
    id: String,
    model_id: String,
    display_name: String,
    lifecycle: String,
    context_window: Option<i64>,
    max_output_tokens: Option<i64>,
    reasoning_supported: Option<bool>,
    reasoning_efforts: Vec<String>,
    default_reasoning: Option<String>,
    compatibility_level: String,
    compatibility_source: String,
    minimum_codex_version: Option<String>,
    compatibility_verified_at: Option<String>,
    capabilities: Vec<NewCapability>,
}

struct NewCapability {
    name: String,
    status: String,
    source: String,
    confidence: String,
}

struct NewUserModel {
    id: String,
    provider_id: String,
    model_id: String,
    display_name: String,
}

impl TryFrom<ModelAddRequest> for NewUserModel {
    type Error = ModelServiceError;

    fn try_from(request: ModelAddRequest) -> Result<Self, Self::Error> {
        let provider_id = parse_uuid(&request.provider_id, "providerId")?;
        validate_model_id(&request.model_id)?;
        let display_name = match request.display_name {
            Some(value) => {
                validate_text(&value, "displayName", 160)?;
                value.trim().to_owned()
            }
            None => request.model_id.clone(),
        };
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            provider_id,
            model_id: request.model_id,
            display_name,
        })
    }
}

fn parse_uuid(value: &str, field: &'static str) -> Result<String, ModelServiceError> {
    Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| ModelServiceError::InvalidField(field))
}

fn validate_model_id(value: &str) -> Result<(), ModelServiceError> {
    (!value.is_empty()
        && value.len() <= 200
        && value.trim() == value
        && !value.bytes().any(|byte| matches!(byte, 0 | b'\r' | b'\n')))
    .then_some(())
    .ok_or(ModelServiceError::InvalidField("modelId"))
}

fn validate_text(
    value: &str,
    field: &'static str,
    max_length: usize,
) -> Result<(), ModelServiceError> {
    (!value.trim().is_empty() && value.len() <= max_length)
        .then_some(())
        .ok_or(ModelServiceError::InvalidField(field))
}

struct ModelRecord {
    id: String,
    provider_id: String,
    provider_name: String,
    model_id: String,
    display_name: String,
    enabled: bool,
    lifecycle: String,
    compatibility_level: String,
    context_window: Option<i64>,
    max_output_tokens: Option<i64>,
    reasoning_supported: Option<bool>,
    reasoning_efforts: Vec<String>,
    default_reasoning: Option<String>,
    compatibility_source: String,
    minimum_codex_version: Option<String>,
    compatibility_verified_at: Option<String>,
    capabilities: Vec<CapabilityRecord>,
    created_at: String,
    updated_at: String,
}

struct CapabilityRecord {
    capability: String,
    status: String,
    source: String,
    confidence: String,
    verified_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelAddRequest {
    provider_id: String,
    model_id: String,
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelGetRequest {
    model_id: String,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelListRequest {
    search: Option<String>,
    provider_id: Option<String>,
    enabled: Option<bool>,
    compatibility: Option<CompatibilityLevel>,
}

#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CompatibilityLevel {
    Native,
    Compatible,
    GatewayRequired,
    Unsupported,
    Unknown,
}

impl CompatibilityLevel {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Native => "NATIVE",
            Self::Compatible => "COMPATIBLE",
            Self::GatewayRequired => "GATEWAY_REQUIRED",
            Self::Unsupported => "UNSUPPORTED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelSummary {
    id: String,
    provider_id: String,
    provider_name: String,
    model_id: String,
    display_name: String,
    enabled: bool,
    lifecycle: String,
    compatibility: String,
    context_window: Option<i64>,
}

impl From<ModelRecord> for ModelSummary {
    fn from(model: ModelRecord) -> Self {
        Self {
            id: model.id,
            provider_id: model.provider_id,
            provider_name: model.provider_name,
            model_id: model.model_id,
            display_name: model.display_name,
            enabled: model.enabled,
            lifecycle: model.lifecycle,
            compatibility: model.compatibility_level,
            context_window: model.context_window,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelDetailResponse {
    id: String,
    provider: ModelProviderResponse,
    model_id: String,
    display_name: String,
    enabled: bool,
    lifecycle: String,
    context_window: Option<i64>,
    max_output_tokens: Option<i64>,
    reasoning: ReasoningResponse,
    capabilities: Vec<CapabilityResponse>,
    compatibility: ModelCompatibilityResponse,
    created_at: String,
    updated_at: String,
}

impl From<ModelRecord> for ModelDetailResponse {
    fn from(model: ModelRecord) -> Self {
        let reasoning_status = match model.reasoning_supported {
            Some(true) => "SUPPORTED",
            Some(false) => "UNSUPPORTED",
            None => "UNKNOWN",
        };
        Self {
            id: model.id,
            provider: ModelProviderResponse {
                id: model.provider_id,
                name: model.provider_name,
            },
            model_id: model.model_id,
            display_name: model.display_name,
            enabled: model.enabled,
            lifecycle: model.lifecycle,
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning: ReasoningResponse {
                status: reasoning_status,
                supported_efforts: model.reasoning_efforts,
                default_effort: model.default_reasoning,
            },
            capabilities: model
                .capabilities
                .into_iter()
                .map(CapabilityResponse::from)
                .collect(),
            compatibility: ModelCompatibilityResponse {
                level: model.compatibility_level,
                source: model.compatibility_source,
                minimum_codex_version: model.minimum_codex_version,
                verified_at: model.compatibility_verified_at,
            },
            created_at: model.created_at,
            updated_at: model.updated_at,
        }
    }
}

#[derive(Serialize)]
struct ModelProviderResponse {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReasoningResponse {
    status: &'static str,
    supported_efforts: Vec<String>,
    default_effort: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CapabilityResponse {
    capability: String,
    status: String,
    source: String,
    confidence: String,
    verified_at: Option<String>,
}

impl From<CapabilityRecord> for CapabilityResponse {
    fn from(capability: CapabilityRecord) -> Self {
        Self {
            capability: capability.capability,
            status: capability.status,
            source: capability.source,
            confidence: capability.confidence,
            verified_at: capability.verified_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelCompatibilityResponse {
    level: String,
    source: String,
    minimum_codex_version: Option<String>,
    verified_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltInModelDefinition {
    schema_version: u32,
    provider_preset_id: String,
    model_id: String,
    display_name: String,
    lifecycle: String,
    context_window: Option<i64>,
    max_output_tokens: Option<i64>,
    reasoning: BuiltInReasoning,
    compatibility: BuiltInCompatibility,
    capabilities: Vec<BuiltInCapability>,
}

impl From<BuiltInModelDefinition> for NewPresetModel {
    fn from(model: BuiltInModelDefinition) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            model_id: model.model_id,
            display_name: model.display_name,
            lifecycle: model.lifecycle,
            context_window: model.context_window,
            max_output_tokens: model.max_output_tokens,
            reasoning_supported: match model.reasoning.status.as_str() {
                "SUPPORTED" => Some(true),
                "UNSUPPORTED" => Some(false),
                _ => None,
            },
            reasoning_efforts: model.reasoning.supported_efforts,
            default_reasoning: model.reasoning.default_effort,
            compatibility_level: model.compatibility.level,
            compatibility_source: model.compatibility.source,
            minimum_codex_version: model.compatibility.minimum_codex_version,
            compatibility_verified_at: model.compatibility.verified_at,
            capabilities: model
                .capabilities
                .into_iter()
                .map(|capability| NewCapability {
                    name: capability.name,
                    status: capability.status,
                    source: capability.source,
                    confidence: capability.confidence,
                })
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltInReasoning {
    status: String,
    supported_efforts: Vec<String>,
    default_effort: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuiltInCompatibility {
    level: String,
    source: String,
    minimum_codex_version: Option<String>,
    verified_at: Option<String>,
}

#[derive(Deserialize)]
struct BuiltInCapability {
    name: String,
    status: String,
    source: String,
    confidence: String,
}

#[derive(Debug)]
pub(crate) enum CatalogError {
    UnknownPreset,
    InvalidResource,
}

#[derive(Debug)]
pub(crate) enum ModelServiceError {
    InvalidField(&'static str),
    Repository(ModelRepositoryError),
    DatabaseUnavailable,
}

impl fmt::Display for ModelServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid model field: {field}"),
            Self::Repository(error) => write!(formatter, "model repository failed: {error}"),
            Self::DatabaseUnavailable => formatter.write_str("database unavailable"),
        }
    }
}

impl std::error::Error for ModelServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Repository(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ModelRepositoryError> for ModelServiceError {
    fn from(error: ModelRepositoryError) -> Self {
        Self::Repository(error)
    }
}

#[derive(Debug)]
pub(crate) enum ModelRepositoryError {
    NotFound,
    ProviderNotFound,
    Conflict,
    Persistence(PersistenceError),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for ModelRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("model not found"),
            Self::ProviderNotFound => formatter.write_str("provider not found"),
            Self::Conflict => formatter.write_str("model constraint conflict"),
            Self::Persistence(error) => write!(formatter, "persistence failed: {error}"),
            Self::Sqlite(_) => formatter.write_str("sqlite operation failed"),
        }
    }
}

impl std::error::Error for ModelRepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PersistenceError> for ModelRepositoryError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

impl From<rusqlite::Error> for ModelRepositoryError {
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

impl From<ModelServiceError> for ApiError {
    fn from(error: ModelServiceError) -> Self {
        let mut details = None;
        let (code, message, retryable) = match error {
            ModelServiceError::InvalidField(field) => {
                details = Some(BTreeMap::from([("field", field.to_owned())]));
                ("VALIDATION_ERROR", "Model 字段无效。", false)
            }
            ModelServiceError::Repository(ModelRepositoryError::NotFound) => {
                ("MODEL_NOT_FOUND", "Model 不存在。", false)
            }
            ModelServiceError::Repository(ModelRepositoryError::ProviderNotFound) => {
                ("PROVIDER_NOT_FOUND", "Provider 不存在。", false)
            }
            ModelServiceError::Repository(ModelRepositoryError::Conflict) => (
                "MODEL_ID_CONFLICT",
                "该 Provider 中已存在相同 Model ID。",
                false,
            ),
            ModelServiceError::Repository(ModelRepositoryError::Persistence(
                PersistenceError::SchemaTooNew,
            )) => (
                "DATABASE_SCHEMA_TOO_NEW",
                "数据库版本高于当前应用支持版本。",
                false,
            ),
            ModelServiceError::DatabaseUnavailable
            | ModelServiceError::Repository(ModelRepositoryError::Persistence(
                PersistenceError::Unavailable,
            )) => ("DATABASE_UNAVAILABLE", "CAS 数据库当前不可用。", true),
            ModelServiceError::Repository(_) => {
                ("DATABASE_OPERATION_FAILED", "Model 数据保存失败。", true)
            }
        };
        ApiError::new(code, message, retryable, details)
    }
}

impl From<ModelRepositoryError> for ApiError {
    fn from(error: ModelRepositoryError) -> Self {
        ModelServiceError::Repository(error).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn insert_provider(service: &ModelService) -> String {
        let id = Uuid::new_v4().to_string();
        service
            .repository()
            .unwrap()
            .connection
            .execute(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, created_at, updated_at
                 ) VALUES (?1, 'test-provider', 'Test Provider', 'CUSTOM',
                           'https://provider.example.com', 'RESPONSES', 'BEARER_TOKEN',
                           1, 'USER', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                [&id],
            )
            .unwrap();
        id
    }

    #[test]
    fn built_in_model_resource_is_valid() {
        let models = initial_models_for_preset("deepseek").unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "deepseek-v4-flash");
        assert_eq!(models[0].compatibility_level, "NATIVE");
        assert!(matches!(
            initial_models_for_preset("unknown"),
            Err(CatalogError::UnknownPreset)
        ));
    }

    #[test]
    fn custom_model_defaults_to_unknown_and_is_unique_per_provider() {
        let service = ModelService::in_memory();
        let provider_id = insert_provider(&service);
        let request = || ModelAddRequest {
            provider_id: provider_id.clone(),
            model_id: "custom/model-v1".to_owned(),
            display_name: None,
        };

        let created = match service.add(request()) {
            Ok(created) => created,
            Err(_) => panic!("valid model must be created"),
        };
        assert_eq!(created.compatibility.level, "UNKNOWN");
        assert!(created.capabilities.is_empty());
        let error = match service.add(request()) {
            Ok(_) => panic!("duplicate model must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "MODEL_ID_CONFLICT");
    }

    #[test]
    fn model_add_requires_existing_provider() {
        let service = ModelService::in_memory();
        let result = service.add(ModelAddRequest {
            provider_id: Uuid::new_v4().to_string(),
            model_id: "model-v1".to_owned(),
            display_name: None,
        });
        let error = match result {
            Ok(_) => panic!("missing provider must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "PROVIDER_NOT_FOUND");
    }
}
