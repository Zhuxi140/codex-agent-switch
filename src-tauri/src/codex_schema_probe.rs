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
        Some(inspect_schema_capabilities(&probe_root))
    })()
    .unwrap_or_default();
    let _ = fs::remove_dir_all(&probe_root);
    result
}

fn inspect_schema_capabilities(probe_root: &Path) -> SchemaCapabilities {
    SchemaCapabilities {
        usage: if find_schema_file(probe_root, "ThreadTokenUsageUpdatedNotification.json").is_some()
        {
            SchemaCapability::Supported
        } else {
            SchemaCapability::NotDeclared
        },
        managed_session: managed_session_schema_capability(probe_root),
        agent_execution: agent_execution_schema_capability(probe_root),
    }
}

fn managed_session_schema_capability(probe_root: &Path) -> SchemaCapability {
    let schemas = [
        ("ThreadStartParams.json", &["cwd"][..]),
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
            &["cwd", "model", "developerInstructions", "sandbox"][..],
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
            &[
                "threadId",
                "cwd",
                "model",
                "developerInstructions",
                "sandbox",
            ][..],
        ),
        (
            "TurnStartParams.json",
            &["threadId", "input", "effort"][..],
            &["threadId", "input", "effort"][..],
        ),
    ];
    for (name, properties, supported_required) in schemas {
        let Some(path) = find_schema_file(probe_root, name) else {
            return SchemaCapability::NotDeclared;
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return SchemaCapability::Incompatible;
        };
        let Ok(schema) = serde_json::from_str::<Value>(&contents) else {
            return SchemaCapability::Incompatible;
        };
        if !schema_requires_only(&schema, supported_required)
            || !properties.iter().all(|property| {
                schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|declared| declared.contains_key(*property))
            })
        {
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

    fn fixture_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("cas-schema-fixture-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn write_schema(root: &Path, name: &str, required: &[&str], properties: &[&str]) {
        let properties = properties
            .iter()
            .map(|name| ((*name).to_owned(), json!({})))
            .collect::<serde_json::Map<_, _>>();
        fs::write(
            root.join(name),
            serde_json::to_vec(&json!({
                "required": required,
                "properties": properties
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn write_supported_fixture(root: &Path) {
        write_schema(
            root,
            "ThreadStartParams.json",
            &["cwd"],
            &[
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
                "futureOptionalField",
            ],
        );
        write_schema(
            root,
            "ThreadResumeParams.json",
            &["threadId"],
            &[
                "threadId",
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
            ],
        );
        write_schema(
            root,
            "TurnStartParams.json",
            &["threadId", "input"],
            &["threadId", "input", "effort"],
        );
        fs::write(root.join("ThreadTokenUsageUpdatedNotification.json"), b"{}").unwrap();
    }

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

    #[test]
    fn schema_fixture_matrix_is_capability_driven() {
        let supported = fixture_root();
        write_supported_fixture(&supported);
        let capabilities = inspect_schema_capabilities(&supported);
        assert_eq!(capabilities.usage, SchemaCapability::Supported);
        assert_eq!(capabilities.managed_session, SchemaCapability::Supported);
        assert_eq!(capabilities.agent_execution, SchemaCapability::Supported);

        write_schema(
            &supported,
            "ThreadStartParams.json",
            &["cwd"],
            &["cwd", "developerInstructions", "sandbox"],
        );
        let missing_agent_field = inspect_schema_capabilities(&supported);
        assert_eq!(
            missing_agent_field.managed_session,
            SchemaCapability::Supported
        );
        assert_eq!(
            missing_agent_field.agent_execution,
            SchemaCapability::Incompatible
        );

        write_schema(
            &supported,
            "ThreadStartParams.json",
            &["cwd", "newRequiredField"],
            &[
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
                "newRequiredField",
            ],
        );
        let incompatible = inspect_schema_capabilities(&supported);
        assert_eq!(incompatible.managed_session, SchemaCapability::Incompatible);
        assert_eq!(incompatible.agent_execution, SchemaCapability::Incompatible);

        let missing = fixture_root();
        let undeclared = inspect_schema_capabilities(&missing);
        assert_eq!(undeclared.usage, SchemaCapability::NotDeclared);
        assert_eq!(undeclared.managed_session, SchemaCapability::NotDeclared);
        assert_eq!(undeclared.agent_execution, SchemaCapability::NotDeclared);

        let _ = fs::remove_dir_all(supported);
        let _ = fs::remove_dir_all(missing);
    }
}
