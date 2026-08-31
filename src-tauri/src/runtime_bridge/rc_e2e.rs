use super::*;

use std::env;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use crate::codex_config::ORCHESTRATION_RUNTIME_CONTRACT;
use crate::configuration::{ConfigurationService, RuntimeModeSwitchRequest};
use cas_native_lifecycle::rollout_state;
use cas_scheduler::normalize_workspace_scope_key;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde_json::{Value, json};

const TASK_SCOPE_KEY: &str = "cas-rc1-proof";
const CONCURRENT_TASK_SCOPE_KEY: &str = "cas-rc2-concurrent";

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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
            "DELETE FROM token_usage_records;
             DELETE FROM agent_schedule_decisions;
             DELETE FROM agent_spawn_reservations;
             DELETE FROM agent_thread_instances;
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
}

fn active_agent(connection: &Connection) -> Result<ActiveAgent, Rc1Failure> {
    let requested = env::var("CAS_E2E_AGENT_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let agent = stage(
        connection
            .query_row(
                "SELECT a.id, a.agent_key, a.name, m.model_id, p.provider_key
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
        "thread/read",
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

fn wait_for_turn(
    bridge: &RuntimeBridgeService,
    database_path: &Path,
    parent_thread_id: &str,
    turn_id: &str,
    agent_id: &str,
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
                            "Child 已完成，但僵尸 Primary Turn 无法中断",
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
    drop(restore_connection);
    let fingerprint_decision = fingerprint_result?;
    require_decision(
        &fingerprint_decision,
        "SPAWN",
        Some("RUNTIME_FINGERPRINT_MISMATCH"),
        "RC2_FINGERPRINT_ISOLATION_FAILED",
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
    let _cleanup = TempRoot(root.clone());
    let source_database = required_path("CAS_E2E_SOURCE_DATABASE_PATH")?;
    let source_codex_home = required_path("CAS_E2E_SOURCE_CODEX_HOME")?;
    let helper_source = required_path("CAS_E2E_HELPER_PATH")?;
    let database_path = required_path("CAS_DATABASE_PATH")?;
    let codex_home = root.join("codex-home");
    let data_home = root.join("cas-data");
    let workspace = root.join("workspace");
    if database_path.parent() != Some(data_home.as_path()) {
        return Err(Rc1Failure::new(
            "E2E_CONFIGURATION_INVALID",
            "CAS_DATABASE_PATH 必须位于 CAS_E2E_ROOT/cas-data 内",
        ));
    }
    stage(fs::create_dir_all(&codex_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&data_home), "TEMP_DIRECTORY_FAILED")?;
    stage(fs::create_dir_all(&workspace), "TEMP_DIRECTORY_FAILED")?;
    let helper_path = data_home.join("cas-helper.exe");
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
        RuntimeBridgeService::open(&database_path, &data_home),
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
                    "在当前工作目录完成稳定任务 `{TASK_SCOPE_KEY}` 的第一步：创建 cas-rc1-first.txt，内容只写 CAS_RC1_FIRST，不得修改其他文件。按当前 CAS 编排规则执行。"
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
                    "继续同一个稳定任务 `{TASK_SCOPE_KEY}`：创建 cas-rc1-second.txt，内容只写 CAS_RC1_SECOND，不得修改其他文件。按当前 CAS 编排规则重新预检并执行。"
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
        let native_bind_count: i64 = stage(
            connection.query_row(
                "SELECT COUNT(*) FROM runtime_enforcement_events
                 WHERE session_id = ?1
                   AND reason_code = 'DELEGATION_NATIVE_BIND_CONFIRMED'",
                [&primary_thread_id],
                |row| row.get(0),
            ),
            "EVIDENCE_QUERY_FAILED",
        )?;
        let native_idle_release_count: i64 = stage(
            connection.query_row(
                "SELECT COUNT(*) FROM runtime_enforcement_events
                 WHERE session_id = ?1
                   AND reason_code = 'DELEGATION_LEASE_NATIVE_IDLE_RELEASED'",
                [&primary_thread_id],
                |row| row.get(0),
            ),
            "EVIDENCE_QUERY_FAILED",
        )?;
        let native_reuse_count: i64 = stage(
            connection.query_row(
                "SELECT COUNT(*) FROM runtime_enforcement_events
                 WHERE session_id = ?1
                   AND reason_code = 'DELEGATION_NATIVE_REUSE_CONFIRMED'",
                [&primary_thread_id],
                |row| row.get(0),
            ),
            "EVIDENCE_QUERY_FAILED",
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
        if native_bind_count != 1 || native_idle_release_count != 1 || native_reuse_count != 1 {
            return Err(Rc1Failure::new(
                "RUNTIME_ENFORCEMENT_EVIDENCE_MISSING",
                format!(
                    "兼容准入证据不完整：bind={native_bind_count}, idleRelease={native_idle_release_count}, reuse={native_reuse_count}"
                ),
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
            "firstDecision": "SPAWN",
            "secondDecision": "REUSE",
            "firstPrimaryCompletion": first_completion.as_str(),
            "secondPrimaryCompletion": second_completion.as_str(),
            "bindVerified": true,
            "finalLifecycle": final_instance.status,
            "firstTotalTokens": first_instance.total_tokens,
            "finalTotalTokens": final_instance.total_tokens,
            "usageAttributed": true,
            "nativeBindEvidenceCount": native_bind_count,
            "nativeIdleReleaseEvidenceCount": native_idle_release_count,
            "nativeReuseEvidenceCount": native_reuse_count,
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

fn run_phase6_recovery_e2e() -> Result<Value, Rc1Failure> {
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
        RuntimeBridgeService::open(&database_path, &data_home),
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
                        format!("准备 Turn 结束状态为 {session_status:?}"),
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
    write_e2e_result(run_phase6_recovery_e2e(), "CAS_PHASE6_RESULT");
}

fn run_e2e_test(include_rc2_matrix: bool, result_label: &str) {
    write_e2e_result(run_native_e2e(include_rc2_matrix), result_label);
}

fn write_e2e_result(outcome: Result<Value, Rc1Failure>, result_label: &str) {
    let result_path =
        required_path("CAS_E2E_RESULT_PATH").expect("CAS_E2E_RESULT_PATH is required");
    let payload = match &outcome {
        Ok(value) => value.clone(),
        Err(error) => json!({
            "status": "FAIL",
            "failureCode": error.code,
            "message": error.message
        }),
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

#[test]
fn schedule_protocol_parser_rejects_inconsistent_thread_fields() {
    assert!(parse_schedule_protocol("CAS1|REUSE|-|EXACT_WORKSPACE_SCOPE_IDLE").is_err());
    assert!(parse_schedule_protocol("CAS1|SPAWN|child|NO_WORKSPACE_SCOPE_MATCH").is_err());
    let evidence = parse_schedule_protocol("CAS1|WAIT|-|SPAWN_RESERVED").unwrap();
    assert_eq!(evidence.decision, "WAIT");
    assert_eq!(evidence.reason_code, "SPAWN_RESERVED");
}
