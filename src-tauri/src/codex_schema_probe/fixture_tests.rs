use super::*;
use serde_json::Value;
use std::path::Path;

fn fixture_root(profile: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-fixtures/schema-profiles")
        .join(profile)
}

fn capabilities(profile: &str) -> SchemaCapabilities {
    inspect_schema_capabilities(&fixture_root(profile))
}

#[test]
fn modern_and_earlier_wire_contract_schema_samples_are_supported() {
    // 目录名 legacy 表示较早字段形态的兼容样本；不据此宣称独立 Legacy RPC
    // method/schema 已被当前实现或真实 Codex 安装证明支持。
    for profile in ["modern", "legacy"] {
        let result = capabilities(profile);
        assert_eq!(result.usage, SchemaCapability::Supported, "{profile} usage");
        assert_eq!(
            result.managed_session,
            SchemaCapability::Supported,
            "{profile} managed"
        );
        assert_eq!(
            result.agent_execution,
            SchemaCapability::Supported,
            "{profile} agent"
        );
    }
}

#[test]
fn unsupported_required_field_fails_closed() {
    let result = capabilities("unsupported");
    assert_eq!(result.usage, SchemaCapability::Supported);
    assert_eq!(result.managed_session, SchemaCapability::Incompatible);
    assert_eq!(result.agent_execution, SchemaCapability::Incompatible);
}

#[test]
fn missing_schema_files_are_not_declared() {
    let root = std::env::temp_dir().join(format!("cas-schema-missing-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let result = inspect_schema_capabilities(&root);
    assert_eq!(result.usage, SchemaCapability::NotDeclared);
    assert_eq!(result.managed_session, SchemaCapability::NotDeclared);
    assert_eq!(result.agent_execution, SchemaCapability::NotDeclared);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn unknown_optional_schema_fields_remain_compatible() {
    let schema: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/test-fixtures/schema-profiles/modern/ThreadStartParams.json"
    )))
    .unwrap();
    assert!(schema_requires_only(&schema, &["cwd"]));
    assert!(
        schema
            .get("properties")
            .unwrap()
            .get("futureOptionalField")
            .is_some()
    );
}
