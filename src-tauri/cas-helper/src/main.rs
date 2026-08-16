use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use cas_native_lifecycle::{
    ThreadState as NativeThreadState, rollout_state, thread_state_from_rollout,
};
use cas_scheduler::{
    Candidate, Profile, Recommendation, REUSE_CLAIM_TTL_SECONDS,
    normalize_workspace_scope_key, recommend, runtime_fingerprint as shared_runtime_fingerprint,
};
use cas_secret_store::{CredentialId, SecretStoreError, read};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};

const EXIT_INVALID_ARGUMENTS: u8 = 2;
const EXIT_NOT_FOUND: u8 = 3;
const EXIT_STORE_UNAVAILABLE: u8 = 4;
const EXIT_PERMISSION_DENIED: u8 = 5;
const EXIT_RETRIEVAL_FAILED: u8 = 6;
const EXIT_SCHEDULING_UNAVAILABLE: u8 = 7;

fn main() -> ExitCode {
    match parse_args(env::args_os()) {
        Ok(Command::Token(id)) => deliver(id),
        Ok(Command::Schedule {
            database_path,
            agent_key,
            scope_key,
            task_scope_key,
        }) => {
            let database_path = database_path.or_else(default_database_path);
            let scope_key = scope_key.or_else(|| {
                env::current_dir()
                    .ok()
                    .and_then(|path| normalize_workspace_scope_key(&path.to_string_lossy()))
            });
            match (database_path, scope_key) {
                (Some(database_path), Some(scope_key)) => {
                    schedule(database_path, &agent_key, &scope_key, task_scope_key.as_deref())
                }
                _ => {
                    eprintln!("CAS scheduling environment unavailable.");
                    ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE)
                }
            }
        }
        Ok(Command::Bind {
            agent_key,
            child_thread_id,
            task_scope_key,
        }) => {
            let database_path = default_database_path();
            let scope_key = env::current_dir()
                .ok()
                .and_then(|path| normalize_workspace_scope_key(&path.to_string_lossy()));
            let parent_thread_id = env::var("CODEX_THREAD_ID")
                .ok()
                .and_then(|value| valid_runtime_key(&value));
            match (database_path, scope_key, parent_thread_id) {
                (Some(database_path), Some(scope_key), Some(parent_thread_id)) => bind(
                    database_path,
                    &agent_key,
                    &child_thread_id,
                    &scope_key,
                    &parent_thread_id,
                    task_scope_key.as_deref(),
                ),
                _ => {
                    eprintln!("CAS bind environment unavailable.");
                    ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE)
                }
            }
        }
        Err(()) => {
            eprintln!(
                "Usage:\n  cas-helper token <credential-id>\n  \
                 cas-helper schedule <agent-key>\n  \
                 cas-helper schedule <database-path> <agent-key> <workspace-scope> [task-key]\n  \
                 cas-helper bind <agent-key> <child-thread-id> [task-key]"
            );
            ExitCode::from(EXIT_INVALID_ARGUMENTS)
        }
    }
}

enum Command {
    Token(CredentialId),
    Schedule {
        database_path: Option<PathBuf>,
        agent_key: String,
        scope_key: Option<String>,
        task_scope_key: Option<String>,
    },
    Bind {
        agent_key: String,
        child_thread_id: String,
        task_scope_key: Option<String>,
    },
}

fn valid_task_key(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 200
        && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}

fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, ()> {
    let mut args = args.into_iter();
    let _program = args.next();
    let command = args.next().ok_or(())?;
    if command == OsStr::new("token") {
        let credential_id = args.next().ok_or(())?;
        if args.next().is_some() {
            return Err(());
        }
        return CredentialId::from_str(credential_id.to_str().ok_or(())?)
            .map(Command::Token)
            .map_err(|_| ());
    }
    if command == OsStr::new("schedule") {
        let first = args.next().ok_or(())?;
        let second = args.next();
        if second.is_none() {
            if args.next().is_some() {
                return Err(());
            }
            return Ok(Command::Schedule {
                database_path: None,
                agent_key: valid_argument(first)?,
                scope_key: None,
                task_scope_key: None,
            });
        }
        let database_path = PathBuf::from(first);
        let agent_key = valid_argument(second.expect("checked above"))?;
        let scope_key =
            normalize_workspace_scope_key(&valid_argument(args.next().ok_or(())?)?).ok_or(())?;
        let task_scope_key = match args.next() {
            Some(value) => Some(
                valid_task_key(&valid_argument(value)?).ok_or(())?,
            ),
            None => None,
        };
        if args.next().is_some() || database_path.as_os_str().is_empty() {
            return Err(());
        }
        return Ok(Command::Schedule {
            database_path: Some(database_path),
            agent_key,
            scope_key: Some(scope_key),
            task_scope_key,
        });
    }
    if command == OsStr::new("bind") {
        let agent_key = valid_argument(args.next().ok_or(())?)?;
        let child_thread_id =
            valid_runtime_key(&valid_argument(args.next().ok_or(())?)?).ok_or(())?;
        let task_scope_key = match args.next() {
            Some(value) => Some(
                valid_task_key(&valid_argument(value)?).ok_or(())?,
            ),
            None => None,
        };
        if args.next().is_some() {
            return Err(());
        }
        return Ok(Command::Bind {
            agent_key,
            child_thread_id,
            task_scope_key,
        });
    }
    Err(())
}

fn valid_argument(value: OsString) -> Result<String, ()> {
    let value = value.into_string().map_err(|_| ())?;
    let value = value.trim();
    (!value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
        .ok_or(())
}

fn default_database_path() -> Option<PathBuf> {
    if let Some(path) = env::var_os("CAS_DATABASE_PATH").map(PathBuf::from)
        && path.is_absolute()
    {
        return Some(path);
    }
    let base = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library").join("Application Support"))
    } else {
        env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
    }?;
    Some(base.join("com.codexagentswitch.desktop").join("cas.db"))
}

fn deliver(id: CredentialId) -> ExitCode {
    match read(id) {
        Ok(secret) => {
            let mut stdout = io::stdout().lock();
            if stdout.write_all(secret.expose()).is_err()
                || stdout.write_all(b"\n").is_err()
                || stdout.flush().is_err()
            {
                eprintln!("Credential output failed.");
                return ExitCode::from(EXIT_RETRIEVAL_FAILED);
            }
            ExitCode::SUCCESS
        }
        Err(SecretStoreError::NotFound) => {
            eprintln!("Credential not found.");
            ExitCode::from(EXIT_NOT_FOUND)
        }
        Err(SecretStoreError::Unavailable) => {
            eprintln!("Credential store unavailable.");
            ExitCode::from(EXIT_STORE_UNAVAILABLE)
        }
        Err(SecretStoreError::AccessDenied) => {
            eprintln!("Credential access denied.");
            ExitCode::from(EXIT_PERMISSION_DENIED)
        }
        Err(_) => {
            eprintln!("Credential retrieval failed.");
            ExitCode::from(EXIT_RETRIEVAL_FAILED)
        }
    }
}

fn schedule(
    database_path: PathBuf,
    agent_key: &str,
    scope_key: &str,
    task_scope_key: Option<&str>,
) -> ExitCode {
    let parent_thread_id = match env::var("CODEX_THREAD_ID")
        .ok()
        .and_then(|value| valid_runtime_key(&value))
    {
        Some(value) => value,
        None => {
            eprintln!("CODEX_THREAD_ID unavailable; native scheduling stopped.");
            return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
        }
    };
    let mut connection = match Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => {
            eprintln!("CAS scheduling database unavailable.");
            return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
        }
    };
    if connection.busy_timeout(Duration::from_secs(5)).is_err() {
        eprintln!("CAS scheduling database unavailable.");
        return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
    }
    let codex_home = match resolve_codex_home(
        &connection,
        env::var_os("CODEX_HOME"),
        env::var_os("USERPROFILE"),
        env::var_os("HOME"),
    ) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Codex home unavailable; native scheduling stopped.");
            return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
        }
    };
    match load_recommendation(
        &mut connection,
        &codex_home,
        agent_key,
        scope_key,
        &parent_thread_id,
        task_scope_key,
    ) {
        Ok(Some(recommendation)) => match protocol_line(&recommendation) {
            Some(line) => {
                println!("{line}");
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("CAS scheduling result invalid.");
                ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE)
            }
        },
        Ok(None) => {
            eprintln!("CAS agent is not active.");
            ExitCode::from(EXIT_NOT_FOUND)
        }
        Err(_) => {
            eprintln!("CAS scheduling query failed.");
            ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE)
        }
    }
}

fn bind(
    database_path: PathBuf,
    agent_key: &str,
    child_thread_id: &str,
    scope_key: &str,
    parent_thread_id: &str,
    task_scope_key: Option<&str>,
) -> ExitCode {
    let mut connection = match Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => {
            eprintln!("CAS bind database unavailable.");
            return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
        }
    };
    if connection.busy_timeout(Duration::from_secs(5)).is_err() {
        eprintln!("CAS bind database unavailable.");
        return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
    }
    let codex_home = match resolve_codex_home(
        &connection,
        env::var_os("CODEX_HOME"),
        env::var_os("USERPROFILE"),
        env::var_os("HOME"),
    ) {
        Ok(path) => path,
        Err(_) => {
            eprintln!("Codex home unavailable; bind stopped.");
            return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
        }
    };
    match bind_native_thread(
        &mut connection,
        &codex_home,
        agent_key,
        child_thread_id,
        scope_key,
        parent_thread_id,
        task_scope_key,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            eprintln!("CAS bind verification failed.");
            ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE)
        }
    }
}

fn resolve_codex_home(
    connection: &Connection,
    codex_home: Option<OsString>,
    user_profile: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, ScheduleError> {
    let custom_codex_home = connection
        .query_row(
            "SELECT setting_value FROM application_settings
             WHERE setting_key = 'custom_codex_home'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .map(OsString::from);
    let path = select_codex_home_path(
        custom_codex_home,
        codex_home,
        user_profile,
        home,
        cfg!(windows),
    )
    .ok_or(ScheduleError::NativeStateUnavailable)?;
    path.is_dir()
        .then_some(path)
        .ok_or(ScheduleError::NativeStateUnavailable)
}

fn select_codex_home_path(
    custom_codex_home: Option<OsString>,
    codex_home: Option<OsString>,
    user_profile: Option<OsString>,
    home: Option<OsString>,
    use_user_profile: bool,
) -> Option<PathBuf> {
    non_empty_path(custom_codex_home)
        .or_else(|| non_empty_path(codex_home))
        .or_else(|| {
            if use_user_profile {
                non_empty_path(user_profile).map(|path| path.join(".codex"))
            } else {
                non_empty_path(home).map(|path| path.join(".codex"))
            }
        })
}

fn non_empty_path(value: Option<OsString>) -> Option<PathBuf> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

fn load_recommendation(
    connection: &mut Connection,
    codex_home: &Path,
    agent_key: &str,
    scope_key: &str,
    parent_thread_id: &str,
    task_scope_key: Option<&str>,
) -> Result<Option<Recommendation>, ScheduleError> {
    let Some(active) = load_active_agent_profile(connection, agent_key)? else {
        return Ok(None);
    };
    // F-11：候选读取与 REUSE 租约写入必须在同一 Immediate 事务内，
    // 否则两个并发 helper 预检可以同时选中同一个 IDLE Thread（双重 REUSE）。
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // 显式 Task Scope：同键才可复用；无键任务不得复用绑定了任务键的 Thread（fail-closed）。
    let (task_condition, task_parameter) = match task_scope_key {
        Some(key) => ("AND task_scope_key = ?4", Some(key.to_owned())),
        None => ("AND task_scope_key IS NULL", None),
    };
    let mut statement = transaction.prepare(&format!(
        "SELECT id, codex_thread_id, status, input_tokens, cached_input_tokens,
                output_tokens, total_tokens, current_context_tokens, context_window, runtime_fingerprint,
                CAST(MAX(0, (julianday('now') - julianday(last_model_usage_at)) * 86400) AS INTEGER),
                CASE WHEN claimed_until IS NOT NULL
                          AND julianday(claimed_until) > julianday('now')
                     THEN 1 ELSE 0 END
         FROM agent_thread_instances
         WHERE agent_id = ?1 AND scope_key = ?2 AND parent_thread_id = ?3 {task_condition}
         ORDER BY last_used_at DESC, codex_thread_id ASC",
    ))?;
    let mut bound_parameters = vec![
        active.agent_id.clone(),
        scope_key.to_owned(),
        parent_thread_id.to_owned(),
    ];
    if let Some(task_parameter) = task_parameter {
        bound_parameters.push(task_parameter);
    }
    let candidates = statement
        .query_map(rusqlite::params_from_iter(bound_parameters), |row| {
                Ok(Candidate {
                    instance_id: row.get(0)?,
                    thread_id: row.get(1)?,
                    status: row.get(2)?,
                    input_tokens: row.get(3)?,
                    cached_input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    total_tokens: row.get(6)?,
                    current_context_tokens: row.get(7)?,
                    context_window: row.get(8)?,
                    runtime_fingerprint: row.get(9)?,
                    age_seconds: row.get(10)?,
                    claimed: row.get(11)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    drop(statement);
    let native_candidates = load_native_candidates(
        &transaction,
        codex_home,
        &active.agent_id,
        agent_key,
        active.role_key.as_deref(),
        scope_key,
        parent_thread_id,
    )?;
    let mut candidates = candidates
        .into_iter()
        .map(|candidate| (candidate.thread_id.clone(), candidate))
        .collect::<std::collections::BTreeMap<_, _>>();
    for mut candidate in native_candidates {
        if let Some(existing) = candidates.get(&candidate.thread_id) {
            candidate.runtime_fingerprint = existing.runtime_fingerprint.clone();
            // 原生 updated_at 不是可证明的模型使用时间；保留 CAS 已知值，否则保持未知。
            candidate.age_seconds = existing.age_seconds;
            candidate.claimed = existing.claimed;
            candidate.instance_id = existing.instance_id.clone();
        }
        candidates.insert(candidate.thread_id.clone(), candidate);
    }
    let mut candidates = candidates.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.age_seconds
            .unwrap_or(i64::MAX)
            .cmp(&right.age_seconds.unwrap_or(i64::MAX))
            .then_with(|| left.thread_id.cmp(&right.thread_id))
    });
    let runtime_fingerprint = active.profile.runtime_fingerprint.clone();
    let mut recommendation = recommend(scope_key.to_owned(), candidates, active.profile);
    if recommendation.decision == "REUSE"
        && let Some(instance_id) = recommendation.candidate_instance_id.clone()
    {
        // 不校验 status：原生合并后的候选状态来自 rollout 新鲜事实，
        // CAS 行内状态可能尚未同步；租约只依赖身份与租约空闲条件。
        let changed = transaction.execute(
            "UPDATE agent_thread_instances
             SET claimed_until = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?2)
             WHERE id = ?1
               AND (
                   claimed_until IS NULL
                   OR claimed_until <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
               )",
            params![instance_id, format!("+{REUSE_CLAIM_TTL_SECONDS} seconds")],
        )?;
        if changed != 1 {
            return Err(ScheduleError::Database);
        }
    }
    // F-13：与决策同事务写入只追加审计记录，UI 预览不能冒充实际执行日志。
    transaction.execute(
        "INSERT INTO agent_schedule_decisions (
            id, created_at, source, agent_id, agent_name_snapshot, workspace_scope_key,
            parent_thread_id, candidate_thread_id, decision, reason_code, runtime_fingerprint,
            context_pressure_percent, context_pressure_limit_percent, cache_hint,
            candidate_age_seconds, claimed, task_scope_key
         ) VALUES (
            lower(hex(randomblob(16))),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            'HELPER', ?1, (SELECT name FROM agents WHERE id = ?1), ?2,
            ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13
         )",
        params![
            active.agent_id,
            scope_key,
            parent_thread_id,
            recommendation.candidate_thread_id,
            recommendation.decision,
            recommendation.reason_code,
            runtime_fingerprint,
            recommendation.context_pressure_percent,
            recommendation.context_pressure_limit_percent,
            recommendation.cache_hint,
            recommendation.candidate_age_seconds,
            i64::from(recommendation.decision == "REUSE"
                || recommendation.reason_code == "THREAD_CLAIMED"),
            task_scope_key,
        ],
    )?;
    transaction.commit()?;
    Ok(Some(recommendation))
}

struct ActiveAgentProfile {
    agent_id: String,
    role_key: Option<String>,
    profile: Profile,
}

fn load_active_agent_profile(
    connection: &Connection,
    agent_key: &str,
) -> Result<Option<ActiveAgentProfile>, ScheduleError> {
    let profile = connection
        .query_row(
            "SELECT a.id, a.role_key, a.reuse_strategy, a.cache_retention_override_seconds,
                    COALESCE(p.cache_support, 'UNKNOWN'),
                    COALESCE(p.cache_retention_type, 'UNKNOWN'),
                    p.cache_retention_hint_seconds, a.instruction, a.sandbox_policy,
                    a.reasoning_policy, m.id, m.model_id, p.provider_key, p.preset_id,
                    p.base_url, p.protocol, p.custom_headers_json
             FROM active_agent_bindings active
             JOIN agents a ON a.id = active.agent_id AND a.enabled = 1
             LEFT JOIN agent_model_bindings b ON b.agent_id = a.id AND b.enabled = 1
             LEFT JOIN models m ON m.id = b.model_id
             LEFT JOIN providers p ON p.id = m.provider_id
            WHERE a.agent_key = ?1",
            [agent_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, String>(14)?,
                    row.get::<_, String>(15)?,
                    row.get::<_, Option<String>>(16)?,
                ))
            },
        )
        .optional()?;
    let Some((
        agent_id,
        role_key,
        reuse_strategy,
        agent_cache_retention_override_seconds,
        cache_support,
        cache_retention_type,
        cache_retention_hint_seconds,
        instruction,
        sandbox_policy,
        reasoning_policy,
        model_id,
        model_slug,
        provider_key,
        preset_id,
        base_url,
        protocol,
        custom_headers_json,
    )) = profile
    else {
        return Ok(None);
    };
    Ok(Some(ActiveAgentProfile {
        agent_id: agent_id.clone(),
        role_key,
        profile: Profile {
            reuse_strategy,
            agent_cache_retention_override_seconds,
            cache_support,
            cache_retention_type,
            cache_retention_hint_seconds,
            runtime_fingerprint: Some(runtime_fingerprint(
                connection,
                &agent_id,
                &model_id,
                &provider_key,
                preset_id.as_deref(),
                &base_url,
                &protocol,
                custom_headers_json.as_deref(),
                &model_slug,
                &reasoning_policy,
                &sandbox_policy,
                &instruction,
            )?),
        },
    }))
}

#[derive(Debug)]
enum ScheduleError {
    Database,
    NativeStateUnavailable,
    NativeStateIncompatible,
    BindRejected,
}

impl From<rusqlite::Error> for ScheduleError {
    fn from(_: rusqlite::Error) -> Self {
        Self::Database
    }
}

#[derive(Debug)]
struct NativeCandidateRecord {
    thread_id: String,
    parent_thread_id: String,
    agent_role: Option<String>,
    model_provider: String,
    model_slug: Option<String>,
    edge_status: String,
    total_tokens: i64,
    scope_key: String,
    updated_at: i64,
    rollout_path: String,
}

fn load_native_candidates(
    cas_connection: &Connection,
    codex_home: &Path,
    agent_id: &str,
    agent_key: &str,
    role_key: Option<&str>,
    scope_key: &str,
    parent_thread_id: &str,
) -> Result<Vec<Candidate>, ScheduleError> {
    let state_path = find_codex_state_database(codex_home)?;
    let state_connection = Connection::open_with_flags(
        state_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| ScheduleError::NativeStateUnavailable)?;
    if !native_state_schema_supported(&state_connection) {
        return Err(ScheduleError::NativeStateIncompatible);
    }
    let mut candidates = Vec::new();
    for record in load_native_candidate_records(&state_connection)? {
        if record.parent_thread_id != parent_thread_id
            || record.scope_key != scope_key
            || !native_record_matches_agent(cas_connection, agent_id, agent_key, role_key, &record)?
        {
            continue;
        }
        let rollout = rollout_state(Path::new(&record.rollout_path)).ok();
        let status = match thread_state_from_rollout(&record.edge_status, rollout.as_ref()) {
            NativeThreadState::Closed => "CLOSED",
            NativeThreadState::Idle => "IDLE",
            NativeThreadState::Running => "RUNNING",
            NativeThreadState::Unknown => "UNKNOWN",
        };
        candidates.push(Candidate {
            instance_id: format!("native-{}", record.thread_id),
            thread_id: record.thread_id,
            status: status.to_owned(),
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            total_tokens: record.total_tokens,
            current_context_tokens: rollout.and_then(|state| state.current_context_tokens),
            context_window: rollout.and_then(|state| state.model_context_window),
            runtime_fingerprint: None,
            // F-10：threads.updated_at 无法证明为最近模型请求时间，缓存年龄保持未知。
            age_seconds: None,
            claimed: false,
        });
    }
    Ok(candidates)
}

fn bind_native_thread(
    connection: &mut Connection,
    codex_home: &Path,
    agent_key: &str,
    child_thread_id: &str,
    scope_key: &str,
    parent_thread_id: &str,
    task_scope_key: Option<&str>,
) -> Result<(), ScheduleError> {
    let Some(active) = load_active_agent_profile(connection, agent_key)? else {
        return Err(ScheduleError::BindRejected);
    };
    let state_path = find_codex_state_database(codex_home)?;
    let state_connection = Connection::open_with_flags(
        state_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| ScheduleError::NativeStateUnavailable)?;
    if !native_state_schema_supported(&state_connection) {
        return Err(ScheduleError::NativeStateIncompatible);
    }
    let record = load_native_candidate_records(&state_connection)?
        .into_iter()
        .find(|record| record.thread_id == child_thread_id)
        .ok_or(ScheduleError::BindRejected)?;
    if record
        .model_slug
        .as_deref()
        .is_none_or(|model| model.trim().is_empty())
        || record.model_provider.trim().is_empty()
    {
        return Err(ScheduleError::BindRejected);
    }
    if record.parent_thread_id != parent_thread_id
        || record.scope_key != scope_key
        || !(record.agent_role.as_deref() == active.role_key.as_deref()
            || record.agent_role.as_deref() == Some(agent_key))
        || !native_record_matches_agent(
            connection,
            &active.agent_id,
            agent_key,
            active.role_key.as_deref(),
            &record,
        )?
    {
        return Err(ScheduleError::BindRejected);
    }
    let fingerprint = active
        .profile
        .runtime_fingerprint
        .as_deref()
        .ok_or(ScheduleError::BindRejected)?;
    let transaction = connection.transaction()?;
    let existing = transaction
        .query_row(
            "SELECT agent_id, parent_thread_id, scope_key, runtime_fingerprint
             FROM agent_thread_instances WHERE codex_thread_id = ?1",
            [child_thread_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    if let Some((existing_agent_id, existing_parent_thread_id, existing_scope_key, existing)) =
        existing
        && (existing_agent_id
            .as_deref()
            .is_some_and(|existing| existing != active.agent_id)
            || existing_parent_thread_id
                .as_deref()
                .is_some_and(|existing| existing != parent_thread_id)
            || existing_scope_key
                .as_deref()
                .is_some_and(|existing| existing != scope_key)
            || existing
                .as_deref()
                .is_some_and(|existing| existing != fingerprint))
    {
        return Err(ScheduleError::BindRejected);
    }
    transaction.execute(
        "INSERT INTO agent_thread_instances (
            id, agent_id, codex_thread_id, parent_thread_id, scope_key, status,
            input_tokens, cached_input_tokens, output_tokens, total_tokens,
            current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at,
            last_observed_at, task_scope_key
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, 'UNKNOWN', 0, 0, 0, 0, NULL, NULL, ?6,
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            ?7
         )
         ON CONFLICT(codex_thread_id) DO UPDATE SET
            agent_id = COALESCE(agent_thread_instances.agent_id, excluded.agent_id),
            parent_thread_id = COALESCE(
                agent_thread_instances.parent_thread_id,
                excluded.parent_thread_id
            ),
            scope_key = COALESCE(agent_thread_instances.scope_key, excluded.scope_key),
            runtime_fingerprint = COALESCE(
                agent_thread_instances.runtime_fingerprint,
                excluded.runtime_fingerprint
            ),
            last_observed_at = excluded.last_observed_at,
            task_scope_key = COALESCE(agent_thread_instances.task_scope_key, excluded.task_scope_key)",
        params![
            format!("native-{child_thread_id}"),
            active.agent_id,
            child_thread_id,
            parent_thread_id,
            scope_key,
            fingerprint,
            task_scope_key,
        ],
    )?;
    transaction.commit()?;
    Ok(())
}

fn find_codex_state_database(codex_home: &Path) -> Result<PathBuf, ScheduleError> {
    let path = std::fs::read_dir(codex_home)
        .map_err(|_| ScheduleError::NativeStateUnavailable)?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            let version = name
                .strip_prefix("state_")?
                .strip_suffix(".sqlite")?
                .parse::<u64>()
                .ok()?;
            path.is_file().then_some((version, path))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path);
    path.ok_or(ScheduleError::NativeStateUnavailable)
}

fn native_state_schema_supported(connection: &Connection) -> bool {
    let required = [
        (
            "threads",
            [
                "id",
                "agent_role",
                "model_provider",
                "model",
                "tokens_used",
                "cwd",
                "rollout_path",
                "created_at",
                "updated_at",
            ]
            .as_slice(),
        ),
        (
            "thread_spawn_edges",
            ["parent_thread_id", "child_thread_id", "status"].as_slice(),
        ),
    ];
    required.iter().all(|(table, columns)| {
        table_columns(connection, table)
            .is_ok_and(|found| columns.iter().all(|column| found.contains(*column)))
    })
}

fn table_columns(
    connection: &Connection,
    table: &str,
) -> rusqlite::Result<std::collections::BTreeSet<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    statement
        .query_map([], |row| row.get(1))?
        .collect::<Result<_, _>>()
}

fn load_native_candidate_records(
    connection: &Connection,
) -> Result<Vec<NativeCandidateRecord>, ScheduleError> {
    let mut statement = connection.prepare(
        "SELECT child.id, edge.parent_thread_id, child.agent_role,
                child.model_provider, child.model, child.tokens_used, edge.status,
                child.cwd, child.rollout_path, child.updated_at
         FROM thread_spawn_edges edge
         JOIN threads child ON child.id = edge.child_thread_id
         ORDER BY child.updated_at DESC",
    )?;
    statement
        .query_map([], |row| {
            let thread_id = row.get::<_, String>(0)?;
            let cwd = row.get::<_, String>(7)?;
            let scope_key =
                normalize_workspace_scope_key(&cwd).ok_or(rusqlite::Error::InvalidQuery)?;
            let rollout_path = row.get::<_, String>(8)?;
            Ok(NativeCandidateRecord {
                thread_id,
                parent_thread_id: row.get(1)?,
                agent_role: row.get(2)?,
                model_provider: row.get(3)?,
                model_slug: row.get(4)?,
                edge_status: row.get(6)?,
                total_tokens: row.get::<_, i64>(5)?.max(0),
                scope_key,
                updated_at: row.get::<_, i64>(9)?.max(0),
                rollout_path,
            })
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(ScheduleError::from)
}

fn native_record_matches_agent(
    connection: &Connection,
    agent_id: &str,
    agent_key: &str,
    role_key: Option<&str>,
    record: &NativeCandidateRecord,
) -> Result<bool, ScheduleError> {
    let Some(model_slug) = record
        .model_slug
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(record.agent_role.as_deref() == role_key
            || record.agent_role.as_deref() == Some(agent_key));
    };
    if record.model_provider.trim().is_empty() {
        return Ok(record.agent_role.as_deref() == role_key
            || record.agent_role.as_deref() == Some(agent_key));
    }
    let mut statement = connection.prepare(
        "SELECT p.provider_key, p.preset_id
         FROM agent_model_bindings binding
         JOIN models m ON m.id = binding.model_id AND m.enabled = 1
         JOIN providers p ON p.id = m.provider_id AND p.enabled = 1
         WHERE binding.agent_id = ?1 AND binding.enabled = 1 AND m.model_id = ?2",
    )?;
    let providers = statement
        .query_map(params![agent_id, model_slug], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(providers.into_iter().any(|(provider_key, preset_id)| {
        record.model_provider == format!("cas_{provider_key}")
            || (preset_id.as_deref() == Some("codex-native")
                && matches!(record.model_provider.as_str(), "openai" | "chatgpt"))
    }))
}

fn protocol_line(recommendation: &Recommendation) -> Option<String> {
    let thread_id = match recommendation.decision {
        "REUSE" => protocol_field(recommendation.candidate_thread_id.as_deref())?,
        "SPAWN" => "-",
        _ => return None,
    };
    let reason_code = protocol_field(Some(recommendation.reason_code))?;
    Some(format!(
        "CAS1|{}|{}|{}",
        recommendation.decision, thread_id, reason_code,
    ))
}

fn runtime_fingerprint(
    connection: &Connection,
    agent_id: &str,
    model_id: &str,
    provider_key: &str,
    preset_id: Option<&str>,
    base_url: &str,
    protocol: &str,
    custom_headers_json: Option<&str>,
    model_slug: &str,
    reasoning_policy: &str,
    sandbox_policy: &str,
    instruction: &str,
) -> Result<String, rusqlite::Error> {
    Ok(shared_runtime_fingerprint(&[
        ("provider_key", vec![provider_key.to_owned()]),
        (
            "provider_preset_id",
            vec![preset_id.unwrap_or_default().to_owned()],
        ),
        ("provider_base_url", vec![base_url.to_owned()]),
        ("provider_protocol", vec![protocol.to_owned()]),
        (
            "provider_custom_headers_json",
            vec![custom_headers_json.unwrap_or_default().to_owned()],
        ),
        ("model", vec![model_slug.to_owned()]),
        ("reasoning", vec![reasoning_policy.to_owned()]),
        ("sandbox", vec![sandbox_policy.to_owned()]),
        ("instruction", vec![instruction.to_owned()]),
        (
            "required_capabilities",
            fingerprint_values(
                connection,
                "SELECT capability FROM agent_required_capabilities WHERE agent_id = ?1",
                agent_id,
            )?,
        ),
        (
            "preferred_capabilities",
            fingerprint_values(
                connection,
                "SELECT capability FROM agent_preferred_capabilities WHERE agent_id = ?1",
                agent_id,
            )?,
        ),
        (
            "model_capabilities",
            fingerprint_values(
                connection,
                "SELECT capability || '=' || status
                 FROM model_capabilities WHERE model_id = ?1",
                model_id,
            )?,
        ),
    ]))
}

fn fingerprint_values(
    connection: &Connection,
    sql: &str,
    parameter: &str,
) -> Result<Vec<String>, rusqlite::Error> {
    connection
        .prepare(sql)?
        .query_map([parameter], |row| row.get(0))?
        .collect()
}

fn protocol_field(value: Option<&str>) -> Option<&str> {
    let value = value?.trim();
    (!value.is_empty()
        && value.len() <= 256
        && !value.contains('|')
        && !value.chars().any(char::is_control))
    .then_some(value)
}

fn valid_runtime_key(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn accepts_only_token_and_uuid() {
        let valid = [
            OsString::from("cas-helper"),
            OsString::from("token"),
            OsString::from("0198ae47-1234-5678-9abc-0123456789ef"),
        ];
        let injected = [
            OsString::from("cas-helper"),
            OsString::from("token"),
            OsString::from("../../secret"),
        ];

        assert!(matches!(parse_args(valid), Ok(Command::Token(_))));
        assert!(parse_args(injected).is_err());
    }

    #[test]
    fn schedule_arguments_normalize_workspace_scope() {
        let args = [
            OsString::from("cas-helper"),
            OsString::from("schedule"),
            OsString::from(r"C:\CAS Data\cas.db"),
            OsString::from("executor"),
            OsString::from(r"\\?\C:\Workspace\Project"),
        ];
        let Ok(Command::Schedule {
            scope_key: Some(scope_key),
            ..
        }) = parse_args(args)
        else {
            panic!("schedule command should parse");
        };
        assert_eq!(scope_key, "c:/workspace/project");
        assert!(
            parse_args([
                OsString::from("cas-helper"),
                OsString::from("schedule"),
                OsString::from(r"C:\CAS Data\cas.db"),
                OsString::from("executor"),
                OsString::from("order/refund"),
            ])
            .is_err()
        );
    }

    #[test]
    fn short_schedule_needs_only_agent_key() {
        let args = [
            OsString::from("cas-helper"),
            OsString::from("schedule"),
            OsString::from("executor"),
        ];
        let Ok(Command::Schedule {
            database_path,
            agent_key,
            scope_key,
            task_scope_key,
        }) = parse_args(args)
        else {
            panic!("short schedule command should parse");
        };
        assert_eq!(database_path, None);
        assert_eq!(agent_key, "executor");
        assert_eq!(scope_key, None);
        assert_eq!(task_scope_key, None);
    }

    #[test]
    fn bind_requires_exact_agent_and_child_thread_arguments() {
        let valid = [
            OsString::from("cas-helper"),
            OsString::from("bind"),
            OsString::from("executor"),
            OsString::from("thread-child"),
        ];
        assert!(matches!(
            parse_args(valid),
            Ok(Command::Bind {
                agent_key,
                child_thread_id,
                task_scope_key: None
            }) if agent_key == "executor" && child_thread_id == "thread-child"
        ));
        assert!(
            parse_args([
                OsString::from("cas-helper"),
                OsString::from("bind"),
                OsString::from("executor"),
            ])
            .is_err()
        );
    }

    #[test]
    fn custom_codex_home_precedes_environment_and_platform_defaults() {
        let root = unique_temp_dir("codex-home-precedence");
        let custom = root.join("custom");
        let environment = root.join("environment");
        let user_profile = root.join("user-profile");
        let home = root.join("home");
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::create_dir_all(&environment).unwrap();
        std::fs::create_dir_all(user_profile.join(".codex")).unwrap();
        std::fs::create_dir_all(home.join(".codex")).unwrap();
        let connection = settings_connection();
        connection
            .execute(
                "INSERT INTO application_settings VALUES ('custom_codex_home', ?1)",
                [custom.to_string_lossy().as_ref()],
            )
            .unwrap();

        assert_eq!(
            resolve_codex_home(
                &connection,
                Some(environment.clone().into_os_string()),
                Some(user_profile.clone().into_os_string()),
                Some(home.clone().into_os_string()),
            )
            .unwrap(),
            custom
        );
        connection
            .execute(
                "DELETE FROM application_settings WHERE setting_key = 'custom_codex_home'",
                [],
            )
            .unwrap();
        assert_eq!(
            resolve_codex_home(
                &connection,
                Some(environment.clone().into_os_string()),
                Some(user_profile.clone().into_os_string()),
                Some(home.clone().into_os_string()),
            )
            .unwrap(),
            environment
        );
        assert_eq!(
            select_codex_home_path(
                None,
                None,
                Some(user_profile.clone().into_os_string()),
                Some(home.clone().into_os_string()),
                true,
            ),
            Some(user_profile.join(".codex"))
        );
        assert_eq!(
            select_codex_home_path(
                None,
                None,
                Some(user_profile.into_os_string()),
                Some(home.clone().into_os_string()),
                false,
            ),
            Some(home.join(".codex"))
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn empty_custom_codex_home_does_not_hide_later_sources() {
        let root = unique_temp_dir("empty-custom-codex-home");
        let environment = root.join("environment");
        std::fs::create_dir_all(&environment).unwrap();
        let connection = settings_connection();
        connection
            .execute(
                "INSERT INTO application_settings VALUES ('custom_codex_home', '')",
                [],
            )
            .unwrap();

        assert_eq!(
            resolve_codex_home(
                &connection,
                Some(environment.clone().into_os_string()),
                None,
                None,
            )
            .unwrap(),
            environment
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_selected_codex_home_stops_scheduling() {
        let root = unique_temp_dir("missing-codex-home");
        let connection = settings_connection();
        connection
            .execute(
                "INSERT INTO application_settings VALUES ('custom_codex_home', ?1)",
                [root.join("missing").to_string_lossy().as_ref()],
            )
            .unwrap();

        assert!(matches!(
            resolve_codex_home(
                &connection,
                Some(root.join("environment").into_os_string()),
                None,
                None,
            ),
            Err(ScheduleError::NativeStateUnavailable)
        ));
    }

    #[test]
    fn compact_protocol_contains_only_action_fields() {
        let recommendation = Recommendation {
            decision: "REUSE",
            reason_code: "EXACT_WORKSPACE_SCOPE_IDLE",
            message: "unused",
            workspace_scope_key: "c:/workspace/project".to_owned(),
            candidate_instance_id: Some("instance-1".to_owned()),
            candidate_thread_id: Some("thread-child".to_owned()),
            context_pressure_percent: Some(20),
            context_pressure_limit_percent: 80,
            reuse_strategy: "AUTO".to_owned(),
            cache_support: "SUPPORTED".to_owned(),
            cache_retention_type: "APPROXIMATE".to_owned(),
            cache_retention_hint_seconds: Some(300),
            cache_retention_source: "PROVIDER",
            cache_hint: "WITHIN_RETENTION_HINT",
            candidate_age_seconds: Some(10),
        };
        assert_eq!(
            protocol_line(&recommendation),
            Some("CAS1|REUSE|thread-child|EXACT_WORKSPACE_SCOPE_IDLE".to_owned())
        );
        let mut invalid = recommendation;
        invalid.candidate_thread_id = Some("thread|injected".to_owned());
        assert_eq!(protocol_line(&invalid), None);
    }

    #[test]
    fn scheduling_is_limited_to_current_primary_thread() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-1', 'agent-1', 'thread-child', 'thread-root',
                    'c:/workspace/project', 'IDLE', 20, 0, 5, 25, 100,
                    '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', 25, NULL, NULL, NULL, NULL,
                    NULL
                 )",
                [],
            )
            .unwrap();

        let reuse = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reuse.decision, "SPAWN");
        assert_eq!(reuse.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");

        let other_primary = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-other",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(other_primary.decision, "SPAWN");
        assert_eq!(other_primary.candidate_thread_id, None);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_child_is_scheduled_without_cas_sync() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "thread-native", "thread-root", 10, "open");
        std::fs::create_dir_all(home.join("thread-writer-locks")).unwrap();
        std::fs::write(home.join("thread-writer-locks/thread-native.lock"), "").unwrap();

        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(recommendation.decision, "SPAWN");
        assert_eq!(recommendation.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        // F-10：原生 threads.updated_at 不得被当作模型使用时间，候选年龄必须保持未知。
        assert_eq!(recommendation.candidate_age_seconds, None);
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn active_native_turn_is_not_scheduled_for_reuse() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "thread-native", "thread-root", 10, "open");
        write_native_rollout(&home, "thread-native", "task_started");

        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(recommendation.decision, "SPAWN");
        assert_eq!(recommendation.reason_code, "NO_HEALTHY_IDLE_THREAD");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_state_overwrites_stale_cas_candidate_status_and_tokens() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-old', 'agent-1', 'thread-native', 'thread-root',
                    'c:/workspace/project', 'RUNNING', 5, 0, 0, 5, 100,
                    '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', NULL, NULL, NULL, NULL, NULL,
                    NULL
                 )",
                [],
            )
            .unwrap();
        insert_native_child(&home, "thread-native", "thread-root", 90, "open");
        std::fs::write(
            native_rollout_path(&home, "thread-native"),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":90},\"model_context_window\":100}}}\n",
        )
        .unwrap();

        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(recommendation.decision, "SPAWN");
        assert_eq!(recommendation.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_refresh_preserves_reliable_cas_runtime_fingerprint() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-old', 'agent-1', 'thread-native', 'thread-root',
                    'c:/workspace/project', 'IDLE', 0, 0, 0, 10, 100,
                    '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', 10, ?1, NULL, NULL, NULL,
                    NULL
                 )",
                [&runtime_fingerprint(
                    &connection,
                    "agent-1",
                    "model-1",
                    "deepseek",
                    None,
                    "https://api.deepseek.example/v1",
                    "RESPONSES",
                    None,
                    "deepseek-v4",
                    "HIGH",
                    "WORKSPACE_WRITE",
                    "",
                )
                .unwrap()],
            )
            .unwrap();
        insert_native_child(&home, "thread-native", "thread-root", 10, "open");
        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            recommendation.decision, "REUSE",
            "{}",
            recommendation.reason_code
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bind_native_child_makes_the_next_schedule_reusable() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "thread-native", "thread-root", 10, "open");

        let unbound = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(unbound.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");

        bind_native_thread(
            &mut connection,
            &home,
            "executor",
            "thread-native",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap();
        bind_native_thread(
            &mut connection,
            &home,
            "executor",
            "thread-native",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM agent_thread_instances
                     WHERE codex_thread_id = 'thread-native'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        let reuse = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reuse.decision, "REUSE");
        assert_eq!(reuse.candidate_thread_id.as_deref(), Some("thread-native"));

        connection
            .execute(
                "UPDATE providers SET base_url = 'https://api.changed.example/v1'
                 WHERE id = 'provider-1'",
                [],
            )
            .unwrap();
        let changed = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(changed.decision, "SPAWN");
        assert_eq!(changed.reason_code, "RUNTIME_FINGERPRINT_MISMATCH");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bind_rejects_invalid_native_identity_without_writing() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "thread-valid", "thread-root", 10, "open");
        insert_native_child_with_identity(
            &home,
            "thread-other-agent",
            "thread-root",
            "C:/workspace/project",
            "other",
            "cas_deepseek",
            "deepseek-v4",
            10,
            "open",
        );
        for (thread_id, model_provider, model) in [
            ("thread-no-model", "cas_deepseek", ""),
            ("thread-no-provider", "", "deepseek-v4"),
            ("thread-wrong-model", "cas_deepseek", "other-model"),
            ("thread-wrong-provider", "cas-other", "deepseek-v4"),
        ] {
            insert_native_child_with_identity(
                &home,
                thread_id,
                "thread-root",
                "C:/workspace/project",
                "executor",
                model_provider,
                model,
                10,
                "open",
            );
        }
        for (thread_id, scope_key, parent_thread_id) in [
            ("thread-missing", "c:/workspace/project", "thread-root"),
            ("thread-valid", "c:/workspace/other", "thread-root"),
            ("thread-valid", "c:/workspace/project", "thread-other"),
            ("thread-other-agent", "c:/workspace/project", "thread-root"),
            ("thread-no-model", "c:/workspace/project", "thread-root"),
            ("thread-no-provider", "c:/workspace/project", "thread-root"),
            ("thread-wrong-model", "c:/workspace/project", "thread-root"),
            (
                "thread-wrong-provider",
                "c:/workspace/project",
                "thread-root",
            ),
        ] {
            assert!(
                bind_native_thread(
                    &mut connection,
                    &home,
                    "executor",
                    thread_id,
                    scope_key,
                    parent_thread_id,
                    None,
                )
                .is_err()
            );
        }
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM agent_thread_instances", [], |row| row
                    .get::<_, i64>(0),)
                .unwrap(),
            0
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn bind_rejects_a_different_existing_fingerprint_without_overwrite() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "thread-native", "thread-root", 10, "open");
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-old', 'agent-1', 'thread-native', 'thread-root',
                    'c:/workspace/project', 'UNKNOWN', 0, 0, 0, 0, NULL,
                    '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', NULL, 'old-fingerprint',
                    NULL, NULL, NULL, NULL
                 )",
                [],
            )
            .unwrap();
        assert!(
            bind_native_thread(
                &mut connection,
                &home,
                "executor",
                "thread-native",
                "c:/workspace/project",
                "thread-root",
                None,
            )
            .is_err()
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT runtime_fingerprint FROM agent_thread_instances
                     WHERE codex_thread_id = 'thread-native'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "old-fingerprint"
        );
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn runtime_fingerprint_reads_runtime_rows_and_ignores_insert_order() {
        let mut connection = scheduling_connection();
        let fingerprint = || {
            runtime_fingerprint(
                &connection,
                "agent-1",
                "model-1",
                "deepseek",
                None,
                "https://api.deepseek.example/v1",
                "RESPONSES",
                None,
                "deepseek-v4",
                "HIGH",
                "WORKSPACE_WRITE",
                "",
            )
            .unwrap()
        };
        let baseline = fingerprint();

        connection
            .execute(
                "UPDATE providers SET base_url = 'https://api.changed.example/v1',
                 custom_headers_json = '{\"x-cas\":\"one\"}' WHERE id = 'provider-1'",
                [],
            )
            .unwrap();
        let provider_changed = runtime_fingerprint(
            &connection,
            "agent-1",
            "model-1",
            "deepseek",
            None,
            "https://api.changed.example/v1",
            "RESPONSES",
            Some("{\"x-cas\":\"one\"}"),
            "deepseek-v4",
            "HIGH",
            "WORKSPACE_WRITE",
            "",
        )
        .unwrap();
        assert_ne!(baseline, provider_changed);

        connection
            .execute_batch(
                "INSERT INTO agent_required_capabilities VALUES ('agent-1', 'alpha');
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'beta');
                 INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'gamma');
                 INSERT INTO model_capabilities VALUES ('model-1', 'shell', 'SUPPORTED');",
            )
            .unwrap();
        let ordered = runtime_fingerprint(
            &connection,
            "agent-1",
            "model-1",
            "deepseek",
            None,
            "https://api.changed.example/v1",
            "RESPONSES",
            Some("{\"x-cas\":\"one\"}"),
            "deepseek-v4",
            "HIGH",
            "WORKSPACE_WRITE",
            "",
        )
        .unwrap();
        assert_ne!(provider_changed, ordered);

        connection
            .execute_batch(
                "DELETE FROM agent_required_capabilities;
                 DELETE FROM agent_preferred_capabilities;
                 DELETE FROM model_capabilities;
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'beta');
                 INSERT INTO agent_required_capabilities VALUES ('agent-1', 'alpha');
                 INSERT INTO agent_preferred_capabilities VALUES ('agent-1', 'gamma');
                 INSERT INTO model_capabilities VALUES ('model-1', 'shell', 'SUPPORTED');",
            )
            .unwrap();
        assert_eq!(
            ordered,
            runtime_fingerprint(
                &connection,
                "agent-1",
                "model-1",
                "deepseek",
                None,
                "https://api.changed.example/v1",
                "RESPONSES",
                Some("{\"x-cas\":\"one\"}"),
                "deepseek-v4",
                "HIGH",
                "WORKSPACE_WRITE",
                "",
            )
            .unwrap()
        );
    }

    #[test]
    fn helper_profile_reads_provider_runtime_fields_from_database() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        let fingerprint = runtime_fingerprint(
            &connection,
            "agent-1",
            "model-1",
            "deepseek",
            None,
            "https://api.deepseek.example/v1",
            "RESPONSES",
            None,
            "deepseek-v4",
            "HIGH",
            "WORKSPACE_WRITE",
            "",
        )
        .unwrap();
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-1', 'agent-1', 'thread-child', 'thread-root',
                    'c:/workspace/project', 'IDLE', 0, 0, 0, 10, 100,
                    '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', 10, ?1, NULL, NULL, NULL,
                    NULL
                 )",
                [&fingerprint],
            )
            .unwrap();

        let reuse = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(reuse.decision, "REUSE");
        // last_model_usage_at 为 NULL 时缓存提示必须按未知输出，而不是伪装成命中窗口。
        assert_eq!(reuse.cache_hint, "UNKNOWN");
        assert_eq!(reuse.candidate_age_seconds, None);

        connection
            .execute(
                "UPDATE providers
                 SET base_url = 'https://api.changed.example/v1',
                     custom_headers_json = '{\"x-cas\":\"one\"}'
                 WHERE id = 'provider-1'",
                [],
            )
            .unwrap();
        let changed = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(changed.decision, "SPAWN");
        assert_eq!(changed.reason_code, "RUNTIME_FINGERPRINT_MISMATCH");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn reuse_claim_blocks_concurrent_schedule_until_lease_expires() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        let fingerprint = runtime_fingerprint(
            &connection,
            "agent-1",
            "model-1",
            "deepseek",
            None,
            "https://api.deepseek.example/v1",
            "RESPONSES",
            None,
            "deepseek-v4",
            "HIGH",
            "WORKSPACE_WRITE",
            "",
        )
        .unwrap();
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-1', 'agent-1', 'thread-child', 'thread-root',
                    'c:/workspace/project', 'IDLE', 0, 0, 0, 10, 100,
                    '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', 10, ?1, NULL, NULL, NULL,
                    NULL
                 )",
                [&fingerprint],
            )
            .unwrap();

        let first = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.decision, "REUSE");
        // F-13：决策与租约同事务写入只追加审计记录。
        let audit = connection
            .query_row(
                "SELECT source, decision, candidate_thread_id, claimed
                 FROM agent_schedule_decisions ORDER BY rowid DESC LIMIT 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(audit, ("HELPER".to_owned(), "REUSE".to_owned(), "thread-child".to_owned(), 1));

        // F-11：租约内的第二次并发预检不得再次选中同一 IDLE Thread。
        let second = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(second.decision, "SPAWN");
        assert_eq!(second.reason_code, "THREAD_CLAIMED");

        // 租约过期后 Thread 重新可复用。
        connection
            .execute(
                "UPDATE agent_thread_instances
                 SET claimed_until = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-1 seconds')
                 WHERE id = 'instance-1'",
                [],
            )
            .unwrap();
        let third = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(third.decision, "REUSE");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn task_scope_key_gates_reuse_and_bind_persists_it() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        let fingerprint = runtime_fingerprint(
            &connection,
            "agent-1",
            "model-1",
            "deepseek",
            None,
            "https://api.deepseek.example/v1",
            "RESPONSES",
            None,
            "deepseek-v4",
            "HIGH",
            "WORKSPACE_WRITE",
            "",
        )
        .unwrap();
        for (instance, thread, task) in [
            ("instance-a", "thread-a", None),
            ("instance-b", "thread-b", Some("auth-oauth2")),
        ] {
            connection
                .execute(
                    "INSERT INTO agent_thread_instances VALUES (
                        ?1, 'agent-1', ?2, 'thread-root',
                        'c:/workspace/project', 'IDLE', 0, 0, 0, 10, 100,
                        '2026-08-13T00:00:00Z', '2026-08-13T00:00:00Z', 10, ?3,
                        NULL, NULL, NULL, ?4
                     )",
                    params![instance, thread, fingerprint, task],
                )
                .unwrap();
        }

        // 同键任务复用同键 Thread。
        let same_task = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            Some("auth-oauth2"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(same_task.decision, "REUSE");
        assert_eq!(same_task.candidate_thread_id.as_deref(), Some("thread-b"));

        // 无键任务不得复用绑定了任务键的 Thread（fail-closed）。
        let no_key = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(no_key.decision, "REUSE");
        assert_eq!(no_key.candidate_thread_id.as_deref(), Some("thread-a"));

        // 异键任务不匹配任何候选。
        let other_task = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            Some("payments"),
        )
        .unwrap()
        .unwrap();
        assert_eq!(other_task.decision, "SPAWN");
        assert_eq!(other_task.reason_code, "NO_WORKSPACE_SCOPE_MATCH");
        assert_eq!(other_task.candidate_thread_id, None);

        // bind 固化任务键，既有键不被后续 bind 覆盖。
        insert_native_child(&home, "thread-native", "thread-root", 10, "open");
        bind_native_thread(
            &mut connection,
            &home,
            "executor",
            "thread-native",
            "c:/workspace/project",
            "thread-root",
            Some("auth-oauth2"),
        )
        .unwrap();
        bind_native_thread(
            &mut connection,
            &home,
            "executor",
            "thread-native",
            "c:/workspace/project",
            "thread-root",
            Some("payments"),
        )
        .unwrap();
        let bound_key = connection
            .query_row(
                "SELECT task_scope_key FROM agent_thread_instances
                 WHERE codex_thread_id = 'thread-native'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap();
        assert_eq!(bound_key.as_deref(), Some("auth-oauth2"));
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_current_context_is_separate_from_cumulative_tokens() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "thread-native", "thread-root", 1_667_247, "open");
        std::fs::write(
            native_rollout_path(&home, "thread-native"),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":50000},\"model_context_window\":258400}}}\n",
        )
        .unwrap();
        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(recommendation.decision, "SPAWN");
        assert_eq!(recommendation.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn merged_candidates_prefer_the_most_recent_idle_thread() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        connection
            .execute(
                "INSERT INTO agent_thread_instances VALUES (
                    'instance-old', 'agent-1', 'a-old', 'thread-root',
                    'c:/workspace/project', 'IDLE', 10, 0, 0, 10, 100,
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-3600 seconds'),
                    strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '-3600 seconds'), NULL, NULL,
                    NULL, NULL, NULL, NULL
                 )",
                [],
            )
            .unwrap();
        insert_native_child(&home, "z-recent", "thread-root", 10, "open");

        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(recommendation.decision, "SPAWN");
        assert_eq!(recommendation.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn native_candidates_require_current_parent_scope_and_agent_identity() {
        let mut connection = scheduling_connection();
        let home = native_state_home();
        insert_native_child(&home, "z-valid", "thread-root", 10, "open");
        insert_native_child_with_identity(
            &home,
            "a-other-parent",
            "thread-other",
            "C:/workspace/project",
            "executor",
            "cas_deepseek",
            "deepseek-v4",
            10,
            "open",
        );
        insert_native_child_with_identity(
            &home,
            "b-other-scope",
            "thread-root",
            "C:/workspace/other",
            "executor",
            "cas_deepseek",
            "deepseek-v4",
            10,
            "open",
        );
        insert_native_child_with_identity(
            &home,
            "c-role-but-wrong-model",
            "thread-root",
            "C:/workspace/project",
            "executor",
            "cas_other",
            "other-model",
            10,
            "open",
        );

        let recommendation = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(recommendation.decision, "SPAWN");
        assert_eq!(recommendation.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn unknown_native_state_schema_stops_scheduling() {
        let mut connection = scheduling_connection();
        let home = unique_temp_dir("unknown-native-state");
        std::fs::create_dir_all(&home).unwrap();
        Connection::open(home.join("state_99.sqlite"))
            .unwrap()
            .execute("CREATE TABLE threads (id TEXT PRIMARY KEY)", [])
            .unwrap();

        let result = load_recommendation(
            &mut connection,
            &home,
            "executor",
            "c:/workspace/project",
            "thread-root",
            None,
        );

        assert!(matches!(
            result,
            Err(ScheduleError::NativeStateIncompatible)
        ));
        std::fs::remove_dir_all(home).unwrap();
    }

    fn settings_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute(
                "CREATE TABLE application_settings (
                    setting_key TEXT PRIMARY KEY,
                    setting_value TEXT
                 )",
                [],
            )
            .unwrap();
        connection
    }

    fn scheduling_connection() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE agents (
                    id TEXT PRIMARY KEY,
                    agent_key TEXT NOT NULL,
                    name TEXT,
                    role_key TEXT,
                    enabled INTEGER NOT NULL,
                    reuse_strategy TEXT NOT NULL,
                    cache_retention_override_seconds INTEGER,
                    instruction TEXT NOT NULL DEFAULT '',
                    sandbox_policy TEXT NOT NULL DEFAULT 'WORKSPACE_WRITE',
                    reasoning_policy TEXT NOT NULL DEFAULT 'HIGH'
                 );
                 CREATE TABLE active_agent_bindings (
                    agent_id TEXT NOT NULL
                 );
                 CREATE TABLE agent_model_bindings (
                    agent_id TEXT NOT NULL,
                    model_id TEXT NOT NULL,
                    enabled INTEGER NOT NULL
                 );
                 CREATE TABLE models (
                    id TEXT PRIMARY KEY,
                    provider_id TEXT NOT NULL,
                    model_id TEXT NOT NULL,
                    enabled INTEGER NOT NULL,
                    context_window INTEGER
                 );
                 CREATE TABLE providers (
                    id TEXT PRIMARY KEY,
                    provider_key TEXT NOT NULL,
                    preset_id TEXT,
                    enabled INTEGER NOT NULL,
                    cache_support TEXT,
                    cache_retention_type TEXT,
                    cache_retention_hint_seconds INTEGER,
                    base_url TEXT NOT NULL,
                    protocol TEXT NOT NULL,
                    custom_headers_json TEXT
                 );
                 CREATE TABLE agent_required_capabilities (
                    agent_id TEXT NOT NULL,
                    capability TEXT NOT NULL
                 );
                 CREATE TABLE agent_preferred_capabilities (
                    agent_id TEXT NOT NULL,
                    capability TEXT NOT NULL
                 );
                 CREATE TABLE model_capabilities (
                    model_id TEXT NOT NULL,
                    capability TEXT NOT NULL,
                    status TEXT NOT NULL
                 );
                 CREATE TABLE agent_thread_instances (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    codex_thread_id TEXT NOT NULL UNIQUE,
                    parent_thread_id TEXT,
                    scope_key TEXT,
                    status TEXT NOT NULL,
                    input_tokens INTEGER NOT NULL,
                    cached_input_tokens INTEGER NOT NULL,
                    output_tokens INTEGER NOT NULL,
                    total_tokens INTEGER NOT NULL,
                    context_window INTEGER,
                    created_at TEXT NOT NULL,
                    last_used_at TEXT NOT NULL,
                    current_context_tokens INTEGER,
                    runtime_fingerprint TEXT,
                    last_model_usage_at TEXT,
                    last_observed_at TEXT,
                    claimed_until TEXT,
                    task_scope_key TEXT
                 );
                 CREATE TABLE agent_schedule_decisions (
                    id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    source TEXT NOT NULL,
                    agent_id TEXT,
                    agent_name_snapshot TEXT,
                    workspace_scope_key TEXT NOT NULL,
                    parent_thread_id TEXT,
                    candidate_thread_id TEXT,
                    decision TEXT NOT NULL,
                    reason_code TEXT NOT NULL,
                    runtime_fingerprint TEXT,
                    context_pressure_percent INTEGER,
                    context_pressure_limit_percent INTEGER,
                    cache_hint TEXT NOT NULL,
                    candidate_age_seconds INTEGER,
                    claimed INTEGER NOT NULL DEFAULT 0,
                    task_scope_key TEXT
                 );
                 INSERT INTO agents (
                    id, agent_key, role_key, enabled, reuse_strategy, cache_retention_override_seconds
                 ) VALUES ('agent-1', 'executor', 'executor', 1, 'AUTO', NULL);
                 INSERT INTO active_agent_bindings VALUES ('agent-1');
                 INSERT INTO providers (
                    id, provider_key, preset_id, enabled, cache_support,
                    cache_retention_type, cache_retention_hint_seconds, base_url, protocol,
                    custom_headers_json
                 ) VALUES (
                    'provider-1', 'deepseek', NULL, 1, 'SUPPORTED', 'APPROXIMATE', 300,
                    'https://api.deepseek.example/v1', 'RESPONSES', NULL
                 );
                 INSERT INTO models VALUES ('model-1', 'provider-1', 'deepseek-v4', 1, 100);
                 INSERT INTO agent_model_bindings VALUES ('agent-1', 'model-1', 1);",
            )
            .unwrap();
        connection
    }

    fn native_state_home() -> PathBuf {
        let home = unique_temp_dir("native-state");
        std::fs::create_dir_all(&home).unwrap();
        Connection::open(home.join("state_7.sqlite"))
            .unwrap()
            .execute_batch(
                "CREATE TABLE threads (
                    id TEXT PRIMARY KEY,
                    rollout_path TEXT NOT NULL,
                    agent_role TEXT,
                    model_provider TEXT NOT NULL,
                    model TEXT,
                    tokens_used INTEGER NOT NULL,
                    cwd TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE thread_spawn_edges (
                    parent_thread_id TEXT NOT NULL,
                    child_thread_id TEXT NOT NULL PRIMARY KEY,
                    status TEXT NOT NULL
                 );",
            )
            .unwrap();
        home
    }

    fn insert_native_child(
        home: &Path,
        thread_id: &str,
        parent_thread_id: &str,
        tokens_used: i64,
        status: &str,
    ) {
        insert_native_child_with_identity(
            home,
            thread_id,
            parent_thread_id,
            "C:/workspace/project",
            "executor",
            "cas_deepseek",
            "deepseek-v4",
            tokens_used,
            status,
        );
    }

    fn insert_native_child_with_identity(
        home: &Path,
        thread_id: &str,
        parent_thread_id: &str,
        cwd: &str,
        agent_role: &str,
        model_provider: &str,
        model: &str,
        tokens_used: i64,
        status: &str,
    ) {
        let state = Connection::open(home.join("state_7.sqlite")).unwrap();
        let updated_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        state
            .execute(
                "INSERT INTO threads VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8
                 )",
                params![
                    thread_id,
                    native_rollout_path(home, thread_id),
                    agent_role,
                    model_provider,
                    model,
                    tokens_used,
                    cwd,
                    updated_at
                ],
            )
            .unwrap();
        write_native_rollout(home, thread_id, "task_complete");
        state
            .execute(
                "INSERT INTO thread_spawn_edges VALUES (?1, ?2, ?3)",
                params![parent_thread_id, thread_id, status],
            )
            .unwrap();
    }

    fn native_rollout_path(home: &Path, thread_id: &str) -> String {
        home.join(format!("{thread_id}.jsonl"))
            .to_string_lossy()
            .into_owned()
    }

    fn write_native_rollout(home: &Path, thread_id: &str, event: &str) {
        std::fs::write(
            native_rollout_path(home, thread_id),
            format!(
                "{{\"type\":\"event_msg\",\"payload\":{{\"type\":\"task_started\"}}}}\n\
                 {{\"type\":\"event_msg\",\"payload\":{{\"type\":\"{event}\"}}}}\n\
                 {{\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"last_token_usage\":{{\"total_tokens\":10}},\"model_context_window\":100}}}}}}\n"
            ),
        )
        .unwrap();
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
    }
}
