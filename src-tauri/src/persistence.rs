use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, TransactionBehavior, params};

const LATEST_SCHEMA_VERSION: i64 = 40;
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
    (
        25,
        "agent_thread_reuse_pool",
        include_str!("../migrations/0025_agent_thread_reuse_pool.sql"),
    ),
    (
        26,
        "agent_skills",
        include_str!("../migrations/0026_agent_skills.sql"),
    ),
    (
        27,
        "agent_mcp_denylist",
        include_str!("../migrations/0027_agent_mcp_denylist.sql"),
    ),
    (
        28,
        "agent_mcp_tool_policies",
        include_str!("../migrations/0028_agent_mcp_tool_policies.sql"),
    ),
    (
        29,
        "runtime_enforcement",
        include_str!("../migrations/0029_runtime_enforcement.sql"),
    ),
    (
        30,
        "runtime_delegation_leases",
        include_str!("../migrations/0030_runtime_delegation_leases.sql"),
    ),
    (
        31,
        "runtime_delegation_admission",
        include_str!("../migrations/0031_runtime_delegation_admission.sql"),
    ),
    (
        32,
        "runtime_delegation_confirmation",
        include_str!("../migrations/0032_runtime_delegation_confirmation.sql"),
    ),
    (
        33,
        "orchestration_jobs_and_attempts",
        include_str!("../migrations/0033_orchestration_jobs_and_attempts.sql"),
    ),
    (
        34,
        "execution_kind_observations",
        include_str!("../migrations/0034_execution_kind_observations.sql"),
    ),
    (
        35,
        "atomic_scheduling_ownership",
        include_str!("../migrations/0035_atomic_scheduling_ownership.sql"),
    ),
    (
        36,
        "delivery_receipts",
        include_str!("../migrations/0036_delivery_receipts.sql"),
    ),
    (
        37,
        "runtime_receipt_events",
        include_str!("../migrations/0037_runtime_receipt_events.sql"),
    ),
    (
        38,
        "review_decisions",
        include_str!("../migrations/0038_review_decisions.sql"),
    ),
    (
        39,
        "reviewer_reports",
        include_str!("../migrations/0039_reviewer_reports.sql"),
    ),
    (
        40,
        "cached_input_provenance",
        include_str!("../migrations/0040_cached_input_provenance.sql"),
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
    use rusqlite::{OptionalExtension, params};

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
                VALUES (?1, 'future', '2026-01-01T00:00:00Z')",
                [LATEST_SCHEMA_VERSION + 1],
            )
            .unwrap();
        assert!(matches!(
            apply_migrations(&mut connection, MIGRATIONS),
            Err(PersistenceError::SchemaTooNew)
        ));
    }

    #[test]
    fn execution_kind_migration_supports_fresh_and_0033_upgrade() {
        let mut fresh = Connection::open_in_memory().unwrap();
        apply_migrations(&mut fresh, MIGRATIONS).unwrap();
        assert_eq!(
            fresh
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            40
        );
        assert!(
            fresh
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'job_attempts'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .unwrap()
                .is_some()
        );
        assert_eq!(
            fresh
                .query_row(
                    "SELECT execution_kind FROM token_usage_records LIMIT 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .unwrap(),
            None
        );

        let mut upgraded = Connection::open_in_memory().unwrap();
        apply_migrations(&mut upgraded, &MIGRATIONS[..33]).unwrap();
        upgraded
            .execute(
                "INSERT INTO token_usage_records (
                    id, codex_session_id, codex_thread_id, input_tokens, cached_input_tokens,
                    cache_write_input_tokens, output_tokens, reasoning_output_tokens, total_tokens,
                    usage_status, source, started_at, updated_at
                 ) VALUES (
                    'usage-before-0034', 'session-1', 'thread-1', 0, 0, 0, 0, 0, 0,
                    'UNKNOWN', 'CODEX_APP_SERVER', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 )",
                [],
            )
            .unwrap();
        apply_migrations(&mut upgraded, MIGRATIONS).unwrap();
        assert_eq!(
            upgraded
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            40
        );
        assert_eq!(
            upgraded
                .query_row(
                    "SELECT execution_kind FROM token_usage_records WHERE id = 'usage-before-0034'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "OBSERVED_EXTERNAL"
        );
        assert!(
            upgraded
                .execute(
                    "UPDATE token_usage_records SET execution_kind = 'UNKNOWN' WHERE id = 'usage-before-0034'",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn runtime_receipt_event_migration_supports_fresh_and_0036_upgrade() {
        let mut fresh = Connection::open_in_memory().unwrap();
        apply_migrations(&mut fresh, MIGRATIONS).unwrap();
        assert_eq!(
            fresh
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            40
        );
        assert!(
            fresh
                .query_row(
                    "SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'runtime_receipt_events'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .unwrap()
                .is_some()
        );

        let mut upgraded = Connection::open_in_memory().unwrap();
        apply_migrations(&mut upgraded, &MIGRATIONS[..35]).unwrap();
        apply_migrations(&mut upgraded, MIGRATIONS).unwrap();
        assert_eq!(
            upgraded
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            40
        );
        assert!(
            upgraded
                .query_row(
                    "SELECT 1 FROM sqlite_master
                     WHERE type = 'index' AND name = 'idx_runtime_receipt_events_pending'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn review_migration_preserves_threads_and_adds_held_for_review() {
        let mut upgraded = Connection::open_in_memory().unwrap();
        apply_migrations(&mut upgraded, &MIGRATIONS[..37]).unwrap();
        upgraded
            .execute(
                "INSERT INTO agent_thread_instances (
                    id, codex_thread_id, status, created_at, last_used_at, reuse_state
                 ) VALUES (
                    'instance-before-0038', 'thread-before-0038', 'IDLE',
                    '2026-09-10T00:00:00Z', '2026-09-10T00:00:00Z', 'ACTIVE'
                 )",
                [],
            )
            .unwrap();

        apply_migrations(&mut upgraded, MIGRATIONS).unwrap();
        assert_eq!(
            upgraded
                .query_row(
                    "SELECT reuse_state FROM agent_thread_instances
                     WHERE id = 'instance-before-0038'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "ACTIVE"
        );
        upgraded
            .execute(
                "UPDATE agent_thread_instances SET reuse_state = 'HELD_FOR_REVIEW'
                 WHERE id = 'instance-before-0038'",
                [],
            )
            .unwrap();
        assert!(
            upgraded
                .query_row(
                    "SELECT 1 FROM sqlite_master
                     WHERE type = 'table' AND name = 'review_decisions'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .unwrap()
                .is_some()
        );
        assert_eq!(
            upgraded
                .prepare("PRAGMA foreign_key_check")
                .unwrap()
                .query_map([], |_| Ok(()))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn atomic_scheduling_ownership_backfills_agent_type_and_enforces_one_live_slot() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &MIGRATIONS[..34]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    created_at, updated_at
                 ) VALUES
                    ('agent-executor', 'executor-key', 'Executor', 'test', 'test', 'CUSTOM', 1,
                     'WORKSPACE_WRITE', 'HIGH', 'CAS', 1, 'executor',
                     '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'),
                    ('agent-tester', 'tester-key', 'Tester', 'test', 'test', 'CUSTOM', 1,
                     'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, NULL,
                     '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z');
                 INSERT INTO agent_schedule_decisions (
                    id, created_at, source, workspace_scope_key, decision, reason_code, cache_hint
                 ) VALUES
                    ('decision-executor', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE'),
                    ('decision-tester', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE'),
                    ('decision-next', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE');
                 INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id, workspace_scope_key,
                    schedule_decision_id, state, expires_at
                 ) VALUES
                    ('lease-executor', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z',
                     'agent-executor', 'parent-1', 'workspace-1', 'decision-executor', 'PENDING',
                     '2026-09-10T00:00:00Z'),
                    ('lease-tester', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z',
                     'agent-tester', 'parent-1', 'workspace-1', 'decision-tester', 'ACTIVE',
                     '2026-09-10T00:00:00Z');",
            )
            .unwrap();

        apply_migrations(&mut connection, MIGRATIONS).unwrap();
        let types = connection
            .prepare("SELECT agent_type FROM runtime_delegation_leases ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(types, vec!["executor", "tester-key"]);

        let same_type = connection.execute(
            "INSERT INTO runtime_delegation_leases (
                id, created_at, updated_at, agent_id, parent_thread_id, workspace_scope_key,
                schedule_decision_id, state, expires_at, agent_type
             ) VALUES (
                'lease-conflict', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z',
                'agent-executor', 'parent-1', 'workspace-1', 'decision-next', 'ACTIVE',
                '2026-09-10T00:00:00Z', 'executor'
             )",
            [],
        );
        assert!(same_type.is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM runtime_delegation_leases
                     WHERE workspace_scope_key = 'workspace-1' AND parent_thread_id = 'parent-1'
                       AND state IN ('PENDING', 'ACTIVE')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
    }

    #[test]
    fn duplicate_legacy_live_agent_type_rolls_back_0035_migration() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &MIGRATIONS[..34]).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    created_at, updated_at
                 ) VALUES (
                    'agent-1', 'executor-key', 'Executor', 'test', 'test', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'HIGH', 'CAS', 1, 'executor',
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO agent_schedule_decisions (
                    id, created_at, source, workspace_scope_key, decision, reason_code, cache_hint
                 ) VALUES
                    ('decision-1', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE'),
                    ('decision-2', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE');
                 INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id, workspace_scope_key,
                    schedule_decision_id, state, expires_at
                 ) VALUES
                    ('lease-1', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z', 'agent-1',
                     'parent-1', 'workspace-1', 'decision-1', 'PENDING', '2026-09-10T00:00:00Z'),
                    ('lease-2', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z', 'agent-1',
                     'parent-1', 'workspace-1', 'decision-2', 'ACTIVE', '2026-09-10T00:00:00Z');",
            )
            .unwrap();

        assert!(apply_migrations(&mut connection, MIGRATIONS).is_err());
        assert_eq!(
            connection
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            34
        );
        let columns = connection
            .prepare("PRAGMA table_info(runtime_delegation_leases)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "agent_type"));
    }

    #[test]
    fn failed_0033_migration_does_not_advance_schema_version() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, &MIGRATIONS[..32]).unwrap();

        assert!(
            apply_migrations(
                &mut connection,
                &[(
                    33,
                    "orchestration_jobs_and_attempts",
                    "CREATE TABLE orchestration_jobs_partial (id TEXT); INVALID SQL;"
                )]
            )
            .is_err()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            32
        );
        assert!(
            connection
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE name = 'orchestration_jobs_partial'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn orchestration_job_idempotency_key_is_unique_within_its_frozen_scope() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, MIGRATIONS).unwrap();
        connection
            .execute(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                 ) VALUES (
                    'agent-1', 'agent-1', 'Agent 1', 'test', 'test', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'INHERIT', 'CAS', 1,
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 )",
                [],
            )
            .unwrap();

        let insert = "INSERT INTO orchestration_jobs (
                job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                parent_thread_id, workspace_scope_key, task_scope_key, state,
                created_at, updated_at
            ) VALUES (
                ?1, 'key-1', '{\"schema_version\":1}', lower(hex(zeroblob(32))), 'agent-1',
                'parent-1', 'workspace-1', 'scope-1', 'CREATED',
                '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
            )";
        connection.execute(insert, ["job-1"]).unwrap();
        assert!(connection.execute(insert, ["job-2"]).is_err());
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM orchestration_jobs", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn job_attempt_constraints_reject_external_execution_and_multiple_active_attempts() {
        let mut connection = Connection::open_in_memory().unwrap();
        apply_migrations(&mut connection, MIGRATIONS).unwrap();
        connection
            .execute_batch(
                "INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                 ) VALUES (
                    'agent-1', 'agent-1', 'Agent 1', 'test', 'test', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'INHERIT', 'CAS', 1,
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO orchestration_jobs (
                    job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                    parent_thread_id, workspace_scope_key, task_scope_key, state,
                    created_at, updated_at
                 ) VALUES (
                    'job-1', 'key-1', '{\"schema_version\":1}', lower(hex(zeroblob(32))), 'agent-1',
                    'parent-1', 'workspace-1', 'scope-1', 'CREATED',
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO agent_schedule_decisions (
                    id, created_at, source, workspace_scope_key, decision, reason_code, cache_hint
                 ) VALUES
                    ('decision-1', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE'),
                    ('decision-2', '2026-09-09T00:00:00Z', 'TEST', 'workspace-1', 'SPAWN', 'TEST', 'NONE');
                 INSERT INTO runtime_delegation_leases (
                    id, created_at, updated_at, agent_id, parent_thread_id, workspace_scope_key,
                    schedule_decision_id, state, expires_at, agent_type
                 ) VALUES
                    ('lease-1', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z', 'agent-1', 'parent-1',
                     'workspace-1', 'decision-1', 'PENDING', '2026-09-10T00:00:00Z', 'executor'),
                    ('lease-2', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z', 'agent-1', 'parent-1',
                     'workspace-1', 'decision-2', 'PENDING', '2026-09-10T00:00:00Z', 'tester');",
            )
            .unwrap();

        let insert_attempt = "INSERT INTO job_attempts (
                attempt_id, job_id, attempt_no, previous_attempt_id, schedule_decision_id, lease_id,
                route_action, planned_execution_kind, execution_kind, state, created_at, updated_at
            ) VALUES (
                ?1, 'job-1', ?2, ?3, ?4, ?5, 'SPAWN', 'NATIVE_CHILD', ?6, 'PLANNED',
                '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
            )";
        assert!(
            connection
                .execute(
                    insert_attempt,
                    params![
                        "attempt-external",
                        1,
                        Option::<&str>::None,
                        "decision-1",
                        "lease-1",
                        "OBSERVED_EXTERNAL",
                    ],
                )
                .is_err()
        );
        connection
            .execute(
                insert_attempt,
                params![
                    "attempt-1",
                    1,
                    Option::<&str>::None,
                    "decision-1",
                    "lease-1",
                    Option::<&str>::None,
                ],
            )
            .unwrap();
        assert!(
            connection
                .execute(
                    insert_attempt,
                    params![
                        "attempt-2",
                        2,
                        "attempt-1",
                        "decision-2",
                        "lease-2",
                        Option::<&str>::None,
                    ],
                )
                .is_err()
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM job_attempts", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
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
