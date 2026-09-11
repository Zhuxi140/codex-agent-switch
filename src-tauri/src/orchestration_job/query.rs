//! F-01：Job 分层只读查询（Project → Job → Attempt → Thread/Turn）。
//! 聚合 Decision、Lease、Receipt、Review 摘要，UI 不需要自己关联多张表。

use serde::{Deserialize, Serialize};

use super::OrchestrationJobService;
use crate::orchestration_contract::OrchestrationError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobListRequest {
    #[serde(default)]
    pub(crate) workspace_scope_key: Option<String>,
    #[serde(default)]
    pub(crate) agent_id: Option<String>,
    #[serde(default)]
    pub(crate) page: u32,
    #[serde(default)]
    pub(crate) page_size: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobPageResponse {
    pub(crate) jobs: Vec<OrchestrationJobTracking>,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
    pub(crate) total_count: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationJobTracking {
    pub(crate) job_id: String,
    pub(crate) idempotency_key: String,
    pub(crate) state: String,
    pub(crate) agent_id: String,
    pub(crate) parent_thread_id: String,
    pub(crate) workspace_scope_key: String,
    pub(crate) task_scope_key: String,
    pub(crate) last_error_code: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) terminal_at: Option<String>,
    pub(crate) attempts: Vec<OrchestrationAttemptTracking>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OrchestrationAttemptTracking {
    pub(crate) attempt_id: String,
    pub(crate) attempt_no: i64,
    pub(crate) state: String,
    pub(crate) route_action: String,
    pub(crate) planned_execution_kind: String,
    pub(crate) execution_kind: Option<String>,
    pub(crate) codex_thread_id: Option<String>,
    pub(crate) codex_turn_id: Option<String>,
    pub(crate) lease_state: Option<String>,
    pub(crate) receipt_stage: Option<String>,
    pub(crate) review_decision: Option<String>,
    pub(crate) updated_at: String,
}

const MAX_PAGE_SIZE: u32 = 100;
const DEFAULT_PAGE_SIZE: u32 = 20;

impl OrchestrationJobService {
    /// 只读分页查询；排序固定 `created_at DESC, job_id ASC` 保证分页稳定。
    pub(crate) fn list_tracking(
        &self,
        request: OrchestrationJobListRequest,
    ) -> Result<OrchestrationJobPageResponse, OrchestrationError> {
        let connection = self.connection()?;
        let page_size = if request.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            request.page_size.min(MAX_PAGE_SIZE)
        };
        let offset = request.page.saturating_mul(page_size);

        let mut conditions: Vec<&'static str> = Vec::new();
        let mut parameters: Vec<String> = Vec::new();
        if let Some(scope) = request
            .workspace_scope_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            conditions.push("j.workspace_scope_key = ?");
            parameters.push(scope.to_owned());
        }
        if let Some(agent_id) = request
            .agent_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            conditions.push("j.agent_id = ?");
            parameters.push(agent_id.to_owned());
        }
        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let total_count: i64 = connection
            .query_row(
                &format!("SELECT COUNT(*) FROM orchestration_jobs j {where_clause}"),
                rusqlite::params_from_iter(parameters.iter()),
                |row| row.get(0),
            )
            .map_err(|_| super::persistence_error())?;

        let mut statement = connection
            .prepare(&format!(
                "SELECT j.job_id, j.idempotency_key, j.state, j.agent_id, j.parent_thread_id,
                        j.workspace_scope_key, j.task_scope_key, j.last_error_code,
                        j.created_at, j.updated_at, j.terminal_at
                 FROM orchestration_jobs j {where_clause}
                 ORDER BY j.created_at DESC, j.job_id ASC
                 LIMIT ? OFFSET ?"
            ))
            .map_err(|_| super::persistence_error())?;
        let mut bound = parameters.clone();
        bound.push(page_size.to_string());
        bound.push(offset.to_string());
        let jobs = statement
            .query_map(rusqlite::params_from_iter(bound.iter()), |row| {
                Ok(OrchestrationJobTracking {
                    job_id: row.get(0)?,
                    idempotency_key: row.get(1)?,
                    state: row.get(2)?,
                    agent_id: row.get(3)?,
                    parent_thread_id: row.get(4)?,
                    workspace_scope_key: row.get(5)?,
                    task_scope_key: row.get(6)?,
                    last_error_code: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                    terminal_at: row.get(10)?,
                    attempts: Vec::new(),
                })
            })
            .map_err(|_| super::persistence_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| super::persistence_error())?;
        drop(statement);

        let mut jobs = jobs;
        for job in &mut jobs {
            job.attempts = load_attempt_tracking(&connection, &job.job_id)?;
        }

        Ok(OrchestrationJobPageResponse {
            jobs,
            page: request.page,
            page_size,
            total_count,
        })
    }
}

fn load_attempt_tracking(
    connection: &rusqlite::Connection,
    job_id: &str,
) -> Result<Vec<OrchestrationAttemptTracking>, OrchestrationError> {
    let mut statement = connection
        .prepare(
            "SELECT a.attempt_id, a.attempt_no, a.state, a.route_action,
                    a.planned_execution_kind, a.execution_kind, a.codex_turn_id, a.updated_at,
                    (SELECT i.codex_thread_id FROM agent_thread_instances i
                     WHERE i.id = a.thread_instance_id),
                    (SELECT l.state FROM runtime_delegation_leases l WHERE l.id = a.lease_id),
                    (SELECT r.stage FROM delivery_receipts r
                     WHERE r.attempt_id = a.attempt_id
                     ORDER BY CASE r.stage
                         WHEN 'DISPATCH_RECORDED' THEN 0
                         WHEN 'TURN_ACCEPTED' THEN 1
                         WHEN 'RESULT_OBSERVED' THEN 2
                         WHEN 'PARENT_ACKNOWLEDGED' THEN 3
                         ELSE 4 END DESC
                     LIMIT 1),
                    (SELECT rv.decision FROM review_decisions rv
                     WHERE rv.attempt_id = a.attempt_id
                     ORDER BY rv.created_at DESC
                     LIMIT 1)
             FROM job_attempts a
             WHERE a.job_id = ?1
             ORDER BY a.attempt_no ASC",
        )
        .map_err(|_| super::persistence_error())?;
    let attempts = statement
        .query_map([job_id], |row| {
            Ok(OrchestrationAttemptTracking {
                attempt_id: row.get(0)?,
                attempt_no: row.get(1)?,
                state: row.get(2)?,
                route_action: row.get(3)?,
                planned_execution_kind: row.get(4)?,
                execution_kind: row.get(5)?,
                codex_turn_id: row.get(6)?,
                updated_at: row.get(7)?,
                codex_thread_id: row.get(8)?,
                lease_state: row.get(9)?,
                receipt_stage: row.get(10)?,
                review_decision: row.get(11)?,
            })
        })
        .map_err(|_| super::persistence_error())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| super::persistence_error())?;
    Ok(attempts)
}
