use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, Value as TomlValue, value};

pub(crate) use cas_scheduler::render_delegated_agent_instructions_for_phase;

use crate::agent::AgentMcpToolPolicy;
use crate::domain::{Agent, BaseBinding, Model, ResponsesProvider};

const AUTH_TIMEOUT_MS: i64 = 5_000;
const AUTH_REFRESH_INTERVAL_MS: i64 = 300_000;
const ORCHESTRATION_BEGIN: &str = "<<< CAS ORCHESTRATION v1 >>>";
const ORCHESTRATION_END: &str = "<<< END CAS ORCHESTRATION v1 >>>";
const RUNTIME_HOOK_MARKER: &str = "cas-runtime-enforcement-v1";
const RUNTIME_HOOK_EVENTS: [(&str, &str); 4] = [
    ("SubagentStart", ".*"),
    ("SubagentStop", ".*"),
    (
        "PreToolUse",
        "^(Bash|shell_command|exec_command|apply_patch|Edit|Write|spawn_agent|Agent|followup_task|send_input)$",
    ),
    (
        "PostToolUse",
        "^(spawn_agent|Agent|followup_task|send_input)$",
    ),
];
const GLOBAL_ORCHESTRATION_BEGIN: &str = "<!-- CAS ORCHESTRATION v1 BEGIN -->";
const GLOBAL_ORCHESTRATION_END: &str = "<!-- CAS ORCHESTRATION v1 END -->";
pub(crate) const ORCHESTRATION_RUNTIME_CONTRACT: &str =
    "CAS_RUNTIME_CONTRACT=V1_PLAINTEXT_WORKSPACE";
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
    pub(crate) multi_agent_v2_enabled: Option<bool>,
    #[serde(default)]
    pub(crate) multi_agent_v2_captured: bool,
    #[serde(default)]
    pub(crate) global_instructions_path: Option<String>,
    #[serde(default)]
    pub(crate) global_instructions_existed: bool,
    #[serde(default)]
    pub(crate) global_instructions_content: Option<String>,
    #[serde(default)]
    pub(crate) model_providers_existed: Option<bool>,
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
    pub(crate) provider_id: Option<&'a str>,
    pub(crate) reasoning_effort: Option<&'a str>,
    pub(crate) sandbox_mode: Option<&'a str>,
    pub(crate) developer_instructions: &'a str,
    pub(crate) orchestration_phase: Option<&'a str>,
    pub(crate) model_catalog_path: Option<&'a Path>,
    pub(crate) skill_keys: &'a [String],
    pub(crate) skill_paths: &'a [PathBuf],
    pub(crate) disabled_mcp_server_ids: &'a [String],
    pub(crate) mcp_tool_policies: &'a [AgentMcpToolPolicy],
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
    remove_empty_parent: bool,
) -> Result<String, ConfigError> {
    let mut document = existing.parse::<DocumentMut>()?;
    if document.contains_key("model_providers") {
        let providers = document["model_providers"]
            .as_table_mut()
            .ok_or(ConfigError::InvalidStructure("model_providers"))?;
        providers.remove(provider_id);
        if remove_empty_parent && providers.is_empty() {
            document.remove("model_providers");
        }
    }
    Ok(render_document(document, existing))
}

pub(crate) fn restore_provider_projection(
    current: &str,
    snapshot: &str,
    provider_id: &str,
) -> Result<String, ConfigError> {
    let snapshot = snapshot.parse::<DocumentMut>()?;
    let parent_existed = snapshot.contains_key("model_providers");
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
        if !parent_existed && providers.is_empty() {
            document.remove("model_providers");
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
    let multi_agent_v2_enabled = optional_multi_agent_v2(&document)?;
    Ok(OrchestrationBaseline {
        permission_style,
        default_permissions,
        sandbox_mode,
        agents_enabled,
        multi_agent_v2_enabled,
        multi_agent_v2_captured: true,
        global_instructions_path: None,
        global_instructions_existed: false,
        global_instructions_content: None,
        model_providers_existed: Some(document.contains_key("model_providers")),
    })
}

pub(crate) fn upgrade_orchestration_baseline(
    existing: &str,
    baseline: &mut OrchestrationBaseline,
) -> Result<bool, ConfigError> {
    if baseline.multi_agent_v2_captured {
        return Ok(false);
    }
    let document = existing.parse::<DocumentMut>()?;
    baseline.multi_agent_v2_enabled = optional_multi_agent_v2(&document)?;
    baseline.multi_agent_v2_captured = true;
    Ok(true)
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

#[cfg(test)]
pub(crate) fn upsert_orchestration_projection(
    existing: &str,
    instructions: &str,
    baseline: &OrchestrationBaseline,
) -> Result<String, ConfigError> {
    upsert_orchestration_projection_with_hooks(existing, instructions, baseline, None)
}

pub(crate) fn upsert_orchestration_projection_with_hooks(
    existing: &str,
    instructions: &str,
    baseline: &OrchestrationBaseline,
    runtime_hook_command: Option<&str>,
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
            document["default_permissions"] = value(":workspace");
        }
        PermissionStyle::SandboxMode => {
            document.remove("default_permissions");
            document["sandbox_mode"] = value("workspace-write");
        }
    }
    ensure_agents_table(&mut document)?;
    document["agents"]["enabled"] = value(true);
    set_multi_agent_v2(&mut document, false)?;
    remove_runtime_hooks(&mut document)?;
    if let Some(command) = runtime_hook_command {
        append_runtime_hooks(&mut document, command)?;
    }
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
    restore_multi_agent_v2(&mut document, baseline.multi_agent_v2_enabled)?;
    remove_runtime_hooks(&mut document)?;
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
    restore_multi_agent_v2(&mut document, optional_multi_agent_v2(&snapshot_document)?)?;
    remove_runtime_hooks(&mut document)?;
    restore_runtime_hooks(&mut document, &snapshot_document)?;
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
    let mut semantic = serde_json::json!({
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
    if block.contains(ORCHESTRATION_RUNTIME_CONTRACT) {
        semantic["multiAgentV2"] = optional_multi_agent_v2(&document)?.into();
        semantic["runtimeHooks"] = runtime_hooks_semantic(&document).into();
    }
    Ok(Some(
        serde_json::to_string(&semantic).expect("JSON value must serialize"),
    ))
}

fn append_runtime_hooks(document: &mut DocumentMut, command: &str) -> Result<(), ConfigError> {
    if !document.contains_key("hooks") {
        document["hooks"] = Item::Table(Table::new());
    }
    let hooks = document["hooks"]
        .as_table_mut()
        .ok_or(ConfigError::InvalidStructure("hooks"))?;
    for (event, matcher) in RUNTIME_HOOK_EVENTS {
        if !hooks.contains_key(event) {
            hooks[event] = Item::ArrayOfTables(ArrayOfTables::new());
        }
        let entries = hooks[event]
            .as_array_of_tables_mut()
            .ok_or(ConfigError::InvalidStructure("hooks event"))?;
        let mut entry = Table::new();
        entry["matcher"] = value(matcher);
        let mut handlers = ArrayOfTables::new();
        let mut handler = Table::new();
        handler["type"] = value("command");
        handler["command"] = value(command);
        handler["command_windows"] = value(command);
        handlers.push(handler);
        entry["hooks"] = Item::ArrayOfTables(handlers);
        entries.push(entry);
    }
    Ok(())
}

fn remove_runtime_hooks(document: &mut DocumentMut) -> Result<(), ConfigError> {
    let Some(hooks) = document.get_mut("hooks") else {
        return Ok(());
    };
    let hooks = hooks
        .as_table_mut()
        .ok_or(ConfigError::InvalidStructure("hooks"))?;
    for (event, _) in RUNTIME_HOOK_EVENTS {
        let remove_event = if let Some(item) = hooks.get_mut(event) {
            let entries = item
                .as_array_of_tables_mut()
                .ok_or(ConfigError::InvalidStructure("hooks event"))?;
            for index in (0..entries.len()).rev() {
                if entries.get(index).is_some_and(runtime_hook_entry_is_owned) {
                    entries.remove(index);
                }
            }
            entries.is_empty()
        } else {
            false
        };
        if remove_event {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        document.remove("hooks");
    }
    Ok(())
}

fn restore_runtime_hooks(
    document: &mut DocumentMut,
    snapshot: &DocumentMut,
) -> Result<(), ConfigError> {
    let Some(snapshot_hooks) = snapshot.get("hooks") else {
        return Ok(());
    };
    let snapshot_hooks = snapshot_hooks
        .as_table()
        .ok_or(ConfigError::InvalidStructure("hooks"))?;
    for (event, _) in RUNTIME_HOOK_EVENTS {
        let Some(entries) = snapshot_hooks.get(event) else {
            continue;
        };
        let entries = entries
            .as_array_of_tables()
            .ok_or(ConfigError::InvalidStructure("hooks event"))?;
        for entry in entries
            .iter()
            .filter(|entry| runtime_hook_entry_is_owned(entry))
        {
            if !document.contains_key("hooks") {
                document["hooks"] = Item::Table(Table::new());
            }
            let hooks = document["hooks"]
                .as_table_mut()
                .ok_or(ConfigError::InvalidStructure("hooks"))?;
            if !hooks.contains_key(event) {
                hooks[event] = Item::ArrayOfTables(ArrayOfTables::new());
            }
            hooks[event]
                .as_array_of_tables_mut()
                .ok_or(ConfigError::InvalidStructure("hooks event"))?
                .push(entry.clone());
        }
    }
    Ok(())
}

fn runtime_hook_entry_is_owned(entry: &Table) -> bool {
    entry
        .get("hooks")
        .and_then(Item::as_array_of_tables)
        .is_some_and(|handlers| {
            handlers.iter().any(|handler| {
                ["command", "command_windows"].iter().any(|key| {
                    handler
                        .get(key)
                        .and_then(Item::as_value)
                        .and_then(TomlValue::as_str)
                        .is_some_and(|command| command.contains(RUNTIME_HOOK_MARKER))
                })
            })
        })
}

fn runtime_hooks_semantic(document: &DocumentMut) -> String {
    let Some(hooks) = document.get("hooks").and_then(Item::as_table) else {
        return "[]".to_owned();
    };
    let mut entries = Vec::new();
    for (event, _) in RUNTIME_HOOK_EVENTS {
        if let Some(event_entries) = hooks.get(event).and_then(Item::as_array_of_tables) {
            entries.extend(
                event_entries
                    .iter()
                    .filter(|entry| runtime_hook_entry_is_owned(entry))
                    .map(|entry| format!("{}:{}", json_string(event), canonical_table(entry))),
            );
        }
    }
    format!("[{}]", entries.join(","))
}

#[cfg(test)]
pub(crate) fn upsert_global_orchestration_projection(
    existing: &str,
    instructions: &str,
) -> Result<String, ConfigError> {
    let unmanaged = remove_global_orchestration_projection(existing, None)?;
    let unmanaged = unmanaged.trim_end_matches(['\r', '\n']);
    let separator = if unmanaged.is_empty() { "" } else { "\n\n" };
    Ok(format!(
        "{unmanaged}{separator}{GLOBAL_ORCHESTRATION_BEGIN}\n{instructions}\n{GLOBAL_ORCHESTRATION_END}\n"
    ))
}

pub(crate) fn remove_global_orchestration_projection(
    existing: &str,
    original: Option<&str>,
) -> Result<String, ConfigError> {
    let unmanaged = remove_marked_blocks(
        existing,
        GLOBAL_ORCHESTRATION_BEGIN,
        GLOBAL_ORCHESTRATION_END,
        "AGENTS.md",
    )?;
    if let Some(original) = original
        && unmanaged.trim_end() == original.trim_end()
    {
        return Ok(original.to_owned());
    }
    Ok(unmanaged)
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

fn optional_multi_agent_v2(document: &DocumentMut) -> Result<Option<bool>, ConfigError> {
    document
        .get("features")
        .map(|item| {
            item.as_table()
                .ok_or(ConfigError::InvalidStructure("features"))?
                .get("multi_agent_v2")
                .map(|enabled| {
                    enabled
                        .as_value()
                        .and_then(TomlValue::as_bool)
                        .ok_or(ConfigError::InvalidStructure("features.multi_agent_v2"))
                })
                .transpose()
        })
        .transpose()
        .map(Option::flatten)
}

fn ensure_features_table(document: &mut DocumentMut) -> Result<(), ConfigError> {
    if !document.contains_key("features") {
        document["features"] = Item::Table(Table::new());
    }
    document["features"]
        .as_table()
        .ok_or(ConfigError::InvalidStructure("features"))?;
    Ok(())
}

fn set_multi_agent_v2(document: &mut DocumentMut, enabled: bool) -> Result<(), ConfigError> {
    ensure_features_table(document)?;
    document["features"]["multi_agent_v2"] = value(enabled);
    Ok(())
}

fn restore_multi_agent_v2(
    document: &mut DocumentMut,
    original: Option<bool>,
) -> Result<(), ConfigError> {
    match original {
        Some(original) => set_multi_agent_v2(document, original),
        None => {
            if let Some(features) = document.get_mut("features") {
                let features = features
                    .as_table_mut()
                    .ok_or(ConfigError::InvalidStructure("features"))?;
                features.remove("multi_agent_v2");
                if features.is_empty() {
                    document.remove("features");
                }
            }
            Ok(())
        }
    }
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
    Ok(remove_marked_blocks(
        instructions,
        ORCHESTRATION_BEGIN,
        ORCHESTRATION_END,
        "developer_instructions",
    )?
    .trim_end()
    .to_owned())
}

fn remove_marked_blocks(
    content: &str,
    begin: &str,
    end: &str,
    field: &'static str,
) -> Result<String, ConfigError> {
    let mut unmanaged = content.to_owned();
    while let Some((start, block_end)) = marked_block_range(&unmanaged, begin, end, field)? {
        unmanaged.replace_range(start..block_end, "");
    }
    Ok(unmanaged)
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
    if content.find(end).is_some_and(|end| end < start) {
        return Err(ConfigError::InvalidStructure(field));
    }
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
    Ok(Some((start, block_end)))
}

pub(crate) fn render_agent_projection(agent: &AgentProjection<'_>) -> Result<String, ConfigError> {
    let mut document = DocumentMut::new();
    document["name"] = value(agent.agent_key);
    document["description"] = value(agent.description);
    document["model"] = value(agent.model_id);
    if let Some(provider_id) = agent.provider_id {
        document["model_provider"] = value(provider_id);
    }
    if let Some(reasoning_effort) = agent.reasoning_effort {
        document["model_reasoning_effort"] = value(reasoning_effort);
    }
    if let Some(sandbox_mode) = agent.sandbox_mode {
        document["sandbox_mode"] = value(sandbox_mode);
    }
    let mut developer_instructions = render_delegated_agent_instructions_for_phase(
        agent.developer_instructions,
        agent.orchestration_phase,
    );
    if agent.skill_keys.iter().any(|key| key == "caveman") {
        developer_instructions.push_str(
            "\n\n必须使用 caveman full 压缩进度与最终汇报；安全警告、不可逆操作确认和验收证据保持完整清晰。",
        );
    }
    if agent.skill_keys.iter().any(|key| key == "ponytail") {
        developer_instructions.push_str(
            "\n必须使用 ponytail full 处理编码、修复和技术设计；不得削弱输入校验、数据安全、无障碍或明确验收要求。",
        );
    }
    if agent.skill_keys.iter().any(|key| key == "caveman-slim") {
        developer_instructions.push_str(
            "\n\n必须使用 caveman-slim 压缩进度与最终汇报；保留技术事实、验收证据和安全信息。",
        );
    }
    if agent.skill_keys.iter().any(|key| key == "ponytail-slim") {
        developer_instructions.push_str(
            "\n必须使用 ponytail-slim 完成任务所需的最小改动；不得扩展范围或削弱明确验收要求。",
        );
    }
    document["developer_instructions"] = value(developer_instructions);
    if let Some(path) = agent.model_catalog_path {
        document["model_catalog_json"] = value(path.to_string_lossy().into_owned());
    }
    if !agent.skill_paths.is_empty() {
        let mut configs = ArrayOfTables::new();
        for path in agent.skill_paths {
            let mut config = Table::new();
            config["path"] = value(path.to_string_lossy().into_owned());
            config["enabled"] = value(true);
            configs.push(config);
        }
        let mut skills = Table::new();
        skills["config"] = Item::ArrayOfTables(configs);
        document["skills"] = Item::Table(skills);
    }
    if !agent.disabled_mcp_server_ids.is_empty() || !agent.mcp_tool_policies.is_empty() {
        let mut servers = Table::new();
        for policy in agent.mcp_tool_policies {
            let mut server = Table::new();
            let mut tool_names = Array::new();
            for tool_name in &policy.tool_names {
                tool_names.push(tool_name.as_str());
            }
            let key = match policy.mode.as_str() {
                "ALLOW_ONLY" => "enabled_tools",
                "DENY" => "disabled_tools",
                _ => return Err(ConfigError::InvalidStructure("mcp tool policy mode")),
            };
            server[key] = Item::Value(TomlValue::Array(tool_names));
            servers.insert(&policy.server_id, Item::Table(server));
        }
        for server_id in agent.disabled_mcp_server_ids {
            let mut server = Table::new();
            server["enabled"] = value(false);
            servers.insert(server_id, Item::Table(server));
        }
        document["mcp_servers"] = Item::Table(servers);
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
    fn provider_cleanup_restores_parent_table_ownership_exactly() {
        let projection = ProviderProjection {
            provider_id: "cas_example",
            display_name: "Example",
            base_url: "https://example.com/",
            helper_command: "cas-helper",
            credential_id: "credential",
        };
        let without_parent = "user_setting = 'keep-me'\n";
        let active = upsert_provider_projection(without_parent, &projection).unwrap();
        assert_eq!(
            remove_provider_projection(&active, projection.provider_id, true).unwrap(),
            without_parent
        );

        let with_empty_parent = "user_setting = 'keep-me'\n\n[model_providers]\n";
        let active = upsert_provider_projection(with_empty_parent, &projection).unwrap();
        assert_eq!(
            remove_provider_projection(&active, projection.provider_id, false).unwrap(),
            with_empty_parent
        );
    }

    #[test]
    fn legacy_orchestration_baseline_keeps_unknown_provider_parent_state() {
        let baseline: OrchestrationBaseline = serde_json::from_str(
            r#"{
                "permissionStyle": "DEFAULT_PERMISSIONS",
                "defaultPermissions": ":workspace",
                "sandboxMode": null,
                "agentsEnabled": null
            }"#,
        )
        .unwrap();

        assert_eq!(baseline.model_providers_existed, None);
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

[features]
multi_agent_v2 = true
"#;
        let baseline = capture_orchestration_baseline(existing).unwrap();
        let instructions = format!("{ORCHESTRATION_RUNTIME_CONTRACT}\n必须委派写入。");
        let active = upsert_orchestration_projection(existing, &instructions, &baseline)
            .unwrap()
            .parse::<DocumentMut>()
            .unwrap();
        assert_eq!(active["sandbox_mode"].as_str(), Some("workspace-write"));
        assert!(active.get("default_permissions").is_none());
        assert_eq!(active["agents"]["enabled"].as_bool(), Some(true));
        assert_eq!(active["agents"]["max_threads"].as_integer(), Some(6));
        assert_eq!(active["features"]["multi_agent_v2"].as_bool(), Some(false));
        let active_semantic = orchestration_projection_semantic(&active.to_string()).unwrap();
        assert!(
            active_semantic
                .as_deref()
                .is_some_and(|value| value.contains("\"multiAgentV2\":false"))
        );

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
        assert_eq!(restored["features"]["multi_agent_v2"].as_bool(), Some(true));
    }

    #[test]
    fn runtime_hooks_preserve_user_entries_and_default_removes_only_cas_owned_entries() {
        let existing = r#"default_permissions = ":workspace"

[[hooks.PreToolUse]]
matcher = "^MCP$"

[[hooks.PreToolUse.hooks]]
type = "command"
command = "user-hook --check"
"#;
        let baseline = capture_orchestration_baseline(existing).unwrap();
        let instructions = format!("{ORCHESTRATION_RUNTIME_CONTRACT}\n必须委派写入。");
        let active = upsert_orchestration_projection_with_hooks(
            existing,
            &instructions,
            &baseline,
            Some(r#""C:\Program Files\CAS\cas-helper.exe" hook "C:\CAS\cas.db" cas-runtime-enforcement-v1"#),
        )
        .unwrap();
        assert!(active.contains("user-hook --check"));
        assert_eq!(active.matches(RUNTIME_HOOK_MARKER).count(), 8);
        assert!(active.contains("spawn_agent"));
        assert!(active.contains("followup_task"));
        assert!(active.contains("send_input"));
        assert!(
            orchestration_projection_semantic(&active)
                .unwrap()
                .is_some_and(|semantic| semantic.contains("runtimeHooks"))
        );

        let restored = remove_orchestration_projection(&active, &baseline).unwrap();
        assert!(restored.contains("user-hook --check"));
        assert!(!restored.contains(RUNTIME_HOOK_MARKER));
    }

    #[test]
    fn orchestration_snapshot_restore_reinstates_only_snapshot_cas_hooks() {
        let baseline = capture_orchestration_baseline("").unwrap();
        let instructions = format!("{ORCHESTRATION_RUNTIME_CONTRACT}\n必须委派写入。");
        let snapshot = upsert_orchestration_projection_with_hooks(
            "",
            &instructions,
            &baseline,
            Some("old-helper hook old.db cas-runtime-enforcement-v1"),
        )
        .unwrap();
        let current = upsert_orchestration_projection_with_hooks(
            "",
            &instructions,
            &baseline,
            Some("new-helper hook new.db cas-runtime-enforcement-v1"),
        )
        .unwrap();

        let restored = restore_orchestration_projection(&current, &snapshot).unwrap();
        assert!(restored.contains("old-helper hook old.db"));
        assert!(!restored.contains("new-helper hook new.db"));
    }

    #[test]
    fn legacy_orchestration_baseline_captures_multi_agent_v2_once() {
        let mut baseline = OrchestrationBaseline {
            permission_style: PermissionStyle::DefaultPermissions,
            default_permissions: Some(":workspace".to_owned()),
            sandbox_mode: None,
            agents_enabled: None,
            multi_agent_v2_enabled: None,
            multi_agent_v2_captured: false,
            global_instructions_path: None,
            global_instructions_existed: false,
            global_instructions_content: None,
            model_providers_existed: None,
        };

        assert!(
            upgrade_orchestration_baseline("[features]\nmulti_agent_v2 = true\n", &mut baseline)
                .unwrap()
        );
        assert_eq!(baseline.multi_agent_v2_enabled, Some(true));
        assert!(baseline.multi_agent_v2_captured);
        assert!(
            !upgrade_orchestration_baseline("[features]\nmulti_agent_v2 = false\n", &mut baseline)
                .unwrap()
        );
        assert_eq!(baseline.multi_agent_v2_enabled, Some(true));
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
    fn global_orchestration_rebuilds_duplicate_managed_blocks() {
        let existing = format!(
            "# 用户规则\n\n{GLOBAL_ORCHESTRATION_BEGIN}\n旧规则一\n{GLOBAL_ORCHESTRATION_END}\n\n{GLOBAL_ORCHESTRATION_BEGIN}\n旧规则二\n{GLOBAL_ORCHESTRATION_END}\n"
        );

        let active = upsert_global_orchestration_projection(&existing, "最新规则").unwrap();

        assert_eq!(active.matches(GLOBAL_ORCHESTRATION_BEGIN).count(), 1);
        assert_eq!(active.matches(GLOBAL_ORCHESTRATION_END).count(), 1);
        assert!(active.starts_with("# 用户规则"));
        assert!(active.contains("最新规则"));
        assert!(!active.contains("旧规则一"));
        assert!(!active.contains("旧规则二"));
    }

    #[test]
    fn agent_projection_keeps_user_instructions_and_adds_execution_identity() {
        let projection = AgentProjection {
            agent_key: "executor",
            description: "执行实现任务",
            model_id: "model",
            provider_id: Some("cas_provider"),
            reasoning_effort: Some("medium"),
            sandbox_mode: Some("workspace-write"),
            developer_instructions: "保留用户规则。",
            orchestration_phase: Some("EXECUTION"),
            model_catalog_path: None,
            skill_keys: &["caveman".to_owned(), "ponytail".to_owned()],
            skill_paths: &[
                PathBuf::from("C:/codex/cas/bundled-skills/caveman/SKILL.md"),
                PathBuf::from("C:/codex/cas/bundled-skills/ponytail/SKILL.md"),
            ],
            disabled_mcp_server_ids: &["browser".to_owned(), "github.readonly".to_owned()],
            mcp_tool_policies: &[
                AgentMcpToolPolicy {
                    server_id: "filesystem".to_owned(),
                    mode: "ALLOW_ONLY".to_owned(),
                    tool_names: vec!["read_file".to_owned(), "list_directory".to_owned()],
                },
                AgentMcpToolPolicy {
                    server_id: "git".to_owned(),
                    mode: "DENY".to_owned(),
                    tool_names: vec!["force_push".to_owned()],
                },
            ],
        };

        let rendered = render_agent_projection(&projection).unwrap();
        let document = rendered.parse::<DocumentMut>().unwrap();
        let instructions = document["developer_instructions"].as_str().unwrap();

        assert!(instructions.starts_with("保留用户规则。"));
        assert!(instructions.contains("你是由 Primary 委派的 Child Agent，不是 Primary"));
        assert!(instructions.contains("阶段契约：EXECUTION"));
        assert!(instructions.contains("`TOOLS: -` 表示禁用"));
        assert!(instructions.contains("外部写入、消息发送、发布、登录、授权或安装"));
        assert!(instructions.contains("CHANGED"));
        assert!(instructions.contains("不得递归创建同职责子 Agent"));
        assert!(instructions.contains("磁盘事实冲突会改变方向"));
        assert!(instructions.contains("RESULT: NEEDS_DECISION"));
        assert!(instructions.contains("必须使用 caveman full"));
        assert!(instructions.contains("必须使用 ponytail full"));
        assert_eq!(
            document["skills"]["config"]
                .as_array_of_tables()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            document["mcp_servers"]["browser"]["enabled"].as_bool(),
            Some(false)
        );
        assert_eq!(
            document["mcp_servers"]["github.readonly"]["enabled"].as_bool(),
            Some(false)
        );
        assert_eq!(
            document["mcp_servers"]["filesystem"]["enabled_tools"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>(),
            vec!["read_file", "list_directory"]
        );
        assert_eq!(
            document["mcp_servers"]["git"]["disabled_tools"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>(),
            vec!["force_push"]
        );
    }

    #[test]
    fn native_agent_projection_omits_custom_provider() {
        let projection = AgentProjection {
            agent_key: "executor",
            description: "执行实现任务",
            model_id: "gpt-5.6-luna",
            provider_id: None,
            reasoning_effort: Some("high"),
            sandbox_mode: Some("workspace-write"),
            developer_instructions: "执行任务。",
            orchestration_phase: Some("EXECUTION"),
            model_catalog_path: None,
            skill_keys: &[],
            skill_paths: &[],
            disabled_mcp_server_ids: &[],
            mcp_tool_policies: &[],
        };

        let rendered = render_agent_projection(&projection).unwrap();
        let document = rendered.parse::<DocumentMut>().unwrap();

        assert_eq!(document["model"].as_str(), Some("gpt-5.6-luna"));
        assert!(document.get("model_provider").is_none());
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
