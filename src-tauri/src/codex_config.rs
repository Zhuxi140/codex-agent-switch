use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue, value};

use crate::domain::{Agent, BaseBinding, Model, ResponsesProvider};

const AUTH_TIMEOUT_MS: i64 = 5_000;
const AUTH_REFRESH_INTERVAL_MS: i64 = 300_000;
const ORCHESTRATION_BEGIN: &str = "<<< CAS ORCHESTRATION v1 >>>";
const ORCHESTRATION_END: &str = "<<< END CAS ORCHESTRATION v1 >>>";
const GLOBAL_ORCHESTRATION_BEGIN: &str = "<!-- CAS ORCHESTRATION v1 BEGIN -->";
const GLOBAL_ORCHESTRATION_END: &str = "<!-- CAS ORCHESTRATION v1 END -->";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum PermissionStyle {
    DefaultPermissions,
    SandboxMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationBaseline {
    pub(crate) permission_style: PermissionStyle,
    pub(crate) default_permissions: Option<String>,
    pub(crate) sandbox_mode: Option<String>,
    pub(crate) agents_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) global_instructions_path: Option<String>,
    #[serde(default)]
    pub(crate) global_instructions_existed: bool,
    #[serde(default)]
    pub(crate) global_instructions_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProjectExclusionBaseline {
    projection_style: PermissionStyle,
    default_permissions: Option<String>,
    sandbox_mode: Option<String>,
    agents_enabled: Option<bool>,
}

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

pub(crate) fn upsert_model_catalog_projection(
    existing: &str,
    path: &Path,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    document["model_catalog_json"] = value(path.to_string_lossy().into_owned());
    Ok(render_document(document, existing))
}

pub(crate) fn remove_model_catalog_projection(existing: &str) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    document.remove("model_catalog_json");
    Ok(render_document(document, existing))
}

pub(crate) fn restore_model_catalog_projection(
    current: &str,
    snapshot: &str,
) -> Result<String, ConfigError> {
    let snapshot = snapshot.parse::<DocumentMut>()?;
    let mut document = current.parse::<DocumentMut>()?;
    match snapshot.get("model_catalog_json").cloned() {
        Some(item) => {
            document.insert("model_catalog_json", item);
        }
        None => {
            document.remove("model_catalog_json");
        }
    }
    Ok(render_document(document, current))
}

pub(crate) fn model_catalog_projection_semantic(
    document: &str,
) -> Result<Option<String>, ConfigError> {
    let document = document.parse::<DocumentMut>()?;
    Ok(document.get("model_catalog_json").map(canonical_item))
}

pub(crate) fn capture_orchestration_baseline(
    existing: &str,
) -> Result<OrchestrationBaseline, ConfigError> {
    let document = existing.parse::<DocumentMut>()?;
    let default_permissions = optional_string(&document, "default_permissions")?;
    let sandbox_mode = optional_string(&document, "sandbox_mode")?;
    if default_permissions.is_some() && sandbox_mode.is_some() {
        return Err(ConfigError::InvalidStructure(
            "default_permissions + sandbox_mode",
        ));
    }
    let permission_style =
        if sandbox_mode.is_some() || document.contains_key("sandbox_workspace_write") {
            PermissionStyle::SandboxMode
        } else {
            PermissionStyle::DefaultPermissions
        };
    let agents_enabled = document
        .get("agents")
        .map(|item| {
            item.as_table()
                .ok_or(ConfigError::InvalidStructure("agents"))?
                .get("enabled")
                .map(|enabled| {
                    enabled
                        .as_value()
                        .and_then(TomlValue::as_bool)
                        .ok_or(ConfigError::InvalidStructure("agents.enabled"))
                })
                .transpose()
        })
        .transpose()?
        .flatten();
    Ok(OrchestrationBaseline {
        permission_style,
        default_permissions,
        sandbox_mode,
        agents_enabled,
        global_instructions_path: None,
        global_instructions_existed: false,
        global_instructions_content: None,
    })
}

pub(crate) fn capture_project_exclusion_baseline(
    existing: &str,
    projection_style: PermissionStyle,
) -> Result<ProjectExclusionBaseline, ConfigError> {
    let document = existing.parse::<DocumentMut>()?;
    let default_permissions = optional_string(&document, "default_permissions")?;
    let sandbox_mode = optional_string(&document, "sandbox_mode")?;
    if default_permissions.is_some() && sandbox_mode.is_some() {
        return Err(ConfigError::InvalidStructure(
            "default_permissions + sandbox_mode",
        ));
    }
    let agents_enabled = document
        .get("agents")
        .map(|item| {
            item.as_table()
                .ok_or(ConfigError::InvalidStructure("agents"))?
                .get("enabled")
                .map(|enabled| {
                    enabled
                        .as_value()
                        .and_then(TomlValue::as_bool)
                        .ok_or(ConfigError::InvalidStructure("agents.enabled"))
                })
                .transpose()
        })
        .transpose()?
        .flatten();
    Ok(ProjectExclusionBaseline {
        projection_style,
        default_permissions,
        sandbox_mode,
        agents_enabled,
    })
}

pub(crate) fn upsert_project_exclusion_projection(
    existing: &str,
    permission_style: PermissionStyle,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    match permission_style {
        PermissionStyle::DefaultPermissions => {
            document.remove("sandbox_mode");
            document["default_permissions"] = value(":workspace");
        }
        PermissionStyle::SandboxMode => {
            document.remove("default_permissions");
            document["sandbox_mode"] = value("workspace-write");
        }
    }
    ensure_agents_table(&mut document)?;
    document["agents"]["enabled"] = value(false);
    Ok(render_document(document, existing))
}

pub(crate) fn project_exclusion_projection_matches(
    existing: &str,
    baseline: &ProjectExclusionBaseline,
) -> Result<bool, ConfigError> {
    let document = existing.parse::<DocumentMut>()?;
    let permissions_match = match baseline.projection_style {
        PermissionStyle::DefaultPermissions => {
            optional_string(&document, "default_permissions")?.as_deref() == Some(":workspace")
                && !document.contains_key("sandbox_mode")
        }
        PermissionStyle::SandboxMode => {
            optional_string(&document, "sandbox_mode")?.as_deref() == Some("workspace-write")
                && !document.contains_key("default_permissions")
        }
    };
    Ok(permissions_match
        && document
            .get("agents")
            .and_then(Item::as_table)
            .and_then(|agents| agents.get("enabled"))
            .and_then(Item::as_value)
            .and_then(TomlValue::as_bool)
            == Some(false))
}

pub(crate) fn restore_project_exclusion_projection(
    current: &str,
    baseline: &ProjectExclusionBaseline,
) -> Result<String, ConfigError> {
    let mut document = current.parse::<DocumentMut>()?;
    restore_optional_string(
        &mut document,
        "default_permissions",
        baseline.default_permissions.as_deref(),
    );
    restore_optional_string(
        &mut document,
        "sandbox_mode",
        baseline.sandbox_mode.as_deref(),
    );
    restore_agents_enabled(&mut document, baseline.agents_enabled)?;
    Ok(render_document(document, current))
}

pub(crate) fn upsert_orchestration_projection(
    existing: &str,
    instructions: &str,
    baseline: &OrchestrationBaseline,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    let current_instructions =
        optional_string(&document, "developer_instructions")?.unwrap_or_default();
    let unmanaged = remove_orchestration_block(&current_instructions)?;
    let block = format!("{ORCHESTRATION_BEGIN}\n{instructions}\n{ORCHESTRATION_END}");
    let combined = if unmanaged.trim().is_empty() {
        block
    } else {
        format!("{}\n\n{block}", unmanaged.trim_end())
    };
    document["developer_instructions"] = value(combined);
    match baseline.permission_style {
        PermissionStyle::DefaultPermissions => {
            document.remove("sandbox_mode");
            document["default_permissions"] = value(":read-only");
        }
        PermissionStyle::SandboxMode => {
            document.remove("default_permissions");
            document["sandbox_mode"] = value("read-only");
        }
    }
    ensure_agents_table(&mut document)?;
    document["agents"]["enabled"] = value(true);
    Ok(render_document(document, existing))
}

pub(crate) fn remove_orchestration_projection(
    existing: &str,
    baseline: &OrchestrationBaseline,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    let current_instructions =
        optional_string(&document, "developer_instructions")?.unwrap_or_default();
    let restored_instructions = remove_orchestration_block(&current_instructions)?;
    if restored_instructions.trim().is_empty() {
        document.remove("developer_instructions");
    } else {
        document["developer_instructions"] = value(restored_instructions.trim_end());
    }
    match baseline.permission_style {
        PermissionStyle::DefaultPermissions => {
            document.remove("sandbox_mode");
            restore_optional_string(
                &mut document,
                "default_permissions",
                baseline.default_permissions.as_deref(),
            );
        }
        PermissionStyle::SandboxMode => {
            document.remove("default_permissions");
            restore_optional_string(
                &mut document,
                "sandbox_mode",
                baseline.sandbox_mode.as_deref(),
            );
        }
    }
    restore_agents_enabled(&mut document, baseline.agents_enabled)?;
    Ok(render_document(document, existing))
}

pub(crate) fn restore_orchestration_projection(
    current: &str,
    snapshot: &str,
) -> Result<String, ConfigError> {
    let snapshot_document = snapshot.parse::<DocumentMut>()?;
    let mut document = current.parse::<DocumentMut>()?;
    let current_instructions =
        optional_string(&document, "developer_instructions")?.unwrap_or_default();
    let unmanaged = remove_orchestration_block(&current_instructions)?;
    let snapshot_instructions =
        optional_string(&snapshot_document, "developer_instructions")?.unwrap_or_default();
    let restored = match orchestration_block(&snapshot_instructions)? {
        Some(block) if unmanaged.trim().is_empty() => block,
        Some(block) => format!("{}\n\n{block}", unmanaged.trim_end()),
        None => unmanaged,
    };
    if restored.trim().is_empty() {
        document.remove("developer_instructions");
    } else {
        document["developer_instructions"] = value(restored.trim_end());
    }
    for key in ["default_permissions", "sandbox_mode"] {
        match snapshot_document.get(key).cloned() {
            Some(item) => {
                document.insert(key, item);
            }
            None => {
                document.remove(key);
            }
        }
    }
    let snapshot_agents_enabled = snapshot_document
        .get("agents")
        .and_then(Item::as_table)
        .and_then(|agents| agents.get("enabled"))
        .and_then(Item::as_value)
        .and_then(TomlValue::as_bool);
    restore_agents_enabled(&mut document, snapshot_agents_enabled)?;
    Ok(render_document(document, current))
}

pub(crate) fn orchestration_projection_semantic(
    document: &str,
) -> Result<Option<String>, ConfigError> {
    let document = document.parse::<DocumentMut>()?;
    let instructions = optional_string(&document, "developer_instructions")?.unwrap_or_default();
    let Some(block) = orchestration_block(&instructions)? else {
        return Ok(None);
    };
    let semantic = serde_json::json!({
        "block": block,
        "defaultPermissions": optional_string(&document, "default_permissions")?,
        "sandboxMode": optional_string(&document, "sandbox_mode")?,
        "agentsEnabled": document
            .get("agents")
            .and_then(Item::as_table)
            .and_then(|agents| agents.get("enabled"))
            .and_then(Item::as_value)
            .and_then(TomlValue::as_bool),
    });
    Ok(Some(
        serde_json::to_string(&semantic).expect("JSON value must serialize"),
    ))
}

pub(crate) fn upsert_global_orchestration_projection(
    existing: &str,
    instructions: &str,
) -> Result<String, ConfigError> {
    let unmanaged = remove_global_orchestration_projection(existing, None)?;
    let separator = if unmanaged.is_empty() || unmanaged.ends_with("\n\n") {
        ""
    } else if unmanaged.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(format!(
        "{unmanaged}{separator}{GLOBAL_ORCHESTRATION_BEGIN}\n{instructions}\n{GLOBAL_ORCHESTRATION_END}\n"
    ))
}

pub(crate) fn remove_global_orchestration_projection(
    existing: &str,
    original: Option<&str>,
) -> Result<String, ConfigError> {
    let Some((start, end)) = marked_block_range(
        existing,
        GLOBAL_ORCHESTRATION_BEGIN,
        GLOBAL_ORCHESTRATION_END,
        "AGENTS.md",
    )?
    else {
        return Ok(existing.to_owned());
    };
    if end == existing.len()
        && let Some(original) = original
    {
        let separator = if original.is_empty() || original.ends_with("\n\n") {
            ""
        } else if original.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        if existing[..start] == format!("{original}{separator}") {
            return Ok(original.to_owned());
        }
    }
    Ok(format!("{}{}", &existing[..start], &existing[end..]))
}

pub(crate) fn global_orchestration_projection_semantic(
    document: &str,
) -> Result<Option<String>, ConfigError> {
    Ok(marked_block_range(
        document,
        GLOBAL_ORCHESTRATION_BEGIN,
        GLOBAL_ORCHESTRATION_END,
        "AGENTS.md",
    )?
    .map(|(start, end)| {
        document[start..end]
            .replace("\r\n", "\n")
            .replace('\r', "\n")
    }))
}

fn optional_string(
    document: &DocumentMut,
    key: &'static str,
) -> Result<Option<String>, ConfigError> {
    document
        .get(key)
        .map(|item| {
            item.as_value()
                .and_then(TomlValue::as_str)
                .map(str::to_owned)
                .ok_or(ConfigError::InvalidStructure(key))
        })
        .transpose()
}

fn restore_optional_string(document: &mut DocumentMut, key: &str, original: Option<&str>) {
    match original {
        Some(original) => document[key] = value(original),
        None => {
            document.remove(key);
        }
    }
}

fn ensure_agents_table(document: &mut DocumentMut) -> Result<(), ConfigError> {
    if !document.contains_key("agents") {
        document["agents"] = Item::Table(Table::new());
    }
    document["agents"]
        .as_table()
        .ok_or(ConfigError::InvalidStructure("agents"))?;
    Ok(())
}

fn restore_agents_enabled(
    document: &mut DocumentMut,
    original: Option<bool>,
) -> Result<(), ConfigError> {
    match original {
        Some(original) => {
            ensure_agents_table(document)?;
            document["agents"]["enabled"] = value(original);
        }
        None => {
            if let Some(agents) = document.get_mut("agents") {
                let agents = agents
                    .as_table_mut()
                    .ok_or(ConfigError::InvalidStructure("agents"))?;
                agents.remove("enabled");
                if agents.is_empty() {
                    document.remove("agents");
                }
            }
        }
    }
    Ok(())
}

fn orchestration_block(instructions: &str) -> Result<Option<String>, ConfigError> {
    Ok(marked_block_range(
        instructions,
        ORCHESTRATION_BEGIN,
        ORCHESTRATION_END,
        "developer_instructions",
    )?
    .map(|(start, end)| instructions[start..end].to_owned()))
}

fn remove_orchestration_block(instructions: &str) -> Result<String, ConfigError> {
    let Some(block) = orchestration_block(instructions)? else {
        return Ok(instructions.to_owned());
    };
    Ok(instructions.replacen(&block, "", 1).trim_end().to_owned())
}

fn marked_block_range(
    content: &str,
    begin: &str,
    end: &str,
    field: &'static str,
) -> Result<Option<(usize, usize)>, ConfigError> {
    let Some(start) = content.find(begin) else {
        if content.contains(end) {
            return Err(ConfigError::InvalidStructure(field));
        }
        return Ok(None);
    };
    let tail = &content[start..];
    let Some(end_offset) = tail.find(end) else {
        return Err(ConfigError::InvalidStructure(field));
    };
    let block_end = start + end_offset + end.len();
    let block_end = if content[block_end..].starts_with("\r\n") {
        block_end + 2
    } else if content[block_end..].starts_with('\n') {
        block_end + 1
    } else {
        block_end
    };
    if content[block_end..].contains(begin) {
        return Err(ConfigError::InvalidStructure(field));
    }
    Ok(Some((start, block_end)))
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

    #[test]
    fn orchestration_restores_legacy_permission_style_and_user_instructions() {
        let existing = r#"sandbox_mode = "workspace-write"
developer_instructions = "用户规则"

[agents]
enabled = false
max_threads = 6
"#;
        let baseline = capture_orchestration_baseline(existing).unwrap();
        let active = upsert_orchestration_projection(existing, "必须委派写入。", &baseline)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(active["sandbox_mode"].as_str(), Some("read-only"));
        assert!(active.get("default_permissions").is_none());
        assert_eq!(active["agents"]["enabled"].as_bool(), Some(true));
        assert_eq!(active["agents"]["max_threads"].as_integer(), Some(6));

        let restored = remove_orchestration_projection(&active.to_string(), &baseline)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(restored["sandbox_mode"].as_str(), Some("workspace-write"));
        assert_eq!(
            restored["developer_instructions"].as_str(),
            Some("用户规则")
        );
        assert_eq!(restored["agents"]["enabled"].as_bool(), Some(false));
        assert_eq!(restored["agents"]["max_threads"].as_integer(), Some(6));
    }

    #[test]
    fn global_orchestration_preserves_and_restores_user_instructions() {
        let existing = "# 用户规则\n\n保留这段内容。\n";
        let active = upsert_global_orchestration_projection(existing, "必须委派写入。").unwrap();

        assert!(
            global_orchestration_projection_semantic(&active)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            remove_global_orchestration_projection(&active, Some(existing)).unwrap(),
            existing
        );
        assert_eq!(
            global_orchestration_projection_semantic(&active).unwrap(),
            global_orchestration_projection_semantic(&active.replace('\n', "\r\n")).unwrap()
        );
    }

    #[test]
    fn project_exclusion_preserves_user_config_and_restores_owned_fields() {
        let existing = r#"# 用户注释
default_permissions = ":read-only"

[mcp_servers.example]
command = "keep-me"

[agents]
enabled = true
max_threads = 6
"#;
        let baseline =
            capture_project_exclusion_baseline(existing, PermissionStyle::DefaultPermissions)
                .unwrap();
        let active =
            upsert_project_exclusion_projection(existing, PermissionStyle::DefaultPermissions)
                .unwrap();
        let active_document = active.parse::<DocumentMut>().unwrap();
        assert!(active.contains("# 用户注释"));
        assert_eq!(
            active_document["default_permissions"].as_str(),
            Some(":workspace")
        );
        assert_eq!(active_document["agents"]["enabled"].as_bool(), Some(false));
        assert!(project_exclusion_projection_matches(&active, &baseline).unwrap());

        let later = active.replace("keep-me", "changed-later");
        let restored = restore_project_exclusion_projection(&later, &baseline).unwrap();
        let restored_document = restored.parse::<DocumentMut>().unwrap();
        assert_eq!(
            restored_document["default_permissions"].as_str(),
            Some(":read-only")
        );
        assert_eq!(restored_document["agents"]["enabled"].as_bool(), Some(true));
        assert_eq!(
            restored_document["agents"]["max_threads"].as_integer(),
            Some(6)
        );
        assert_eq!(
            restored_document["mcp_servers"]["example"]["command"].as_str(),
            Some("changed-later")
        );
    }

    #[test]
    fn project_exclusion_uses_legacy_sandbox_permission_style_when_required() {
        let existing = "sandbox_mode = 'read-only'\n";
        let baseline =
            capture_project_exclusion_baseline(existing, PermissionStyle::SandboxMode).unwrap();
        let active =
            upsert_project_exclusion_projection(existing, PermissionStyle::SandboxMode).unwrap();
        let active_document = active.parse::<DocumentMut>().unwrap();
        assert!(active_document.get("default_permissions").is_none());
        assert_eq!(
            active_document["sandbox_mode"].as_str(),
            Some("workspace-write")
        );
        assert!(project_exclusion_projection_matches(&active, &baseline).unwrap());
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
