use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

const SCHEMA_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum SchemaCapability {
    Supported,
    NotDeclared,
    Incompatible,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SchemaCapabilities {
    pub(crate) usage: SchemaCapability,
    pub(crate) managed_session: SchemaCapability,
    pub(crate) agent_execution: SchemaCapability,
}

/// 通过 `codex app-server generate-json-schema` 探测当前可执行文件的真实能力。
/// 任何启动 / 超时 / 写盘失败都折叠为 `Unavailable`：无法证明能力时调用方必须 Fail Closed。
pub(crate) fn probe_schema_capabilities(executable: &Path, data_home: &Path) -> SchemaCapabilities {
    let probe_root = data_home
        .join("runtime-schema-probes")
        .join(Uuid::new_v4().to_string());
    let Ok(()) = fs::create_dir_all(&probe_root) else {
        return SchemaCapabilities::default();
    };
    let result = (|| {
        let mut child = Command::new(executable)
            .args(["app-server", "generate-json-schema", "--out"])
            .arg(&probe_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let deadline = Instant::now() + SCHEMA_PROBE_TIMEOUT;
        let succeeded = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(_) => return None,
            }
        };
        if !succeeded {
            return Some(SchemaCapabilities::default());
        }
        let usage = if find_schema_file(&probe_root, "ThreadTokenUsageUpdatedNotification.json")
            .is_some()
        {
            SchemaCapability::Supported
        } else {
            SchemaCapability::NotDeclared
        };
        let managed_session = managed_session_schema_capability(&probe_root);
        let agent_execution = agent_execution_schema_capability(&probe_root);
        Some(SchemaCapabilities {
            usage,
            managed_session,
            agent_execution,
        })
    })()
    .unwrap_or_default();
    let _ = fs::remove_dir_all(&probe_root);
    result
}

fn managed_session_schema_capability(probe_root: &Path) -> SchemaCapability {
    let schemas = [
        ("ThreadStartParams.json", &[][..]),
        ("ThreadResumeParams.json", &["threadId"][..]),
        ("TurnStartParams.json", &["input", "threadId"][..]),
    ];
    let mut loaded = Vec::with_capacity(schemas.len());
    for (name, supported_required) in schemas {
        let Some(path) = find_schema_file(probe_root, name) else {
            return SchemaCapability::NotDeclared;
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return SchemaCapability::Incompatible;
        };
        let Ok(schema) = serde_json::from_str::<Value>(&contents) else {
            return SchemaCapability::Incompatible;
        };
        loaded.push((schema, supported_required));
    }
    if loaded
        .iter()
        .all(|(schema, supported)| schema_requires_only(schema, supported))
    {
        SchemaCapability::Supported
    } else {
        SchemaCapability::Incompatible
    }
}

fn agent_execution_schema_capability(probe_root: &Path) -> SchemaCapability {
    let managed_capability = managed_session_schema_capability(probe_root);
    if managed_capability != SchemaCapability::Supported {
        return managed_capability;
    }

    let schemas = [
        (
            "ThreadStartParams.json",
            &[
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
            ][..],
        ),
        (
            "ThreadResumeParams.json",
            &[
                "threadId",
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
            ][..],
        ),
        ("TurnStartParams.json", &["threadId", "input", "effort"][..]),
    ];
    for (name, properties) in schemas {
        let Some(path) = find_schema_file(probe_root, name) else {
            return SchemaCapability::NotDeclared;
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return SchemaCapability::Incompatible;
        };
        let Ok(schema) = serde_json::from_str::<Value>(&contents) else {
            return SchemaCapability::Incompatible;
        };
        if !properties.iter().all(|property| {
            schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|declared| declared.contains_key(*property))
        }) {
            return SchemaCapability::Incompatible;
        }
    }
    SchemaCapability::Supported
}

fn schema_requires_only(schema: &Value, supported_required: &[&str]) -> bool {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .all(|field| {
            field
                .as_str()
                .is_some_and(|field| supported_required.contains(&field))
        })
}

fn find_schema_file(root: &Path, file_name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find_schema_file(&path, file_name)
        {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_requires_only_accepts_known_required_fields() {
        assert!(schema_requires_only(
            &json!({"required": ["input", "threadId"]}),
            &["input", "threadId"],
        ));
        assert!(!schema_requires_only(
            &json!({"required": ["input", "threadId", "approvalPolicy"]}),
            &["input", "threadId"],
        ));
    }
}
