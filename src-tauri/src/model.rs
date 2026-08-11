use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use cas_secret_store::{CredentialId, SecretStoreError, SecretValue, read as read_secret};
use reqwest::StatusCode;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, HeaderValue};
use reqwest::redirect::Policy;
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;
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

    pub(crate) fn update(
        &self,
        request: ModelUpdateRequest,
    ) -> Result<ModelDetailResponse, ApiError> {
        let pending = PendingModelUpdate::try_from(request)?;
        self.repository()?
            .update(&pending)
            .map(ModelDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn set_enabled(
        &self,
        request: ModelSetEnabledRequest,
    ) -> Result<ModelDetailResponse, ApiError> {
        let id = parse_uuid(&request.model_id, "modelId")?;
        self.repository()?
            .set_enabled(&id, request.enabled)
            .map(ModelDetailResponse::from)
            .map_err(ApiError::from)
    }

    pub(crate) fn delete(&self, request: ModelDeleteRequest) -> Result<(), ApiError> {
        let id = parse_uuid(&request.model_id, "modelId")?;
        self.repository()?.delete(&id).map_err(ApiError::from)
    }

    pub(crate) async fn test_connection(
        &self,
        request: ModelTestConnectionRequest,
    ) -> Result<ModelConnectionTestResponse, ApiError> {
        let id = parse_uuid(&request.model_id, "modelId")?;
        let target = self.repository()?.probe_target(&id)?;
        let result = match target.credential_id.as_deref() {
            None => ModelConnectionTestResponse::credential_missing(),
            Some(credential_id) => {
                let credential_id = CredentialId::from_str(credential_id)
                    .map_err(|_| ModelServiceError::InvalidCredentialReference)?;
                match read_secret(credential_id) {
                    Ok(secret) => tauri::async_runtime::spawn_blocking(move || {
                        run_model_probe(target, secret)
                    })
                    .await
                    .map_err(|_| ModelServiceError::ProbeTaskFailed)?,
                    Err(SecretStoreError::NotFound) => {
                        ModelConnectionTestResponse::credential_missing()
                    }
                    Err(error) => return Err(ModelServiceError::SecretStore(error).into()),
                }
            }
        };
        self.repository()?.record_connection_test(&id, &result)?;
        Ok(result)
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
                compatibility_level, compatibility_source, context_window, metadata_source,
                created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 1, 'USER', 'UNKNOWN', 'UNKNOWN', 'UNKNOWN', ?5,
                       ?6, ?7, ?7)",
            params![
                model.id,
                model.provider_id,
                model.model_id,
                model.display_name,
                model.context_window,
                model.context_window.map(|_| "USER_DECLARED"),
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
        let mut models = statement
            .query_map(
                params![search, provider_id, enabled, compatibility],
                map_model,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        for model in &mut models {
            model.reasoning_efforts = self.reasoning_efforts(&model.id)?;
        }
        Ok(models)
    }

    fn update(&mut self, model: &PendingModelUpdate) -> Result<ModelRecord, ModelRepositoryError> {
        let changed = self.connection.execute(
            "UPDATE models
             SET display_name = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![model.id, model.display_name],
        )?;
        if changed == 0 {
            return Err(ModelRepositoryError::NotFound);
        }
        self.find_by_id(&model.id)
    }

    fn set_enabled(
        &mut self,
        id: &str,
        enabled: bool,
    ) -> Result<ModelRecord, ModelRepositoryError> {
        let changed = self.connection.execute(
            "UPDATE models
             SET enabled = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1",
            params![id, enabled],
        )?;
        if changed == 0 {
            return Err(ModelRepositoryError::NotFound);
        }
        self.find_by_id(id)
    }

    fn delete(&mut self, id: &str) -> Result<(), ModelRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let in_use = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM agent_model_bindings WHERE model_id = m.id)
                 FROM models m WHERE m.id = ?1",
                [id],
                |row| row.get::<_, bool>(0),
            )
            .optional()?
            .ok_or(ModelRepositoryError::NotFound)?;
        if in_use {
            return Err(ModelRepositoryError::InUse);
        }
        transaction.execute("DELETE FROM models WHERE id = ?1", [id])?;
        transaction.commit()?;
        Ok(())
    }

    fn probe_target(&self, id: &str) -> Result<ModelProbeTarget, ModelRepositoryError> {
        self.connection
            .query_row(
                "SELECT m.model_id, p.base_url, c.id
                 FROM models m
                 JOIN providers p ON p.id = m.provider_id
                 LEFT JOIN credentials c ON c.provider_id = p.id AND c.credential_key = 'primary'
                 WHERE m.id = ?1",
                [id],
                |row| {
                    Ok(ModelProbeTarget {
                        model_id: row.get(0)?,
                        base_url: row.get(1)?,
                        credential_id: row.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or(ModelRepositoryError::NotFound)
    }

    fn record_connection_test(
        &mut self,
        id: &str,
        result: &ModelConnectionTestResponse,
    ) -> Result<(), ModelRepositoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let latency_ms = result
            .latency_ms
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX));
        let timestamp =
            transaction.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get::<_, String>(0)
            })?;
        let changed = transaction.execute(
            "UPDATE models
             SET last_test_status = ?2,
                 last_tested_at = ?4,
                 last_test_latency_ms = ?3
             WHERE id = ?1",
            params![id, result.status.as_str(), latency_ms, timestamp],
        )?;
        if changed == 0 {
            return Err(ModelRepositoryError::NotFound);
        }
        if let Some(supported) = result.responses_api_verified {
            upsert_probe_capability(
                &transaction,
                id,
                "RESPONSES_API",
                supported,
                &timestamp,
                &result.message,
            )?;
        }
        if let Some(supported) = result.tool_loop_verified {
            for capability in ["TOOL_CALLING", "CODEX_MULTI_AGENT"] {
                upsert_probe_capability(
                    &transaction,
                    id,
                    capability,
                    supported,
                    &timestamp,
                    &result.message,
                )?;
            }
        }
        match (result.responses_api_verified, result.tool_loop_verified) {
            (_, Some(true)) => {
                transaction.execute(
                    "UPDATE models
                     SET compatibility_level =
                            CASE WHEN source = 'PRESET' THEN 'NATIVE' ELSE 'COMPATIBLE' END,
                         compatibility_source = 'RUNTIME_PROBE',
                         compatibility_verified_at = ?2,
                         updated_at = ?2
                     WHERE id = ?1",
                    params![id, timestamp],
                )?;
            }
            (_, Some(false)) => {
                transaction.execute(
                    "UPDATE models
                     SET compatibility_level = 'GATEWAY_REQUIRED',
                         compatibility_source = 'RUNTIME_PROBE',
                         compatibility_verified_at = ?2,
                         updated_at = ?2
                     WHERE id = ?1",
                    params![id, timestamp],
                )?;
            }
            (Some(false), _) => {
                transaction.execute(
                    "UPDATE models
                     SET compatibility_level = 'UNSUPPORTED',
                         compatibility_source = 'RUNTIME_PROBE',
                         compatibility_verified_at = ?2,
                         updated_at = ?2
                     WHERE id = ?1",
                    params![id, timestamp],
                )?;
            }
            _ => {}
        }
        transaction.commit()?;
        Ok(())
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

fn upsert_probe_capability(
    transaction: &Transaction<'_>,
    model_id: &str,
    capability: &str,
    supported: bool,
    verified_at: &str,
    message: &str,
) -> Result<(), rusqlite::Error> {
    let status = if supported {
        "SUPPORTED"
    } else {
        "UNSUPPORTED"
    };
    let details = serde_json::to_string(&serde_json::json!({
        "probe": "responses-tool-loop",
        "version": 2,
        "message": message
    }))
    .expect("probe details must serialize");
    transaction.execute(
        "INSERT INTO model_capabilities (
            model_id, capability, status, source, confidence, verified_at,
            evidence_version, details_json
         ) VALUES (?1, ?2, ?3, 'RUNTIME_PROBE', 'VERIFIED', ?4,
                   'responses-tool-loop-v2', ?5)
         ON CONFLICT(model_id, capability) DO UPDATE SET
            status = excluded.status,
            source = excluded.source,
            confidence = excluded.confidence,
            verified_at = excluded.verified_at,
            evidence_version = excluded.evidence_version,
            details_json = excluded.details_json",
        params![model_id, capability, status, verified_at, details],
    )?;
    Ok(())
}

fn model_select() -> &'static str {
    "SELECT m.id, m.provider_id, p.name, m.model_id, m.display_name, m.enabled,
            m.lifecycle, m.compatibility_level, m.context_window, m.max_output_tokens,
            m.reasoning_supported, m.default_reasoning, m.compatibility_source,
            m.minimum_codex_version, m.compatibility_verified_at, m.created_at, m.updated_at,
            m.last_test_status, m.last_tested_at, m.last_test_latency_ms, m.source
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
        last_test_status: row.get(17)?,
        last_tested_at: row.get(18)?,
        last_test_latency_ms: row.get(19)?,
        source: row.get(20)?,
        reasoning_efforts: Vec::new(),
        capabilities: Vec::new(),
    })
}

struct ModelProbeTarget {
    model_id: String,
    base_url: String,
    credential_id: Option<String>,
}

fn run_model_probe(target: ModelProbeTarget, secret: SecretValue) -> ModelConnectionTestResponse {
    let started = Instant::now();
    let endpoint = match responses_endpoint(&target.base_url) {
        Ok(endpoint) => endpoint,
        Err(()) => return ModelConnectionTestResponse::protocol_error(None, None),
    };
    let client = match Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .redirect(Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return ModelConnectionTestResponse::unreachable(None),
    };

    let mut bearer = Vec::with_capacity(7 + secret.expose().len());
    bearer.extend_from_slice(b"Bearer ");
    bearer.extend_from_slice(secret.expose());
    let authorization = HeaderValue::from_bytes(&bearer);
    bearer.fill(0);
    let mut authorization = match authorization {
        Ok(value) => value,
        Err(_) => return ModelConnectionTestResponse::auth_failed(None, None),
    };
    authorization.set_sensitive(true);

    let basic_response = execute_probe_request(
        &client,
        &endpoint,
        &authorization,
        &serde_json::json!({
            "model": target.model_id,
            "input": "Reply with CAS_RESPONSES_OK.",
            "max_output_tokens": 64,
            "stream": false,
            "store": false
        }),
        started,
    );
    let (status, body, request_id) = match basic_response {
        Ok(response) => response,
        Err(error) => return error,
    };
    let latency_ms = Some(elapsed_ms(started));
    if !status.is_success() {
        return classify_probe_response(status, &body, latency_ms, request_id);
    }
    if parse_responses_body(&body).is_none() {
        return ModelConnectionTestResponse::protocol_error_with_message(
            latency_ms,
            request_id,
            "Endpoint 基础请求未返回有效的 Responses API 响应。",
        );
    }

    let probe_tool = serde_json::json!({
        "type": "function",
        "name": "cas_probe",
        "description": "Return the supplied probe value.",
        "parameters": {
            "type": "object",
            "properties": {
                "value": { "type": "string" }
            },
            "required": ["value"]
        }
    });
    let probe_prompt = "Call cas_probe exactly once with value CAS_OK.";
    let tool_call_response = execute_probe_request(
        &client,
        &endpoint,
        &authorization,
        &serde_json::json!({
            "model": target.model_id,
            "input": [{ "role": "user", "content": probe_prompt }],
            "max_output_tokens": 128,
            "tools": [probe_tool.clone()],
            // 与 Codex 实际运行一致；DeepSeek 思考模式会拒绝 required。
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": false,
            "store": false
        }),
        started,
    );
    let (status, body, request_id) = match tool_call_response {
        Ok(response) => response,
        Err(error) => return error,
    };
    let latency_ms = Some(elapsed_ms(started));
    if !status.is_success() {
        let classified = classify_probe_response(status, &body, latency_ms, request_id.clone());
        return if classified.status == ModelConnectionTestStatus::ProtocolError {
            ModelConnectionTestResponse::tool_call_error(
                latency_ms,
                request_id,
                provider_error_message(&body).as_deref(),
            )
        } else {
            classified
        };
    }
    let first_response = match parse_responses_body(&body) {
        Some(response) => response,
        None => {
            return ModelConnectionTestResponse::tool_call_error(latency_ms, request_id, None);
        }
    };
    let (tool_input, _) = match build_tool_result_input(&first_response, probe_prompt) {
        Some(input) => input,
        None => {
            return ModelConnectionTestResponse::tool_call_error(latency_ms, request_id, None);
        }
    };

    let tool_result_response = execute_probe_request(
        &client,
        &endpoint,
        &authorization,
        &serde_json::json!({
            "model": target.model_id,
            "input": tool_input,
            "max_output_tokens": 128,
            "tools": [probe_tool],
            "tool_choice": "auto",
            "parallel_tool_calls": false,
            "stream": false,
            "store": false
        }),
        started,
    );
    let (status, body, request_id) = match tool_result_response {
        Ok(response) => response,
        Err(error) => return error,
    };
    let latency_ms = Some(elapsed_ms(started));
    if status.is_success() {
        return if parse_responses_body(&body).is_some() {
            ModelConnectionTestResponse::success(latency_ms, request_id)
        } else {
            ModelConnectionTestResponse::tool_result_error(latency_ms, request_id, None)
        };
    }
    let classified = classify_probe_response(status, &body, latency_ms, request_id.clone());
    if classified.status == ModelConnectionTestStatus::ProtocolError {
        ModelConnectionTestResponse::tool_result_error(
            latency_ms,
            request_id,
            provider_error_message(&body).as_deref(),
        )
    } else {
        classified
    }
}

fn execute_probe_request(
    client: &Client,
    endpoint: &Url,
    authorization: &HeaderValue,
    body: &Value,
    started: Instant,
) -> Result<(StatusCode, Vec<u8>, Option<String>), ModelConnectionTestResponse> {
    let response = client
        .post(endpoint.clone())
        .header(AUTHORIZATION, authorization.clone())
        .header(
            "user-agent",
            concat!("Codex-Agent-Switch/", env!("CARGO_PKG_VERSION")),
        )
        .json(body)
        .send()
        .map_err(|_| ModelConnectionTestResponse::unreachable(Some(elapsed_ms(started))))?;
    read_probe_response(response, started)
}

fn read_probe_response(
    mut response: reqwest::blocking::Response,
    started: Instant,
) -> Result<(StatusCode, Vec<u8>, Option<String>), ModelConnectionTestResponse> {
    let status = response.status();
    let header_request_id = ["x-request-id", "request-id"]
        .into_iter()
        .find_map(|name| response.headers().get(name))
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 256)
        .map(str::to_owned);
    let mut body = Vec::new();
    let read_result = response
        .by_ref()
        .take(256 * 1024 + 1)
        .read_to_end(&mut body);
    let latency_ms = Some(elapsed_ms(started));
    if read_result.is_err() || body.len() > 256 * 1024 {
        return Err(ModelConnectionTestResponse::protocol_error(
            latency_ms,
            header_request_id,
        ));
    }
    let request_id = header_request_id.or_else(|| {
        serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("request_id")
                    .or_else(|| value.get("requestId"))
                    .or_else(|| value.get("error")?.get("request_id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .filter(|value| value.len() <= 256)
    });
    Ok((status, body, request_id))
}

fn parse_responses_body(body: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(body)
        .ok()
        .filter(|value| value.get("object").and_then(Value::as_str) == Some("response"))
}

fn build_tool_result_input(response: &Value, prompt: &str) -> Option<(Vec<Value>, String)> {
    let output = response.get("output")?.as_array()?;
    let function_call = output
        .iter()
        .find(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))?;
    let call_id = function_call.get("call_id")?.as_str()?.to_owned();
    let mut input = vec![serde_json::json!({ "role": "user", "content": prompt })];
    for item in output {
        input.push(item.clone());
        if item.get("type").and_then(Value::as_str) == Some("function_call")
            && item.get("call_id").and_then(Value::as_str) == Some(call_id.as_str())
        {
            input.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": call_id,
                "output": "CAS_PROBE_RESULT"
            }));
        }
    }
    Some((input, call_id))
}

fn provider_error_message(body: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(body).ok()?;
    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .or_else(|| value.get("error").filter(|error| error.is_string()))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)?;
    Some(message.chars().take(1000).collect())
}

fn responses_endpoint(base_url: &str) -> Result<Url, ()> {
    let mut url = Url::parse(base_url).map_err(|_| ())?;
    url.set_query(None);
    url.set_fragment(None);
    if url.path().trim_end_matches('/').ends_with("/responses") {
        return Ok(url);
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    url.join("responses").map_err(|_| ())
}

fn classify_probe_response(
    status: StatusCode,
    body: &[u8],
    latency_ms: Option<u64>,
    provider_request_id: Option<String>,
) -> ModelConnectionTestResponse {
    if status.is_success() {
        return if parse_responses_body(body).is_some() {
            ModelConnectionTestResponse::success(latency_ms, provider_request_id)
        } else {
            ModelConnectionTestResponse::protocol_error(latency_ms, provider_request_id)
        };
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        return ModelConnectionTestResponse::auth_failed(latency_ms, provider_request_id);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return ModelConnectionTestResponse::rate_limited(latency_ms, provider_request_id);
    }
    if status.is_server_error() {
        return ModelConnectionTestResponse::server_error(latency_ms, provider_request_id);
    }
    let model_missing = serde_json::from_slice::<Value>(body)
        .ok()
        .is_some_and(|value| {
            let error = value.get("error").unwrap_or(&value);
            let marker = ["code", "type"]
                .into_iter()
                .filter_map(|key| error.get(key).and_then(Value::as_str))
                .any(|value| {
                    matches!(
                        value.to_ascii_lowercase().as_str(),
                        "model_not_found" | "unknown_model" | "invalid_model"
                    )
                });
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_ascii_lowercase();
            marker
                || (message.contains("model")
                    && (message.contains("not found")
                        || message.contains("does not exist")
                        || message.contains("unknown")))
        });
    if model_missing {
        ModelConnectionTestResponse::model_not_found(latency_ms, provider_request_id)
    } else {
        ModelConnectionTestResponse::protocol_error(latency_ms, provider_request_id)
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
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
    context_window: Option<i64>,
}

struct PendingModelUpdate {
    id: String,
    display_name: String,
}

impl TryFrom<ModelUpdateRequest> for PendingModelUpdate {
    type Error = ModelServiceError;

    fn try_from(request: ModelUpdateRequest) -> Result<Self, Self::Error> {
        validate_text(&request.display_name, "displayName", 160)?;
        Ok(Self {
            id: parse_uuid(&request.model_id, "modelId")?,
            display_name: request.display_name.trim().to_owned(),
        })
    }
}

impl TryFrom<ModelAddRequest> for NewUserModel {
    type Error = ModelServiceError;

    fn try_from(request: ModelAddRequest) -> Result<Self, Self::Error> {
        let provider_id = parse_uuid(&request.provider_id, "providerId")?;
        validate_model_id(&request.model_id)?;
        if request.context_window.is_some_and(|value| value <= 0) {
            return Err(ModelServiceError::InvalidField("contextWindow"));
        }
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
            context_window: request.context_window,
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
    last_test_status: Option<String>,
    last_tested_at: Option<String>,
    last_test_latency_ms: Option<i64>,
    source: String,
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
    context_window: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelUpdateRequest {
    model_id: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelSetEnabledRequest {
    model_id: String,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelDeleteRequest {
    model_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelTestConnectionRequest {
    model_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelConnectionTestResponse {
    status: ModelConnectionTestStatus,
    latency_ms: Option<u64>,
    provider_request_id: Option<String>,
    message: String,
    #[serde(skip)]
    responses_api_verified: Option<bool>,
    #[serde(skip)]
    tool_loop_verified: Option<bool>,
}

impl ModelConnectionTestResponse {
    fn new(
        status: ModelConnectionTestStatus,
        latency_ms: Option<u64>,
        provider_request_id: Option<String>,
        message: impl Into<String>,
        responses_api_verified: Option<bool>,
        tool_loop_verified: Option<bool>,
    ) -> Self {
        Self {
            status,
            latency_ms,
            provider_request_id,
            message: message.into(),
            responses_api_verified,
            tool_loop_verified,
        }
    }

    fn success(latency_ms: Option<u64>, provider_request_id: Option<String>) -> Self {
        Self::new(
            ModelConnectionTestStatus::Success,
            latency_ms,
            provider_request_id,
            "Model 已通过 Responses API 与 Function Calling 工具闭环验证。",
            Some(true),
            Some(true),
        )
    }

    fn credential_missing() -> Self {
        Self::new(
            ModelConnectionTestStatus::CredentialMissing,
            None,
            None,
            "Provider Credential 不存在或已从系统凭据库移除。",
            None,
            None,
        )
    }

    fn auth_failed(latency_ms: Option<u64>, provider_request_id: Option<String>) -> Self {
        Self::new(
            ModelConnectionTestStatus::AuthFailed,
            latency_ms,
            provider_request_id,
            "Provider 拒绝了当前 Credential。",
            None,
            None,
        )
    }

    fn model_not_found(latency_ms: Option<u64>, provider_request_id: Option<String>) -> Self {
        Self::new(
            ModelConnectionTestStatus::ModelNotFound,
            latency_ms,
            provider_request_id,
            "Provider 不识别当前 Model ID。",
            None,
            None,
        )
    }

    fn rate_limited(latency_ms: Option<u64>, provider_request_id: Option<String>) -> Self {
        Self::new(
            ModelConnectionTestStatus::RateLimited,
            latency_ms,
            provider_request_id,
            "Provider 当前触发限流，请稍后重试。",
            None,
            None,
        )
    }

    fn protocol_error(latency_ms: Option<u64>, provider_request_id: Option<String>) -> Self {
        Self::protocol_error_with_message(
            latency_ms,
            provider_request_id,
            "Endpoint 未返回有效的 Responses API 响应。",
        )
    }

    fn protocol_error_with_message(
        latency_ms: Option<u64>,
        provider_request_id: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            ModelConnectionTestStatus::ProtocolError,
            latency_ms,
            provider_request_id,
            message,
            Some(false),
            None,
        )
    }

    fn tool_call_error(
        latency_ms: Option<u64>,
        provider_request_id: Option<String>,
        provider_message: Option<&str>,
    ) -> Self {
        let mut message =
            "Responses API 基础请求成功，但 Provider 未完成 Function Calling 工具调用，不能用于 Codex Agent。"
                .to_owned();
        if let Some(provider_message) = provider_message {
            message.push_str(" Provider: ");
            message.push_str(provider_message);
        }
        Self::new(
            ModelConnectionTestStatus::ProtocolError,
            latency_ms,
            provider_request_id,
            message,
            Some(true),
            Some(false),
        )
    }

    fn tool_result_error(
        latency_ms: Option<u64>,
        provider_request_id: Option<String>,
        provider_message: Option<&str>,
    ) -> Self {
        let mut message =
            "Responses API 首轮成功，但 Provider 拒绝 Function Calling 工具结果，不能用于 Codex Agent。"
                .to_owned();
        if let Some(provider_message) = provider_message {
            message.push_str(" Provider: ");
            message.push_str(provider_message);
        }
        Self::new(
            ModelConnectionTestStatus::ProtocolError,
            latency_ms,
            provider_request_id,
            message,
            Some(true),
            Some(false),
        )
    }

    fn unreachable(latency_ms: Option<u64>) -> Self {
        Self::new(
            ModelConnectionTestStatus::Unreachable,
            latency_ms,
            None,
            "无法连接 Provider，或请求在 20 秒内未完成。",
            None,
            None,
        )
    }

    fn server_error(latency_ms: Option<u64>, provider_request_id: Option<String>) -> Self {
        Self::new(
            ModelConnectionTestStatus::ServerError,
            latency_ms,
            provider_request_id,
            "Provider 返回了服务端错误。",
            None,
            None,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ModelConnectionTestStatus {
    Success,
    CredentialMissing,
    AuthFailed,
    ModelNotFound,
    RateLimited,
    ProtocolError,
    Unreachable,
    ServerError,
}

impl ModelConnectionTestStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "SUCCESS",
            Self::CredentialMissing => "CREDENTIAL_MISSING",
            Self::AuthFailed => "AUTH_FAILED",
            Self::ModelNotFound => "MODEL_NOT_FOUND",
            Self::RateLimited => "RATE_LIMITED",
            Self::ProtocolError => "PROTOCOL_ERROR",
            Self::Unreachable => "UNREACHABLE",
            Self::ServerError => "SERVER_ERROR",
        }
    }
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
    source: String,
    reasoning_status: &'static str,
    supported_reasoning_efforts: Vec<String>,
    default_reasoning_effort: Option<String>,
    last_test_status: Option<String>,
    last_tested_at: Option<String>,
    last_test_latency_ms: Option<i64>,
}

impl From<ModelRecord> for ModelSummary {
    fn from(model: ModelRecord) -> Self {
        let reasoning_status = match model.reasoning_supported {
            Some(true) => "SUPPORTED",
            Some(false) => "UNSUPPORTED",
            None => "UNKNOWN",
        };
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
            source: model.source,
            reasoning_status,
            supported_reasoning_efforts: model.reasoning_efforts,
            default_reasoning_effort: model.default_reasoning,
            last_test_status: model.last_test_status,
            last_tested_at: model.last_tested_at,
            last_test_latency_ms: model.last_test_latency_ms,
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
    InvalidCredentialReference,
    SecretStore(SecretStoreError),
    ProbeTaskFailed,
    Repository(ModelRepositoryError),
    DatabaseUnavailable,
}

impl fmt::Display for ModelServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(formatter, "invalid model field: {field}"),
            Self::InvalidCredentialReference => formatter.write_str("invalid credential reference"),
            Self::SecretStore(_) => formatter.write_str("secret store operation failed"),
            Self::ProbeTaskFailed => formatter.write_str("model probe task failed"),
            Self::Repository(error) => write!(formatter, "model repository failed: {error}"),
            Self::DatabaseUnavailable => formatter.write_str("database unavailable"),
        }
    }
}

impl std::error::Error for ModelServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SecretStore(error) => Some(error),
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
    InUse,
    ProviderNotFound,
    Conflict,
    Persistence(PersistenceError),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for ModelRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("model not found"),
            Self::InUse => formatter.write_str("model in use"),
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
            ModelServiceError::SecretStore(SecretStoreError::AccessDenied) => {
                ("SECRET_STORE_ACCESS_DENIED", "系统凭据库拒绝访问。", false)
            }
            ModelServiceError::SecretStore(SecretStoreError::Unavailable) => {
                ("SECRET_STORE_UNAVAILABLE", "系统凭据库当前不可用。", true)
            }
            ModelServiceError::SecretStore(_) => {
                ("SECRET_STORE_READ_FAILED", "Credential 读取失败。", true)
            }
            ModelServiceError::InvalidCredentialReference => {
                ("DATABASE_OPERATION_FAILED", "Credential 引用无效。", false)
            }
            ModelServiceError::ProbeTaskFailed => {
                ("MODEL_TEST_FAILED", "Model 测试任务异常终止。", true)
            }
            ModelServiceError::Repository(ModelRepositoryError::NotFound) => {
                ("MODEL_NOT_FOUND", "Model 不存在。", false)
            }
            ModelServiceError::Repository(ModelRepositoryError::InUse) => (
                "MODEL_IN_USE",
                "Model 仍被 Agent 引用，请先解除绑定。",
                false,
            ),
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
    fn model_list_exposes_preset_reasoning_limits() {
        let service = ModelService::in_memory();
        let provider_id = insert_provider(&service);
        let model = initial_models_for_preset("deepseek").unwrap().remove(0);
        {
            let mut repository = service.repository().unwrap();
            let transaction = repository.connection.transaction().unwrap();
            insert_preset_model(&transaction, &provider_id, &model, "2026-01-01T00:00:00Z")
                .unwrap();
            transaction.commit().unwrap();
        }

        let listed = service.list(ModelListRequest::default()).unwrap();

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].source, "PRESET");
        assert_eq!(listed[0].reasoning_status, "SUPPORTED");
        assert_eq!(
            listed[0].supported_reasoning_efforts,
            ["low", "medium", "high"]
        );
        assert_eq!(listed[0].default_reasoning_effort.as_deref(), Some("high"));
    }

    #[test]
    fn custom_model_defaults_to_unknown_and_is_unique_per_provider() {
        let service = ModelService::in_memory();
        let provider_id = insert_provider(&service);
        let request = || ModelAddRequest {
            provider_id: provider_id.clone(),
            model_id: "custom/model-v1".to_owned(),
            display_name: None,
            context_window: Some(128_000),
        };

        let created = match service.add(request()) {
            Ok(created) => created,
            Err(_) => panic!("valid model must be created"),
        };
        assert_eq!(created.compatibility.level, "UNKNOWN");
        assert_eq!(created.context_window, Some(128_000));
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
            context_window: None,
        });
        let error = match result {
            Ok(_) => panic!("missing provider must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "PROVIDER_NOT_FOUND");
    }

    #[test]
    fn model_can_be_updated_disabled_and_deleted() {
        let service = ModelService::in_memory();
        let provider_id = insert_provider(&service);
        let model = service
            .add(ModelAddRequest {
                provider_id,
                model_id: "model-v1".to_owned(),
                display_name: None,
                context_window: None,
            })
            .unwrap();

        let updated = service
            .update(ModelUpdateRequest {
                model_id: model.id.clone(),
                display_name: "Model One".to_owned(),
            })
            .unwrap();
        assert_eq!(updated.display_name, "Model One");
        let disabled = service
            .set_enabled(ModelSetEnabledRequest {
                model_id: model.id.clone(),
                enabled: false,
            })
            .unwrap();
        assert!(!disabled.enabled);

        service
            .delete(ModelDeleteRequest {
                model_id: model.id.clone(),
            })
            .unwrap();
        assert!(matches!(
            service.repository().unwrap().find_by_id(&model.id),
            Err(ModelRepositoryError::NotFound)
        ));
    }

    #[test]
    fn model_delete_refuses_agent_binding() {
        let service = ModelService::in_memory();
        let provider_id = insert_provider(&service);
        let model = service
            .add(ModelAddRequest {
                provider_id,
                model_id: "model-v1".to_owned(),
                display_name: None,
                context_window: None,
            })
            .unwrap();
        let agent_id = Uuid::new_v4().to_string();
        let binding_id = Uuid::new_v4().to_string();
        {
            let repository = service.repository().unwrap();
            repository
                .connection
                .execute(
                    "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                 ) VALUES (?1, 'executor', 'Executor', 'description', 'instruction', 'CUSTOM', 1,
                           'READ_ONLY', 'INHERIT', 'USER', 1, ?2, ?2)",
                    params![agent_id, "2026-01-01T00:00:00Z"],
                )
                .unwrap();
            repository
                .connection
                .execute(
                    "INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, enabled, priority, source, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 1, 0, 'USER', ?4, ?4)",
                    params![binding_id, agent_id, model.id, "2026-01-01T00:00:00Z"],
                )
                .unwrap();
        }

        let error = service
            .delete(ModelDeleteRequest { model_id: model.id })
            .unwrap_err();
        assert_eq!(error.code(), "MODEL_IN_USE");
    }

    #[test]
    fn responses_probe_uses_provider_base_path() {
        assert_eq!(
            responses_endpoint("https://provider.example.com/")
                .unwrap()
                .as_str(),
            "https://provider.example.com/responses"
        );
        assert_eq!(
            responses_endpoint("https://provider.example.com/v1")
                .unwrap()
                .as_str(),
            "https://provider.example.com/v1/responses"
        );
        assert_eq!(
            responses_endpoint("https://provider.example.com/v1/responses")
                .unwrap()
                .as_str(),
            "https://provider.example.com/v1/responses"
        );
    }

    #[test]
    fn tool_loop_test_updates_runtime_compatibility_and_capabilities() {
        let service = ModelService::in_memory();
        let provider_id = insert_provider(&service);
        let model = service
            .add(ModelAddRequest {
                provider_id,
                model_id: "model-v1".to_owned(),
                display_name: None,
                context_window: None,
            })
            .unwrap();
        let result = ModelConnectionTestResponse::success(Some(42), None);

        let mut repository = service.repository().unwrap();
        repository
            .record_connection_test(&model.id, &result)
            .unwrap();
        let saved = repository.find_by_id(&model.id).unwrap();

        assert_eq!(saved.last_test_status.as_deref(), Some("SUCCESS"));
        assert_eq!(saved.last_test_latency_ms, Some(42));
        assert!(saved.last_tested_at.is_some());
        assert_eq!(saved.compatibility_level, "COMPATIBLE");
        assert_eq!(saved.compatibility_source, "RUNTIME_PROBE");
        for capability in ["RESPONSES_API", "TOOL_CALLING", "CODEX_MULTI_AGENT"] {
            assert!(
                saved
                    .capabilities
                    .iter()
                    .any(|record| record.capability == capability && record.status == "SUPPORTED")
            );
        }

        let transient = ModelConnectionTestResponse::rate_limited(
            Some(45),
            Some("request-rate-limit".to_owned()),
        );
        repository
            .record_connection_test(&model.id, &transient)
            .unwrap();
        assert_eq!(
            repository
                .find_by_id(&model.id)
                .unwrap()
                .compatibility_level,
            "COMPATIBLE"
        );

        let failed = ModelConnectionTestResponse::tool_call_error(
            Some(48),
            Some("request-tool-call".to_owned()),
            Some("tools are not supported"),
        );
        repository
            .record_connection_test(&model.id, &failed)
            .unwrap();
        let saved = repository.find_by_id(&model.id).unwrap();
        assert_eq!(saved.compatibility_level, "GATEWAY_REQUIRED");
        assert!(saved.capabilities.iter().any(|record| {
            record.capability == "RESPONSES_API" && record.status == "SUPPORTED"
        }));
        for capability in ["TOOL_CALLING", "CODEX_MULTI_AGENT"] {
            assert!(
                saved.capabilities.iter().any(
                    |record| record.capability == capability && record.status == "UNSUPPORTED"
                )
            );
        }

        let failed = ModelConnectionTestResponse::tool_result_error(
            Some(50),
            Some("request-tool-result".to_owned()),
            Some("role tool is invalid"),
        );
        repository
            .record_connection_test(&model.id, &failed)
            .unwrap();
        let saved = repository.find_by_id(&model.id).unwrap();
        assert_eq!(saved.last_test_status.as_deref(), Some("PROTOCOL_ERROR"));
        assert_eq!(saved.compatibility_level, "GATEWAY_REQUIRED");
        for capability in ["TOOL_CALLING", "CODEX_MULTI_AGENT"] {
            assert!(
                saved.capabilities.iter().any(
                    |record| record.capability == capability && record.status == "UNSUPPORTED"
                )
            );
        }

        let failed = ModelConnectionTestResponse::protocol_error_with_message(
            Some(20),
            Some("request-basic".to_owned()),
            "not a Responses endpoint",
        );
        repository
            .record_connection_test(&model.id, &failed)
            .unwrap();
        let saved = repository.find_by_id(&model.id).unwrap();
        assert_eq!(saved.compatibility_level, "UNSUPPORTED");
        assert!(saved.capabilities.iter().any(|record| {
            record.capability == "RESPONSES_API" && record.status == "UNSUPPORTED"
        }));
    }

    #[test]
    fn tool_result_is_inserted_immediately_after_matching_function_call() {
        let response = serde_json::json!({
            "object": "response",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "准备调用工具" }]
                },
                {
                    "type": "function_call",
                    "name": "cas_probe",
                    "arguments": "{\"value\":\"CAS_OK\"}",
                    "call_id": "call-1"
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "稍后完成" }]
                }
            ]
        });

        let (input, call_id) = build_tool_result_input(&response, "probe").unwrap();

        assert_eq!(call_id, "call-1");
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call-1");
    }

    #[test]
    fn custom_model_rejects_non_positive_context_window() {
        let service = ModelService::in_memory();
        let result = service.add(ModelAddRequest {
            provider_id: insert_provider(&service),
            model_id: "model-v1".to_owned(),
            display_name: None,
            context_window: Some(0),
        });
        let error = match result {
            Ok(_) => panic!("non-positive context window must fail"),
            Err(error) => error,
        };

        assert_eq!(error.code(), "VALIDATION_ERROR");
    }

    #[test]
    fn responses_probe_classifies_protocol_and_model_errors() {
        let success = classify_probe_response(
            StatusCode::OK,
            br#"{"id":"resp_1","object":"response","output":[]}"#,
            Some(12),
            None,
        );
        assert_eq!(success.status, ModelConnectionTestStatus::Success);

        let wrong_protocol =
            classify_probe_response(StatusCode::OK, br#"{"choices":[]}"#, Some(12), None);
        assert_eq!(
            wrong_protocol.status,
            ModelConnectionTestStatus::ProtocolError
        );

        let missing = classify_probe_response(
            StatusCode::BAD_REQUEST,
            br#"{"error":{"code":"model_not_found","message":"unknown"}}"#,
            Some(12),
            None,
        );
        assert_eq!(missing.status, ModelConnectionTestStatus::ModelNotFound);

        let auth = classify_probe_response(StatusCode::UNAUTHORIZED, b"{}", Some(12), None);
        assert_eq!(auth.status, ModelConnectionTestStatus::AuthFailed);
    }
}
