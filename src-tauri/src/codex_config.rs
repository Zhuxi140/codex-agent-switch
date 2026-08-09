use std::fmt;
use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, value};

use crate::domain::{Agent, BaseBinding, Model, ResponsesProvider};

const AUTH_TIMEOUT_MS: i64 = 5_000;
const AUTH_REFRESH_INTERVAL_MS: i64 = 300_000;

pub(crate) struct ProviderProjection<'a> {
    pub(crate) provider_id: &'a str,
    pub(crate) display_name: &'a str,
    pub(crate) base_url: &'a str,
    pub(crate) helper_command: &'a str,
    pub(crate) credential_id: &'a str,
}

pub(crate) struct AgentProjection<'a> {
    pub(crate) agent_key: &'a str,
    pub(crate) description: &'a str,
    pub(crate) model_id: &'a str,
    pub(crate) provider_id: &'a str,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) sandbox_mode: Option<&'a str>,
    pub(crate) developer_instructions: &'a str,
    pub(crate) model_catalog_path: Option<&'a Path>,
}

pub(crate) fn upsert_provider_projection(
    existing: &str,
    provider: &ProviderProjection<'_>,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    if !document.contains_key("model_providers") {
        document["model_providers"] = Item::Table(Table::new());
    }

    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or(ConfigError::InvalidStructure("model_providers"))?;
    if !providers.contains_key(provider.provider_id) {
        providers.insert(provider.provider_id, Item::Table(Table::new()));
    }
    let provider_table = providers
        .get_mut(provider.provider_id)
        .and_then(Item::as_table_mut)
        .ok_or(ConfigError::InvalidStructure(
            "model_providers.<cas_provider>",
        ))?;

    provider_table["name"] = value(provider.display_name);
    provider_table["base_url"] = value(provider.base_url);
    provider_table["wire_api"] = value("responses");
    for incompatible in [
        "env_key",
        "experimental_bearer_token",
        "requires_openai_auth",
    ] {
        provider_table.remove(incompatible);
    }

    if !provider_table.contains_key("auth") {
        provider_table["auth"] = Item::Table(Table::new());
    }
    let auth = provider_table["auth"]
        .as_table_mut()
        .ok_or(ConfigError::InvalidStructure(
            "model_providers.<cas_provider>.auth",
        ))?;
    let mut args = Array::new();
    args.push("token");
    args.push(provider.credential_id);
    auth["command"] = value(provider.helper_command);
    auth["args"] = value(args);
    auth["timeout_ms"] = value(AUTH_TIMEOUT_MS);
    auth["refresh_interval_ms"] = value(AUTH_REFRESH_INTERVAL_MS);

    Ok(render_document(document, existing))
}

pub(crate) fn remove_provider_projection(
    existing: &str,
    provider_id: &str,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    if document.contains_key("model_providers") {
        let providers = document["model_providers"]
            .as_table_mut()
            .ok_or(ConfigError::InvalidStructure("model_providers"))?;
        providers.remove(provider_id);
    }
    Ok(render_document(document, existing))
}

pub(crate) fn restore_provider_projection(
    current: &str,
    snapshot: &str,
    provider_id: &str,
) -> Result<String, ConfigError> {
    let snapshot = snapshot.parse::<DocumentMut>()?;
    let fragment = snapshot
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(provider_id))
        .cloned();
    let original = current;
    let mut document = original.parse::<DocumentMut>()?;
    if !document.contains_key("model_providers") && fragment.is_some() {
        document["model_providers"] = Item::Table(Table::new());
    }
    if document.contains_key("model_providers") {
        let providers = document["model_providers"]
            .as_table_mut()
            .ok_or(ConfigError::InvalidStructure("model_providers"))?;
        match fragment {
            Some(fragment) => {
                providers.insert(provider_id, fragment);
            }
            None => {
                providers.remove(provider_id);
            }
        }
    }
    Ok(render_document(document, original))
}

pub(crate) fn provider_projection_semantic(
    document: &str,
    provider_id: &str,
) -> Result<Option<String>, ConfigError> {
    let document = document.parse::<DocumentMut>()?;
    Ok(document
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(provider_id))
        .map(canonical_item))
}

pub(crate) fn render_agent_projection(agent: &AgentProjection<'_>) -> Result<String, ConfigError> {
    let mut document = DocumentMut::new();
    document["name"] = value(agent.agent_key);
    document["description"] = value(agent.description);
    document["model"] = value(agent.model_id);
    document["model_provider"] = value(agent.provider_id);
    if let Some(reasoning_effort) = agent.reasoning_effort {
        document["model_reasoning_effort"] = value(reasoning_effort);
    }
    if let Some(sandbox_mode) = agent.sandbox_mode {
        document["sandbox_mode"] = value(sandbox_mode);
    }
    document["developer_instructions"] = value(agent.developer_instructions);
    if let Some(path) = agent.model_catalog_path {
        document["model_catalog_json"] = value(path.to_string_lossy().into_owned());
    }
    let rendered = document.to_string();
    rendered.parse::<DocumentMut>()?;
    Ok(rendered)
}

pub(crate) fn document_semantic(document: &str) -> Result<String, ConfigError> {
    let document = document.parse::<DocumentMut>()?;
    Ok(canonical_table(document.as_table()))
}

fn render_document(document: DocumentMut, existing: &str) -> String {
    let rendered = document.to_string();
    if existing.contains("\r\n") {
        rendered.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        rendered
    }
}

fn canonical_item(item: &Item) -> String {
    match item {
        Item::None => "null".to_owned(),
        Item::Value(value) => canonical_value(value),
        Item::Table(table) => canonical_table(table),
        Item::ArrayOfTables(tables) => format!(
            "[{}]",
            tables
                .iter()
                .map(canonical_table)
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

fn canonical_table(table: &Table) -> String {
    let mut entries = table.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(key, _)| *key);
    format!(
        "{{{}}}",
        entries
            .into_iter()
            .map(|(key, item)| format!("{}:{}", json_string(key), canonical_item(item)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn canonical_value(value: &TomlValue) -> String {
    match value {
        TomlValue::String(value) => json_string(value.value()),
        TomlValue::Integer(value) => value.value().to_string(),
        TomlValue::Float(value) => value.value().to_string(),
        TomlValue::Boolean(value) => value.value().to_string(),
        TomlValue::Datetime(value) => json_string(&value.value().to_string()),
        TomlValue::Array(array) => format!(
            "[{}]",
            array
                .iter()
                .map(canonical_value)
                .collect::<Vec<_>>()
                .join(",")
        ),
        TomlValue::InlineTable(table) => {
            let mut entries = table.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| *key);
            format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| {
                        format!("{}:{}", json_string(key), canonical_value(value))
                    })
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("serializing a string cannot fail")
}

pub(crate) fn upsert_responses_provider(
    existing: &str,
    provider: &ResponsesProvider,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;

    if !document.contains_key("model_providers") {
        document["model_providers"] = Item::Table(Table::new());
    }

    let providers = document["model_providers"]
        .as_table_mut()
        .ok_or(ConfigError::InvalidStructure("model_providers"))?;

    let mut provider_table = Table::new();
    provider_table["name"] = value(provider.display_name());
    provider_table["base_url"] = value(provider.base_url());
    provider_table["wire_api"] = value("responses");

    let mut args = Array::new();
    args.push("token");
    args.push(provider.auth().credential_id());

    let mut auth_table = Table::new();
    auth_table["command"] = value(provider.auth().command().to_string_lossy().into_owned());
    auth_table["args"] = value(args);
    auth_table["timeout_ms"] = value(AUTH_TIMEOUT_MS);
    auth_table["refresh_interval_ms"] = value(AUTH_REFRESH_INTERVAL_MS);
    provider_table["auth"] = Item::Table(auth_table);

    providers.insert(provider.key(), Item::Table(provider_table));

    Ok(document.to_string())
}

pub(crate) fn render_agent_config(
    agent: &Agent,
    model: &Model,
    binding: &BaseBinding,
) -> Result<String, ConfigError> {
    if binding.agent_key() != agent.key() {
        return Err(ConfigError::BindingMismatch("agent_key"));
    }
    if binding.model_id() != model.id() {
        return Err(ConfigError::BindingMismatch("model_id"));
    }

    let mut document = DocumentMut::new();
    document["name"] = value(agent.name());
    document["description"] = value(agent.description());
    document["model"] = value(model.id());
    document["model_provider"] = value(model.provider_key());
    document["model_reasoning_effort"] = value(binding.reasoning_effort().as_str());
    document["sandbox_mode"] = value("workspace-write");
    document["developer_instructions"] = value(agent.developer_instructions());
    document["model_catalog_json"] = value(model.catalog_path().to_string_lossy().into_owned());

    Ok(document.to_string())
}

#[derive(Debug)]
pub(crate) enum ConfigError {
    Parse(toml_edit::TomlError),
    InvalidStructure(&'static str),
    BindingMismatch(&'static str),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(error) => write!(formatter, "invalid TOML: {error}"),
            Self::InvalidStructure(field) => write!(formatter, "invalid TOML structure: {field}"),
            Self::BindingMismatch(field) => write!(formatter, "binding mismatch: {field}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<toml_edit::TomlError> for ConfigError {
    fn from(error: toml_edit::TomlError) -> Self {
        Self::Parse(error)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::{CommandAuth, ReasoningEffort};

    #[test]
    fn preserves_unmanaged_config_when_upserting_provider() {
        let existing = r#"# 用户配置必须保留
[mcp_servers.example]
command = "keep-me"
"#;
        let provider = provider(
            "cas_deepseek",
            "DeepSeek",
            "https://api.deepseek.com/",
            "deepseek-main",
        );

        let rendered = upsert_responses_provider(existing, &provider).unwrap();
        let document = rendered.parse::<DocumentMut>().unwrap();

        assert!(rendered.contains("# 用户配置必须保留"));
        assert_eq!(
            document["mcp_servers"]["example"]["command"].as_str(),
            Some("keep-me")
        );
        assert_eq!(
            document["model_providers"]["cas_deepseek"]["wire_api"].as_str(),
            Some("responses")
        );
        assert_eq!(
            document["model_providers"]["cas_deepseek"]["auth"]["args"]
                .as_array()
                .unwrap()
                .get(1)
                .and_then(|value| value.as_str()),
            Some("deepseek-main")
        );
    }

    #[test]
    fn same_renderer_accepts_custom_responses_provider() {
        let provider = provider(
            "cas_company",
            "Company Gateway",
            "https://models.example.com/v1/",
            "company-main",
        );

        let rendered = upsert_responses_provider("", &provider).unwrap();
        let document = rendered.parse::<DocumentMut>().unwrap();

        assert_eq!(
            document["model_providers"]["cas_company"]["base_url"].as_str(),
            Some("https://models.example.com/v1/")
        );
        assert_eq!(
            document["model_providers"]["cas_company"]["wire_api"].as_str(),
            Some("responses")
        );
    }

    #[test]
    fn renders_agent_from_base_binding() {
        let agent = Agent::new(
            "executor",
            "Executor",
            "执行已批准的实现任务",
            "保持修改小且可验证。",
        )
        .unwrap();
        let model = Model::new(
            "deepseek-v4-flash",
            "cas_deepseek",
            PathBuf::from(r"C:\Users\tester\.codex\models\deepseek-v4-flash.json"),
        )
        .unwrap();
        let binding =
            BaseBinding::new("executor", "deepseek-v4-flash", ReasoningEffort::High).unwrap();

        let rendered = render_agent_config(&agent, &model, &binding).unwrap();
        let document = rendered.parse::<DocumentMut>().unwrap();

        assert_eq!(document["model"].as_str(), Some("deepseek-v4-flash"));
        assert_eq!(document["model_provider"].as_str(), Some("cas_deepseek"));
        assert_eq!(document["model_reasoning_effort"].as_str(), Some("high"));
        assert_eq!(document["sandbox_mode"].as_str(), Some("workspace-write"));
    }

    #[test]
    fn semantic_fingerprint_ignores_comments_and_key_order() {
        let first = r#"# comment
name = "executor"
model = "deepseek-v4-flash"
"#;
        let second = r#"model = "deepseek-v4-flash" # another comment
name = "executor"
"#;

        assert_eq!(
            document_semantic(first).unwrap(),
            document_semantic(second).unwrap()
        );
    }

    fn provider(
        key: &str,
        display_name: &str,
        base_url: &str,
        credential_id: &str,
    ) -> ResponsesProvider {
        let auth = CommandAuth::new(
            r"C:\Program Files\Codex Agent Switch\bin\cas-helper.exe",
            credential_id,
        )
        .unwrap();

        ResponsesProvider::new(key, display_name, base_url, auth).unwrap()
    }
}
