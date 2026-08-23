use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

const LATEST_SCHEMA_VERSION: i64 = 24;
const MIGRATIONS: &[(i64, &str, &str)] = &[
    (
        1,
        "provider_credentials",
        include_str!("../migrations/0001_provider_credentials.sql"),
    ),
    (2, "models", include_str!("../migrations/0002_models.sql")),
    (3, "agents", include_str!("../migrations/0003_agents.sql")),
    (
        4,
        "configuration_projection",
        include_str!("../migrations/0004_configuration_projection.sql"),
    ),
    (
        5,
        "application_settings",
        include_str!("../migrations/0005_application_settings.sql"),
    ),
    (
        6,
        "generic_codex_multi_agent_capability",
        include_str!("../migrations/0006_generic_codex_multi_agent_capability.sql"),
    ),
    (
        7,
        "model_connection_status",
        include_str!("../migrations/0007_model_connection_status.sql"),
    ),
    (
        8,
        "active_agent",
        include_str!("../migrations/0008_active_agent.sql"),
    ),
    (
        9,
        "agent_orchestration",
        include_str!("../migrations/0009_agent_orchestration.sql"),
    ),
    (
        10,
        "orchestration_exclusions",
        include_str!("../migrations/0010_orchestration_exclusions.sql"),
    ),
    (
        11,
        "generic_agent_multi_agent_capability",
        include_str!("../migrations/0011_generic_agent_multi_agent_capability.sql"),
    ),
    (
        12,
        "token_usage_records",
        include_str!("../migrations/0012_token_usage_records.sql"),
    ),
    (
        13,
        "agent_thread_instances",
        include_str!("../migrations/0013_agent_thread_instances.sql"),
    ),
    (
        14,
        "agent_reuse_and_provider_cache",
        include_str!("../migrations/0014_agent_reuse_and_provider_cache.sql"),
    ),
    (
        15,
        "agent_cache_retention_override",
        include_str!("../migrations/0015_agent_cache_retention_override.sql"),
    ),
    (
        16,
        "codex_native_effective_context",
        include_str!("../migrations/0016_codex_native_effective_context.sql"),
    ),
    (
        17,
        "agent_thread_current_context",
        include_str!("../migrations/0017_agent_thread_current_context.sql"),
    ),
    (
        18,
        "agent_thread_runtime_fingerprint",
        include_str!("../migrations/0018_agent_thread_runtime_fingerprint.sql"),
    ),
    (
        19,
        "agent_thread_usage_observation_timestamps",
        include_str!("../migrations/0019_agent_thread_usage_observation_timestamps.sql"),
    ),
    (
        20,
        "agent_thread_reuse_claim",
        include_str!("../migrations/0020_agent_thread_reuse_claim.sql"),
    ),
    (
        21,
        "agent_schedule_decisions",
        include_str!("../migrations/0021_agent_schedule_decisions.sql"),
    ),
    (
        22,
        "agent_thread_task_scope",
        include_str!("../migrations/0022_agent_thread_task_scope.sql"),
    ),
    (
        23,
        "provider_cleanup_and_usage_projects",
        include_str!("../migrations/0023_provider_cleanup_and_usage_projects.sql"),
    ),
    (
        24,
        "agent_spawn_reservations",
        include_str!("../migrations/0024_agent_spawn_reservations.sql"),
    ),
];

pub(crate) fn open_database(path: &Path) -> Result<Connection, PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| PersistenceError::Unavailable)?;
    }
    initialize(Connection::open(path)?)
}

#[cfg(test)]
pub(crate) fn open_in_memory() -> Result<Connection, PersistenceError> {
    initialize(Connection::open_in_memory()?)
}

fn initialize(mut connection: Connection) -> Result<Connection, PersistenceError> {
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.busy_timeout(Duration::from_secs(5))?;
    apply_migrations(&mut connection, MIGRATIONS)?;
    Ok(connection)
}

fn apply_migrations(
    connection: &mut Connection,
    migrations: &[(i64, &str, &str)],
) -> Result<(), PersistenceError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );",
    )?;

    let current = transaction.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get::<_, i64>(0),
    )?;
    if current > LATEST_SCHEMA_VERSION {
        return Err(PersistenceError::SchemaTooNew);
    }

    for (version, name, sql) in migrations
        .iter()
        .filter(|(version, _, _)| *version > current)
    {
        transaction.execute_batch(sql)?;
        transaction.execute(
            "INSERT INTO schema_migrations (version, name, applied_at)
             VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![version, name],
        )?;
    }

    transaction.commit()?;
    Ok(())
}

#[derive(Debug)]
pub(crate) enum PersistenceError {
    SchemaTooNew,
    Unavailable,
    Sqlite(rusqlite::Error),
}

impl fmt::Display for PersistenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaTooNew => formatter.write_str("database schema is too new"),
            Self::Unavailable => formatter.write_str("database unavailable"),
            Self::Sqlite(_) => formatter.write_str("sqlite operation failed"),
        }
    }
}

impl std::error::Error for PersistenceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for PersistenceError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::OptionalExtension;

    use super::*;

    #[test]
    fn migration_is_transactional_and_rejects_newer_schema() {
        let mut connection = Connection::open_in_memory().unwrap();
        let failing = "CREATE TABLE partial (id TEXT); INVALID SQL;";
        assert!(apply_migrations(&mut connection, &[(1, "broken", failing)]).is_err());
        assert!(
            connection
                .query_row(
                    "SELECT name FROM sqlite_master WHERE name = 'partial'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .unwrap()
                .is_none()
        );

        apply_migrations(&mut connection, MIGRATIONS).unwrap();
        connection
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at)
                VALUES (25, 'future', '2026-01-01T00:00:00Z')",
                [],
            )
            .unwrap();
        assert!(matches!(
            apply_migrations(&mut connection, MIGRATIONS),
            Err(PersistenceError::SchemaTooNew)
        ));
    }

    #[test]
    fn migration_replaces_v2_specific_multi_agent_capability() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &MIGRATIONS[..5]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol,
                    auth_type, source, created_at, updated_at
                 ) VALUES (
                    'provider', 'cas_test', 'Test', 'CUSTOM', 'https://example.com/',
                    'RESPONSES', 'BEARER_TOKEN', 'USER', '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO models (
                    id, provider_id, model_id, display_name, source, created_at, updated_at
                 ) VALUES (
                    'model', 'provider', 'model', 'Model', 'USER', '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO model_capabilities (
                    model_id, capability, status, source, confidence
                 ) VALUES (
                    'model', 'CODEX_MULTI_AGENT_V2', 'SUPPORTED', 'TEST', 'VERIFIED'
                 );
                 INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                 ) VALUES (
                    'agent', 'executor', 'Executor', 'description', 'instruction', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'HIGH', 'USER', 1, '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO agent_required_capabilities (agent_id, capability)
                 VALUES ('agent', 'CODEX_MULTI_AGENT_V2');
                 INSERT INTO agent_preferred_capabilities (agent_id, capability)
                 VALUES ('agent', 'CODEX_MULTI_AGENT_V2');
                 INSERT INTO agent_preferred_capabilities (agent_id, capability)
                 VALUES ('agent', 'CODEX_MULTI_AGENT');
                 ",
            )
            .unwrap();

        apply_migrations(&mut connection, MIGRATIONS).unwrap();

        let capabilities = connection
            .prepare("SELECT capability FROM model_capabilities ORDER BY capability")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(capabilities, vec!["CODEX_MULTI_AGENT"]);

        for table in [
            "agent_required_capabilities",
            "agent_preferred_capabilities",
        ] {
            let capabilities = connection
                .prepare(&format!(
                    "SELECT capability FROM {table} WHERE agent_id = 'agent' ORDER BY capability"
                ))
                .unwrap()
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert_eq!(capabilities, vec!["CODEX_MULTI_AGENT"]);
        }
    }

    #[test]
    fn migration_uses_codex_effective_context_for_existing_native_threads() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &MIGRATIONS[..15]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol,
                    auth_type, source, preset_id, created_at, updated_at
                 ) VALUES (
                    'provider-native', 'codex-native', 'Codex Native', 'PRESET',
                    'https://api.openai.com/v1/', 'RESPONSES', 'BEARER_TOKEN',
                    'BUILT_IN', 'codex-native', '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO models (
                    id, provider_id, model_id, display_name, source,
                    context_window, created_at, updated_at
                 ) VALUES (
                    'model-terra', 'provider-native', 'gpt-5.6-terra', 'GPT-5.6 Terra',
                    'PRESET', 1050000, '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type,
                    sandbox_policy, reasoning_policy, source, created_at, updated_at
                 ) VALUES (
                    'agent-terra', 'executor', 'Executor', 'description', 'instruction',
                    'CUSTOM', 'WORKSPACE_WRITE', 'HIGH', 'CAS', '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, source, created_at, updated_at
                 ) VALUES (
                    'binding-terra', 'agent-terra', 'model-terra',
                    'CAS', '2026-01-01', '2026-01-01'
                 );
                 INSERT INTO agent_thread_instances (
                    id, agent_id, codex_thread_id, status, total_tokens,
                    context_window, created_at, last_used_at
                 ) VALUES (
                    'instance-terra', 'agent-terra', 'thread-terra', 'IDLE', 100,
                    1050000, '2026-01-01', '2026-01-01'
                 );",
            )
            .unwrap();

        apply_migrations(&mut connection, MIGRATIONS).unwrap();

        let model_context: i64 = connection
            .query_row(
                "SELECT context_window FROM models WHERE id = 'model-terra'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let thread_context: i64 = connection
            .query_row(
                "SELECT context_window
                 FROM agent_thread_instances
                 WHERE id = 'instance-terra'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(model_context, 258_400);
        assert_eq!(thread_context, 258_400);
    }
}
