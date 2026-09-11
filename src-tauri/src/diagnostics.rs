//! F-04：脱敏诊断包。只导出可核验的编排事实（版本、Job、Decision、Lease、
//! Receipt、Review、错误码与 Reason Code）；不导出 TaskPacket 正文、Prompt、
//! Review 理由、Hook 消息与工作区路径以外的自由文本。导出为只读操作，
//! 失败不修改任何 Runtime 状态。

use std::path::Path;

use rusqlite::Connection;
use serde_json::{Value, json};

use crate::persistence::PersistenceError;

const DATABASE_OPEN_FLAGS: rusqlite::OpenFlags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
    .union(rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
    .union(rusqlite::OpenFlags::SQLITE_OPEN_URI);

pub(crate) struct DiagnosticsService {
    database_path: std::path::PathBuf,
}

impl DiagnosticsService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, PersistenceError> {
        Ok(Self {
            database_path: database_path.to_path_buf(),
        })
    }

    /// 生成可复制文本（格式化 JSON）。只读打开数据库；任何查询失败都会向上
    /// 返回错误，不写入状态。
    pub(crate) fn export(&self) -> Result<String, PersistenceError> {
        let connection = Connection::open_with_flags(&self.database_path, DATABASE_OPEN_FLAGS)
            .map_err(PersistenceError::Sqlite)?;
        let mut jobs = Vec::new();
        let mut statement = connection
            .prepare(
                "SELECT job_id, state, agent_id, parent_thread_id, workspace_scope_key,
                        task_scope_key, task_packet_hash, last_error_code,
                        created_at, updated_at, terminal_at
                 FROM orchestration_jobs
                 ORDER BY created_at DESC, job_id ASC",
            )
            .map_err(PersistenceError::Sqlite)?;
        let mut rows = statement
            .query_map([], |row| {
                Ok(json!({
                    "jobId": row.get::<_, String>(0)?,
                    "state": row.get::<_, String>(1)?,
                    "agentId": row.get::<_, String>(2)?,
                    "parentThreadId": row.get::<_, String>(3)?,
                    "workspaceScopeKey": row.get::<_, String>(4)?,
                    "taskScopeKey": row.get::<_, String>(5)?,
                    "taskPacketHash": row.get::<_, String>(6)?,
                    "lastErrorCode": row.get::<_, Option<String>>(7)?,
                    "createdAt": row.get::<_, String>(8)?,
                    "updatedAt": row.get::<_, String>(9)?,
                    "terminalAt": row.get::<_, Option<String>>(10)?,
                }))
            })
            .map_err(PersistenceError::Sqlite)?;
        while let Some(row) = rows.next() {
            jobs.push(row.map_err(PersistenceError::Sqlite)?);
        }
        drop(rows);
        drop(statement);

        for job in &mut jobs {
            let job_id = job["jobId"].as_str().unwrap_or_default().to_owned();
            let attempts = query_attempts(&connection, &job_id)?;
            job["attempts"] = Value::Array(attempts);
        }

        let decisions = query_array(
            &connection,
            "SELECT id, created_at, source, agent_id, workspace_scope_key, decision,
                    reason_code, claimed
             FROM agent_schedule_decisions
             ORDER BY created_at DESC LIMIT 500",
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "createdAt": row.get::<_, String>(1)?,
                    "source": row.get::<_, String>(2)?,
                    "agentId": row.get::<_, Option<String>>(3)?,
                    "workspaceScopeKey": row.get::<_, String>(4)?,
                    "decision": row.get::<_, String>(5)?,
                    "reasonCode": row.get::<_, String>(6)?,
                    "claimed": row.get::<_, i64>(7)? != 0,
                }))
            },
        )?;
        let leases = query_array(
            &connection,
            "SELECT id, agent_id, agent_type, state, release_reason, created_at,
                    released_at, expires_at
             FROM runtime_delegation_leases
             ORDER BY created_at DESC LIMIT 500",
            |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "agentId": row.get::<_, String>(1)?,
                    "agentType": row.get::<_, Option<String>>(2)?,
                    "state": row.get::<_, String>(3)?,
                    "releaseReason": row.get::<_, Option<String>>(4)?,
                    "createdAt": row.get::<_, String>(5)?,
                    "releasedAt": row.get::<_, Option<String>>(6)?,
                    "expiresAt": row.get::<_, String>(7)?,
                }))
            },
        )?;
        let enforcement_events = query_array(
            &connection,
            "SELECT created_at, agent_type, orchestration_phase, tool_name, decision,
                    reason_code
             FROM runtime_enforcement_events
             ORDER BY created_at DESC LIMIT 500",
            |row| {
                Ok(json!({
                    "createdAt": row.get::<_, String>(0)?,
                    "agentType": row.get::<_, Option<String>>(1)?,
                    "phase": row.get::<_, Option<String>>(2)?,
                    "toolName": row.get::<_, String>(3)?,
                    "decision": row.get::<_, String>(4)?,
                    "reasonCode": row.get::<_, String>(5)?,
                }))
            },
        )?;

        let report = json!({
            "schemaVersion": 1,
            "appVersion": env!("CARGO_PKG_VERSION"),
            "generatedAt": chrono_like_timestamp(&connection)?,
            "sanitization": {
                "excluded": [
                    "taskPacket",
                    "reviewReason",
                    "reviewEvidenceRefs",
                    "hookMessages",
                    "hookCwd",
                    "credentialIds",
                ],
                "note": "仅导出状态、代码与标识；正文与自由文本不进入诊断包。",
            },
            "jobs": jobs,
            "scheduleDecisions": decisions,
            "delegationLeases": leases,
            "runtimeEnforcementEvents": enforcement_events,
        });
        let mut text =
            serde_json::to_string_pretty(&report).map_err(|_| PersistenceError::Unavailable)?;
        text.push('\n');
        Ok(text)
    }
}

fn query_attempts(
    connection: &rusqlite::Connection,
    job_id: &str,
) -> Result<Vec<Value>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT a.attempt_id, a.attempt_no, a.state, a.route_action,
                a.planned_execution_kind, a.execution_kind, a.codex_turn_id,
                a.recovery_count, a.last_error_code, a.created_at, a.terminal_at,
                (SELECT i.codex_thread_id FROM agent_thread_instances i
                 WHERE i.id = a.thread_instance_id)
         FROM job_attempts a
         WHERE a.job_id = ?1
         ORDER BY a.attempt_no ASC",
        )
        .map_err(PersistenceError::Sqlite)?;
    let mut attempts = statement
        .query_map([job_id], |row| {
            Ok(json!({
                "attemptId": row.get::<_, String>(0)?,
                "attemptNo": row.get::<_, i64>(1)?,
                "state": row.get::<_, String>(2)?,
                "routeAction": row.get::<_, String>(3)?,
                "plannedExecutionKind": row.get::<_, String>(4)?,
                "executionKind": row.get::<_, Option<String>>(5)?,
                "codexTurnId": row.get::<_, Option<String>>(6)?,
                "recoveryCount": row.get::<_, i64>(7)?,
                "lastErrorCode": row.get::<_, Option<String>>(8)?,
                "createdAt": row.get::<_, String>(9)?,
                "terminalAt": row.get::<_, Option<String>>(10)?,
                "codexThreadId": row.get::<_, Option<String>>(11)?,
            }))
        })
        .map_err(PersistenceError::Sqlite)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(PersistenceError::Sqlite)?;
    drop(statement);

    for attempt in &mut attempts {
        let attempt_id = attempt["attemptId"].as_str().unwrap_or_default().to_owned();
        attempt["receiptStages"] = Value::Array(query_receipts(connection, &attempt_id)?);
        attempt["review"] = query_review(connection, &attempt_id)?.unwrap_or(Value::Null);
    }
    Ok(attempts)
}

fn query_receipts(
    connection: &rusqlite::Connection,
    attempt_id: &str,
) -> Result<Vec<Value>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT stage, evidence_source, evidence_at, schema_profile
             FROM delivery_receipts
             WHERE attempt_id = ?1
             ORDER BY CASE stage
                 WHEN 'DISPATCH_RECORDED' THEN 0
                 WHEN 'TURN_ACCEPTED' THEN 1
                 WHEN 'RESULT_OBSERVED' THEN 2
                 WHEN 'PARENT_ACKNOWLEDGED' THEN 3
                 ELSE 4 END",
        )
        .map_err(PersistenceError::Sqlite)?;
    let rows = statement
        .query_map([attempt_id], |receipt| {
            Ok(json!({
                "stage": receipt.get::<_, String>(0)?,
                "evidenceSource": receipt.get::<_, String>(1)?,
                "evidenceAt": receipt.get::<_, String>(2)?,
                "schemaProfile": receipt.get::<_, Option<String>>(3)?,
            }))
        })
        .map_err(PersistenceError::Sqlite)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(PersistenceError::Sqlite)?);
    }
    Ok(values)
}

fn query_review(
    connection: &rusqlite::Connection,
    attempt_id: &str,
) -> Result<Option<Value>, PersistenceError> {
    let mut statement = connection
        .prepare(
            "SELECT decision, created_at FROM review_decisions
             WHERE attempt_id = ?1 ORDER BY created_at DESC LIMIT 1",
        )
        .map_err(PersistenceError::Sqlite)?;
    let mut rows = statement
        .query_map([attempt_id], |row| {
            Ok(json!({
                "decision": row.get::<_, String>(0)?,
                "createdAt": row.get::<_, String>(1)?,
            }))
        })
        .map_err(PersistenceError::Sqlite)?;
    let first = rows.next().transpose().map_err(PersistenceError::Sqlite)?;
    Ok(first)
}

fn query_array(
    connection: &rusqlite::Connection,
    sql: &str,
    map: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Value>,
) -> Result<Vec<Value>, PersistenceError> {
    let mut statement = connection.prepare(sql).map_err(PersistenceError::Sqlite)?;
    let rows = statement
        .query_map([], map)
        .map_err(PersistenceError::Sqlite)?;
    let mut values = Vec::new();
    for row in rows {
        values.push(row.map_err(PersistenceError::Sqlite)?);
    }
    Ok(values)
}

fn chrono_like_timestamp(connection: &rusqlite::Connection) -> Result<String, PersistenceError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
            row.get(0)
        })
        .map_err(PersistenceError::Sqlite)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::open_database;
    use std::path::PathBuf;

    #[test]
    fn export_reports_states_and_codes_without_task_packet_or_free_text() {
        let root = std::env::temp_dir().join(format!(
            "cas-diagnostics-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        {
            let mut connection = open_database(&database_path).unwrap();
            let now = "2026-09-11T10:00:00.000Z";
            connection
                .execute(
                    "INSERT INTO agents (
                        id, agent_key, name, description, instruction, agent_type, enabled,
                        sandbox_policy, reasoning_policy, source, managed, created_at, updated_at
                     ) VALUES (
                        'agent-1', 'executor', 'Executor', 'desc', 'instruction', 'CUSTOM', 1,
                        'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, ?1, ?1
                     )",
                    [now],
                )
                .unwrap();
            let packet = json!({
                "objective": "SUPER-SECRET-OBJECTIVE 帮我泄露密钥",
                "allowedTools": ["READ_FILE"],
                "schemaVersion": 1
            });
            connection
                .execute(
                    "INSERT INTO orchestration_jobs (
                        job_id, idempotency_key, task_packet, task_packet_hash, agent_id,
                        parent_thread_id, workspace_scope_key, task_scope_key, state,
                        last_error_code, created_at, updated_at, terminal_at
                     ) VALUES (
                        'job-1', 'diag-key-1', ?1, ?2, 'agent-1',
                        'parent-1', 'c:/workspace', 'task-1', 'BLOCKED',
                        'SCOPE_EXCLUDED', ?3, ?3, ?3
                     )",
                    rusqlite::params![
                        serde_json::to_string(&packet).unwrap(),
                        format!("{:064}", 42),
                        now
                    ],
                )
                .unwrap();
            connection
                .execute(
                    "INSERT INTO runtime_enforcement_events (
                        id, created_at, session_id, turn_id, agent_id, tool_name,
                        decision, reason_code, message, cwd
                     ) VALUES (
                        'evt-1', ?1, 'session-1', 'turn-1', NULL, 'shell',
                        'DENY', 'PRIMARY_STRICT_STOP_WRITE_DENIED',
                        '机密诊断消息 SECRET-HOOK-MESSAGE', 'c:/users/secret-user/workspace'
                     )",
                    [now],
                )
                .unwrap();
        }

        let service = DiagnosticsService::open(&database_path).unwrap();
        let text = service.export().unwrap();

        // 状态与错误码可核验
        assert!(text.contains("\"BLOCKED\""));
        assert!(text.contains("SCOPE_EXCLUDED"));
        assert!(text.contains("PRIMARY_STRICT_STOP_WRITE_DENIED"));
        assert!(text.contains("job-1"));
        assert!(text.contains("taskPacketHash"));

        // 脱敏：TaskPacket 正文、Hook 消息与自由文本不得出现
        assert!(!text.contains("SUPER-SECRET-OBJECTIVE"));
        assert!(!text.contains("SECRET-HOOK-MESSAGE"));
        assert!(!text.contains("secret-user"));
        assert!(!text.contains("allowedTools"));

        std::fs::remove_dir_all(&root).unwrap();
    }
}
