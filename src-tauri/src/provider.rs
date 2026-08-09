use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;

use cas_secret_store::{
    CredentialId, SecretStoreError, SecretValue, delete as delete_secret, store as store_secret,
};
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::model::{CatalogError, NewPresetModel, initial_models_for_preset, insert_preset_model};
use crate::persistence::{PersistenceError, open_database};

pub(crate) struct ProviderService {
    repository: Mutex<SqliteProviderRepository>,
}

impl ProviderService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, ProviderServiceError> {
        Ok(Self {
            repository: Mutex::new(SqliteProviderRepository::open(database_path)?),
        })
    }

    pub(crate) fn create(
        &self,
        request: ProviderCreateRequest,
    ) -> Result<ProviderDetailResponse, ApiError> {
        self.create_with_secret_store(request, store_secret, delete_secret)
            .map(ProviderDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn list(
        &self,
        request: ProviderListRequest,
    ) -> Result<Vec<ProviderSummary>, ApiError> {
        let search = request.search.as_deref().map(str::trim);
        let search = search.filter(|value| !value.is_empty());
        self.repository()?
            .list(search, request.enabled)
            .map(|providers| providers.into_iter().map(ProviderSummary::from).collect())
            .map_err(ApiError::from)
    }

    pub(crate) fn get(
        &self,
        request: ProviderGetRequest,
    ) -> Result<ProviderDetailResponse, ApiError> {
        let id = parse_uuid(&request.provider_id, "providerId")?;
        self.repository()?
            .find_by_id(&id)
            .map(ProviderDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn update(
        &self,
        request: ProviderUpdateRequest,
    ) -> Result<ProviderDetailResponse, ApiError> {
        let pending = PendingProviderUpdate::try_from(request)?;
        let mut repository = self.repository()?;
        let current = repository.find_by_id(&pending.id)?;
        if !pending.confirm_origin_change
            && Url::parse(&current.base_url).map(|url| url.origin())
                != Url::parse(&pending.base_url).map(|url| url.origin())
        {
            return Err(ProviderServiceError::OriginConfirmationRequired.into());
        }
        repository
            .update(&pending)
            .map(ProviderDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn delete(&self, request: ProviderDeleteRequest) -> Result<DeleteResult, ApiError> {
        self.delete_with_secret_store(request, delete_secret)
            .map_err(ApiError::from)
    }

    fn create_with_secret_store<Store, Delete>(
        &self,
        request: ProviderCreateRequest,
        store: Store,
        delete: Delete,
    ) -> Result<ProviderRecord, ProviderServiceError>
    where
        Store: FnOnce(CredentialId, &SecretValue) -> Result<(), SecretStoreError>,
        Delete: FnOnce(CredentialId) -> Result<bool, SecretStoreError>,
    {
        let pending = PendingProvider::try_from(request)?;
        let credential_id_text = Uuid::new_v4().to_string();
        let credential_id = CredentialId::from_str(&credential_id_text)
            .map_err(|_| ProviderServiceError::Unexpected)?;
        let secret = SecretValue::from_string(pending.secret)
            .map_err(|_| ProviderServiceError::InvalidField("auth.secret"))?;

        store(credential_id, &secret).map_err(ProviderServiceError::SecretStore)?;

        let aggregate = NewProviderAggregate {
            id: Uuid::new_v4().to_string(),
            provider_key: pending.provider_key,
            name: pending.name,
            provider_type: pending.provider_type,
            base_url: pending.base_url,
            enabled: pending.enabled,
            source: pending.source,
            preset_id: pending.preset_id,
            credential_id: credential_id_text,
            initial_models: pending.initial_models,
        };

        let result = self.repository().and_then(|mut repository| {
            repository
                .create_aggregate(&aggregate)
                .map_err(ProviderServiceError::from)
        });
        match result {
            Ok(provider) => Ok(provider),
            Err(error) => match delete(credential_id) {
                Ok(true) => Err(error),
                Ok(false) | Err(_) => Err(ProviderServiceError::OrphanSecret(credential_id)),
            },
        }
    }

    fn delete_with_secret_store<Delete>(
        &self,
        request: ProviderDeleteRequest,
        delete: Delete,
    ) -> Result<DeleteResult, ProviderServiceError>
    where
        Delete: FnOnce(CredentialId) -> Result<bool, SecretStoreError>,
    {
        let id = parse_uuid(&request.provider_id, "providerId")?;
        let credential_id = self.repository()?.prepare_delete(&id)?;
        if let Some(credential_id) = credential_id {
            let credential_id = CredentialId::from_str(&credential_id)
                .map_err(|_| ProviderServiceError::Unexpected)?;
            delete(credential_id).map_err(ProviderServiceError::SecretStore)?;
        }
        self.repository()?.delete(&id)?;
        Ok(DeleteResult { deleted: true })
    }

    fn repository(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, SqliteProviderRepository>, ProviderServiceError> {
        self.repository
            .lock()
            .map_err(|_| ProviderServiceError::DatabaseUnavailable)
    }

    #[cfg(test)]
    fn in_memory() -> Self {
        Self {
            repository: Mutex::new(SqliteProviderRepository::in_memory().unwrap()),
        }
    }
}

pub(crate) struct SqliteProviderRepository {
    connection: Connection,
}

impl SqliteProviderRepository {
    fn open(database_path: &Path) -> Result<Self, RepositoryError> {
        Ok(Self {
            connection: open_database(database_path)?,
        })
    }

    #[cfg(test)]
    fn in_memory() -> Result<Self, RepositoryError> {
        Ok(Self {
            connection: crate::persistence::open_in_memory()?,
        })
    }

    fn create_aggregate(
        &mut self,
        provider: &NewProviderAggregate,
    ) -> Result<ProviderRecord, RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(RepositoryError::from)?;
        let timestamp = transaction
            .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get::<_, String>(0)
            })
            .map_err(RepositoryError::from)?;

        transaction
            .execute(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, preset_id, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'RESPONSES', 'BEARER_TOKEN', ?6, ?7, ?8, ?9, ?9)",
                params![
                    provider.id,
                    provider.provider_key,
                    provider.name,
                    provider.provider_type,
                    provider.base_url,
                    provider.enabled,
                    provider.source,
                    provider.preset_id,
                    timestamp,
                ],
            )
            .map_err(RepositoryError::from)?;

        transaction
            .execute(
                "INSERT INTO credentials (
                    id, provider_id, credential_key, secret_type, storage_backend, storage_key,
                    created_at, updated_at
                 ) VALUES (?1, ?2, 'primary', 'BEARER_TOKEN', 'WINDOWS_CREDENTIAL_MANAGER', ?1, ?3, ?3)",
                params![provider.credential_id, provider.id, timestamp],
            )
            .map_err(RepositoryError::from)?;

        for model in &provider.initial_models {
            insert_preset_model(&transaction, &provider.id, model, &timestamp)
                .map_err(RepositoryError::from)?;
        }

        transaction.commit().map_err(RepositoryError::from)?;
        self.find_by_id(&provider.id)
    }

    fn find_by_id(&self, id: &str) -> Result<ProviderRecord, RepositoryError> {
        self.connection
            .query_row(
                "SELECT p.id, p.provider_key, p.name, p.provider_type, p.base_url, p.protocol,
                        p.enabled, p.source, p.preset_id, p.created_at, p.updated_at, c.id,
                        (SELECT COUNT(*) FROM models m WHERE m.provider_id = p.id)
                 FROM providers p
                 LEFT JOIN credentials c ON c.provider_id = p.id AND c.credential_key = 'primary'
                 WHERE p.id = ?1",
                [id],
                map_provider,
            )
            .optional()
            .map_err(RepositoryError::from)?
            .ok_or(RepositoryError::NotFound)
    }

    fn list(
        &self,
        search: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Vec<ProviderRecord>, RepositoryError> {
        let search = search.map(|value| format!("%{value}%"));
        let enabled = enabled.map(i64::from);
        let mut statement = self
            .connection
            .prepare(
                "SELECT p.id, p.provider_key, p.name, p.provider_type, p.base_url, p.protocol,
                        p.enabled, p.source, p.preset_id, p.created_at, p.updated_at, c.id,
                        (SELECT COUNT(*) FROM models m WHERE m.provider_id = p.id)
                 FROM providers p
                 LEFT JOIN credentials c ON c.provider_id = p.id AND c.credential_key = 'primary'
                 WHERE (?1 IS NULL OR p.provider_key LIKE ?1 OR p.name LIKE ?1)
                   AND (?2 IS NULL OR p.enabled = ?2)
                 ORDER BY p.name COLLATE NOCASE, p.id",
            )
            .map_err(RepositoryError::from)?;
        let rows = statement
            .query_map(params![search, enabled], map_provider)
            .map_err(RepositoryError::from)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(RepositoryError::from)
    }

    fn update(
        &mut self,
        provider: &PendingProviderUpdate,
    ) -> Result<ProviderRecord, RepositoryError> {
        let changed = self.connection.execute(
            "UPDATE providers
             SET name = ?2, base_url = ?3, enabled = ?4,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![
                provider.id,
                provider.name,
                provider.base_url,
                provider.enabled
            ],
        )?;
        if changed == 0 {
            return Err(RepositoryError::NotFound);
        }
        self.find_by_id(&provider.id)
    }

    fn prepare_delete(&self, id: &str) -> Result<Option<String>, RepositoryError> {
        let (credential_id, model_count) = self
            .connection
            .query_row(
                "SELECT c.id, (SELECT COUNT(*) FROM models m WHERE m.provider_id = p.id)
                 FROM providers p
                 LEFT JOIN credentials c ON c.provider_id = p.id AND c.credential_key = 'primary'
                 WHERE p.id = ?1",
                [id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()?
            .ok_or(RepositoryError::NotFound)?;
        if model_count > 0 {
            return Err(RepositoryError::InUse);
        }
        Ok(credential_id)
    }

    fn delete(&mut self, id: &str) -> Result<(), RepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let model_count = transaction
            .query_row(
                "SELECT (SELECT COUNT(*) FROM models WHERE provider_id = p.id)
                 FROM providers p WHERE p.id = ?1",
                [id],
                |row| row.get::<_, u32>(0),
            )
            .optional()?
            .ok_or(RepositoryError::NotFound)?;
        if model_count > 0 {
            return Err(RepositoryError::InUse);
        }
        transaction.execute("DELETE FROM credentials WHERE provider_id = ?1", [id])?;
        transaction.execute("DELETE FROM providers WHERE id = ?1", [id])?;
        transaction.commit()?;
        Ok(())
    }
}

fn map_provider(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProviderRecord> {
    Ok(ProviderRecord {
        id: row.get(0)?,
        provider_key: row.get(1)?,
        name: row.get(2)?,
        provider_type: row.get(3)?,
        base_url: row.get(4)?,
        protocol: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
        source: row.get(7)?,
        preset_id: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        credential_configured: row.get::<_, Option<String>>(11)?.is_some(),
        model_count: row.get(12)?,
    })
}

struct PendingProvider {
    provider_key: String,
    name: String,
    provider_type: &'static str,
    base_url: String,
    enabled: bool,
    source: &'static str,
    preset_id: Option<String>,
    secret: String,
    initial_models: Vec<NewPresetModel>,
}

impl TryFrom<ProviderCreateRequest> for PendingProvider {
    type Error = ProviderServiceError;

    fn try_from(request: ProviderCreateRequest) -> Result<Self, Self::Error> {
        validate_provider_key(&request.provider_key)?;
        validate_text(&request.name, "name", 120)?;
        validate_base_url(&request.base_url)?;
        if request.protocol != ProviderProtocol::Responses {
            return Err(ProviderServiceError::InvalidField("protocol"));
        }
        let initial_models = match request.preset_id.as_deref() {
            Some(preset_id) => {
                validate_text(preset_id, "presetId", 128)?;
                initial_models_for_preset(preset_id).map_err(|error| match error {
                    CatalogError::UnknownPreset => ProviderServiceError::InvalidField("presetId"),
                    CatalogError::InvalidResource => ProviderServiceError::Unexpected,
                })?
            }
            None => Vec::new(),
        };
        let secret = match request.auth {
            ProviderAuthInput::OsSecretHelper { secret } => secret,
            ProviderAuthInput::ExternalEnv { env_key } => {
                let _ = env_key;
                return Err(ProviderServiceError::UnsupportedAuthStrategy);
            }
            ProviderAuthInput::None => return Err(ProviderServiceError::UnsupportedAuthStrategy),
        };
        let is_preset = request.preset_id.is_some();

        Ok(Self {
            provider_key: request.provider_key,
            name: request.name.trim().to_owned(),
            provider_type: if is_preset { "PRESET" } else { "CUSTOM" },
            base_url: request.base_url,
            enabled: request.enabled,
            source: if is_preset { "BUILT_IN" } else { "USER" },
            preset_id: request.preset_id,
            secret,
            initial_models,
        })
    }
}

struct PendingProviderUpdate {
    id: String,
    name: String,
    base_url: String,
    enabled: bool,
    confirm_origin_change: bool,
}

impl TryFrom<ProviderUpdateRequest> for PendingProviderUpdate {
    type Error = ProviderServiceError;

    fn try_from(request: ProviderUpdateRequest) -> Result<Self, Self::Error> {
        validate_text(&request.name, "name", 120)?;
        validate_base_url(&request.base_url)?;
        Ok(Self {
            id: parse_uuid(&request.provider_id, "providerId")?,
            name: request.name.trim().to_owned(),
            base_url: request.base_url,
            enabled: request.enabled,
            confirm_origin_change: request.confirm_origin_change,
        })
    }
}

fn validate_provider_key(value: &str) -> Result<(), ProviderServiceError> {
    let mut bytes = value.bytes();
    let first = bytes.next();
    let valid = !value.is_empty()
        && value.len() <= 64
        && first.is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    valid
        .then_some(())
        .ok_or(ProviderServiceError::InvalidField("providerKey"))
}

fn validate_text(
    value: &str,
    field: &'static str,
    max_length: usize,
) -> Result<(), ProviderServiceError> {
    (!value.trim().is_empty() && value.len() <= max_length)
        .then_some(())
        .ok_or(ProviderServiceError::InvalidField(field))
}

fn validate_base_url(value: &str) -> Result<(), ProviderServiceError> {
    if value.len() > 2_048 {
        return Err(ProviderServiceError::InvalidField("baseUrl"));
    }
    let url = Url::parse(value).map_err(|_| ProviderServiceError::InvalidField("baseUrl"))?;
    if !url.username().is_empty() || url.password().is_some() || url.host_str().is_none() {
        return Err(ProviderServiceError::InvalidField("baseUrl"));
    }
    let secure = url.scheme() == "https";
    let local_http = url.scheme() == "http"
        && url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
        });
    (secure || local_http)
        .then_some(())
        .ok_or(ProviderServiceError::InvalidField("baseUrl"))
}

fn parse_uuid(value: &str, field: &'static str) -> Result<String, ProviderServiceError> {
    Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| ProviderServiceError::InvalidField(field))
}

struct NewProviderAggregate {
    id: String,
    provider_key: String,
    name: String,
    provider_type: &'static str,
    base_url: String,
    enabled: bool,
    source: &'static str,
    preset_id: Option<String>,
    credential_id: String,
    initial_models: Vec<NewPresetModel>,
}

struct ProviderRecord {
    id: String,
    provider_key: String,
    name: String,
    provider_type: String,
    base_url: String,
    protocol: String,
    enabled: bool,
    source: String,
    preset_id: Option<String>,
    created_at: String,
    updated_at: String,
    credential_configured: bool,
    model_count: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderCreateRequest {
    provider_key: String,
    name: String,
    preset_id: Option<String>,
    base_url: String,
    protocol: ProviderProtocol,
    auth: ProviderAuthInput,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderUpdateRequest {
    provider_id: String,
    name: String,
    base_url: String,
    enabled: bool,
    #[serde(default)]
    confirm_origin_change: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDeleteRequest {
    provider_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeleteResult {
    deleted: bool,
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProviderProtocol {
    Responses,
    ChatCompletions,
    AnthropicMessages,
    Gemini,
    Custom,
}

#[derive(Deserialize)]
#[serde(tag = "strategy", rename_all = "SCREAMING_SNAKE_CASE")]
enum ProviderAuthInput {
    OsSecretHelper { secret: String },
    ExternalEnv { env_key: String },
    None,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderListRequest {
    search: Option<String>,
    enabled: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderGetRequest {
    provider_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderSummary {
    id: String,
    provider_key: String,
    name: String,
    provider_type: String,
    preset_id: Option<String>,
    protocol: String,
    enabled: bool,
    status: ProviderStatus,
    credential_status: CredentialStatus,
    model_count: u32,
}

impl From<ProviderRecord> for ProviderSummary {
    fn from(provider: ProviderRecord) -> Self {
        let status = if provider.enabled {
            ProviderStatus::Ready
        } else {
            ProviderStatus::Disabled
        };
        Self {
            id: provider.id,
            provider_key: provider.provider_key,
            name: provider.name,
            provider_type: provider.provider_type,
            preset_id: provider.preset_id,
            protocol: provider.protocol,
            enabled: provider.enabled,
            status,
            credential_status: CredentialStatus::from(provider.credential_configured),
            model_count: provider.model_count,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProviderDetailResponse {
    id: String,
    provider_key: String,
    name: String,
    provider_type: String,
    base_url: String,
    protocol: String,
    auth_strategy: AuthStrategy,
    enabled: bool,
    source: String,
    preset_id: Option<String>,
    credential_status: CredentialStatus,
    model_count: u32,
    last_check: Option<ProviderCheckSummary>,
    created_at: String,
    updated_at: String,
}

impl From<ProviderRecord> for ProviderDetailResponse {
    fn from(provider: ProviderRecord) -> Self {
        Self {
            id: provider.id,
            provider_key: provider.provider_key,
            name: provider.name,
            provider_type: provider.provider_type,
            base_url: provider.base_url,
            protocol: provider.protocol,
            auth_strategy: AuthStrategy::OsSecretHelper,
            enabled: provider.enabled,
            source: provider.source,
            preset_id: provider.preset_id,
            credential_status: CredentialStatus::from(provider.credential_configured),
            model_count: provider.model_count,
            last_check: None,
            created_at: provider.created_at,
            updated_at: provider.updated_at,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum AuthStrategy {
    OsSecretHelper,
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum CredentialStatus {
    Configured,
    Missing,
}

impl From<bool> for CredentialStatus {
    fn from(configured: bool) -> Self {
        if configured {
            Self::Configured
        } else {
            Self::Missing
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ProviderStatus {
    Ready,
    Disabled,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProviderCheckSummary;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApiError {
    code: &'static str,
    message: &'static str,
    details: Option<BTreeMap<&'static str, String>>,
    retryable: bool,
    correlation_id: Option<String>,
}

impl ApiError {
    pub(crate) fn new(
        code: &'static str,
        message: &'static str,
        retryable: bool,
        details: Option<BTreeMap<&'static str, String>>,
    ) -> Self {
        Self {
            code,
            message,
            details,
            retryable,
            correlation_id: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn code(&self) -> &str {
        self.code
    }
}

impl From<ProviderServiceError> for ApiError {
    fn from(error: ProviderServiceError) -> Self {
        let mut details = None;
        let (code, message, retryable) = match error {
            ProviderServiceError::InvalidField(field) => {
                details = Some(BTreeMap::from([("field", field.to_owned())]));
                ("VALIDATION_ERROR", "Provider 字段无效。", false)
            }
            ProviderServiceError::UnsupportedAuthStrategy => (
                "PROVIDER_AUTH_STRATEGY_UNSUPPORTED",
                "当前阶段仅支持 OS_SECRET_HELPER。",
                false,
            ),
            ProviderServiceError::OriginConfirmationRequired => (
                "PROVIDER_ORIGIN_CONFIRMATION_REQUIRED",
                "Provider Endpoint Origin 已变化，需要明确确认。",
                false,
            ),
            ProviderServiceError::Repository(RepositoryError::NotFound) => {
                ("PROVIDER_NOT_FOUND", "Provider 不存在。", false)
            }
            ProviderServiceError::Repository(RepositoryError::InUse) => (
                "PROVIDER_IN_USE",
                "Provider 仍被 Model 引用，请先删除相关 Model。",
                false,
            ),
            ProviderServiceError::Repository(RepositoryError::Conflict) => {
                details = Some(BTreeMap::from([("field", "providerKey".to_owned())]));
                (
                    "PROVIDER_KEY_CONFLICT",
                    "您已有一个使用相同 Provider Key 的 Provider 配置。",
                    false,
                )
            }
            ProviderServiceError::Repository(RepositoryError::Persistence(
                PersistenceError::SchemaTooNew,
            )) => (
                "DATABASE_SCHEMA_TOO_NEW",
                "数据库版本高于当前应用支持版本。",
                false,
            ),
            ProviderServiceError::SecretStore(SecretStoreError::AccessDenied) => {
                ("SECRET_STORE_ACCESS_DENIED", "系统凭据库拒绝访问。", false)
            }
            ProviderServiceError::SecretStore(SecretStoreError::Unavailable) => {
                ("SECRET_STORE_UNAVAILABLE", "系统凭据库当前不可用。", true)
            }
            ProviderServiceError::SecretStore(_) => (
                "SECRET_STORE_OPERATION_FAILED",
                "Credential 操作失败。",
                true,
            ),
            ProviderServiceError::OrphanSecret(id) => {
                details = Some(BTreeMap::from([("credentialId", id.to_string())]));
                (
                    "ORPHAN_SECRET_CLEANUP_REQUIRED",
                    "Provider 保存失败，且 Credential 自动清理失败。",
                    false,
                )
            }
            ProviderServiceError::DatabaseUnavailable
            | ProviderServiceError::Repository(RepositoryError::Persistence(
                PersistenceError::Unavailable,
            )) => ("DATABASE_UNAVAILABLE", "CAS 数据库当前不可用。", true),
            ProviderServiceError::Repository(_) | ProviderServiceError::Unexpected => {
                ("DATABASE_OPERATION_FAILED", "Provider 数据保存失败。", true)
            }
        };
        Self::new(code, message, retryable, details)
    }
}

impl From<RepositoryError> for ApiError {
    fn from(error: RepositoryError) -> Self {
        ProviderServiceError::Repository(error).into()
    }
}

#[derive(Debug)]
pub(crate) enum ProviderServiceError {
    InvalidField(&'static str),
    UnsupportedAuthStrategy,
    OriginConfirmationRequired,
    SecretStore(SecretStoreError),
    OrphanSecret(CredentialId),
    Repository(RepositoryError),
    DatabaseUnavailable,
    Unexpected,
}

impl fmt::Display for ProviderServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid provider field: {field}"),
            Self::UnsupportedAuthStrategy => formatter.write_str("unsupported auth strategy"),
            Self::OriginConfirmationRequired => {
                formatter.write_str("provider origin confirmation required")
            }
            Self::SecretStore(_) => formatter.write_str("secret store operation failed"),
            Self::OrphanSecret(_) => formatter.write_str("orphan secret cleanup required"),
            Self::Repository(error) => write!(formatter, "provider repository failed: {error}"),
            Self::DatabaseUnavailable => formatter.write_str("database unavailable"),
            Self::Unexpected => formatter.write_str("unexpected provider operation failure"),
        }
    }
}

impl std::error::Error for ProviderServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SecretStore(error) => Some(error),
            Self::Repository(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RepositoryError> for ProviderServiceError {
    fn from(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

#[derive(Debug)]
pub(crate) enum RepositoryError {
    NotFound,
    InUse,
    Conflict,
    Persistence(PersistenceError),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("not found"),
            Self::InUse => formatter.write_str("provider in use"),
            Self::Conflict => formatter.write_str("constraint conflict"),
            Self::Persistence(error) => write!(formatter, "persistence failed: {error}"),
            Self::Sqlite(_) => formatter.write_str("sqlite operation failed"),
        }
    }
}

impl std::error::Error for RepositoryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PersistenceError> for RepositoryError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

impl From<rusqlite::Error> for RepositoryError {
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn request(key: &str) -> ProviderCreateRequest {
        ProviderCreateRequest {
            provider_key: key.to_owned(),
            name: "Test Provider".to_owned(),
            preset_id: None,
            base_url: "https://provider.example.com/v1".to_owned(),
            protocol: ProviderProtocol::Responses,
            auth: ProviderAuthInput::OsSecretHelper {
                secret: "CAS_SYNTHETIC_TEST_SECRET".to_owned(),
            },
            enabled: true,
        }
    }

    #[test]
    fn provider_create_is_atomic_and_compensates_on_database_failure() {
        let service = ProviderService::in_memory();
        let deleted = Cell::new(false);

        let first = service
            .create_with_secret_store(request("provider-one"), |_, _| Ok(()), |_| Ok(true))
            .unwrap();
        assert!(first.credential_configured);

        let result = service.create_with_secret_store(
            request("provider-one"),
            |_, _| Ok(()),
            |_| {
                deleted.set(true);
                Ok(true)
            },
        );
        assert!(matches!(
            &result,
            Err(ProviderServiceError::Repository(RepositoryError::Conflict))
        ));
        let api_error = ApiError::from(result.err().unwrap());
        assert_eq!(api_error.code(), "PROVIDER_KEY_CONFLICT");
        assert_eq!(
            api_error
                .details
                .as_ref()
                .and_then(|details| details.get("field"))
                .map(String::as_str),
            Some("providerKey")
        );
        assert!(deleted.get());
        assert_eq!(
            service
                .repository()
                .unwrap()
                .list(None, None)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn secret_store_failure_does_not_create_database_rows() {
        let service = ProviderService::in_memory();
        let result = service.create_with_secret_store(
            request("provider-one"),
            |_, _| Err(SecretStoreError::AccessDenied),
            |_| panic!("failed store must not trigger cleanup"),
        );

        assert!(matches!(
            result,
            Err(ProviderServiceError::SecretStore(
                SecretStoreError::AccessDenied
            ))
        ));
        assert!(
            service
                .repository()
                .unwrap()
                .list(None, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn validates_provider_boundary() {
        assert!(PendingProvider::try_from(request("Provider One")).is_err());
        let mut insecure = request("provider-one");
        insecure.base_url = "http://provider.example.com".to_owned();
        assert!(PendingProvider::try_from(insecure).is_err());
        let mut local = request("provider-one");
        local.base_url = "http://127.0.0.1:8080/v1".to_owned();
        assert!(PendingProvider::try_from(local).is_ok());

        let mut unknown_preset = request("provider-one");
        unknown_preset.preset_id = Some("unknown".to_owned());
        assert!(PendingProvider::try_from(unknown_preset).is_err());
    }

    #[test]
    fn preset_provider_creates_initial_models_in_same_transaction() {
        let service = ProviderService::in_memory();
        let mut preset = request("deepseek");
        preset.preset_id = Some("deepseek".to_owned());

        let provider = service
            .create_with_secret_store(preset, |_, _| Ok(()), |_| Ok(true))
            .unwrap();

        assert_eq!(provider.model_count, 1);
        let providers = service.list(ProviderListRequest::default()).unwrap();
        assert_eq!(providers[0].preset_id.as_deref(), Some("deepseek"));
        let count = service
            .repository()
            .unwrap()
            .connection
            .query_row(
                "SELECT COUNT(*) FROM models WHERE provider_id = ?1",
                [&provider.id],
                |row| row.get::<_, u32>(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn provider_update_requires_confirmation_for_origin_change() {
        let service = ProviderService::in_memory();
        let provider = service
            .create_with_secret_store(request("provider-one"), |_, _| Ok(()), |_| Ok(true))
            .unwrap();
        let update = |confirm_origin_change| ProviderUpdateRequest {
            provider_id: provider.id.clone(),
            name: "Updated Provider".to_owned(),
            base_url: "https://other.example.com/v1".to_owned(),
            enabled: false,
            confirm_origin_change,
        };

        let error = match service.update(update(false)) {
            Ok(_) => panic!("origin change without confirmation must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "PROVIDER_ORIGIN_CONFIRMATION_REQUIRED");

        let updated = service.update(update(true)).unwrap();
        assert_eq!(updated.name, "Updated Provider");
        assert_eq!(updated.base_url, "https://other.example.com/v1");
        assert!(!updated.enabled);
    }

    #[test]
    fn provider_delete_cleans_secret_and_refuses_models() {
        let service = ProviderService::in_memory();
        let provider = service
            .create_with_secret_store(request("provider-one"), |_, _| Ok(()), |_| Ok(true))
            .unwrap();
        let deleted = Cell::new(false);
        let result = service.delete_with_secret_store(
            ProviderDeleteRequest {
                provider_id: provider.id.clone(),
            },
            |_| {
                deleted.set(true);
                Ok(true)
            },
        );
        assert!(result.is_ok());
        assert!(deleted.get());
        assert!(matches!(
            service.repository().unwrap().find_by_id(&provider.id),
            Err(RepositoryError::NotFound)
        ));

        let mut preset = request("deepseek");
        preset.preset_id = Some("deepseek".to_owned());
        let provider = service
            .create_with_secret_store(preset, |_, _| Ok(()), |_| Ok(true))
            .unwrap();
        let result = service.delete_with_secret_store(
            ProviderDeleteRequest {
                provider_id: provider.id,
            },
            |_| panic!("in-use provider must not delete its secret"),
        );
        assert!(matches!(
            result,
            Err(ProviderServiceError::Repository(RepositoryError::InUse))
        ));
    }
}
