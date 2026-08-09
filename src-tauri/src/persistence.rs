use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

const LATEST_SCHEMA_VERSION: i64 = 8;
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
                VALUES (9, 'future', '2026-01-01T00:00:00Z')",
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
                 );",
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
    }
}
