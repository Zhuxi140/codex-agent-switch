use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use serde_json::{Value, json};

#[test]
fn mcp_stdio_runs_subagent_lifecycle_hooks_and_records_audit() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("cas-hook-mcp-{}-{unique}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let database_path = root.join("cas.db");
    let connection = Connection::open(&database_path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY, agent_key TEXT NOT NULL, enabled INTEGER NOT NULL,
                orchestration_phase TEXT
             );
             CREATE TABLE active_agent_bindings (agent_id TEXT NOT NULL);
             CREATE TABLE configuration_state (active_agent_id TEXT);
             CREATE TABLE job_attempts (
                lease_id TEXT NOT NULL, state TEXT NOT NULL, dispatch_recorded_at TEXT
             );
             CREATE TABLE agent_spawn_reservations (
                agent_id TEXT NOT NULL, parent_thread_id TEXT NOT NULL,
                workspace_scope_key TEXT NOT NULL, task_scope_key TEXT NOT NULL
             );
             CREATE TABLE agent_thread_instances (
                codex_thread_id TEXT PRIMARY KEY, status TEXT NOT NULL,
                claimed_until TEXT, last_observed_at TEXT
             );
             CREATE TABLE runtime_delegation_leases (
                id TEXT PRIMARY KEY, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
                agent_id TEXT NOT NULL, parent_thread_id TEXT NOT NULL, codex_agent_id TEXT,
                workspace_scope_key TEXT NOT NULL, task_scope_key TEXT,
                schedule_decision_id TEXT NOT NULL, state TEXT NOT NULL, expires_at TEXT NOT NULL,
                released_at TEXT, release_reason TEXT, admission_tool_use_id TEXT,
                admitted_at TEXT, admission_confirmed_at TEXT, agent_type TEXT NOT NULL
             );
             CREATE TABLE runtime_hook_turns (
                turn_id TEXT PRIMARY KEY, session_id TEXT NOT NULL, agent_id TEXT,
                agent_type TEXT NOT NULL, orchestration_phase TEXT, codex_agent_id TEXT,
                lease_id TEXT, workspace_scope_key TEXT, task_scope_key TEXT, lease_state TEXT,
                lease_expires_at TEXT, stopped_at TEXT, created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
             );
             CREATE TABLE runtime_enforcement_events (
                id TEXT PRIMARY KEY, created_at TEXT NOT NULL, session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL, agent_id TEXT, agent_type TEXT,
                orchestration_phase TEXT, tool_name TEXT NOT NULL, decision TEXT NOT NULL,
                reason_code TEXT NOT NULL, cwd TEXT, message TEXT NOT NULL,
                codex_agent_id TEXT, lease_id TEXT, workspace_scope_key TEXT,
                task_scope_key TEXT, lease_state TEXT, lease_expires_at TEXT
             );
             INSERT INTO agents VALUES ('agent-executor', 'executor', 1, 'EXECUTION');
             INSERT INTO active_agent_bindings VALUES ('agent-executor');
             INSERT INTO runtime_delegation_leases (
                id, created_at, updated_at, agent_id, parent_thread_id,
                workspace_scope_key, task_scope_key, schedule_decision_id, state, expires_at,
                admission_tool_use_id, admitted_at, agent_type
             ) VALUES (
                'lease-mcp', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'agent-executor', 'session-mcp',
                'c:/workspace', 'task-mcp', 'decision-mcp', 'PENDING',
                strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+120 seconds'),
                'tool-spawn-mcp', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), 'executor'
             );
             INSERT INTO agent_thread_instances VALUES ('child-mcp', 'RUNNING', NULL, NULL);",
        )
        .unwrap();
    drop(connection);

    let mut child = Command::new(env!("CARGO_BIN_EXE_cas-helper"))
        .arg("mcp-assess")
        .arg(&database_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let common = json!({"session_id":"session-mcp","cwd":"C:/workspace"});
    let events = [
        json!({
            "hook_event_name":"SubagentStart","turn_id":"turn-child-mcp",
            "agent_id":"child-mcp","agent_type":"executor"
        }),
        json!({
            "hook_event_name":"PostToolUse","turn_id":"turn-primary-mcp",
            "tool_name":"spawn_agent","tool_use_id":"tool-spawn-mcp",
            "tool_input":{"agent_type":"executor"},"tool_response":{"ok":true}
        }),
        json!({
            "hook_event_name":"SubagentStop","turn_id":"turn-child-mcp",
            "agent_id":"child-mcp","agent_type":"executor"
        }),
    ];
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"})
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
        )
        .unwrap();
        for (index, event) in events.into_iter().enumerate() {
            let mut arguments = common.clone();
            arguments
                .as_object_mut()
                .unwrap()
                .extend(event.as_object().unwrap().clone());
            writeln!(
                stdin,
                "{}",
                json!({
                    "jsonrpc":"2.0","id":index+3,"method":"tools/call",
                    "params":{"name":"cas_runtime_hook","arguments":arguments}
                })
            )
            .unwrap();
        }
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 5);
    assert_eq!(
        responses[1]["result"]["tools"][1]["name"],
        "cas_runtime_hook"
    );
    for response in &responses[2..] {
        assert_eq!(response["result"]["isError"], false, "{response}");
    }
    assert_eq!(
        responses[2]["result"]["structuredContent"]["hookSpecificOutput"]["hookEventName"],
        "SubagentStart"
    );
    assert!(
        responses[2]["result"]["structuredContent"]["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Lease: lease-mcp")
    );
    assert_eq!(responses[3]["result"]["structuredContent"], json!({}));
    assert_eq!(responses[4]["result"]["structuredContent"], json!({}));

    let connection = Connection::open(&database_path).unwrap();
    let (state, admission_confirmed, released): (String, bool, bool) = connection
        .query_row(
            "SELECT state, admission_confirmed_at IS NOT NULL, released_at IS NOT NULL
             FROM runtime_delegation_leases WHERE id = 'lease-mcp'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(state, "RELEASED");
    assert!(admission_confirmed);
    assert!(released);
    let (lease_state, stopped): (String, bool) = connection
        .query_row(
            "SELECT lease_state, stopped_at IS NOT NULL
             FROM runtime_hook_turns WHERE turn_id = 'turn-child-mcp'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(lease_state, "RELEASED");
    assert!(stopped);
    let mut statement = connection
        .prepare(
            "SELECT tool_name, decision, reason_code
             FROM runtime_enforcement_events ORDER BY rowid",
        )
        .unwrap();
    let audit = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        audit
            .iter()
            .map(|(tool, decision, reason)| { (tool.as_str(), decision.as_str(), reason.as_str()) })
            .collect::<Vec<_>>(),
        [
            ("SubagentStart", "ALLOW", "DELEGATION_LEASE_ACTIVATED"),
            ("spawn_agent", "ALLOW", "DELEGATION_TOOL_CONFIRMED"),
            ("SubagentStop", "ALLOW", "DELEGATION_LEASE_RELEASED"),
        ]
    );
    drop(statement);
    drop(connection);
    std::fs::remove_file(database_path).unwrap();
    std::fs::remove_dir(root).unwrap();
}
