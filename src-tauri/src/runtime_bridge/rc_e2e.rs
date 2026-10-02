use super::*;

use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use crate::codex_config::ORCHESTRATION_RUNTIME_CONTRACT;
use crate::configuration::{ConfigurationService, RuntimeModeSwitchRequest};
use crate::orchestration_contract::{
    ExecutionKindPolicy, JobState, OutputContract, PermissionPolicy, ReviewOutcome, ReviewPolicy,
    TASK_PACKET_SCHEMA_VERSION,
};
use crate::orchestration_job::{
    OrchestrationJobListRequest, OrchestrationJobReviewRequest, OrchestrationJobService,
};
use cas_native_lifecycle::rollout_state;
use cas_scheduler::normalize_workspace_scope_key;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};
use toml_edit::{DocumentMut, value};

const TASK_SCOPE_KEY: &str = "cas-rc1-proof";
const CONCURRENT_TASK_SCOPE_KEY: &str = "cas-rc2-concurrent";
const MANAGED_TASK_SCOPE_KEY: &str = "cas-managed-worker-proof";

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let (Ok(root), Ok(temp)) = (self.0.canonicalize(), env::temp_dir().canonicalize()) else {
            return;
        };
        if root.parent() == Some(temp.as_path())
            && root
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("cas-"))
        {
            let _ = fs::remove_dir_all(root);
        }
    }
}

#[derive(Debug)]
struct Rc1Failure {
    code: &'static str,
    message: String,
}

impl Rc1Failure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

fn stage<T, E: std::fmt::Display>(
    result: Result<T, E>,
    code: &'static str,
) -> Result<T, Rc1Failure> {
    result.map_err(|error| Rc1Failure::new(code, error.to_string()))
}

fn required_path(name: &'static str) -> Result<PathBuf, Rc1Failure> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or_else(|| Rc1Failure::new("E2E_CONFIGURATION_INVALID", format!("缺少 {name}")))
}

fn clone_database(source: &Path, target: &Path) -> Result<(), Rc1Failure> {
    let connection = stage(
        Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY),
        "SOURCE_DATABASE_UNAVAILABLE",
    )?;
    stage(
        connection.execute("VACUUM INTO ?1", [target.to_string_lossy().as_ref()]),
        "DATABASE_CLONE_FAILED",
    )?;
    Ok(())
}

fn reset_e2e_database(connection: &Connection, codex_home: &Path) -> Result<(), Rc1Failure> {
    stage(
        connection.execute_batch(
            "DELETE FROM reviewer_reports;
             DELETE FROM reviewer_assignments;
             DELETE FROM review_decisions;
             DELETE FROM delivery_receipts;
             DELETE FROM runtime_receipt_events;
             DELETE FROM job_attempts;
             DELETE FROM runtime_delegation_leases;
             DELETE FROM runtime_enforcement_events;
             DELETE FROM runtime_hook_turns;
             DELETE FROM token_usage_records;
             DELETE FROM agent_schedule_decisions;
             DELETE FROM agent_spawn_reservations;
             DELETE FROM agent_thread_instances;
             DELETE FROM orchestration_jobs;
             DELETE FROM apply_transactions;
             DELETE FROM configuration_snapshot_resources;
             DELETE FROM configuration_snapshots;
             DELETE FROM managed_resources;
             UPDATE configuration_state
             SET last_applied_desired_hash = NULL,
                 last_applied_at = NULL,
                 last_apply_transaction_id = NULL,
                 active_agent_id = NULL,
                 orchestration_baseline_json = NULL;",
        ),
        "DATABASE_RESET_FAILED",
    )?;
    stage(
        connection.execute(
            "INSERT INTO application_settings (
                setting_key, setting_value, value_type, source, updated_at
             ) VALUES (
                'custom_codex_home', ?1, 'PATH', 'USER',
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
             )
             ON CONFLICT(setting_key) DO UPDATE SET
                setting_value = excluded.setting_value,
                value_type = excluded.value_type,
                source = excluded.source,
                updated_at = excluded.updated_at",
            [codex_home.to_string_lossy().as_ref()],
        ),
        "DATABASE_RESET_FAILED",
    )?;
    Ok(())
}

#[derive(Debug)]
struct ActiveAgent {
    id: String,
    key: String,
    name: String,
    model: String,
    provider: String,
    reasoning_policy: String,
    model_default_reasoning: Option<String>,
}

fn active_agent(connection: &Connection) -> Result<ActiveAgent, Rc1Failure> {
    let requested = env::var("CAS_E2E_AGENT_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let agent = stage(
        connection
            .query_row(
                "SELECT a.id, a.agent_key, a.name, m.model_id, p.provider_key,
                        a.reasoning_policy, m.default_reasoning
                 FROM agents a
                 LEFT JOIN active_agent_bindings active ON active.agent_id = a.id
                 JOIN agent_model_bindings binding
                   ON binding.agent_id = a.id AND binding.enabled = 1
                 JOIN models m ON m.id = binding.model_id AND m.enabled = 1
                 JOIN providers p ON p.id = m.provider_id AND p.enabled = 1
                 WHERE a.enabled = 1
                   AND ((?1 IS NULL AND active.agent_id IS NOT NULL) OR a.agent_key = ?1)
                 ORDER BY CASE WHEN active.agent_id IS NOT NULL THEN 0 ELSE 1 END,
                          CASE a.orchestration_phase WHEN 'EXECUTION' THEN 0 ELSE 1 END,
                          a.agent_key
                 LIMIT 1",
                [requested.as_deref()],
                |row| {
                    Ok(ActiveAgent {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        name: row.get(2)?,
                        model: row.get(3)?,
                        provider: row.get(4)?,
                        reasoning_policy: row.get(5)?,
                        model_default_reasoning: row.get(6)?,
                    })
                },
            )
            .optional(),
        "ACTIVE_AGENT_QUERY_FAILED",
    )?
    .ok_or_else(|| {
        Rc1Failure::new(
            "ACTIVE_AGENT_UNAVAILABLE",
            requested.map_or_else(
                || "没有可用于真实 E2E 的活动 Agent".to_owned(),
                |key| format!("Agent {key} 不存在、未启用或缺少可用模型绑定"),
            ),
        )
    })?;
    Ok(agent)
}

fn copy_runtime_identity(source: &Path, target: &Path) -> Result<(), Rc1Failure> {
    let auth = source.join("auth.json");
    if !auth.is_file() {
        return Err(Rc1Failure::new(
            "AUTH_SOURCE_MISSING",
            "源 CODEX_HOME 缺少 auth.json，无法启动隔离的 Primary",
        ));
    }
    stage(fs::copy(auth, target.join("auth.json")), "AUTH_COPY_FAILED")?;
    for name in ["models_cache.json", "version.json"] {
        let source_file = source.join(name);
        if source_file.is_file() {
            stage(
                fs::copy(source_file, target.join(name)),
                "CODEX_RUNTIME_COPY_FAILED",
            )?;
        }
    }
    Ok(())
}

fn isolated_primary_config(
    source_codex_home: &Path,
) -> Result<(String, Option<String>, Option<String>), Rc1Failure> {
    let source = stage(
        fs::read_to_string(source_codex_home.join("config.toml")),
        "PRIMARY_CONFIG_READ_FAILED",
    )?;
    let source = stage(source.parse::<DocumentMut>(), "PRIMARY_CONFIG_PARSE_FAILED")?;
    let model = source
        .get("model")
        .and_then(|item| item.as_str())
        .map(str::to_owned);
    let reasoning = source
        .get("model_reasoning_effort")
        .and_then(|item| item.as_str())
        .map(str::to_owned);
    let mut isolated = DocumentMut::new();
    if let Some(model) = model.as_deref() {
        isolated["model"] = value(model);
    }
    if let Some(reasoning) = reasoning.as_deref() {
        isolated["model_reasoning_effort"] = value(reasoning);
    }
    isolated["approval_policy"] = value("on-request");
    isolated["sandbox_mode"] = value("workspace-write");
    Ok((isolated.to_string(), model, reasoning))
}

fn copy_directory(source: &Path, target: &Path) -> Result<(), Rc1Failure> {
    stage(fs::create_dir_all(target), "BENCHMARK_DIRECTORY_FAILED")?;
    for entry in stage(fs::read_dir(source), "BENCHMARK_FIXTURE_READ_FAILED")? {
        let entry = stage(entry, "BENCHMARK_FIXTURE_READ_FAILED")?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let file_type = stage(entry.file_type(), "BENCHMARK_FIXTURE_READ_FAILED")?;
        if file_type.is_dir() {
            copy_directory(&source_path, &target_path)?;
        } else if file_type.is_file() {
            stage(
                fs::copy(&source_path, &target_path),
                "BENCHMARK_FIXTURE_COPY_FAILED",
            )?;
        }
    }
    Ok(())
}

fn snapshot_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, Rc1Failure> {
    fn collect(
        root: &Path,
        current: &Path,
        files: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<(), Rc1Failure> {
        for entry in stage(fs::read_dir(current), "BENCHMARK_SCOPE_READ_FAILED")? {
            let entry = stage(entry, "BENCHMARK_SCOPE_READ_FAILED")?;
            let path = entry.path();
            let file_type = stage(entry.file_type(), "BENCHMARK_SCOPE_READ_FAILED")?;
            if file_type.is_dir() {
                collect(root, &path, files)?;
            } else if file_type.is_file() {
                let relative = stage(path.strip_prefix(root), "BENCHMARK_SCOPE_READ_FAILED")?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.insert(
                    relative,
                    stage(fs::read(path), "BENCHMARK_SCOPE_READ_FAILED")?,
                );
            }
        }
        Ok(())
    }

    let mut files = BTreeMap::new();
    collect(root, root, &mut files)?;
    Ok(files)
}

fn scope_violations(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> Vec<String> {
    let mut paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .filter(|path| path != "src/ranges.mjs" && before.get(path) != after.get(path))
        .collect()
}

#[derive(Clone, Debug)]
struct TokenSnapshot {
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
    current_context_tokens: Option<i64>,
    model_context_window: Option<i64>,
    cached_input_provided: Option<bool>,
    usage_status: String,
    source: String,
}

impl TokenSnapshot {
    fn to_json(&self) -> Value {
        json!({
            "inputTokens": self.input_tokens,
            "cachedInputTokens": self.cached_input_tokens,
            "cacheWriteInputTokens": self.cache_write_input_tokens,
            "outputTokens": self.output_tokens,
            "reasoningOutputTokens": self.reasoning_output_tokens,
            "totalTokens": self.total_tokens,
            "currentContextTokens": self.current_context_tokens,
            "modelContextWindow": self.model_context_window,
            "cachedInputProvided": self.cached_input_provided,
            "usageStatus": self.usage_status,
            "source": self.source,
        })
    }

    fn delta_json(&self, previous: &Self) -> Value {
        json!({
            "inputTokens": self.input_tokens - previous.input_tokens,
            "cachedInputTokens": self.cached_input_tokens - previous.cached_input_tokens,
            "cacheWriteInputTokens": self.cache_write_input_tokens - previous.cache_write_input_tokens,
            "outputTokens": self.output_tokens - previous.output_tokens,
            "reasoningOutputTokens": self.reasoning_output_tokens - previous.reasoning_output_tokens,
            "totalTokens": self.total_tokens - previous.total_tokens,
        })
    }
}

fn token_snapshot(
    database_path: &Path,
    thread_id: &str,
) -> Result<Option<TokenSnapshot>, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    stage(
        connection
            .query_row(
                "SELECT input_tokens, cached_input_tokens, cache_write_input_tokens,
                        output_tokens, reasoning_output_tokens, total_tokens,
                        model_context_window, cached_input_provided,
                        usage_status, source
                 FROM token_usage_records WHERE codex_thread_id=?1",
                [thread_id],
                |row| {
                    Ok(TokenSnapshot {
                        input_tokens: row.get(0)?,
                        cached_input_tokens: row.get(1)?,
                        cache_write_input_tokens: row.get(2)?,
                        output_tokens: row.get(3)?,
                        reasoning_output_tokens: row.get(4)?,
                        total_tokens: row.get(5)?,
                        current_context_tokens: None,
                        model_context_window: row.get(6)?,
                        cached_input_provided: row
                            .get::<_, Option<i64>>(7)?
                            .map(|value| value != 0),
                        usage_status: row.get(8)?,
                        source: row.get(9)?,
                    })
                },
            )
            .optional(),
        "TOKEN_EVIDENCE_QUERY_FAILED",
    )
}

fn child_thread_id(
    database_path: &Path,
    parent_thread_id: &str,
    agent_id: &str,
) -> Result<Option<String>, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    stage(
        connection
            .query_row(
                "SELECT codex_thread_id FROM agent_thread_instances
                 WHERE parent_thread_id=?1 AND agent_id=?2
                 ORDER BY last_used_at DESC LIMIT 1",
                params![parent_thread_id, agent_id],
                |row| row.get(0),
            )
            .optional(),
        "CHILD_EVIDENCE_QUERY_FAILED",
    )
}

fn child_token_snapshots(
    database_path: &Path,
    parent_thread_id: &str,
    agent_id: &str,
) -> Result<BTreeMap<String, TokenSnapshot>, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    let mut statement = stage(
        connection.prepare(
            "SELECT codex_thread_id FROM agent_thread_instances
             WHERE parent_thread_id=?1 AND agent_id=?2",
        ),
        "CHILD_EVIDENCE_QUERY_FAILED",
    )?;
    let thread_ids = stage(
        statement.query_map(params![parent_thread_id, agent_id], |row| {
            row.get::<_, String>(0)
        }),
        "CHILD_EVIDENCE_QUERY_FAILED",
    )?;
    let thread_ids = stage(
        thread_ids.collect::<Result<Vec<_>, _>>(),
        "CHILD_EVIDENCE_QUERY_FAILED",
    )?;
    let mut snapshots = BTreeMap::new();
    for thread_id in thread_ids {
        let snapshot = token_snapshot(database_path, &thread_id)?.ok_or_else(|| {
            Rc1Failure::new(
                "CHILD_TOKEN_EVIDENCE_MISSING",
                format!("Child Thread {thread_id} 缺少 Token 记录"),
            )
        })?;
        snapshots.insert(thread_id, snapshot);
    }
    Ok(snapshots)
}

fn benchmark_runtime_evidence(
    database_path: &Path,
    parent_thread_id: &str,
    agent_id: &str,
) -> Result<Value, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    let child_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM agent_thread_instances
             WHERE parent_thread_id=?1 AND agent_id=?2",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let job_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM orchestration_jobs
             WHERE parent_thread_id=?1 AND agent_id=?2",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let completed_job_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM orchestration_jobs
             WHERE parent_thread_id=?1 AND agent_id=?2 AND state='COMPLETED'",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let attempt_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM job_attempts attempt
             JOIN orchestration_jobs job ON job.job_id=attempt.job_id
             WHERE job.parent_thread_id=?1 AND job.agent_id=?2",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let receipt_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM delivery_receipts receipt
             JOIN job_attempts attempt ON attempt.attempt_id=receipt.attempt_id
             JOIN orchestration_jobs job ON job.job_id=attempt.job_id
             WHERE job.parent_thread_id=?1 AND job.agent_id=?2",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let review_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM review_decisions review
             JOIN job_attempts attempt ON attempt.attempt_id=review.attempt_id
             JOIN orchestration_jobs job ON job.job_id=attempt.job_id
             WHERE job.parent_thread_id=?1 AND job.agent_id=?2",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let mut statement = stage(
        connection.prepare(
            "SELECT attempt.route_action, decision.reason_code, decision.candidate_thread_id,
                    job.job_id, job.task_scope_key, job.state
             FROM job_attempts attempt
             JOIN orchestration_jobs job ON job.job_id=attempt.job_id
             JOIN agent_schedule_decisions decision ON decision.id=attempt.schedule_decision_id
             WHERE job.parent_thread_id=?1 AND job.agent_id=?2
             ORDER BY attempt.created_at, attempt.attempt_id",
        ),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let routes = stage(
        statement.query_map(params![parent_thread_id, agent_id], |row| {
            Ok(json!({
                "action": row.get::<_, String>(0)?,
                "reasonCode": row.get::<_, String>(1)?,
                "candidateThreadId": row.get::<_, Option<String>>(2)?,
                "jobId": row.get::<_, String>(3)?,
                "taskScopeKey": row.get::<_, String>(4)?,
                "jobState": row.get::<_, String>(5)?,
            }))
        }),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    let routes = stage(
        routes.collect::<Result<Vec<_>, _>>(),
        "BENCHMARK_EVIDENCE_QUERY_FAILED",
    )?;
    Ok(json!({
        "childCount": child_count,
        "jobCount": job_count,
        "completedJobCount": completed_job_count,
        "attemptCount": attempt_count,
        "receiptCount": receipt_count,
        "reviewCount": review_count,
        "routes": routes,
    }))
}

fn wait_for_primary_idle(
    bridge: &RuntimeBridgeService,
    thread_id: &str,
    timeout: Duration,
) -> Result<(), Rc1Failure> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
        let session = status
            .managed_sessions
            .iter()
            .find(|session| session.thread_id == thread_id)
            .ok_or_else(|| Rc1Failure::new("MANAGED_SESSION_MISSING", "托管 Primary 不存在"))?;
        match session.status {
            ManagedSessionStatus::Idle => return Ok(()),
            ManagedSessionStatus::Running if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(100));
            }
            ManagedSessionStatus::Running => {
                return Err(Rc1Failure::new(
                    "BENCHMARK_TURN_TIMEOUT",
                    format!(
                        "Primary Turn 在 {} 秒内未完成；{}",
                        timeout.as_secs(),
                        primary_summary(bridge, thread_id)
                    ),
                ));
            }
            other => {
                return Err(Rc1Failure::new(
                    "BENCHMARK_TURN_FAILED",
                    format!(
                        "Primary Turn 结束状态为 {other:?}；{}",
                        primary_summary(bridge, thread_id)
                    ),
                ));
            }
        }
    }
}

fn run_acceptance(workspace: &Path, task: &str) -> Result<Value, Rc1Failure> {
    let mut command = Command::new("node");
    command.arg("--test");
    command.arg("acceptance/task-1a.test.mjs");
    if task == "1B" {
        command.arg("acceptance/task-1b.test.mjs");
    }
    let started = Instant::now();
    let output = stage(
        command.current_dir(workspace).output(),
        "BENCHMARK_ACCEPTANCE_START_FAILED",
    )?;
    Ok(json!({
        "passed": output.status.success(),
        "exitCode": output.status.code(),
        "durationMs": started.elapsed().as_millis(),
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
    }))
}

fn optional_token_json(snapshot: Option<&TokenSnapshot>) -> Value {
    snapshot.map(TokenSnapshot::to_json).unwrap_or(Value::Null)
}

fn token_delta_json(current: Option<&TokenSnapshot>, previous: Option<&TokenSnapshot>) -> Value {
    match (current, previous) {
        (Some(current), Some(previous)) => current.delta_json(previous),
        _ => Value::Null,
    }
}

fn child_tokens_json(snapshots: &BTreeMap<String, TokenSnapshot>) -> Value {
    json!(
        snapshots
            .iter()
            .map(|(thread_id, snapshot)| json!({
                "threadId": thread_id,
                "tokens": snapshot.to_json(),
            }))
            .collect::<Vec<_>>()
    )
}

fn sum_child_field(
    snapshots: &BTreeMap<String, TokenSnapshot>,
    field: impl Fn(&TokenSnapshot) -> i64,
) -> i64 {
    snapshots.values().map(field).sum()
}

fn child_token_delta_json(
    current: &BTreeMap<String, TokenSnapshot>,
    previous: &BTreeMap<String, TokenSnapshot>,
) -> Value {
    json!({
        "inputTokens": sum_child_field(current, |s| s.input_tokens) - sum_child_field(previous, |s| s.input_tokens),
        "cachedInputTokens": sum_child_field(current, |s| s.cached_input_tokens) - sum_child_field(previous, |s| s.cached_input_tokens),
        "cacheWriteInputTokens": sum_child_field(current, |s| s.cache_write_input_tokens) - sum_child_field(previous, |s| s.cache_write_input_tokens),
        "outputTokens": sum_child_field(current, |s| s.output_tokens) - sum_child_field(previous, |s| s.output_tokens),
        "reasoningOutputTokens": sum_child_field(current, |s| s.reasoning_output_tokens) - sum_child_field(previous, |s| s.reasoning_output_tokens),
        "totalTokens": sum_child_field(current, |s| s.total_tokens) - sum_child_field(previous, |s| s.total_tokens),
    })
}

fn combined_total(
    primary: Option<&TokenSnapshot>,
    children: &BTreeMap<String, TokenSnapshot>,
) -> Option<i64> {
    primary.map(|snapshot| snapshot.total_tokens + sum_child_field(children, |s| s.total_tokens))
}

fn combined_delta(
    primary: Option<&TokenSnapshot>,
    previous_primary: Option<&TokenSnapshot>,
    children: &BTreeMap<String, TokenSnapshot>,
    previous_children: &BTreeMap<String, TokenSnapshot>,
) -> Option<i64> {
    let primary_delta = primary?.total_tokens - previous_primary?.total_tokens;
    let child_delta = sum_child_field(children, |s| s.total_tokens)
        - sum_child_field(previous_children, |s| s.total_tokens);
    Some(primary_delta + child_delta)
}

#[test]
fn benchmark_token_delta_includes_new_child_thread() {
    let snapshot = |total_tokens| TokenSnapshot {
        input_tokens: total_tokens,
        cached_input_tokens: 0,
        cache_write_input_tokens: 0,
        output_tokens: 0,
        reasoning_output_tokens: 0,
        total_tokens,
        current_context_tokens: None,
        model_context_window: None,
        cached_input_provided: Some(true),
        usage_status: "FINAL".to_owned(),
        source: "CODEX_APP_SERVER".to_owned(),
    };
    let first = BTreeMap::from([("child-a".to_owned(), snapshot(100))]);
    let second = BTreeMap::from([
        ("child-a".to_owned(), snapshot(100)),
        ("child-b".to_owned(), snapshot(200)),
    ]);
    assert_eq!(combined_total(Some(&snapshot(500)), &first), Some(600));
    assert_eq!(
        combined_delta(Some(&snapshot(600)), Some(&snapshot(500)), &second, &first),
        Some(300)
    );
    assert_eq!(child_token_delta_json(&second, &first)["totalTokens"], 200);
}

#[allow(clippy::too_many_arguments)]
fn run_benchmark_mode(
    run_id: &str,
    workspace: &Path,
    database_path: &Path,
    data_home: &Path,
    helper_path: &Path,
    codex_home: &Path,
    executable: &Path,
    agent: &ActiveAgent,
    primary_model: Option<&str>,
    primary_reasoning: Option<&str>,
    prompt_1a: &str,
    prompt_1b: &str,
    timeout: Duration,
) -> Result<Value, Rc1Failure> {
    let before = snapshot_files(workspace)?;
    let bridge = stage(
        RuntimeBridgeService::open(database_path, data_home, helper_path),
        "BRIDGE_OPEN_FAILED",
    )?;
    let outcome = (|| {
        stage(
            bridge.start_inner_for_e2e(executable, codex_home, None),
            "BRIDGE_START_FAILED",
        )?;
        let session = stage(
            bridge.managed_session_start_inner(ManagedSessionStartRequest {
                cwd: workspace.to_string_lossy().into_owned(),
                approval_policy: Some("on-request".to_owned()),
                sandbox: Some("workspace-write".to_owned()),
            }),
            "PRIMARY_START_FAILED",
        )?;
        let primary_thread_id = session.thread_id;
        let sandbox_policy = json!({
            "type": "workspaceWrite",
            "writableRoots": [
                workspace.to_string_lossy(),
                data_home.to_string_lossy()
            ]
        });

        let task_1a_started = Instant::now();
        stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: primary_thread_id.clone(),
                input: prompt_1a.to_owned(),
                effort: primary_reasoning.map(str::to_owned),
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: Some(sandbox_policy.clone()),
            }),
            "BENCHMARK_TASK_1A_START_FAILED",
        )?;
        wait_for_primary_idle(&bridge, &primary_thread_id, timeout)?;
        let task_1a_duration_ms = task_1a_started.elapsed().as_millis();
        thread::sleep(Duration::from_millis(250));
        let task_1a_acceptance = run_acceptance(workspace, "1A")?;
        let primary_1a = token_snapshot(database_path, &primary_thread_id)?;
        let child_1a_id = child_thread_id(database_path, &primary_thread_id, &agent.id)?;
        let child_1a = match child_1a_id.as_deref() {
            Some(thread_id) => token_snapshot(database_path, thread_id)?,
            None => None,
        };
        let children_1a = child_token_snapshots(database_path, &primary_thread_id, &agent.id)?;
        if task_1a_acceptance.get("passed").and_then(Value::as_bool) != Some(true) {
            return Err(Rc1Failure::new(
                "BENCHMARK_TASK_1A_FAILED",
                format!(
                    "{run_id} Task 1A 首次验收失败；{}",
                    primary_summary(&bridge, &primary_thread_id)
                ),
            ));
        }
        let task_1b_started = Instant::now();
        stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: primary_thread_id.clone(),
                input: prompt_1b.to_owned(),
                effort: primary_reasoning.map(str::to_owned),
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: Some(sandbox_policy),
            }),
            "BENCHMARK_TASK_1B_START_FAILED",
        )?;
        wait_for_primary_idle(&bridge, &primary_thread_id, timeout)?;
        let task_1b_duration_ms = task_1b_started.elapsed().as_millis();
        thread::sleep(Duration::from_millis(250));
        let task_1b_acceptance = run_acceptance(workspace, "1B")?;
        let primary_1b = token_snapshot(database_path, &primary_thread_id)?;
        let child_1b_id = child_thread_id(database_path, &primary_thread_id, &agent.id)?;
        let child_1b = match child_1b_id.as_deref() {
            Some(thread_id) => token_snapshot(database_path, thread_id)?,
            None => None,
        };
        let children_1b = child_token_snapshots(database_path, &primary_thread_id, &agent.id)?;
        let summary = primary_summary(&bridge, &primary_thread_id);
        let runtime = benchmark_runtime_evidence(database_path, &primary_thread_id, &agent.id)?;
        let after = snapshot_files(workspace)?;
        let violations = scope_violations(&before, &after);
        let range_changed = before.get("src/ranges.mjs") != after.get("src/ranges.mjs");
        let same_child = child_1a_id.is_some() && child_1a_id == child_1b_id;
        let task_1a_total = combined_total(primary_1a.as_ref(), &children_1a);
        let task_1b_total = combined_delta(
            primary_1b.as_ref(),
            primary_1a.as_ref(),
            &children_1b,
            &children_1a,
        );
        Ok(json!({
            "runId": run_id,
            "workspace": workspace,
            "primaryModel": primary_model,
            "primaryReasoning": primary_reasoning,
            "primaryThreadId": primary_thread_id,
            "childThreadIdAfterTask1A": child_1a_id,
            "childThreadIdAfterTask1B": child_1b_id,
            "sameChildThread": same_child,
            "task1A": {
                "durationMs": task_1a_duration_ms,
                "acceptance": task_1a_acceptance,
                "primaryTokens": optional_token_json(primary_1a.as_ref()),
                "childTokens": optional_token_json(child_1a.as_ref()),
                "allChildThreadTokens": child_tokens_json(&children_1a),
                "combinedTotalTokens": task_1a_total,
            },
            "task1B": {
                "durationMs": task_1b_duration_ms,
                "acceptance": task_1b_acceptance,
                "primaryTokensFinal": optional_token_json(primary_1b.as_ref()),
                "primaryTokenDelta": token_delta_json(primary_1b.as_ref(), primary_1a.as_ref()),
                "childTokensFinal": optional_token_json(child_1b.as_ref()),
                "allChildThreadTokensFinal": child_tokens_json(&children_1b),
                "childTokenDelta": child_token_delta_json(&children_1b, &children_1a),
                "combinedDeltaTokens": task_1b_total,
                "combinedTotalTokensFinal": combined_total(primary_1b.as_ref(), &children_1b),
            },
            "runtime": runtime,
            "scopeIntegrity": {
                "onlyRangesFileChanged": violations.is_empty() && range_changed,
                "rangesFileChanged": range_changed,
                "violations": violations,
            },
            "primarySummary": summary,
        }))
    })();
    let _ = bridge.stop_inner();
    outcome
}

#[allow(clippy::too_many_arguments)]
fn run_qualified_benchmark_mode(
    run_id: &str,
    workspace: &Path,
    database_path: &Path,
    data_home: &Path,
    helper_path: &Path,
    codex_home: &Path,
    executable: &Path,
    agent: &ActiveAgent,
    primary_model: Option<&str>,
    primary_reasoning: Option<&str>,
    prompt: &str,
    timeout: Duration,
) -> Result<Value, Rc1Failure> {
    let before = snapshot_files(workspace)?;
    let bridge = stage(
        RuntimeBridgeService::open(database_path, data_home, helper_path),
        "BRIDGE_OPEN_FAILED",
    )?;
    let outcome = (|| {
        stage(
            bridge.start_inner_for_e2e(executable, codex_home, None),
            "BRIDGE_START_FAILED",
        )?;
        let session = stage(
            bridge.managed_session_start_inner(ManagedSessionStartRequest {
                cwd: workspace.to_string_lossy().into_owned(),
                approval_policy: Some("on-request".to_owned()),
                sandbox: Some("workspace-write".to_owned()),
            }),
            "PRIMARY_START_FAILED",
        )?;
        let primary_thread_id = session.thread_id;
        let sandbox_policy = json!({
            "type": "workspaceWrite",
            "writableRoots": [workspace.to_string_lossy(), data_home.to_string_lossy()]
        });
        let started = Instant::now();
        stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: primary_thread_id.clone(),
                input: prompt.to_owned(),
                effort: primary_reasoning.map(str::to_owned),
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: Some(sandbox_policy),
            }),
            "BENCHMARK_QUALIFIED_START_FAILED",
        )?;
        wait_for_primary_idle(&bridge, &primary_thread_id, timeout)?;
        let duration_ms = started.elapsed().as_millis();
        thread::sleep(Duration::from_millis(250));
        let acceptance = run_acceptance(workspace, "1B")?;
        let primary_tokens = token_snapshot(database_path, &primary_thread_id)?;
        let child_thread_id = child_thread_id(database_path, &primary_thread_id, &agent.id)?;
        let child_tokens = child_token_snapshots(database_path, &primary_thread_id, &agent.id)?;
        let runtime = benchmark_runtime_evidence(database_path, &primary_thread_id, &agent.id)?;
        let after = snapshot_files(workspace)?;
        let violations = scope_violations(&before, &after);
        let range_changed = before.get("src/ranges.mjs") != after.get("src/ranges.mjs");
        Ok(json!({
            "runId": run_id,
            "workspace": workspace,
            "primaryModel": primary_model,
            "primaryReasoning": primary_reasoning,
            "primaryThreadId": primary_thread_id,
            "childThreadId": child_thread_id,
            "taskQualified": {
                "durationMs": duration_ms,
                "acceptance": acceptance,
                "primaryTokens": optional_token_json(primary_tokens.as_ref()),
                "allChildThreadTokens": child_tokens_json(&child_tokens),
                "combinedTotalTokens": combined_total(primary_tokens.as_ref(), &child_tokens),
            },
            "runtime": runtime,
            "scopeIntegrity": {
                "onlyRangesFileChanged": violations.is_empty() && range_changed,
                "rangesFileChanged": range_changed,
                "violations": violations,
            },
            "primarySummary": primary_summary(&bridge, &primary_thread_id),
        }))
    })();
    let _ = bridge.stop_inner();
    outcome
}

fn enforce_one_child_limit(connection: &Connection) -> Result<(), Rc1Failure> {
    stage(
        connection.execute_batch(
            "CREATE TABLE cas_e2e_child_admissions (id INTEGER PRIMARY KEY CHECK (id = 1));
             CREATE TRIGGER cas_e2e_one_child_limit
             BEFORE UPDATE OF admission_tool_use_id ON runtime_delegation_leases
             WHEN NEW.admission_tool_use_id IS NOT NULL
               AND OLD.admission_tool_use_id IS NULL
             BEGIN INSERT INTO cas_e2e_child_admissions (id) VALUES (1); END;",
        ),
        "ONE_CHILD_LIMIT_INSTALL_FAILED",
    )
}

#[test]
fn e2e_one_child_limit_rejects_second_admission() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE runtime_delegation_leases (
                id INTEGER PRIMARY KEY,
                admission_tool_use_id TEXT
            );
            INSERT INTO runtime_delegation_leases (id) VALUES (1), (2);",
        )
        .unwrap();
    enforce_one_child_limit(&connection).unwrap();
    connection
        .execute(
            "UPDATE runtime_delegation_leases SET admission_tool_use_id='first' WHERE id=1",
            [],
        )
        .unwrap();
    assert!(
        connection
            .execute(
                "UPDATE runtime_delegation_leases SET admission_tool_use_id='second' WHERE id=2",
                [],
            )
            .is_err()
    );
}

fn run_efficiency_pair() -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let cleanup = TempRoot(root.clone());
    let variant = env::var("CAS_E2E_PILOT_VARIANT").unwrap_or_else(|_| "SMALL".to_owned());
    let qualified = match variant.as_str() {
        "SMALL" => false,
        "QUALIFIED" => true,
        _ => {
            return Err(Rc1Failure::new(
                "E2E_CONFIGURATION_INVALID",
                format!("CAS_E2E_PILOT_VARIANT 仅支持 SMALL/QUALIFIED，实际为 {variant}"),
            ));
        }
    };
    let source_database = required_path("CAS_E2E_SOURCE_DATABASE_PATH")?;
    let source_codex_home = required_path("CAS_E2E_SOURCE_CODEX_HOME")?;
    let helper_source = required_path("CAS_E2E_HELPER_PATH")?;
    let database_path = required_path("CAS_DATABASE_PATH")?;
    let benchmark_template = required_path("CAS_E2E_BENCHMARK_TEMPLATE")?;
    let benchmark_root = required_path("CAS_E2E_BENCHMARK_ROOT")?;
    let codex_home = root.join("codex-home");
    let data_home = root.join("cas-data");
    let helper_home = root.join("cas-runtime");
    stage(fs::create_dir_all(&codex_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&data_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&helper_home), "TEMP_DIRECTORY_FAILED")?;
    stage(
        fs::create_dir_all(&benchmark_root),
        "BENCHMARK_DIRECTORY_FAILED",
    )?;
    let helper_path = helper_home.join("cas-helper.exe");
    stage(fs::copy(&helper_source, &helper_path), "HELPER_COPY_FAILED")?;
    clone_database(&source_database, &database_path)?;
    let connection = stage(Connection::open(&database_path), "E2E_DATABASE_FAILED")?;
    reset_e2e_database(&connection, &codex_home)?;
    if qualified {
        enforce_one_child_limit(&connection)?;
    }
    let agent = active_agent(&connection)?;
    drop(connection);
    copy_runtime_identity(&source_codex_home, &codex_home)?;
    let (primary_config, primary_model, primary_reasoning) =
        isolated_primary_config(&source_codex_home)?;
    stage(
        fs::write(codex_home.join("config.toml"), primary_config),
        "NON_INTERACTIVE_CONFIG_FAILED",
    )?;
    let prompt_1a = stage(
        fs::read_to_string(benchmark_template.join(if qualified {
            "prompts/task-qualified.txt"
        } else {
            "prompts/task-1a.txt"
        })),
        "BENCHMARK_PROMPT_READ_FAILED",
    )?;
    let prompt_1b = if qualified {
        String::new()
    } else {
        stage(
            fs::read_to_string(benchmark_template.join("prompts/task-1b.txt")),
            "BENCHMARK_PROMPT_READ_FAILED",
        )?
    };
    let selected_runs = env::var("CAS_E2E_PILOT_RUNS").unwrap_or_else(|_| "PAIR".to_owned());
    let (run_off, run_on) = match selected_runs.as_str() {
        "PAIR" => (true, true),
        "OFF" => (true, false),
        "ON" => (false, true),
        _ => {
            return Err(Rc1Failure::new(
                "E2E_CONFIGURATION_INVALID",
                format!("CAS_E2E_PILOT_RUNS 仅支持 PAIR/OFF/ON，实际为 {selected_runs}"),
            ));
        }
    };
    let off_workspace = benchmark_root.join("OFF-01");
    let on_workspace = benchmark_root.join("ON-01");
    if (run_off && off_workspace.exists()) || (run_on && on_workspace.exists()) {
        return Err(Rc1Failure::new(
            "BENCHMARK_RUN_EXISTS",
            format!("基准目录禁止覆盖：{}", benchmark_root.display()),
        ));
    }
    if run_off {
        copy_directory(&benchmark_template, &off_workspace)?;
    }
    if run_on {
        copy_directory(&benchmark_template, &on_workspace)?;
    }
    let configuration = ConfigurationService::for_e2e(
        database_path.clone(),
        data_home.clone(),
        codex_home.clone(),
        helper_path.clone(),
    );
    let timeout_seconds = env::var("CAS_E2E_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 30)
        .unwrap_or(300);
    let timeout = Duration::from_secs(timeout_seconds);
    let executable =
        PathBuf::from(env::var("CAS_E2E_CODEX_EXECUTABLE").unwrap_or_else(|_| "codex".to_owned()));

    let outcome = (|| {
        let off = if run_off {
            let default_request: RuntimeModeSwitchRequest = stage(
                serde_json::from_value(json!({ "activeAgentIds": [] })),
                "CONFIGURATION_REQUEST_FAILED",
            )?;
            stage(
                configuration.switch_runtime_mode(default_request),
                "DEFAULT_MODE_APPLY_FAILED",
            )?;
            if qualified {
                run_qualified_benchmark_mode(
                    "OFF-01",
                    &off_workspace,
                    &database_path,
                    &data_home,
                    &helper_path,
                    &codex_home,
                    &executable,
                    &agent,
                    primary_model.as_deref(),
                    primary_reasoning.as_deref(),
                    &prompt_1a,
                    timeout,
                )?
            } else {
                run_benchmark_mode(
                    "OFF-01",
                    &off_workspace,
                    &database_path,
                    &data_home,
                    &helper_path,
                    &codex_home,
                    &executable,
                    &agent,
                    primary_model.as_deref(),
                    primary_reasoning.as_deref(),
                    &prompt_1a,
                    &prompt_1b,
                    timeout,
                )?
            }
        } else {
            Value::Null
        };

        let (on, thin_entry_verified, thin_entry) = if run_on {
            let agent_request: RuntimeModeSwitchRequest = stage(
                serde_json::from_value(json!({ "activeAgentIds": [agent.id.clone()] })),
                "CONFIGURATION_REQUEST_FAILED",
            )?;
            stage(
                configuration.switch_runtime_mode(agent_request),
                "AGENT_MODE_APPLY_FAILED",
            )?;
            let rules_path = codex_home.join("cas/CAS_ORCHESTRATION.md");
            let rules = stage(
                fs::read_to_string(&rules_path),
                "ORCHESTRATION_RULES_READ_FAILED",
            )?;
            let config = stage(
                fs::read_to_string(codex_home.join("config.toml")),
                "AGENT_MODE_CONFIG_READ_FAILED",
            )?;
            let global_path = ["AGENTS.override.md", "AGENTS.md"]
                .into_iter()
                .map(|name| codex_home.join(name))
                .find(|path| path.is_file())
                .ok_or_else(|| {
                    Rc1Failure::new(
                        "GLOBAL_INSTRUCTIONS_MISSING",
                        "Agent Apply 后没有找到全局兼容入口",
                    )
                })?;
            stage(
                fs::remove_file(&global_path),
                "GLOBAL_INSTRUCTIONS_REMOVE_FAILED",
            )?;
            let verified = config.contains(ORCHESTRATION_RUNTIME_CONTRACT)
                && config.contains("CAS_ORCHESTRATION.md")
                && !config.contains("H job-plan")
                && rules.contains("H job-plan")
                && !global_path.exists();
            let evidence = json!({
                "verified": verified,
                "configContainsShortBootstrap": config.contains(ORCHESTRATION_RUNTIME_CONTRACT)
                    && config.contains("CAS_ORCHESTRATION.md") && !config.contains("H job-schedule"),
                "standaloneRulesContainProtocol": rules.contains("H job-schedule"),
                "globalAgentsFallbackRemoved": !global_path.exists(),
                "rulesPath": rules_path,
                "removedGlobalPath": global_path,
            });
            let run = if qualified {
                run_qualified_benchmark_mode(
                    "ON-01",
                    &on_workspace,
                    &database_path,
                    &data_home,
                    &helper_path,
                    &codex_home,
                    &executable,
                    &agent,
                    primary_model.as_deref(),
                    primary_reasoning.as_deref(),
                    &prompt_1a,
                    timeout,
                )?
            } else {
                run_benchmark_mode(
                    "ON-01",
                    &on_workspace,
                    &database_path,
                    &data_home,
                    &helper_path,
                    &codex_home,
                    &executable,
                    &agent,
                    primary_model.as_deref(),
                    primary_reasoning.as_deref(),
                    &prompt_1a,
                    &prompt_1b,
                    timeout,
                )?
            };
            (run, verified, evidence)
        } else {
            (Value::Null, true, json!({ "skipped": true }))
        };

        let off_success = !run_off
            || (qualified
                && off
                    .pointer("/taskQualified/acceptance/passed")
                    .and_then(Value::as_bool)
                    == Some(true)
                && off
                    .pointer("/scopeIntegrity/onlyRangesFileChanged")
                    .and_then(Value::as_bool)
                    == Some(true)
                && off.pointer("/runtime/childCount").and_then(Value::as_i64) == Some(0))
            || (!qualified
                && off
                    .pointer("/task1A/acceptance/passed")
                    .and_then(Value::as_bool)
                    == Some(true)
                && off
                    .pointer("/task1B/acceptance/passed")
                    .and_then(Value::as_bool)
                    == Some(true)
                && off
                    .pointer("/scopeIntegrity/onlyRangesFileChanged")
                    .and_then(Value::as_bool)
                    == Some(true)
                && off.pointer("/runtime/childCount").and_then(Value::as_i64) == Some(0));
        let on_success = !run_on
            || (qualified
                && on
                    .pointer("/taskQualified/acceptance/passed")
                    .and_then(Value::as_bool)
                    == Some(true)
                && on
                    .pointer("/scopeIntegrity/onlyRangesFileChanged")
                    .and_then(Value::as_bool)
                    == Some(true)
                && on.pointer("/runtime/childCount").and_then(Value::as_i64) == Some(1)
                && on
                    .pointer("/runtime/completedJobCount")
                    .and_then(Value::as_i64)
                    == Some(1)
                && on
                    .pointer("/runtime/reviewCount")
                    .and_then(Value::as_i64)
                    .is_some_and(|count| count >= 1)
                && on
                    .pointer("/childThreadId")
                    .and_then(Value::as_str)
                    .is_some())
            || (!qualified
                && on
                    .pointer("/task1A/acceptance/passed")
                    .and_then(Value::as_bool)
                    == Some(true)
                && on
                    .pointer("/task1B/acceptance/passed")
                    .and_then(Value::as_bool)
                    == Some(true)
                && on
                    .pointer("/scopeIntegrity/onlyRangesFileChanged")
                    .and_then(Value::as_bool)
                    == Some(true)
                && on.pointer("/runtime/childCount").and_then(Value::as_i64) == Some(0)
                && on.pointer("/runtime/jobCount").and_then(Value::as_i64) == Some(0));
        let off_evidence_complete = !run_off
            || off
                .pointer(if qualified {
                    "/taskQualified/primaryTokens"
                } else {
                    "/task1B/primaryTokensFinal"
                })
                .is_some_and(|value| !value.is_null());
        let on_evidence_complete = !run_on
            || on
                .pointer(if qualified {
                    "/taskQualified/primaryTokens"
                } else {
                    "/task1B/primaryTokensFinal"
                })
                .is_some_and(|value| !value.is_null());
        let evidence_complete = off_evidence_complete && on_evidence_complete;
        let status = if off_success && on_success && evidence_complete && thin_entry_verified {
            "PASS"
        } else {
            "FAIL"
        };
        Ok(json!({
            "status": status,
            "scope": format!("EFFICIENCY_{variant}_PILOT_{selected_runs}"),
            "conclusionLimit": if qualified {
                "单个配对样本仅验证本夹具的委派行为与成本，不能外推到全部任务。"
            } else {
                "此夹具用于验证小任务不委派；单个配对样本不能证明大任务的委派收益。"
            },
            "primaryModel": primary_model,
            "primaryReasoning": primary_reasoning,
            "childAgent": {
                "key": agent.key,
                "name": agent.name,
                "model": agent.model,
                "provider": agent.provider,
                "reasoningPolicy": agent.reasoning_policy,
                "modelDefaultReasoning": agent.model_default_reasoning,
            },
            "thinEntry": thin_entry,
            "off": off,
            "on": on,
            "checks": {
                "offSuccessful": off_success,
                "onSuccessful": on_success,
                "evidenceComplete": evidence_complete,
            },
            "benchmarkRoot": benchmark_root,
            "isolatedRoot": root,
            "oneChildLimitInstalled": qualified,
        }))
    })();
    let passed = outcome
        .as_ref()
        .ok()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("PASS");
    if passed && !qualified {
        drop(cleanup);
    } else {
        std::mem::forget(cleanup);
    }
    outcome
}

fn timeout_evidence(database_path: &Path, parent_thread_id: &str) -> String {
    let Ok(connection) = Connection::open(database_path) else {
        return "evidence=unavailable".to_owned();
    };
    let decisions = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(group_concat(decision || ':' || reason_code, ','), '')
             FROM agent_schedule_decisions WHERE parent_thread_id = ?1",
            [parent_thread_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap_or_default();
    let children = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(group_concat(status, ','), '')
             FROM agent_thread_instances WHERE parent_thread_id = ?1",
            [parent_thread_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap_or_default();
    let hooks = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(group_concat(decision || ':' || reason_code, ','), '')
             FROM runtime_enforcement_events WHERE session_id = ?1",
            [parent_thread_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap_or_default();
    let leases = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(group_concat(
                 state || ':admitted=' || (admission_tool_use_id IS NOT NULL)
                 || ':confirmed=' || (admission_confirmed_at IS NOT NULL), ','), '')
             FROM runtime_delegation_leases WHERE parent_thread_id = ?1",
            [parent_thread_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .unwrap_or_default();
    format!(
        "decisions={} [{}], children={} [{}], hooks={} [{}], leases={} [{}]",
        decisions.0, decisions.1, children.0, children.1, hooks.0, hooks.1, leases.0, leases.1
    )
}

fn collect_thread_evidence(value: &Value, messages: &mut Vec<String>, commands: &mut Vec<String>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect_thread_evidence(value, messages, commands);
            }
        }
        Value::Object(object) => {
            match object.get("type").and_then(Value::as_str) {
                Some("agentMessage") => {
                    if let Some(text) = object.get("text").and_then(Value::as_str) {
                        messages.push(text.to_owned());
                    }
                }
                Some("commandExecution") => {
                    commands.push(format!(
                        "status={}, exitCode={}, output={}",
                        object
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown"),
                        object
                            .get("exitCode")
                            .map(Value::to_string)
                            .unwrap_or_else(|| "null".to_owned()),
                        object
                            .get("aggregatedOutput")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .replace(['\r', '\n'], " ")
                            .chars()
                            .take(500)
                            .collect::<String>()
                    ));
                }
                _ => {}
            }
            for value in object.values() {
                collect_thread_evidence(value, messages, commands);
            }
        }
        _ => {}
    }
}

fn primary_summary(bridge: &RuntimeBridgeService, thread_id: &str) -> String {
    let Ok(thread) = bridge.request(
        AppServerMethod::ThreadRead.as_str(),
        json!({"threadId": thread_id, "includeTurns": true}),
    ) else {
        return "primaryMessage=unavailable".to_owned();
    };
    let mut messages = Vec::new();
    let mut commands = Vec::new();
    collect_thread_evidence(&thread, &mut messages, &mut commands);
    let Some(message) = messages.last() else {
        return "primaryMessage=missing".to_owned();
    };
    let message = message.replace(['\r', '\n'], " ");
    let summary = message.chars().take(800).collect::<String>();
    let command = commands
        .last()
        .map(|command| format!("; lastCommand={command}"))
        .unwrap_or_default();
    format!("primaryMessage={summary}{command}")
}

fn thread_turn_evidence(thread: &Value, turn_id: &str) -> Option<(usize, usize)> {
    crate::runtime_adapter::thread_turn_evidence(thread, turn_id)
}

fn verify_output(
    bridge: &RuntimeBridgeService,
    database_path: &Path,
    parent_thread_id: &str,
    path: &Path,
    expected: &str,
    missing_code: &'static str,
    invalid_code: &'static str,
) -> Result<(), Rc1Failure> {
    let content = fs::read_to_string(path).map_err(|error| {
        Rc1Failure::new(
            missing_code,
            format!(
                "{error}; {}; {}",
                timeout_evidence(database_path, parent_thread_id),
                primary_summary(bridge, parent_thread_id)
            ),
        )
    })?;
    if content.trim() != expected {
        return Err(Rc1Failure::new(
            invalid_code,
            format!(
                "输出内容不正确；{}; {}",
                timeout_evidence(database_path, parent_thread_id),
                primary_summary(bridge, parent_thread_id)
            ),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum TurnCompletionMode {
    Native,
    UpstreamStallRecovery,
}

impl TurnCompletionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Native => "NATIVE",
            Self::UpstreamStallRecovery => "UPSTREAM_STALL_RECOVERY",
        }
    }
}

fn child_is_idle(
    database_path: &Path,
    parent_thread_id: &str,
    agent_id: &str,
) -> Result<bool, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    stage(
        connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM agent_thread_instances
                 WHERE parent_thread_id = ?1 AND agent_id = ?2 AND status = 'IDLE'
             )",
            params![parent_thread_id, agent_id],
            |row| row.get(0),
        ),
        "EVIDENCE_QUERY_FAILED",
    )
}

fn job_is_completed(database_path: &Path, job_id: &str) -> Result<bool, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    stage(
        connection.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM orchestration_jobs
                 WHERE job_id = ?1 AND state = 'COMPLETED'
             )",
            [job_id],
            |row| row.get(0),
        ),
        "EVIDENCE_QUERY_FAILED",
    )
}

fn wait_for_turn(
    bridge: &RuntimeBridgeService,
    database_path: &Path,
    parent_thread_id: &str,
    turn_id: &str,
    agent_id: &str,
    job_id: &str,
    expected_output: &Path,
    timeout: Duration,
) -> Result<TurnCompletionMode, Rc1Failure> {
    let deadline = Instant::now() + timeout;
    let mut child_completed_at = None;
    loop {
        let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
        let session = status
            .managed_session
            .ok_or_else(|| Rc1Failure::new("MANAGED_SESSION_MISSING", "托管 Primary 不存在"))?;
        match session.status {
            ManagedSessionStatus::Idle => return Ok(TurnCompletionMode::Native),
            ManagedSessionStatus::Running if Instant::now() < deadline => {
                if expected_output.is_file()
                    && child_is_idle(database_path, parent_thread_id, agent_id)?
                    && job_is_completed(database_path, job_id)?
                {
                    let completed_at = *child_completed_at.get_or_insert_with(Instant::now);
                    if completed_at.elapsed() >= Duration::from_secs(5) {
                        stage(
                            bridge.request(
                                "turn/interrupt",
                                json!({"threadId": parent_thread_id, "turnId": turn_id}),
                            ),
                            "STALLED_TURN_INTERRUPT_FAILED",
                        )?;
                        let interrupt_deadline = Instant::now() + Duration::from_secs(10);
                        while Instant::now() < interrupt_deadline {
                            let status =
                                stage(bridge.status_inner(), "STALLED_TURN_INTERRUPT_FAILED")?;
                            if status.managed_session.as_ref().is_some_and(|session| {
                                session.status != ManagedSessionStatus::Running
                            }) {
                                return Ok(TurnCompletionMode::UpstreamStallRecovery);
                            }
                            thread::sleep(Duration::from_millis(100));
                        }
                        return Err(Rc1Failure::new(
                            "STALLED_TURN_INTERRUPT_FAILED",
                            "Job 已完成，但僵尸 Primary Turn 无法中断",
                        ));
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
            ManagedSessionStatus::Running => {
                return Err(Rc1Failure::new(
                    "TURN_TIMEOUT",
                    format!(
                        "Turn 在 {} 秒内未完成；lastEventAt={:?}, usageEvents={}, malformedEvents={}, {}; {}",
                        timeout.as_secs(),
                        status.last_event_at,
                        status.usage_event_count,
                        status.malformed_event_count,
                        timeout_evidence(database_path, parent_thread_id),
                        primary_summary(bridge, parent_thread_id)
                    ),
                ));
            }
            session_status => {
                return Err(Rc1Failure::new(
                    "TURN_FAILED",
                    format!(
                        "托管 Turn 结束状态为 {session_status:?}{}",
                        status
                            .last_error
                            .as_deref()
                            .map(|message| format!("：{message}"))
                            .unwrap_or_default()
                    ),
                ));
            }
        }
    }
}

#[derive(Debug)]
struct InstanceEvidence {
    thread_id: String,
    status: String,
    total_tokens: i64,
    runtime_fingerprint: Option<String>,
    task_scope_key: Option<String>,
    workspace_scope_key: String,
}

fn wait_for_instance(
    database_path: &Path,
    parent_thread_id: &str,
    agent_id: &str,
    timeout: Duration,
) -> Result<InstanceEvidence, Rc1Failure> {
    let deadline = Instant::now() + timeout;
    loop {
        let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
        let instance = stage(
            connection
                .query_row(
                    "SELECT codex_thread_id, status, total_tokens,
                            runtime_fingerprint, task_scope_key, scope_key
                     FROM agent_thread_instances
                     WHERE parent_thread_id = ?1 AND agent_id = ?2
                     ORDER BY last_used_at DESC LIMIT 1",
                    params![parent_thread_id, agent_id],
                    |row| {
                        Ok(InstanceEvidence {
                            thread_id: row.get(0)?,
                            status: row.get(1)?,
                            total_tokens: row.get(2)?,
                            runtime_fingerprint: row.get(3)?,
                            task_scope_key: row.get(4)?,
                            workspace_scope_key: row.get(5)?,
                        })
                    },
                )
                .optional(),
            "EVIDENCE_QUERY_FAILED",
        )?;
        if let Some(instance) = instance
            && instance.status == "IDLE"
        {
            return Ok(instance);
        }
        if Instant::now() >= deadline {
            return Err(Rc1Failure::new(
                "CHILD_NOT_IDLE",
                "子 Agent 未在期限内进入 IDLE",
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn decision_evidence(
    connection: &Connection,
    parent_thread_id: &str,
) -> Result<Vec<(String, String, Option<String>)>, Rc1Failure> {
    let mut statement = stage(
        connection.prepare(
            "SELECT decision, reason_code, candidate_thread_id
             FROM agent_schedule_decisions
             WHERE parent_thread_id = ?1 AND task_scope_key = ?2
             ORDER BY rowid",
        ),
        "DECISION_QUERY_FAILED",
    )?;
    let rows = stage(
        statement.query_map(params![parent_thread_id, TASK_SCOPE_KEY], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }),
        "DECISION_QUERY_FAILED",
    )?;
    stage(rows.collect::<Result<Vec<_>, _>>(), "DECISION_QUERY_FAILED")
}

#[derive(Debug)]
struct RuntimeFirstEvidence {
    job_ids: Vec<String>,
    attempt_count: i64,
    receipt_count: i64,
    review_count: i64,
    released_lease_count: i64,
    tracking_count: usize,
}

fn runtime_first_evidence(
    database_path: &Path,
    connection: &Connection,
    parent_thread_id: &str,
    agent_id: &str,
    workspace_scope_key: &str,
) -> Result<RuntimeFirstEvidence, Rc1Failure> {
    let mut statement = stage(
        connection.prepare(
            "SELECT job_id
             FROM orchestration_jobs
             WHERE parent_thread_id=?1 AND agent_id=?2
               AND workspace_scope_key=?3 AND task_scope_key=?4
             ORDER BY job_id",
        ),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    let job_ids = stage(
        statement.query_map(
            params![
                parent_thread_id,
                agent_id,
                workspace_scope_key,
                TASK_SCOPE_KEY
            ],
            |row| row.get::<_, String>(0),
        ),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    let job_ids = stage(
        job_ids.collect::<Result<Vec<_>, _>>(),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    if job_ids != ["cas-rc1-first", "cas-rc1-second"] {
        return Err(Rc1Failure::new(
            "RUNTIME_FIRST_JOB_CHAIN_INVALID",
            format!("期望两个确定 Job，实际为 {job_ids:?}"),
        ));
    }

    let invalid_jobs: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*)
             FROM orchestration_jobs
             WHERE job_id IN ('cas-rc1-first', 'cas-rc1-second')
               AND (
                   state <> 'COMPLETED'
                   OR json_extract(task_packet, '$.agent_id') <> agent_id
                   OR json_extract(task_packet, '$.parent_thread_id') <> parent_thread_id
                   OR json_extract(task_packet, '$.workspace_scope_key') <> workspace_scope_key
                   OR json_extract(task_packet, '$.task_scope_key') <> task_scope_key
                   OR json_extract(task_packet, '$.execution_kind_policy') <> 'NATIVE_CHILD_REQUIRED'
               )",
            [],
            |row| row.get(0),
        ),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    let (attempt_count, spawn_count, reuse_count, successful_native_count): (i64, i64, i64, i64) =
        stage(
            connection.query_row(
                "SELECT COUNT(*),
                    SUM(CASE WHEN attempt.route_action='SPAWN' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN attempt.route_action='REUSE' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN attempt.state='SUCCEEDED'
                                  AND attempt.planned_execution_kind='NATIVE_CHILD'
                                  AND attempt.execution_kind='NATIVE_CHILD'
                             THEN 1 ELSE 0 END)
             FROM job_attempts attempt
             WHERE attempt.job_id IN ('cas-rc1-first', 'cas-rc1-second')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ),
            "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
        )?;
    let (receipt_count, receipt_stage_count): (i64, i64) = stage(
        connection.query_row(
            "SELECT COUNT(*), COUNT(DISTINCT stage)
             FROM delivery_receipts
             WHERE job_id IN ('cas-rc1-first', 'cas-rc1-second')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    let review_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM review_decisions
             WHERE job_id IN ('cas-rc1-first', 'cas-rc1-second')
               AND decision='APPROVE'",
            [],
            |row| row.get(0),
        ),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    let released_lease_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*)
             FROM runtime_delegation_leases lease
             JOIN job_attempts attempt ON attempt.lease_id=lease.id
             WHERE attempt.job_id IN ('cas-rc1-first', 'cas-rc1-second')
               AND lease.state='RELEASED'",
            [],
            |row| row.get(0),
        ),
        "RUNTIME_FIRST_EVIDENCE_QUERY_FAILED",
    )?;
    if invalid_jobs != 0
        || attempt_count != 2
        || spawn_count != 1
        || reuse_count != 1
        || successful_native_count != 2
        || receipt_count != 8
        || receipt_stage_count != 4
        || review_count != 2
        || released_lease_count != 2
    {
        return Err(Rc1Failure::new(
            "RUNTIME_FIRST_JOB_CHAIN_INVALID",
            format!(
                "invalidJobs={invalid_jobs}, attempts={attempt_count}, spawn={spawn_count}, reuse={reuse_count}, nativeSuccess={successful_native_count}, receipts={receipt_count}/{receipt_stage_count}, reviews={review_count}, releasedLeases={released_lease_count}"
            ),
        ));
    }

    let tracking = stage(
        OrchestrationJobService::open(database_path),
        "RUNTIME_FIRST_TRACKING_QUERY_FAILED",
    )?;
    let tracking = tracking
        .list_tracking(OrchestrationJobListRequest {
            workspace_scope_key: Some(workspace_scope_key.to_owned()),
            agent_id: Some(agent_id.to_owned()),
            page: 0,
            page_size: 100,
        })
        .map_err(|error| {
            Rc1Failure::new("RUNTIME_FIRST_TRACKING_QUERY_FAILED", format!("{error:?}"))
        })?;
    let tracked_jobs = tracking
        .jobs
        .iter()
        .filter(|job| {
            job.parent_thread_id == parent_thread_id && job.task_scope_key == TASK_SCOPE_KEY
        })
        .collect::<Vec<_>>();
    if tracked_jobs.len() != 2
        || tracked_jobs.iter().any(|job| {
            job.state != "COMPLETED"
                || job.attempts.len() != 1
                || job.attempts[0].planned_execution_kind != "NATIVE_CHILD"
                || job.attempts[0].execution_kind.as_deref() != Some("NATIVE_CHILD")
                || job.attempts[0].receipt_stage.as_deref() != Some("PARENT_ACKNOWLEDGED")
                || job.attempts[0].review_decision.as_deref() != Some("APPROVE")
        })
    {
        return Err(Rc1Failure::new(
            "RUNTIME_FIRST_TRACKING_MISMATCH",
            "UI 查询 DTO 与数据库中的 Job/Attempt/Receipt/Review 不一致",
        ));
    }

    Ok(RuntimeFirstEvidence {
        job_ids,
        attempt_count,
        receipt_count,
        review_count,
        released_lease_count,
        tracking_count: tracked_jobs.len(),
    })
}

#[derive(Clone, Debug)]
struct ScheduleInvocation {
    helper_path: PathBuf,
    database_path: PathBuf,
    codex_home: PathBuf,
    workspace: PathBuf,
    workspace_scope_key: String,
    parent_thread_id: String,
    agent_key: String,
    task_scope_key: String,
}

#[derive(Debug)]
struct ScheduleEvidence {
    decision: String,
    thread_id: Option<String>,
    reason_code: String,
}

impl ScheduleEvidence {
    fn to_json(&self) -> Value {
        json!({
            "decision": self.decision,
            "threadId": self.thread_id,
            "reasonCode": self.reason_code
        })
    }
}

fn parse_schedule_protocol(line: &str) -> Result<ScheduleEvidence, Rc1Failure> {
    let fields = line.trim().split('|').collect::<Vec<_>>();
    if fields.len() != 4 || fields[0] != "CAS1" {
        return Err(Rc1Failure::new(
            "RC2_PROTOCOL_INVALID",
            format!("cas-helper 返回了无效协议行：{line}"),
        ));
    }
    if !matches!(fields[1], "SPAWN" | "REUSE" | "WAIT") || fields[3].is_empty() {
        return Err(Rc1Failure::new(
            "RC2_PROTOCOL_INVALID",
            format!("cas-helper 返回了未知决策：{line}"),
        ));
    }
    let thread_id = (fields[2] != "-").then(|| fields[2].to_owned());
    if (fields[1] == "REUSE") != thread_id.is_some() {
        return Err(Rc1Failure::new(
            "RC2_PROTOCOL_INVALID",
            format!("cas-helper 决策与 Thread 字段不一致：{line}"),
        ));
    }
    Ok(ScheduleEvidence {
        decision: fields[1].to_owned(),
        thread_id,
        reason_code: fields[3].to_owned(),
    })
}

fn invoke_schedule(invocation: &ScheduleInvocation) -> Result<ScheduleEvidence, Rc1Failure> {
    let output = stage(
        Command::new(&invocation.helper_path)
            .arg("schedule")
            .arg(&invocation.database_path)
            .arg(&invocation.agent_key)
            .arg(&invocation.workspace_scope_key)
            .arg(&invocation.task_scope_key)
            .env("CODEX_HOME", &invocation.codex_home)
            .env("CODEX_THREAD_ID", &invocation.parent_thread_id)
            .current_dir(&invocation.workspace)
            .output(),
        "RC2_HELPER_LAUNCH_FAILED",
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if !output.status.success() {
        return Err(Rc1Failure::new(
            "RC2_HELPER_FAILED",
            format!(
                "cas-helper 退出码 {:?}；stdout={stdout}；stderr={stderr}",
                output.status.code()
            ),
        ));
    }
    parse_schedule_protocol(&stdout)
}

fn invoke_concurrent_schedule(
    invocation: &ScheduleInvocation,
) -> Result<Vec<ScheduleEvidence>, Rc1Failure> {
    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let invocation = invocation.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            invoke_schedule(&invocation)
        }));
    }
    barrier.wait();
    handles
        .into_iter()
        .map(|handle| {
            handle.join().map_err(|_| {
                Rc1Failure::new("RC2_CONCURRENT_PROBE_FAILED", "并发 helper 线程异常退出")
            })?
        })
        .collect()
}

fn release_rc2_preflight_probe(
    database_path: &Path,
    agent_id: &str,
    parent_thread_id: &str,
    workspace_scope_key: &str,
    task_scope_key: &str,
) -> Result<(), Rc1Failure> {
    let mut connection = stage(Connection::open(database_path), "RC2_PROBE_CLEANUP_FAILED")?;
    let transaction = stage(connection.transaction(), "RC2_PROBE_CLEANUP_FAILED")?;
    let released = stage(
        transaction.execute(
            "UPDATE runtime_delegation_leases
             SET state = 'RELEASED',
                 released_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                 release_reason = 'RC2_PREFLIGHT_PROBE_COMPLETE'
             WHERE agent_id = ?1 AND parent_thread_id = ?2
               AND workspace_scope_key = ?3 AND task_scope_key = ?4
               AND state = 'PENDING' AND codex_agent_id IS NULL
               AND admission_tool_use_id IS NULL
               AND NOT EXISTS (
                   SELECT 1 FROM job_attempts attempt
                   WHERE attempt.lease_id = runtime_delegation_leases.id
               )",
            params![
                agent_id,
                parent_thread_id,
                workspace_scope_key,
                task_scope_key
            ],
        ),
        "RC2_PROBE_CLEANUP_FAILED",
    )?;
    let reservations = stage(
        transaction.execute(
            "DELETE FROM agent_spawn_reservations
             WHERE agent_id = ?1 AND parent_thread_id = ?2
               AND workspace_scope_key = ?3 AND task_scope_key = ?4",
            params![
                agent_id,
                parent_thread_id,
                workspace_scope_key,
                task_scope_key
            ],
        ),
        "RC2_PROBE_CLEANUP_FAILED",
    )?;
    if released != 1 || reservations != 1 {
        return Err(Rc1Failure::new(
            "RC2_PROBE_CLEANUP_FAILED",
            format!("预检清理数量异常：releasedLeases={released}, reservations={reservations}"),
        ));
    }
    stage(transaction.commit(), "RC2_PROBE_CLEANUP_FAILED")?;
    Ok(())
}

fn codex_state_database(codex_home: &Path) -> Result<PathBuf, Rc1Failure> {
    let entries = stage(
        fs::read_dir(codex_home),
        "NATIVE_STATE_DATABASE_UNAVAILABLE",
    )?;
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u64>()
                .ok()?;
            Some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
        .ok_or_else(|| {
            Rc1Failure::new(
                "NATIVE_STATE_DATABASE_UNAVAILABLE",
                "隔离 CODEX_HOME 中没有 state_<version>.sqlite",
            )
        })
}

fn child_rollout_path(codex_home: &Path, child_thread_id: &str) -> Result<PathBuf, Rc1Failure> {
    let state_database = codex_state_database(codex_home)?;
    let connection = stage(
        Connection::open_with_flags(state_database, OpenFlags::SQLITE_OPEN_READ_ONLY),
        "NATIVE_STATE_DATABASE_UNAVAILABLE",
    )?;
    let rollout_path = stage(
        connection
            .query_row(
                "SELECT rollout_path FROM threads WHERE id = ?1",
                [child_thread_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional(),
        "NATIVE_ROLLOUT_QUERY_FAILED",
    )?
    .flatten()
    .filter(|path| !path.trim().is_empty())
    .ok_or_else(|| {
        Rc1Failure::new(
            "NATIVE_ROLLOUT_UNAVAILABLE",
            format!("Codex state DB 没有 Child {child_thread_id} 的 rollout_path"),
        )
    })?;
    let rollout_path = PathBuf::from(rollout_path);
    Ok(if rollout_path.is_absolute() {
        rollout_path
    } else {
        codex_home.join(rollout_path)
    })
}

fn append_context_pressure_probe(
    codex_home: &Path,
    child_thread_id: &str,
) -> Result<(PathBuf, i64), Rc1Failure> {
    let rollout_path = child_rollout_path(codex_home, child_thread_id)?;
    let state = stage(rollout_state(&rollout_path), "NATIVE_ROLLOUT_READ_FAILED")?;
    let context_window = state.model_context_window.ok_or_else(|| {
        Rc1Failure::new(
            "NATIVE_CONTEXT_WINDOW_UNAVAILABLE",
            "原生 Child rollout 没有 model_context_window，无法构造压力探针",
        )
    })?;
    let probe = json!({
        "type": "event_msg",
        "payload": {
            "type": "token_count",
            "info": {
                "last_token_usage": { "total_tokens": context_window },
                "model_context_window": context_window
            }
        }
    });
    let mut file = stage(
        OpenOptions::new().append(true).open(&rollout_path),
        "NATIVE_ROLLOUT_WRITE_FAILED",
    )?;
    stage(file.write_all(b"\n"), "NATIVE_ROLLOUT_WRITE_FAILED")?;
    stage(
        serde_json::to_writer(&mut file, &probe),
        "NATIVE_ROLLOUT_WRITE_FAILED",
    )?;
    stage(file.write_all(b"\n"), "NATIVE_ROLLOUT_WRITE_FAILED")?;
    stage(file.flush(), "NATIVE_ROLLOUT_WRITE_FAILED")?;
    Ok((rollout_path, context_window))
}

fn require_decision(
    evidence: &ScheduleEvidence,
    decision: &str,
    reason_code: Option<&str>,
    failure_code: &'static str,
) -> Result<(), Rc1Failure> {
    if evidence.decision != decision
        || reason_code.is_some_and(|reason| evidence.reason_code != reason)
    {
        return Err(Rc1Failure::new(
            failure_code,
            format!(
                "期望 {decision}/{:?}，实际为 {}/{}",
                reason_code, evidence.decision, evidence.reason_code
            ),
        ));
    }
    Ok(())
}

fn run_rc2_matrix(
    helper_path: &Path,
    database_path: &Path,
    codex_home: &Path,
    workspace: &Path,
    parent_thread_id: &str,
    agent: &ActiveAgent,
    instance: &InstanceEvidence,
    root: &Path,
) -> Result<Value, Rc1Failure> {
    let base = ScheduleInvocation {
        helper_path: helper_path.to_path_buf(),
        database_path: database_path.to_path_buf(),
        codex_home: codex_home.to_path_buf(),
        workspace: workspace.to_path_buf(),
        workspace_scope_key: instance.workspace_scope_key.clone(),
        parent_thread_id: parent_thread_id.to_owned(),
        agent_key: agent.key.clone(),
        task_scope_key: TASK_SCOPE_KEY.to_owned(),
    };

    let mut concurrent_invocation = base.clone();
    concurrent_invocation.task_scope_key = CONCURRENT_TASK_SCOPE_KEY.to_owned();
    let concurrent = invoke_concurrent_schedule(&concurrent_invocation)?;
    let spawn_count = concurrent
        .iter()
        .filter(|evidence| evidence.decision == "SPAWN")
        .count();
    let wait_count = concurrent
        .iter()
        .filter(|evidence| evidence.decision == "WAIT" && evidence.reason_code == "SPAWN_RESERVED")
        .count();
    if spawn_count != 1 || wait_count != 1 {
        return Err(Rc1Failure::new(
            "RC2_CONCURRENT_RESERVATION_FAILED",
            format!("期望 SPAWN=1/WAIT=1，实际 SPAWN={spawn_count}/WAIT={wait_count}"),
        ));
    }
    release_rc2_preflight_probe(
        database_path,
        &agent.id,
        parent_thread_id,
        &base.workspace_scope_key,
        CONCURRENT_TASK_SCOPE_KEY,
    )?;

    let alternate_workspace = root.join("workspace-other");
    stage(
        fs::create_dir_all(&alternate_workspace),
        "RC2_WORKSPACE_FIXTURE_FAILED",
    )?;
    let alternate_scope = normalize_workspace_scope_key(&alternate_workspace.to_string_lossy())
        .ok_or_else(|| {
            Rc1Failure::new("RC2_WORKSPACE_FIXTURE_FAILED", "无法规范化替代工作区路径")
        })?;
    let mut workspace_invocation = base.clone();
    workspace_invocation.workspace = alternate_workspace;
    workspace_invocation.workspace_scope_key = alternate_scope;
    let workspace_decision = invoke_schedule(&workspace_invocation)?;
    require_decision(
        &workspace_decision,
        "SPAWN",
        Some("NO_WORKSPACE_SCOPE_MATCH"),
        "RC2_WORKSPACE_ISOLATION_FAILED",
    )?;
    release_rc2_preflight_probe(
        database_path,
        &agent.id,
        parent_thread_id,
        &workspace_invocation.workspace_scope_key,
        TASK_SCOPE_KEY,
    )?;

    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    let original_instruction = stage(
        connection.query_row(
            "SELECT instruction FROM agents WHERE id = ?1",
            [&agent.id],
            |row| row.get::<_, String>(0),
        ),
        "RC2_FINGERPRINT_FIXTURE_FAILED",
    )?;
    stage(
        connection.execute(
            "UPDATE agents SET instruction = ?2 WHERE id = ?1",
            params![
                agent.id,
                format!("{original_instruction}\nRC2 fingerprint probe")
            ],
        ),
        "RC2_FINGERPRINT_FIXTURE_FAILED",
    )?;
    drop(connection);
    let fingerprint_result = invoke_schedule(&base);
    let restore_connection = stage(
        Connection::open(database_path),
        "RC2_FINGERPRINT_RESTORE_FAILED",
    )?;
    stage(
        restore_connection.execute(
            "UPDATE agents SET instruction = ?2 WHERE id = ?1",
            params![agent.id, original_instruction],
        ),
        "RC2_FINGERPRINT_RESTORE_FAILED",
    )?;
    stage(
        restore_connection.execute(
            "UPDATE agent_thread_instances
             SET reuse_state = 'ACTIVE', reuse_state_reason = NULL
             WHERE codex_thread_id = ?1 AND reuse_state = 'RETIRED'
               AND reuse_state_reason IN (
                   'RUNTIME_FINGERPRINT_MISMATCH', 'AGENT_RUNTIME_CHANGED'
               )",
            [&instance.thread_id],
        ),
        "RC2_FINGERPRINT_RESTORE_FAILED",
    )?;
    drop(restore_connection);
    let fingerprint_decision = fingerprint_result?;
    // agents.instruction 更新会触发 retire_threads_after_agent_runtime_change 立即退役，
    // 因此调度器可能给 RUNTIME_FINGERPRINT_MISMATCH（推荐时判定），也可能给兜底的
    // CANDIDATE_RETIRED（触发器先行）。两者都证明指纹变化后未复用旧 Thread。
    if fingerprint_decision.decision != "SPAWN"
        || !matches!(
            fingerprint_decision.reason_code.as_str(),
            "RUNTIME_FINGERPRINT_MISMATCH" | "CANDIDATE_RETIRED"
        )
    {
        return Err(Rc1Failure::new(
            "RC2_FINGERPRINT_ISOLATION_FAILED",
            format!(
                "期望 SPAWN/RUNTIME_FINGERPRINT_MISMATCH 或 SPAWN/CANDIDATE_RETIRED，实际为 {}/{}",
                fingerprint_decision.decision, fingerprint_decision.reason_code
            ),
        ));
    }
    release_rc2_preflight_probe(
        database_path,
        &agent.id,
        parent_thread_id,
        &base.workspace_scope_key,
        TASK_SCOPE_KEY,
    )?;

    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    stage(
        connection.execute("DELETE FROM agent_spawn_reservations", []),
        "RC2_CONTEXT_FIXTURE_FAILED",
    )?;
    stage(
        connection.execute(
            "UPDATE agent_thread_instances
             SET claimed_until = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 second')
             WHERE codex_thread_id = ?1",
            [&instance.thread_id],
        ),
        "RC2_CONTEXT_FIXTURE_FAILED",
    )?;
    drop(connection);
    let (rollout_path, context_window) =
        append_context_pressure_probe(codex_home, &instance.thread_id)?;
    let context_decision = invoke_schedule(&base)?;
    require_decision(
        &context_decision,
        "SPAWN",
        Some("CONTEXT_PRESSURE"),
        "RC2_CONTEXT_PRESSURE_FAILED",
    )?;

    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    let child_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM agent_thread_instances
             WHERE parent_thread_id = ?1 AND agent_id = ?2",
            params![parent_thread_id, agent.id],
            |row| row.get(0),
        ),
        "EVIDENCE_QUERY_FAILED",
    )?;
    if child_count != 1 {
        return Err(Rc1Failure::new(
            "RC2_PROBE_CREATED_CHILD",
            format!("矩阵预检不得创建 Child，实际记录数为 {child_count}"),
        ));
    }

    Ok(json!({
        "status": "PASS",
        "taskScopeChanged": {
            "taskScopeKey": CONCURRENT_TASK_SCOPE_KEY,
            "spawnCount": spawn_count,
            "waitCount": wait_count,
            "decisions": concurrent.iter().map(ScheduleEvidence::to_json).collect::<Vec<_>>()
        },
        "workspaceChanged": workspace_decision.to_json(),
        "runtimeFingerprintChanged": fingerprint_decision.to_json(),
        "contextPressure": {
            "decision": context_decision.to_json(),
            "probeSource": "SYNTHETIC_NATIVE_ROLLOUT",
            "contextTokens": context_window,
            "contextWindow": context_window,
            "rolloutPath": rollout_path
        },
        "childCountAfterPreflightOnlyProbes": child_count
    }))
}

fn run_native_e2e(include_rc2_matrix: bool) -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let cleanup = TempRoot(root.clone());
    let source_database = required_path("CAS_E2E_SOURCE_DATABASE_PATH")?;
    let source_codex_home = required_path("CAS_E2E_SOURCE_CODEX_HOME")?;
    let helper_source = required_path("CAS_E2E_HELPER_PATH")?;
    let database_path = required_path("CAS_DATABASE_PATH")?;
    let codex_home = root.join("codex-home");
    let data_home = root.join("cas-data");
    let helper_home = root.join("cas-runtime");
    let workspace = root.join("workspace");
    if database_path.parent() != Some(data_home.as_path()) {
        return Err(Rc1Failure::new(
            "E2E_CONFIGURATION_INVALID",
            "CAS_DATABASE_PATH 必须位于 CAS_E2E_ROOT/cas-data 内",
        ));
    }
    stage(fs::create_dir_all(&codex_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&data_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&helper_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&workspace), "TEMP_DIRECTORY_FAILED")?;
    let helper_path = helper_home.join("cas-helper.exe");
    stage(fs::copy(&helper_source, &helper_path), "HELPER_COPY_FAILED")?;
    stage(
        fs::write(
            workspace.join("package.json"),
            b"{\n  \"name\": \"cas-rc1-e2e\",\n  \"private\": true\n}\n",
        ),
        "FIXTURE_WRITE_FAILED",
    )?;
    clone_database(&source_database, &database_path)?;
    let connection = stage(Connection::open(&database_path), "E2E_DATABASE_FAILED")?;
    reset_e2e_database(&connection, &codex_home)?;
    let agent = active_agent(&connection)?;
    drop(connection);
    copy_runtime_identity(&source_codex_home, &codex_home)?;
    stage(
        fs::write(
            codex_home.join("config.toml"),
            b"approval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n",
        ),
        "NON_INTERACTIVE_CONFIG_FAILED",
    )?;

    let configuration = ConfigurationService::for_e2e(
        database_path.clone(),
        data_home.clone(),
        codex_home.clone(),
        helper_path.clone(),
    );
    let switch_request: RuntimeModeSwitchRequest = stage(
        serde_json::from_value(json!({ "activeAgentIds": [agent.id.clone()] })),
        "CONFIGURATION_REQUEST_FAILED",
    )?;
    stage(
        configuration.switch_runtime_mode(switch_request),
        "CONFIGURATION_APPLY_FAILED",
    )?;

    let executable = env::var("CAS_E2E_CODEX_EXECUTABLE").unwrap_or_else(|_| "codex".to_owned());
    let timeout_seconds = env::var("CAS_E2E_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value >= 30)
        .unwrap_or(180);
    let timeout = Duration::from_secs(timeout_seconds);
    let bridge = stage(
        RuntimeBridgeService::open(&database_path, &data_home, &helper_path),
        "BRIDGE_OPEN_FAILED",
    )?;
    let outcome = (|| {
        stage(
            bridge.start_inner_for_e2e(Path::new(&executable), &codex_home, None),
            "BRIDGE_START_FAILED",
        )?;
        let helper_probe = stage(
            bridge.request(
                "command/exec",
                json!({
                    "command": [helper_path.to_string_lossy()],
                    "cwd": workspace.to_string_lossy(),
                    "sandboxPolicy": {
                        "type": "workspaceWrite",
                        "writableRoots": [
                            workspace.to_string_lossy(),
                            data_home.to_string_lossy()
                        ]
                    },
                    "timeoutMs": 10_000
                }),
            ),
            "HELPER_SANDBOX_PREFLIGHT_FAILED",
        )?;
        if helper_probe.get("exitCode").and_then(Value::as_i64) != Some(2) {
            return Err(Rc1Failure::new(
                "HELPER_SANDBOX_PREFLIGHT_FAILED",
                format!("unexpected command/exec response: {helper_probe}"),
            ));
        }
        let session = stage(
            bridge.managed_session_start_inner(ManagedSessionStartRequest {
                cwd: workspace.to_string_lossy().into_owned(),
                approval_policy: Some("on-request".to_owned()),
                sandbox: Some("workspace-write".to_owned()),
            }),
            "PRIMARY_START_FAILED",
        )?;
        let primary_thread_id = session.thread_id;

        let first_turn = stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: primary_thread_id.clone(),
                input: format!(
                    "在当前工作目录完成稳定任务 `{TASK_SCOPE_KEY}` 的第一步：创建 cas-rc1-first.txt，内容只写 CAS_RC1_FIRST，不得修改其他文件。必须把本步骤作为新 Job `cas-rc1-first`（idempotency_key 同名）完整执行 CAS2 job-schedule → 原生委派 → job-bind → 等待完成 → job-observe → Primary 审查 → job-review(APPROVE)；spawn_agent 的初始 message 必须直接执行文件任务，禁止等待绑定占位或对同一 Attempt 二次补发；禁止使用兼容 CAS1 schedule/bind。"
                ),
                effort: None,
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: Some(json!({
                    "type": "workspaceWrite",
                    "writableRoots": [
                        workspace.to_string_lossy(),
                        data_home.to_string_lossy()
                    ]
                })),
            }),
            "FIRST_TURN_START_FAILED",
        )?;
        let first_output = workspace.join("cas-rc1-first.txt");
        let first_completion = wait_for_turn(
            &bridge,
            &database_path,
            &primary_thread_id,
            &first_turn.turn_id,
            &agent.id,
            "cas-rc1-first",
            &first_output,
            timeout,
        )?;
        verify_output(
            &bridge,
            &database_path,
            &primary_thread_id,
            &first_output,
            "CAS_RC1_FIRST",
            "FIRST_OUTPUT_MISSING",
            "FIRST_OUTPUT_INVALID",
        )?;
        let first_instance = wait_for_instance(
            &database_path,
            &primary_thread_id,
            &agent.id,
            Duration::from_secs(10),
        )?;

        let second_turn = stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: primary_thread_id.clone(),
                input: format!(
                    "继续同一个稳定任务 `{TASK_SCOPE_KEY}`：创建 cas-rc1-second.txt，内容只写 CAS_RC1_SECOND，不得修改其他文件。必须把本步骤作为新 Job `cas-rc1-second`（idempotency_key 同名），保持相同 task_scope_key，完整执行 CAS2 job-schedule → 原生复用 → job-bind → 等待完成 → job-observe → Primary 审查 → job-review(APPROVE)；禁止使用兼容 CAS1 schedule/bind。"
                ),
                effort: None,
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: Some(json!({
                    "type": "workspaceWrite",
                    "writableRoots": [
                        workspace.to_string_lossy(),
                        data_home.to_string_lossy()
                    ]
                })),
            }),
            "SECOND_TURN_START_FAILED",
        )?;
        let second_output = workspace.join("cas-rc1-second.txt");
        let second_completion = wait_for_turn(
            &bridge,
            &database_path,
            &primary_thread_id,
            &second_turn.turn_id,
            &agent.id,
            "cas-rc1-second",
            &second_output,
            timeout,
        )?;
        verify_output(
            &bridge,
            &database_path,
            &primary_thread_id,
            &second_output,
            "CAS_RC1_SECOND",
            "SECOND_OUTPUT_MISSING",
            "SECOND_OUTPUT_INVALID",
        )?;
        let final_instance = wait_for_instance(
            &database_path,
            &primary_thread_id,
            &agent.id,
            Duration::from_secs(10),
        )?;
        let connection = stage(Connection::open(&database_path), "EVIDENCE_DATABASE_FAILED")?;
        let decisions = decision_evidence(&connection, &primary_thread_id)?;
        let child_count: i64 = stage(
            connection.query_row(
                "SELECT COUNT(*) FROM agent_thread_instances
                 WHERE parent_thread_id = ?1 AND agent_id = ?2",
                params![primary_thread_id, agent.id],
                |row| row.get(0),
            ),
            "EVIDENCE_QUERY_FAILED",
        )?;
        let usage_count: i64 = stage(
            connection.query_row(
                "SELECT COUNT(*) FROM token_usage_records
                 WHERE parent_thread_id = ?1 AND agent_id = ?2 AND total_tokens > 0",
                params![primary_thread_id, agent.id],
                |row| row.get(0),
            ),
            "EVIDENCE_QUERY_FAILED",
        )?;
        let runtime_first = runtime_first_evidence(
            &database_path,
            &connection,
            &primary_thread_id,
            &agent.id,
            &final_instance.workspace_scope_key,
        )?;
        let spawn = decisions
            .iter()
            .find(|(decision, _, _)| decision == "SPAWN");
        let reuse = decisions.iter().rev().find(|(decision, _, candidate)| {
            decision == "REUSE" && candidate.as_deref() == Some(final_instance.thread_id.as_str())
        });
        if spawn.is_none() {
            return Err(Rc1Failure::new(
                "SPAWN_NOT_RECORDED",
                "未找到首次 SPAWN 调度证据",
            ));
        }
        if reuse.is_none() {
            return Err(Rc1Failure::new(
                "REUSE_NOT_SELECTED",
                "第二步没有复用首次创建的 Child Thread",
            ));
        }
        if child_count != 1 || first_instance.thread_id != final_instance.thread_id {
            return Err(Rc1Failure::new(
                "DUPLICATE_CHILD_CREATED",
                format!("期望一个 Child Thread，实际为 {child_count}"),
            ));
        }
        if final_instance.runtime_fingerprint.is_none()
            || final_instance.task_scope_key.as_deref() != Some(TASK_SCOPE_KEY)
        {
            return Err(Rc1Failure::new(
                "BIND_NOT_VERIFIED",
                "Child Thread 缺少 Runtime Fingerprint 或 Task Scope",
            ));
        }
        if usage_count == 0 || final_instance.total_tokens <= 0 {
            return Err(Rc1Failure::new(
                "USAGE_ATTRIBUTION_FAILED",
                "没有找到归属于目标 Agent 的有效 Token 记录",
            ));
        }
        drop(connection);

        let rc2 = if include_rc2_matrix {
            Some(run_rc2_matrix(
                &helper_path,
                &database_path,
                &codex_home,
                &workspace,
                &primary_thread_id,
                &agent,
                &final_instance,
                &root,
            )?)
        } else {
            None
        };
        stage(bridge.stop_inner(), "BRIDGE_STOP_FAILED")?;
        let default_request: RuntimeModeSwitchRequest = stage(
            serde_json::from_value(json!({ "activeAgentIds": [] })),
            "CONFIGURATION_REQUEST_FAILED",
        )?;
        stage(
            configuration.switch_runtime_mode(default_request),
            "DEFAULT_MODE_APPLY_FAILED",
        )?;
        let default_config = stage(
            fs::read_to_string(codex_home.join("config.toml")),
            "DEFAULT_MODE_CONFIG_READ_FAILED",
        )?;
        let connection = stage(Connection::open(&database_path), "EVIDENCE_DATABASE_FAILED")?;
        let default_bindings: i64 = stage(
            connection.query_row("SELECT COUNT(*) FROM active_agent_bindings", [], |row| {
                row.get(0)
            }),
            "EVIDENCE_QUERY_FAILED",
        )?;
        let default_live_leases: i64 = stage(
            connection.query_row(
                "SELECT COUNT(*) FROM runtime_delegation_leases
                 WHERE state IN ('PENDING', 'ACTIVE')",
                [],
                |row| row.get(0),
            ),
            "EVIDENCE_QUERY_FAILED",
        )?;
        drop(connection);
        if default_bindings != 0
            || default_live_leases != 0
            || default_config.contains(ORCHESTRATION_RUNTIME_CONTRACT)
            || default_config.contains("cas-runtime-enforcement-v1")
        {
            return Err(Rc1Failure::new(
                "DEFAULT_MODE_CLEANUP_FAILED",
                format!(
                    "Default 清理不完整：bindings={default_bindings}, liveLeases={default_live_leases}"
                ),
            ));
        }
        let restore_request: RuntimeModeSwitchRequest = stage(
            serde_json::from_value(json!({ "activeAgentIds": [agent.id.clone()] })),
            "CONFIGURATION_REQUEST_FAILED",
        )?;
        stage(
            configuration.switch_runtime_mode(restore_request),
            "AGENT_MODE_RESTORE_FAILED",
        )?;
        let restored_config = stage(
            fs::read_to_string(codex_home.join("config.toml")),
            "AGENT_MODE_CONFIG_READ_FAILED",
        )?;
        let connection = stage(Connection::open(&database_path), "EVIDENCE_DATABASE_FAILED")?;
        let restored_bindings: i64 = stage(
            connection.query_row("SELECT COUNT(*) FROM active_agent_bindings", [], |row| {
                row.get(0)
            }),
            "EVIDENCE_QUERY_FAILED",
        )?;
        drop(connection);
        if restored_bindings != 1
            || !restored_config.contains(ORCHESTRATION_RUNTIME_CONTRACT)
            || !restored_config.contains("cas-runtime-enforcement-v1")
        {
            return Err(Rc1Failure::new(
                "AGENT_MODE_RESTORE_FAILED",
                format!("Agent 恢复不完整：bindings={restored_bindings}"),
            ));
        }
        let mut result = json!({
            "status": "PASS",
            "agentKey": agent.key,
            "agentName": agent.name,
            "providerKey": agent.provider,
            "model": agent.model,
            "taskScopeKey": TASK_SCOPE_KEY,
            "primaryThreadId": primary_thread_id,
            "childThreadId": final_instance.thread_id,
            "controlProtocol": "CAS2",
            "firstDecision": "SPAWN",
            "secondDecision": "REUSE",
            "firstPrimaryCompletion": first_completion.as_str(),
            "secondPrimaryCompletion": second_completion.as_str(),
            "bindVerified": true,
            "finalLifecycle": final_instance.status,
            "firstTotalTokens": first_instance.total_tokens,
            "finalTotalTokens": final_instance.total_tokens,
            "usageAttributed": true,
            "runtimeFirst": {
                "jobIds": runtime_first.job_ids,
                "attemptCount": runtime_first.attempt_count,
                "receiptCount": runtime_first.receipt_count,
                "reviewCount": runtime_first.review_count,
                "releasedLeaseCount": runtime_first.released_lease_count,
                "trackingDtoCount": runtime_first.tracking_count
            },
            "modeRoundTripVerified": true,
            "duplicateChildCount": child_count - 1,
            "decisionCount": decisions.len()
        });
        if let Some(rc2) = rc2 {
            result["rc2"] = rc2;
        }
        Ok(result)
    })();
    let _ = bridge.stop_inner();
    if outcome.is_ok() {
        drop(cleanup);
    } else {
        // 失败保留隔离现场（codex-home 的 rules/config/rollout 是定位
        // 授权类失败的关键证据），由 write_e2e_result 折叠进诊断。
        std::mem::forget(cleanup);
    }
    outcome
}

#[test]
#[ignore = "requires a configured CAS database, Codex login and a real provider"]
fn managed_session_rc1_spawn_bind_idle_reuse() {
    run_e2e_test(false, "CAS_RC1_RESULT");
}

#[test]
#[ignore = "requires a configured CAS database, Codex login and a real provider"]
fn managed_session_rc2_scheduling_matrix() {
    run_e2e_test(true, "CAS_RC2_RESULT");
}

#[test]
#[ignore = "requires Codex login and runs the real OFF-01/ON-01 efficiency pair"]
fn managed_session_efficiency_pair_off01_on01() {
    let outcome = run_efficiency_pair();
    let passed = outcome
        .as_ref()
        .ok()
        .and_then(|value| value.get("status"))
        .and_then(Value::as_str)
        == Some("PASS");
    write_e2e_result(outcome, "CAS_EFFICIENCY_PAIR_RESULT");
    assert!(passed, "效率配对 Pilot 未通过；请检查结构化证据 JSON");
}

fn managed_task_packet(
    agent_id: &str,
    parent_thread_id: &str,
    workspace_scope_key: &str,
    job_id: &str,
    expected_output: &str,
) -> TaskPacket {
    TaskPacket {
        schema_version: TASK_PACKET_SCHEMA_VERSION,
        job_id: job_id.to_owned(),
        idempotency_key: job_id.to_owned(),
        agent_id: agent_id.to_owned(),
        parent_thread_id: parent_thread_id.to_owned(),
        workspace_scope_key: workspace_scope_key.to_owned(),
        task_scope_key: MANAGED_TASK_SCOPE_KEY.to_owned(),
        objective: format!("不调用工具，只回复 {expected_output}"),
        allowed_scope: vec!["package.json".to_owned()],
        constraints: vec!["不得调用工具或修改文件".to_owned()],
        success_criteria: vec![format!("最终回复精确包含 {expected_output}")],
        allowed_tools: Vec::new(),
        permission_policy: PermissionPolicy::ReadOnly,
        execution_kind_policy: ExecutionKindPolicy::ManagedWorkerRequired,
        context_references: Vec::new(),
        output_contract: OutputContract::StandardV1,
        review_policy: ReviewPolicy::PrimaryRequired,
    }
}

fn execute_managed(
    bridge: &RuntimeBridgeService,
    orchestration: &OrchestrationJobService,
    task_packet: TaskPacket,
    workspace: &Path,
    input: &str,
    expected_decision: RouteAction,
    expected_candidate_thread_id: Option<String>,
    failure_code: &'static str,
) -> Result<AgentThreadExecutionResponse, Rc1Failure> {
    bridge
        .execute_agent_thread(
            orchestration,
            AgentThreadExecutionRequest {
                task_packet,
                cwd: workspace.to_string_lossy().into_owned(),
                input: input.to_owned(),
                expected_decision,
                expected_candidate_thread_id,
            },
        )
        .map_err(|error| Rc1Failure::new(failure_code, format!("{error:?}")))
}

fn wait_for_managed_review_pending(
    bridge: &RuntimeBridgeService,
    database_path: &Path,
    thread_id: &str,
    job_id: &str,
    timeout: Duration,
) -> Result<(), Rc1Failure> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = stage(bridge.status_inner(), "MANAGED_BRIDGE_STATUS_FAILED")?;
        let session = status
            .managed_sessions
            .iter()
            .find(|session| session.thread_id == thread_id)
            .ok_or_else(|| {
                Rc1Failure::new(
                    "MANAGED_SESSION_MISSING",
                    format!("未找到 Managed Worker Thread {thread_id}"),
                )
            })?;
        if matches!(
            session.status,
            ManagedSessionStatus::Failed | ManagedSessionStatus::RecoveryRequired
        ) {
            return Err(Rc1Failure::new(
                "MANAGED_TURN_FAILED",
                format!(
                    "Managed Worker 结束状态为 {:?}：{:?}",
                    session.status, status.last_error
                ),
            ));
        }
        let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
        let job_state = stage(
            connection
                .query_row(
                    "SELECT state FROM orchestration_jobs WHERE job_id=?1",
                    [job_id],
                    |row| row.get::<_, String>(0),
                )
                .optional(),
            "EVIDENCE_QUERY_FAILED",
        )?;
        if session.status == ManagedSessionStatus::Idle
            && job_state.as_deref() == Some("REVIEW_PENDING")
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Rc1Failure::new(
                "MANAGED_TURN_TIMEOUT",
                format!(
                    "Managed Worker 未进入 REVIEW_PENDING；thread={thread_id}, session={:?}, job={job_state:?}, summary={}",
                    session.status,
                    primary_summary(bridge, thread_id)
                ),
            ));
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn verify_managed_turn(
    bridge: &RuntimeBridgeService,
    thread_id: &str,
    turn_id: &str,
    expected_output: &str,
    expected_turn_count: usize,
) -> Result<(), Rc1Failure> {
    let thread = stage(
        bridge.request(
            AppServerMethod::ThreadRead.as_str(),
            json!({"threadId": thread_id, "includeTurns": true}),
        ),
        "MANAGED_THREAD_READ_FAILED",
    )?;
    let (turn_count, occurrences) = thread_turn_evidence(&thread, turn_id).ok_or_else(|| {
        Rc1Failure::new(
            "MANAGED_NATIVE_TURN_MISSING",
            "Native thread/read 缺少 turns",
        )
    })?;
    let summary = primary_summary(bridge, thread_id);
    if turn_count != expected_turn_count || occurrences != 1 || !summary.contains(expected_output) {
        return Err(Rc1Failure::new(
            "MANAGED_NATIVE_TURN_INVALID",
            format!(
                "期望 turns={expected_turn_count}/occurrences=1/output={expected_output}，实际 turns={turn_count}/occurrences={occurrences}/{summary}"
            ),
        ));
    }
    Ok(())
}

fn approve_managed_job(
    orchestration: &OrchestrationJobService,
    response: &AgentThreadExecutionResponse,
    reviewer_thread_id: &str,
) -> Result<(), Rc1Failure> {
    let reviewed = orchestration
        .review(OrchestrationJobReviewRequest {
            job_id: response.job_id.clone(),
            attempt_id: response.attempt_id.clone(),
            decision: ReviewOutcome::Approve,
            reviewer_thread_id: reviewer_thread_id.to_owned(),
            reason: "真实 Managed Worker 输出与 Native thread/read 证据一致。".to_owned(),
            evidence_refs: vec![
                format!("thread:{}", response.thread_id),
                format!("turn:{}", response.turn_id),
            ],
        })
        .map_err(|error| Rc1Failure::new("MANAGED_REVIEW_FAILED", format!("{error:?}")))?;
    if reviewed.job.state != JobState::Completed {
        return Err(Rc1Failure::new(
            "MANAGED_REVIEW_INCOMPLETE",
            format!("Review 后 Job 状态为 {:?}", reviewed.job.state),
        ));
    }
    Ok(())
}

fn verify_managed_evidence(
    database_path: &Path,
    orchestration: &OrchestrationJobService,
    agent: &ActiveAgent,
    parent_thread_id: &str,
    workspace_scope_key: &str,
    thread_id: &str,
) -> Result<Value, Rc1Failure> {
    let connection = stage(Connection::open(database_path), "EVIDENCE_DATABASE_FAILED")?;
    let invalid_jobs: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM orchestration_jobs
             WHERE job_id IN ('cas-managed-first','cas-managed-second')
               AND (state<>'COMPLETED' OR agent_id<>?1 OR parent_thread_id<>?2
                    OR workspace_scope_key<>?3 OR task_scope_key<>?4
                    OR json_extract(task_packet,'$.execution_kind_policy')<>'MANAGED_WORKER_REQUIRED')",
            params![agent.id, parent_thread_id, workspace_scope_key, MANAGED_TASK_SCOPE_KEY],
            |row| row.get(0),
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    let (attempt_count, spawn_count, reuse_count, managed_count, distinct_threads): (
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = stage(
        connection.query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN route_action='SPAWN' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN route_action='REUSE' THEN 1 ELSE 0 END),
                    SUM(CASE WHEN state='SUCCEEDED'
                                  AND planned_execution_kind='MANAGED_WORKER'
                                  AND execution_kind='MANAGED_WORKER'
                                  AND codex_turn_id IS NOT NULL THEN 1 ELSE 0 END),
                    COUNT(DISTINCT thread_instance_id)
             FROM job_attempts
             WHERE job_id IN ('cas-managed-first','cas-managed-second')",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    let (receipt_count, receipt_stages, dispatch_receipts, managed_receipts): (i64, i64, i64, i64) =
        stage(
            connection.query_row(
                "SELECT COUNT(*), COUNT(DISTINCT stage),
                    SUM(CASE WHEN stage='DISPATCH_RECORDED' AND execution_kind IS NULL
                             THEN 1 ELSE 0 END),
                    SUM(CASE WHEN stage<>'DISPATCH_RECORDED'
                                  AND execution_kind='MANAGED_WORKER'
                             THEN 1 ELSE 0 END)
             FROM delivery_receipts
             WHERE job_id IN ('cas-managed-first','cas-managed-second')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            ),
            "MANAGED_EVIDENCE_QUERY_FAILED",
        )?;
    let review_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM review_decisions
             WHERE job_id IN ('cas-managed-first','cas-managed-second') AND decision='APPROVE'",
            [],
            |row| row.get(0),
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    let released_lease_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM runtime_delegation_leases lease
             JOIN job_attempts attempt ON attempt.lease_id=lease.id
             WHERE attempt.job_id IN ('cas-managed-first','cas-managed-second')
               AND lease.state='RELEASED'",
            [],
            |row| row.get(0),
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    let managed_instance_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM agent_thread_instances
             WHERE codex_thread_id=?1 AND agent_id=?2 AND parent_thread_id=?3
               AND scope_key=?4 AND task_scope_key=?5 AND execution_kind='MANAGED_WORKER'
               AND status='IDLE' AND reuse_state='ACTIVE' AND total_tokens>0",
            params![
                thread_id,
                agent.id,
                parent_thread_id,
                workspace_scope_key,
                MANAGED_TASK_SCOPE_KEY
            ],
            |row| row.get(0),
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    let usage_count: i64 = stage(
        connection.query_row(
            "SELECT COUNT(*) FROM token_usage_records
             WHERE codex_thread_id=?1 AND parent_thread_id=?2 AND agent_id=?3
               AND execution_kind='MANAGED_WORKER' AND total_tokens>0",
            params![thread_id, parent_thread_id, agent.id],
            |row| row.get(0),
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    let (finished_events, parent_child_events): (i64, i64) = stage(
        connection.query_row(
            "SELECT
                SUM(CASE WHEN event_type='TURN_FINISHED' THEN 1 ELSE 0 END),
                SUM(CASE WHEN event_type='PARENT_CHILD' THEN 1 ELSE 0 END)
             FROM runtime_receipt_events WHERE codex_thread_id=?1",
            [thread_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ),
        "MANAGED_EVIDENCE_QUERY_FAILED",
    )?;
    if invalid_jobs != 0
        || attempt_count != 2
        || spawn_count != 1
        || reuse_count != 1
        || managed_count != 2
        || distinct_threads != 1
        || receipt_count != 8
        || receipt_stages != 4
        || dispatch_receipts != 2
        || managed_receipts != 6
        || review_count != 2
        || released_lease_count != 2
        || managed_instance_count != 1
        || usage_count == 0
        || finished_events < 2
        || parent_child_events != 0
    {
        return Err(Rc1Failure::new(
            "MANAGED_EVIDENCE_INVALID",
            format!(
                "invalidJobs={invalid_jobs}, attempts={attempt_count}, spawn={spawn_count}, reuse={reuse_count}, managed={managed_count}, threads={distinct_threads}, receipts={receipt_count}/{receipt_stages}/{dispatch_receipts}/{managed_receipts}, reviews={review_count}, leases={released_lease_count}, instance={managed_instance_count}, usage={usage_count}, events={finished_events}/{parent_child_events}"
            ),
        ));
    }
    let tracking = orchestration
        .list_tracking(OrchestrationJobListRequest {
            workspace_scope_key: Some(workspace_scope_key.to_owned()),
            agent_id: Some(agent.id.clone()),
            page: 0,
            page_size: 10,
        })
        .map_err(|error| Rc1Failure::new("MANAGED_TRACKING_QUERY_FAILED", format!("{error:?}")))?;
    let tracking_jobs = tracking
        .jobs
        .iter()
        .filter(|job| {
            matches!(
                job.job_id.as_str(),
                "cas-managed-first" | "cas-managed-second"
            )
        })
        .collect::<Vec<_>>();
    let tracking_valid = tracking_jobs.len() == 2
        && tracking_jobs.iter().all(|job| {
            job.state == "COMPLETED"
                && job.attempts.len() == 1
                && job.attempts[0].state == "SUCCEEDED"
                && job.attempts[0].execution_kind.as_deref() == Some("MANAGED_WORKER")
                && job.attempts[0].codex_thread_id.as_deref() == Some(thread_id)
                && job.attempts[0].receipt_stage.as_deref() == Some("PARENT_ACKNOWLEDGED")
                && job.attempts[0].review_decision.as_deref() == Some("APPROVE")
                && job.attempts[0].lease_state.as_deref() == Some("RELEASED")
        });
    if !tracking_valid {
        return Err(Rc1Failure::new(
            "MANAGED_TRACKING_DTO_INVALID",
            "UI 使用的 Tracking DTO 与数据库证据不一致",
        ));
    }
    Ok(json!({
        "attemptCount": attempt_count,
        "spawnCount": spawn_count,
        "reuseCount": reuse_count,
        "managedAttemptCount": managed_count,
        "distinctThreadCount": distinct_threads,
        "receiptCount": receipt_count,
        "receiptStageCount": receipt_stages,
        "dispatchReceiptCount": dispatch_receipts,
        "managedReceiptCount": managed_receipts,
        "reviewCount": review_count,
        "releasedLeaseCount": released_lease_count,
        "usageRecordCount": usage_count,
        "turnFinishedEventCount": finished_events,
        "parentChildEventCount": parent_child_events,
        "trackingDtoCount": tracking_jobs.len()
    }))
}

fn run_managed_worker_e2e() -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let cleanup = TempRoot(root.clone());
    let source_database = required_path("CAS_E2E_SOURCE_DATABASE_PATH")?;
    let source_codex_home = required_path("CAS_E2E_SOURCE_CODEX_HOME")?;
    let helper_source = required_path("CAS_E2E_HELPER_PATH")?;
    let database_path = required_path("CAS_DATABASE_PATH")?;
    let codex_home = root.join("codex-home");
    let data_home = root.join("cas-data");
    let helper_home = root.join("cas-runtime");
    let workspace = root.join("workspace");
    stage(fs::create_dir_all(&codex_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&data_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&helper_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&workspace), "TEMP_DIRECTORY_FAILED")?;
    let helper_path = helper_home.join("cas-helper.exe");
    stage(fs::copy(&helper_source, &helper_path), "HELPER_COPY_FAILED")?;
    stage(
        fs::write(
            workspace.join("package.json"),
            b"{\n  \"name\": \"cas-managed-e2e\",\n  \"private\": true\n}\n",
        ),
        "FIXTURE_WRITE_FAILED",
    )?;
    clone_database(&source_database, &database_path)?;
    let connection = stage(Connection::open(&database_path), "E2E_DATABASE_FAILED")?;
    reset_e2e_database(&connection, &codex_home)?;
    let agent = active_agent(&connection)?;
    drop(connection);
    copy_runtime_identity(&source_codex_home, &codex_home)?;
    stage(
        fs::write(
            codex_home.join("config.toml"),
            b"approval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n",
        ),
        "NON_INTERACTIVE_CONFIG_FAILED",
    )?;
    let configuration = ConfigurationService::for_e2e(
        database_path.clone(),
        data_home.clone(),
        codex_home.clone(),
        helper_path.clone(),
    );
    let switch_request: RuntimeModeSwitchRequest = stage(
        serde_json::from_value(json!({ "activeAgentIds": [agent.id.clone()] })),
        "CONFIGURATION_REQUEST_FAILED",
    )?;
    stage(
        configuration.switch_runtime_mode(switch_request),
        "CONFIGURATION_APPLY_FAILED",
    )?;
    let executable = env::var("CAS_E2E_CODEX_EXECUTABLE").unwrap_or_else(|_| "codex".to_owned());
    let timeout = Duration::from_secs(
        env::var("CAS_E2E_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value >= 30)
            .unwrap_or(180),
    );
    let workspace_scope_key = normalize_workspace_scope_key(&workspace.to_string_lossy())
        .ok_or_else(|| Rc1Failure::new("WORKSPACE_SCOPE_INVALID", "无法规范化 E2E workspace"))?;
    let bridge = stage(
        RuntimeBridgeService::open(&database_path, &data_home, &helper_path),
        "BRIDGE_OPEN_FAILED",
    )?;
    let orchestration = stage(
        OrchestrationJobService::open(&database_path),
        "ORCHESTRATION_OPEN_FAILED",
    )?;
    let outcome = (|| {
        stage(
            bridge.start_inner_for_e2e(Path::new(&executable), &codex_home, None),
            "BRIDGE_START_FAILED",
        )?;
        let primary = stage(
            bridge.managed_session_start_inner(ManagedSessionStartRequest {
                cwd: workspace.to_string_lossy().into_owned(),
                approval_policy: Some("on-request".to_owned()),
                sandbox: Some("workspace-write".to_owned()),
            }),
            "PRIMARY_START_FAILED",
        )?;
        let first = execute_managed(
            &bridge,
            &orchestration,
            managed_task_packet(
                &agent.id,
                &primary.thread_id,
                &workspace_scope_key,
                "cas-managed-first",
                "CAS_MANAGED_FIRST",
            ),
            &workspace,
            "不要调用任何工具，只回复 CAS_MANAGED_FIRST。",
            RouteAction::Spawn,
            None,
            "MANAGED_FIRST_DISPATCH_FAILED",
        )?;
        if first.action != AgentThreadExecutionAction::Spawned
            || first.decision != RouteAction::Spawn
        {
            return Err(Rc1Failure::new(
                "MANAGED_FIRST_DECISION_INVALID",
                format!("首次决策不为 SPAWN：{first:?}"),
            ));
        }
        wait_for_managed_review_pending(
            &bridge,
            &database_path,
            &first.thread_id,
            &first.job_id,
            timeout,
        )?;
        verify_managed_turn(
            &bridge,
            &first.thread_id,
            &first.turn_id,
            "CAS_MANAGED_FIRST",
            1,
        )?;
        approve_managed_job(&orchestration, &first, &primary.thread_id)?;

        let second = execute_managed(
            &bridge,
            &orchestration,
            managed_task_packet(
                &agent.id,
                &primary.thread_id,
                &workspace_scope_key,
                "cas-managed-second",
                "CAS_MANAGED_SECOND",
            ),
            &workspace,
            "继续同一任务，不要调用任何工具，只回复 CAS_MANAGED_SECOND。",
            RouteAction::Reuse,
            Some(first.thread_id.clone()),
            "MANAGED_SECOND_DISPATCH_FAILED",
        )?;
        if second.action != AgentThreadExecutionAction::Reused
            || second.decision != RouteAction::Reuse
            || second.thread_id != first.thread_id
        {
            return Err(Rc1Failure::new(
                "MANAGED_SECOND_DECISION_INVALID",
                format!("第二次未复用首次 Worker：{second:?}"),
            ));
        }
        wait_for_managed_review_pending(
            &bridge,
            &database_path,
            &second.thread_id,
            &second.job_id,
            timeout,
        )?;
        verify_managed_turn(
            &bridge,
            &second.thread_id,
            &second.turn_id,
            "CAS_MANAGED_SECOND",
            2,
        )?;
        approve_managed_job(&orchestration, &second, &primary.thread_id)?;
        let evidence = verify_managed_evidence(
            &database_path,
            &orchestration,
            &agent,
            &primary.thread_id,
            &workspace_scope_key,
            &second.thread_id,
        )?;
        stage(bridge.stop_inner(), "BRIDGE_STOP_FAILED")?;
        Ok(json!({
            "status": "PASS",
            "agentKey": agent.key,
            "agentName": agent.name,
            "providerKey": agent.provider,
            "model": agent.model,
            "executionKind": "MANAGED_WORKER",
            "taskScopeKey": MANAGED_TASK_SCOPE_KEY,
            "primaryThreadId": primary.thread_id,
            "workerThreadId": second.thread_id,
            "firstTurnId": first.turn_id,
            "secondTurnId": second.turn_id,
            "firstDecision": "SPAWN",
            "secondDecision": "REUSE",
            "nativeThreadReadVerified": true,
            "databaseVerified": true,
            "trackingDtoVerified": true,
            "evidence": evidence
        }))
    })();
    let _ = bridge.stop_inner();
    if outcome.is_ok() {
        drop(cleanup);
    } else {
        std::mem::forget(cleanup);
    }
    outcome
}

#[test]
#[ignore = "requires a configured CAS database, Codex login and a real provider"]
fn managed_worker_spawn_reuse_receipt_review() {
    write_e2e_result(run_managed_worker_e2e(), "CAS_MANAGED_RESULT");
}

fn run_phase6_idle_recovery_e2e() -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let _cleanup = TempRoot(root.clone());
    let source_codex_home = required_path("CAS_E2E_SOURCE_CODEX_HOME")?;
    let database_path = required_path("CAS_DATABASE_PATH")?;
    let codex_home = root.join("codex-home");
    let data_home = root.join("cas-data");
    let workspace = root.join("workspace");
    stage(fs::create_dir_all(&codex_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&data_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&workspace), "TEMP_DIRECTORY_FAILED")?;
    copy_runtime_identity(&source_codex_home, &codex_home)?;
    stage(
        fs::write(
            codex_home.join("config.toml"),
            b"model = \"gpt-5.6-terra\"\napproval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n",
        ),
        "NON_INTERACTIVE_CONFIG_FAILED",
    )?;
    let executable = env::var("CAS_E2E_CODEX_EXECUTABLE").unwrap_or_else(|_| "codex".to_owned());
    let bridge = stage(
        RuntimeBridgeService::open(
            &database_path,
            &data_home,
            &root.join("cas-runtime").join("cas-helper.exe"),
        ),
        "BRIDGE_OPEN_FAILED",
    )?;
    let outcome = (|| {
        stage(
            bridge.start_inner(Path::new(&executable), &codex_home, None),
            "BRIDGE_START_FAILED",
        )?;
        let session = stage(
            bridge.managed_session_start_inner(ManagedSessionStartRequest {
                cwd: workspace.to_string_lossy().into_owned(),
                approval_policy: Some("on-request".to_owned()),
                sandbox: Some("workspace-write".to_owned()),
            }),
            "PRIMARY_START_FAILED",
        )?;
        stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: session.thread_id.clone(),
                input: "不要调用工具，只回复 PHASE6_READY。".to_owned(),
                effort: Some("low".to_owned()),
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: None,
            }),
            "RECOVERY_FIXTURE_TURN_FAILED",
        )?;
        let timeout_seconds = env::var("CAS_E2E_TIMEOUT_SECONDS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120);
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        loop {
            let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
            let session_status = status
                .managed_session
                .as_ref()
                .map(|session| session.status)
                .ok_or_else(|| Rc1Failure::new("MANAGED_SESSION_MISSING", "托管 Primary 不存在"))?;
            match session_status {
                ManagedSessionStatus::Idle => break,
                ManagedSessionStatus::Running if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(100));
                }
                _ => {
                    return Err(Rc1Failure::new(
                        "RECOVERY_FIXTURE_TURN_FAILED",
                        format!(
                            "准备 Turn 结束状态为 {session_status:?}{}；{}",
                            status
                                .last_error
                                .as_deref()
                                .map(|message| format!("：{message}"))
                                .unwrap_or_default(),
                            primary_summary(&bridge, &session.thread_id)
                        ),
                    ));
                }
            }
        }
        if !primary_summary(&bridge, &session.thread_id).contains("PHASE6_READY") {
            return Err(Rc1Failure::new(
                "RECOVERY_FIXTURE_OUTPUT_INVALID",
                "准备 Turn 没有返回 PHASE6_READY",
            ));
        }
        {
            let mut workers = stage(bridge.worker(), "BRIDGE_WORKER_UNAVAILABLE")?;
            let worker = workers
                .as_mut()
                .ok_or_else(|| Rc1Failure::new("BRIDGE_WORKER_UNAVAILABLE", "Worker 不存在"))?;
            let mut child = stage(worker.child.lock(), "BRIDGE_PROCESS_UNAVAILABLE")?;
            stage(child.kill(), "BRIDGE_PROCESS_KILL_FAILED")?;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
            if status.status == RuntimeBridgeStatus::Failed {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Rc1Failure::new(
                    "BRIDGE_FAILURE_NOT_OBSERVED",
                    "App Server 被终止后没有进入 FAILED",
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
        let recovered = stage(bridge.recover_inner(true), "BRIDGE_RECOVERY_FAILED")?;
        let recovered_session = recovered
            .managed_session
            .as_ref()
            .ok_or_else(|| Rc1Failure::new("PRIMARY_RECOVERY_FAILED", "恢复后没有托管 Primary"))?;
        if recovered.status != RuntimeBridgeStatus::Running
            || recovered_session.thread_id != session.thread_id
            || recovered_session.origin != ManagedSessionOrigin::Resumed
            || recovered_session.status != ManagedSessionStatus::Idle
        {
            return Err(Rc1Failure::new(
                "PRIMARY_RECOVERY_FAILED",
                format!("恢复状态不符合预期：{recovered:?}"),
            ));
        }
        let stopped = stage(bridge.stop_inner(), "BRIDGE_STOP_FAILED")?;
        let after_stop = stage(bridge.recover_inner(false), "BRIDGE_STOP_GUARD_FAILED")?;
        if stopped.status != RuntimeBridgeStatus::Stopped
            || after_stop.status != RuntimeBridgeStatus::Stopped
        {
            return Err(Rc1Failure::new(
                "BRIDGE_STOP_GUARD_FAILED",
                "用户主动停止后仍触发了自动恢复",
            ));
        }
        Ok(json!({
            "status": "PASS",
            "primaryThreadId": session.thread_id,
            "recoveredPrimaryThreadId": recovered_session.thread_id,
            "recoveredOrigin": recovered_session.origin,
            "recoveredSessionStatus": recovered_session.status,
            "lastRecoveryAt": recovered.last_recovery_at,
            "recoveryAttemptCount": recovered.recovery_attempt_count,
            "explicitStopStayedStopped": true,
            "turnWasSubmitted": true,
            "model": "gpt-5.6-terra"
        }))
    })();
    let _ = bridge.stop_inner();
    outcome
}

#[test]
#[ignore = "requires Codex login and a real Codex App Server"]
fn managed_session_phase6_idle_disconnect_recovers_same_primary() {
    write_e2e_result(run_phase6_idle_recovery_e2e(), "CAS_PHASE6_IDLE_RESULT");
}

fn run_phase12_running_recovery_e2e() -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let _cleanup = TempRoot(root.clone());
    let source_codex_home = required_path("CAS_E2E_SOURCE_CODEX_HOME")?;
    let database_path = required_path("CAS_DATABASE_PATH")?;
    let codex_home = root.join("codex-home");
    let data_home = root.join("cas-data");
    let workspace = root.join("workspace");
    stage(fs::create_dir_all(&codex_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&data_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&workspace), "TEMP_DIRECTORY_FAILED")?;
    copy_runtime_identity(&source_codex_home, &codex_home)?;
    stage(
        fs::write(
            codex_home.join("config.toml"),
            b"model = \"gpt-5.6-terra\"\napproval_policy = \"on-request\"\nsandbox_mode = \"workspace-write\"\n",
        ),
        "NON_INTERACTIVE_CONFIG_FAILED",
    )?;
    let executable = env::var("CAS_E2E_CODEX_EXECUTABLE").unwrap_or_else(|_| "codex".to_owned());
    let bridge = stage(
        RuntimeBridgeService::open(
            &database_path,
            &data_home,
            &root.join("cas-runtime").join("cas-helper.exe"),
        ),
        "BRIDGE_OPEN_FAILED",
    )?;
    let outcome = (|| {
        stage(
            bridge.start_inner(Path::new(&executable), &codex_home, None),
            "BRIDGE_START_FAILED",
        )?;
        let session = stage(
            bridge.managed_session_start_inner(ManagedSessionStartRequest {
                cwd: workspace.to_string_lossy().into_owned(),
                approval_policy: Some("on-request".to_owned()),
                sandbox: Some("workspace-write".to_owned()),
            }),
            "PRIMARY_START_FAILED",
        )?;
        let turn = stage(
            bridge.managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: session.thread_id.clone(),
                input: "不要调用任何工具。请详细分析多 Agent 编排中重复执行的风险，并给出不少于 3000 字的说明。"
                    .to_owned(),
                effort: Some("high".to_owned()),
                approval_policy: Some("on-request".to_owned()),
                sandbox_policy: None,
            }),
            "RUNNING_RECOVERY_TURN_FAILED",
        )?;
        let running = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
        if !running.managed_session.as_ref().is_some_and(|managed| {
            managed.status == ManagedSessionStatus::Running
                && managed.active_turn_id.as_deref() == Some(turn.turn_id.as_str())
        }) {
            return Err(Rc1Failure::new(
                "RUNNING_RECOVERY_TURN_NOT_ACTIVE",
                format!("Turn 未保持 RUNNING：{running:?}"),
            ));
        }
        let persistence_deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
            if !status.managed_session.as_ref().is_some_and(|managed| {
                managed.status == ManagedSessionStatus::Running
                    && managed.active_turn_id.as_deref() == Some(turn.turn_id.as_str())
            }) {
                return Err(Rc1Failure::new(
                    "RUNNING_RECOVERY_TURN_COMPLETED_EARLY",
                    format!("等待持久化时 Turn 已不再运行：{status:?}"),
                ));
            }
            if bridge
                .request(
                    "thread/read",
                    json!({"threadId": session.thread_id, "includeTurns": true}),
                )
                .ok()
                .and_then(|thread| thread_turn_evidence(&thread, &turn.turn_id))
                .is_some_and(|(_, occurrences)| occurrences == 1)
            {
                break;
            }
            if Instant::now() >= persistence_deadline {
                return Err(Rc1Failure::new(
                    "RUNNING_RECOVERY_ROLLOUT_NOT_PERSISTED",
                    "运行中 Turn 在 10 秒内未形成可恢复 rollout",
                ));
            }
            thread::sleep(Duration::from_millis(50));
        }
        {
            let mut workers = stage(bridge.worker(), "BRIDGE_WORKER_UNAVAILABLE")?;
            let worker = workers
                .as_mut()
                .ok_or_else(|| Rc1Failure::new("BRIDGE_WORKER_UNAVAILABLE", "Worker 不存在"))?;
            let mut child = stage(worker.child.lock(), "BRIDGE_PROCESS_UNAVAILABLE")?;
            stage(child.kill(), "BRIDGE_PROCESS_KILL_FAILED")?;
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?.status
                == RuntimeBridgeStatus::Failed
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Rc1Failure::new(
                    "BRIDGE_FAILURE_NOT_OBSERVED",
                    "App Server 被终止后没有进入 FAILED",
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
        let recovered = stage(bridge.recover_inner(true), "BRIDGE_RECOVERY_FAILED")?;
        let recovered_session = recovered
            .managed_session
            .as_ref()
            .ok_or_else(|| Rc1Failure::new("PRIMARY_RECOVERY_FAILED", "恢复后没有托管 Primary"))?;
        if recovered.status != RuntimeBridgeStatus::Running
            || recovered_session.thread_id != session.thread_id
            || recovered_session.origin != ManagedSessionOrigin::Resumed
            || !matches!(
                recovered_session.status,
                ManagedSessionStatus::Idle | ManagedSessionStatus::RecoveryRequired
            )
        {
            return Err(Rc1Failure::new(
                "PRIMARY_RECOVERY_FAILED",
                format!("恢复状态不符合预期：{recovered:?}"),
            ));
        }
        if recovered_session.status == ManagedSessionStatus::RecoveryRequired
            && recovered_session.active_turn_id.as_deref() != Some(turn.turn_id.as_str())
        {
            return Err(Rc1Failure::new(
                "UNCERTAIN_TURN_IDENTITY_LOST",
                "恢复后未保留原始不确定 Turn ID",
            ));
        }
        let thread = stage(
            bridge.request(
                "thread/read",
                json!({"threadId": session.thread_id, "includeTurns": true}),
            ),
            "RUNNING_RECOVERY_THREAD_READ_FAILED",
        )?;
        let (thread_turn_count, original_turn_occurrences) =
            thread_turn_evidence(&thread, &turn.turn_id).ok_or_else(|| {
                Rc1Failure::new(
                    "RUNNING_RECOVERY_TURNS_MISSING",
                    "恢复后的 Thread 没有 turns",
                )
            })?;
        if original_turn_occurrences != 1 || thread_turn_count != 1 {
            return Err(Rc1Failure::new(
                "INTERRUPTED_TURN_REPLAYED",
                format!(
                    "恢复后 turns={}，原 Turn 出现 {} 次",
                    thread_turn_count, original_turn_occurrences
                ),
            ));
        }
        Ok(json!({
            "status": "PASS",
            "primaryThreadId": session.thread_id,
            "recoveredPrimaryThreadId": recovered_session.thread_id,
            "recoveredOrigin": recovered_session.origin,
            "recoveredSessionStatus": recovered_session.status,
            "originalTurnId": turn.turn_id,
            "recoveredActiveTurnId": recovered_session.active_turn_id,
            "threadTurnCount": thread_turn_count,
            "originalTurnOccurrences": original_turn_occurrences,
            "turnWasNotReplayed": true,
            "model": "gpt-5.6-terra"
        }))
    })();
    let _ = bridge.stop_inner();
    outcome
}

fn run_phase12_startup_failure_e2e() -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let _cleanup = TempRoot(root.clone());
    stage(fs::create_dir_all(&root), "TEMP_DIRECTORY_FAILED")?;
    let bridge = stage(
        RuntimeBridgeService::open(&root.join("cas.db"), &root, &root.join("cas-helper.exe")),
        "BRIDGE_OPEN_FAILED",
    )?;
    let missing = root.join("missing-codex.exe");
    let error = bridge
        .start_inner(&missing, &root, Some("missing".to_owned()))
        .expect_err("missing executable must fail");
    if !matches!(error, RuntimeBridgeError::Spawn(_)) {
        return Err(Rc1Failure::new(
            "STARTUP_FAILURE_CLASSIFICATION_INVALID",
            error.to_string(),
        ));
    }
    let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
    if status.status != RuntimeBridgeStatus::Failed
        || status.last_error.is_none()
        || !stage(bridge.launch(), "BRIDGE_LAUNCH_STATE_FAILED")?.is_none()
    {
        return Err(Rc1Failure::new(
            "STARTUP_FAILURE_STATE_INVALID",
            format!("启动失败后状态不符合预期：{status:?}"),
        ));
    }
    Ok(json!({
        "status": "PASS",
        "bridgeStatus": status.status,
        "lastError": status.last_error,
        "launchStatePersisted": false
    }))
}

fn run_phase12_recovery_storm_e2e() -> Result<Value, Rc1Failure> {
    let root = required_path("CAS_E2E_ROOT")?;
    let _cleanup = TempRoot(root.clone());
    stage(fs::create_dir_all(&root), "TEMP_DIRECTORY_FAILED")?;
    let bridge = stage(
        RuntimeBridgeService::open(&root.join("cas.db"), &root, &root.join("cas-helper.exe")),
        "BRIDGE_OPEN_FAILED",
    )?;
    *stage(bridge.launch(), "BRIDGE_LAUNCH_STATE_FAILED")? = Some(RuntimeBridgeLaunch {
        executable: root.join("missing-codex.exe"),
        codex_home: root.clone(),
        codex_version: Some("missing".to_owned()),
    });
    stage(bridge.state(), "BRIDGE_STATUS_FAILED")?.status = RuntimeBridgeStatus::Failed;
    for expected_attempt in 1..=MAX_AUTO_RECOVERY_ATTEMPTS {
        let error = bridge
            .recover_inner(false)
            .expect_err("missing executable must fail recovery");
        if !matches!(error, RuntimeBridgeError::Spawn(_)) {
            return Err(Rc1Failure::new(
                "RECOVERY_FAILURE_CLASSIFICATION_INVALID",
                error.to_string(),
            ));
        }
        let status = stage(bridge.status_inner(), "BRIDGE_STATUS_FAILED")?;
        if status.status != RuntimeBridgeStatus::Failed
            || status.recovery_attempt_count != expected_attempt
        {
            return Err(Rc1Failure::new(
                "RECOVERY_ATTEMPT_STATE_INVALID",
                format!("第 {expected_attempt} 次恢复状态不符合预期：{status:?}"),
            ));
        }
    }
    let exhausted = stage(bridge.recover_inner(false), "BRIDGE_STATUS_FAILED")?;
    if !exhausted.auto_recovery_exhausted
        || exhausted.recovery_attempt_count != MAX_AUTO_RECOVERY_ATTEMPTS
    {
        return Err(Rc1Failure::new(
            "RECOVERY_RETRY_CEILING_BYPASSED",
            format!("恢复上限状态不符合预期：{exhausted:?}"),
        ));
    }
    Ok(json!({
        "status": "PASS",
        "bridgeStatus": exhausted.status,
        "recoveryAttemptCount": exhausted.recovery_attempt_count,
        "maxAutoRecoveryAttempts": exhausted.max_auto_recovery_attempts,
        "autoRecoveryExhausted": exhausted.auto_recovery_exhausted
    }))
}

#[test]
#[ignore = "requires Codex login and a real Codex App Server"]
fn managed_session_phase12_running_disconnect_is_not_replayed() {
    write_e2e_result(
        run_phase12_running_recovery_e2e(),
        "CAS_PHASE12_RUNNING_RESULT",
    );
}

#[test]
#[ignore = "writes structured Phase 12 startup-failure evidence"]
fn managed_session_phase12_startup_failure_is_terminal() {
    write_e2e_result(
        run_phase12_startup_failure_e2e(),
        "CAS_PHASE12_STARTUP_RESULT",
    );
}

#[test]
#[ignore = "writes structured Phase 12 retry-ceiling evidence"]
fn managed_session_phase12_recovery_storm_stops_at_ceiling() {
    write_e2e_result(run_phase12_recovery_storm_e2e(), "CAS_PHASE12_STORM_RESULT");
}

fn run_e2e_test(include_rc2_matrix: bool, result_label: &str) {
    write_e2e_result(run_native_e2e(include_rc2_matrix), result_label);
}

fn write_e2e_result(outcome: Result<Value, Rc1Failure>, result_label: &str) {
    let result_path =
        required_path("CAS_E2E_RESULT_PATH").expect("CAS_E2E_RESULT_PATH is required");
    let payload = match &outcome {
        Ok(value) => value.clone(),
        Err(error) => {
            let mut payload = json!({
                "status": "FAIL",
                "failureCode": error.code,
                "message": error.message
            });
            if let Some(diagnostics) = collect_preserved_diagnostics() {
                payload["diagnostics"] = diagnostics;
            }
            payload
        }
    };
    if let Some(parent) = result_path.parent() {
        fs::create_dir_all(parent).expect("create result directory");
    }
    fs::write(
        &result_path,
        serde_json::to_vec_pretty(&payload).expect("serialize E2E result"),
    )
    .expect("write E2E result");
    println!("{result_label}={}", result_path.display());
    if let Err(error) = outcome {
        panic!("{}: {}", error.code, error.message);
    }
}

/// 失败时把保留的隔离现场关键文件折叠进证据 JSON：
/// config.toml、CAS rules 文件与 Codex rollout 尾部（含真实命令与拒绝详情）。
fn collect_preserved_diagnostics() -> Option<Value> {
    let root = required_path("CAS_E2E_ROOT").ok()?;
    let codex_home = root.join("codex-home");
    if !codex_home.is_dir() {
        return None;
    }
    let read_tail = |path: &Path, max_bytes: usize| -> Option<String> {
        let bytes = fs::read(path).ok()?;
        let start = bytes.len().saturating_sub(max_bytes);
        String::from_utf8(bytes[start..].to_vec()).ok()
    };
    let mut diagnostics = json!({
        "e2eRootPreserved": root.display().to_string(),
        "note": "现场未清理；config/rules/rollout 摘要如下，完整目录请检查 e2eRootPreserved。"
    });
    if let Some(config) = read_tail(&codex_home.join("config.toml"), 8_000) {
        diagnostics["configToml"] = json!(config);
    }
    let rules_dir = codex_home.join("rules");
    if let Ok(entries) = fs::read_dir(&rules_dir) {
        let mut rules = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|value| value == "rules")
                && let Some(content) = read_tail(&path, 8_000)
            {
                rules.push(json!({
                    "file": path.display().to_string(),
                    "content": content
                }));
            }
        }
        diagnostics["rulesFiles"] = Value::Array(rules);
    }
    let sessions_dir = codex_home.join("sessions");
    let mut rollouts = Vec::new();
    if let Ok(entries) = fs::read_dir(&sessions_dir) {
        for entry in entries.flatten().take(20) {
            collect_rollout_tails(&entry.path(), 6_000, &mut rollouts);
        }
    }
    if !rollouts.is_empty() {
        diagnostics["rolloutTails"] = Value::Array(rollouts);
    }
    Some(diagnostics)
}

fn collect_rollout_tails(dir: &Path, max_bytes: usize, rollouts: &mut Vec<Value>) {
    if rollouts.len() >= 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rollout_tails(&path, max_bytes, rollouts);
        } else if path.extension().is_some_and(|value| value == "jsonl")
            && let Some(tail) = read_rollout_tail(&path, max_bytes)
        {
            rollouts.push(json!({
                "file": path.display().to_string(),
                "tail": tail
            }));
        }
        if rollouts.len() >= 6 {
            return;
        }
    }
}

fn read_rollout_tail(path: &Path, max_bytes: usize) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let start = bytes.len().saturating_sub(max_bytes);
    String::from_utf8(bytes[start..].to_vec()).ok()
}

#[test]
fn schedule_protocol_parser_rejects_inconsistent_thread_fields() {
    assert!(parse_schedule_protocol("CAS1|REUSE|-|EXACT_WORKSPACE_SCOPE_IDLE").is_err());
    assert!(parse_schedule_protocol("CAS1|SPAWN|child|NO_WORKSPACE_SCOPE_MATCH").is_err());
    let evidence = parse_schedule_protocol("CAS1|WAIT|-|SPAWN_RESERVED").unwrap();
    assert_eq!(evidence.decision, "WAIT");
    assert_eq!(evidence.reason_code, "SPAWN_RESERVED");
}
