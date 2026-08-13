use std::env;
use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;

use cas_scheduler::{Candidate, Profile, Recommendation, normalize_scope_key, recommend};
use cas_secret_store::{CredentialId, SecretStoreError, read};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

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
        }) => {
            let database_path = database_path.or_else(default_database_path);
            let scope_key = scope_key.or_else(|| {
                env::current_dir()
                    .ok()
                    .and_then(|path| normalize_scope_key(&path.to_string_lossy()))
            });
            match (database_path, scope_key) {
                (Some(database_path), Some(scope_key)) => {
                    schedule(database_path, &agent_key, &scope_key)
                }
                _ => {
                    eprintln!("CAS scheduling environment unavailable.");
                    ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE)
                }
            }
        }
        Err(()) => {
            eprintln!(
                "Usage:\n  cas-helper token <credential-id>\n  \
                 cas-helper schedule <agent-key>\n  \
                 cas-helper schedule <database-path> <agent-key> <scope-key>"
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
    },
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
            return Ok(Command::Schedule {
                database_path: None,
                agent_key: valid_argument(first)?,
                scope_key: None,
            });
        }
        let database_path = PathBuf::from(first);
        let agent_key = valid_argument(second.expect("checked above"))?;
        let scope_key = normalize_scope_key(&valid_argument(args.next().ok_or(())?)?).ok_or(())?;
        if args.next().is_some() || database_path.as_os_str().is_empty() {
            return Err(());
        }
        return Ok(Command::Schedule {
            database_path: Some(database_path),
            agent_key,
            scope_key: Some(scope_key),
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

fn schedule(database_path: PathBuf, agent_key: &str, scope_key: &str) -> ExitCode {
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
    let connection = match Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(connection) => connection,
        Err(_) => {
            eprintln!("CAS scheduling database unavailable.");
            return ExitCode::from(EXIT_SCHEDULING_UNAVAILABLE);
        }
    };
    match load_recommendation(&connection, agent_key, scope_key, &parent_thread_id) {
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

fn load_recommendation(
    connection: &Connection,
    agent_key: &str,
    scope_key: &str,
    parent_thread_id: &str,
) -> rusqlite::Result<Option<Recommendation>> {
    let profile = connection
        .query_row(
            "SELECT a.id, a.reuse_strategy, a.cache_retention_override_seconds,
                    COALESCE(p.cache_support, 'UNKNOWN'),
                    COALESCE(p.cache_retention_type, 'UNKNOWN'),
                    p.cache_retention_hint_seconds
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
                    Profile {
                        reuse_strategy: row.get(1)?,
                        agent_cache_retention_override_seconds: row.get(2)?,
                        cache_support: row.get(3)?,
                        cache_retention_type: row.get(4)?,
                        cache_retention_hint_seconds: row.get(5)?,
                    },
                ))
            },
        )
        .optional()?;
    let Some((agent_id, profile)) = profile else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "SELECT id, codex_thread_id, status, input_tokens, cached_input_tokens,
                output_tokens, total_tokens, context_window,
                COALESCE(
                    CAST(MAX(0, (julianday('now') - julianday(last_used_at)) * 86400) AS INTEGER),
                    0
                )
         FROM agent_thread_instances
         WHERE agent_id = ?1 AND scope_key = ?2 AND parent_thread_id = ?3
         ORDER BY last_used_at DESC, codex_thread_id ASC",
    )?;
    let candidates = statement
        .query_map(params![agent_id, scope_key, parent_thread_id], |row| {
            Ok(Candidate {
                instance_id: row.get(0)?,
                thread_id: row.get(1)?,
                status: row.get(2)?,
                input_tokens: row.get(3)?,
                cached_input_tokens: row.get(4)?,
                output_tokens: row.get(5)?,
                total_tokens: row.get(6)?,
                context_window: row.get(7)?,
                age_seconds: row.get(8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(recommend(scope_key.to_owned(), candidates, profile)))
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
        }) = parse_args(args)
        else {
            panic!("short schedule command should parse");
        };
        assert_eq!(database_path, None);
        assert_eq!(agent_key, "executor");
        assert_eq!(scope_key, None);
    }

    #[test]
    fn compact_protocol_contains_only_action_fields() {
        let recommendation = Recommendation {
            decision: "REUSE",
            reason_code: "EXACT_SCOPE_IDLE",
            message: "unused",
            scope_key: "c:/workspace/project".to_owned(),
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
            Some("CAS1|REUSE|thread-child|EXACT_SCOPE_IDLE".to_owned())
        );
        let mut invalid = recommendation;
        invalid.candidate_thread_id = Some("thread|injected".to_owned());
        assert_eq!(protocol_line(&invalid), None);
    }

    #[test]
    fn scheduling_is_limited_to_current_primary_thread() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE agents (
                    id TEXT PRIMARY KEY,
                    agent_key TEXT NOT NULL,
                    enabled INTEGER NOT NULL,
                    reuse_strategy TEXT NOT NULL,
                    cache_retention_override_seconds INTEGER
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
                    provider_id TEXT NOT NULL
                 );
                 CREATE TABLE providers (
                    id TEXT PRIMARY KEY,
                    cache_support TEXT,
                    cache_retention_type TEXT,
                    cache_retention_hint_seconds INTEGER
                 );
                 CREATE TABLE agent_thread_instances (
                    id TEXT PRIMARY KEY,
                    agent_id TEXT NOT NULL,
                    codex_thread_id TEXT NOT NULL,
                    parent_thread_id TEXT,
                    scope_key TEXT,
                    status TEXT NOT NULL,
                    input_tokens INTEGER NOT NULL,
                    cached_input_tokens INTEGER NOT NULL,
                    output_tokens INTEGER NOT NULL,
                    total_tokens INTEGER NOT NULL,
                    context_window INTEGER,
                    last_used_at TEXT NOT NULL
                 );
                 INSERT INTO agents VALUES ('agent-1', 'executor', 1, 'AUTO', NULL);
                 INSERT INTO active_agent_bindings VALUES ('agent-1');
                 INSERT INTO providers VALUES ('provider-1', 'SUPPORTED', 'APPROXIMATE', 300);
                 INSERT INTO models VALUES ('model-1', 'provider-1');
                 INSERT INTO agent_model_bindings VALUES ('agent-1', 'model-1', 1);
                 INSERT INTO agent_thread_instances VALUES (
                    'instance-1', 'agent-1', 'thread-child', 'thread-root',
                    'c:/workspace/project', 'IDLE', 20, 0, 5, 25, 100,
                    '2026-08-13T00:00:00Z'
                 );",
            )
            .unwrap();

        let reuse = load_recommendation(
            &connection,
            "executor",
            "c:/workspace/project",
            "thread-root",
        )
        .unwrap()
        .unwrap();
        assert_eq!(reuse.decision, "REUSE");
        assert_eq!(reuse.candidate_thread_id.as_deref(), Some("thread-child"));

        let other_primary = load_recommendation(
            &connection,
            "executor",
            "c:/workspace/project",
            "thread-other",
        )
        .unwrap()
        .unwrap();
        assert_eq!(other_primary.decision, "SPAWN");
        assert_eq!(other_primary.candidate_thread_id, None);
    }
}
