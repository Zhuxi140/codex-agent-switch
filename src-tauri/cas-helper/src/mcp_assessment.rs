//! CAS STDIO MCP 入口；委派评估只读，Runtime Hook 沿用现有 helper 实现。

use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use codex_agent_switch_lib::native_control;
use serde_json::{Value, json};

const PROTOCOL_VERSION: &str = "2025-06-18";
const TOOL_NAME: &str = "assess_delegation";
const RUNTIME_HOOK_TOOL: &str = "cas_runtime_hook";
const RUNTIME_HOOK_MARKER: &str = "cas-runtime-enforcement-v1";

pub(super) fn serve(database_path: &Path) -> io::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    serve_io(database_path, stdin.lock(), &mut stdout)
}

fn serve_io(database_path: &Path, input: impl BufRead, output: &mut impl Write) -> io::Result<()> {
    for line in input.lines() {
        let response = match serde_json::from_str::<Value>(&line?) {
            Ok(message) => handle_message(database_path, &message),
            Err(_) => Some(rpc_error(Value::Null, -32700, "JSON 解析失败")),
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut *output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

fn handle_message(database_path: &Path, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str);
    Some(match method {
        Some("initialize") => rpc_result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "cas-delegation", "version": env!("CARGO_PKG_VERSION")}
            }),
        ),
        Some("ping") => rpc_result(id, json!({})),
        Some("tools/list") => rpc_result(
            id,
            json!({"tools": [tool_definition(), runtime_hook_definition()]}),
        ),
        Some("tools/call") => match call_tool(database_path, message.get("params")) {
            Ok(result) => rpc_result(id, result),
            Err(error) => rpc_result(
                id,
                json!({"content": [{"type": "text", "text": error}], "isError": true}),
            ),
        },
        _ => rpc_error(id, -32601, "未知方法"),
    })
}

fn call_tool(database_path: &Path, params: Option<&Value>) -> Result<Value, String> {
    match params
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
    {
        Some(TOOL_NAME) => assess_tool(database_path, params),
        Some(RUNTIME_HOOK_TOOL) => runtime_hook_tool(database_path, params),
        _ => Err("未知工具".to_owned()),
    }
}

fn runtime_hook_definition() -> Value {
    json!({
        "name": RUNTIME_HOOK_TOOL,
        "description": "由受信任的 CAS Runtime Hook 调用，处理生命周期事件。",
        "inputSchema": {
            "type": "object",
            "required": ["hook_event_name", "session_id", "turn_id", "cwd"],
            "additionalProperties": true,
            "properties": {
                "hook_event_name": {"type": "string"},
                "session_id": {"type": "string"},
                "turn_id": {"type": "string"},
                "cwd": {"type": "string"}
            }
        },
        "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}
    })
}

fn runtime_hook_tool(database_path: &Path, params: Option<&Value>) -> Result<Value, String> {
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .and_then(Value::as_object)
        .ok_or("缺少 Hook 参数")?;
    let event = arguments
        .get("hook_event_name")
        .and_then(Value::as_str)
        .ok_or("缺少 Hook 事件")?;
    if !matches!(
        event,
        "UserPromptSubmit" | "SubagentStart" | "SubagentStop" | "PreToolUse" | "PostToolUse"
    ) {
        return Err("不支持的 Hook 事件".to_owned());
    }
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .arg("hook")
        .arg(database_path)
        .arg(RUNTIME_HOOK_MARKER)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    {
        let mut stdin = child.stdin.take().ok_or("Hook 输入管道不可用")?;
        serde_json::to_writer(&mut stdin, arguments).map_err(|error| error.to_string())?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let hook_output = if output.stdout.is_empty() {
        json!({})
    } else {
        serde_json::from_slice::<Value>(&output.stdout)
            .map_err(|_| "Hook 返回的 JSON 无效".to_owned())?
    };
    Ok(json!({
        "content": [{"type": "text", "text": hook_output.to_string()}],
        "structuredContent": hook_output,
        "isError": false
    }))
}

fn tool_definition() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "只读评估一个有边界、可独立验收的委派候选。仅在明显可能受益时调用；结果是建议，不是分发许可。",
        "inputSchema": {
            "type": "object",
            "required": ["agent_key", "workspace_scope", "assessment"],
            "additionalProperties": false,
            "properties": {
                "agent_key": {"type": "string", "description": "当前 Active Agent 的 key"},
                "workspace_scope": {"type": "string", "description": "当前工作区绝对路径"},
                "assessment": {
                    "type": "object",
                    "required": ["bounded", "acceptance_defined", "independent", "handoff_small"],
                    "additionalProperties": false,
                    "properties": {
                        "bounded": {"type": "boolean"},
                        "acceptance_defined": {"type": "boolean"},
                        "independent": {"type": "boolean"},
                        "handoff_small": {"type": "boolean"},
                        "delegation_forbidden": {"type": "boolean"},
                        "estimated_minutes": {"type": "integer", "minimum": 0, "maximum": 65535},
                        "work_units": {"type": "integer", "minimum": 0, "maximum": 255},
                        "modules": {"type": "integer", "minimum": 0, "maximum": 255},
                        "call_chains": {"type": "integer", "minimum": 0, "maximum": 255},
                        "tests_defined": {"type": "boolean"},
                        "high_risk": {"type": "boolean"},
                        "environments": {"type": "integer", "minimum": 0, "maximum": 255},
                        "stages": {"type": "integer", "minimum": 0, "maximum": 255}
                    }
                }
            }
        },
        "annotations": {"readOnlyHint": true, "destructiveHint": false, "openWorldHint": false}
    })
}

fn assess_tool(database_path: &Path, params: Option<&Value>) -> Result<Value, String> {
    if params
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
        != Some(TOOL_NAME)
    {
        return Err("未知工具".to_owned());
    }
    let arguments = params
        .and_then(|params| params.get("arguments"))
        .and_then(Value::as_object)
        .ok_or("缺少工具参数")?;
    if arguments.len() != 3 {
        return Err("仅接受 agent_key、workspace_scope 和 assessment".to_owned());
    }
    let agent_key = arguments
        .get("agent_key")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or("agent_key 无效")?;
    let workspace_scope = arguments
        .get("workspace_scope")
        .and_then(Value::as_str)
        .ok_or("workspace_scope 无效")?;
    let assessment = arguments
        .get("assessment")
        .and_then(Value::as_object)
        .ok_or("assessment 无效")?;
    if assessment.contains_key("explicit_request") {
        return Err("只读 MCP 不接受 explicit_request；明确委派请使用 CAS Skill".to_owned());
    }
    let assessment = serde_json::to_vec(assessment).map_err(|_| "assessment 无效")?;
    let result =
        native_control::assess_native_task(database_path, agent_key, workspace_scope, &assessment)
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
    let data = json!({
        "action": result.action,
        "score": result.score,
        "threshold": result.threshold,
        "reason_code": result.reason_code,
        "phase": result.phase
    });
    Ok(json!({
        "content": [{"type": "text", "text": data.to_string()}],
        "structuredContent": data,
        "isError": false
    }))
}

fn rpc_result(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::io::Cursor;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn mcp_assessment_is_read_only_and_rejects_explicit_request() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database_path =
            std::env::temp_dir().join(format!("cas-mcp-{}-{unique}.db", std::process::id()));
        let connection = Connection::open(&database_path).unwrap();
        connection.execute_batch(
            "CREATE TABLE agents (id TEXT, agent_key TEXT, enabled INTEGER, orchestration_phase TEXT);
             CREATE TABLE active_agent_bindings (agent_id TEXT);
             CREATE TABLE project_orchestration_exclusions (project_path TEXT);
             INSERT INTO agents VALUES ('a1', 'executor', 1, 'EXECUTION');
             INSERT INTO active_agent_bindings VALUES ('a1');"
        ).unwrap();
        drop(connection);

        let workspace_scope = std::env::temp_dir().to_string_lossy().into_owned();
        let assessment = json!({
            "bounded": true, "acceptance_defined": true,
            "independent": true, "handoff_small": true,
            "work_units": 2
        });
        let call = |id: i32, assessment: Value| {
            json!({
                "jsonrpc": "2.0", "id": id, "method": "tools/call",
                "params": {"name": TOOL_NAME, "arguments": {
                    "agent_key": "executor", "workspace_scope": workspace_scope,
                    "assessment": assessment
                }}
            })
        };
        let mut input = format!(
            "{}\n{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})
        );
        input.push_str(&format!("{}\n", call(3, assessment.clone())));
        let mut explicit = assessment;
        explicit["explicit_request"] = true.into();
        input.push_str(&format!("{}\n", call(4, explicit)));
        let mut output = Vec::new();
        serve_io(&database_path, Cursor::new(input), &mut output).unwrap();
        let responses = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), 4);
        assert_eq!(
            responses[0]["result"]["capabilities"]["tools"]["listChanged"],
            false
        );
        assert_eq!(
            responses[1]["result"]["tools"][1]["name"],
            RUNTIME_HOOK_TOOL
        );
        assert_eq!(
            responses[1]["result"]["tools"][0]["annotations"]["readOnlyHint"],
            true
        );
        assert_eq!(
            responses[2]["result"]["structuredContent"]["action"],
            "SUGGEST"
        );
        assert_eq!(responses[3]["result"]["isError"], true);
        let connection = Connection::open(&database_path).unwrap();
        let job_table_exists: i64 = connection.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='orchestration_jobs'",
            [], |row| row.get(0)
        ).unwrap();
        assert_eq!(job_table_exists, 0);
        drop(connection);
        std::fs::remove_file(database_path).unwrap();
    }

    #[test]
    fn assessment_replay_covers_small_tasks_and_active_role_bindings() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database_path = std::env::temp_dir().join(format!(
            "cas-assessment-replay-{}-{unique}.db",
            std::process::id()
        ));
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE agents (
                    id TEXT PRIMARY KEY, agent_key TEXT, enabled INTEGER, orchestration_phase TEXT
                 );
                 CREATE TABLE active_agent_bindings (agent_id TEXT);
                 CREATE TABLE project_orchestration_exclusions (project_path TEXT);
                 INSERT INTO agents VALUES
                    ('a-exec', 'executor', 1, 'EXECUTION'),
                     ('a-disc', 'discoverer', 1, 'DISCOVERY'),
                     ('a-review', 'reviewer', 1, 'REVIEW'),
                     ('a-verify', 'verifier', 1, 'VERIFICATION'),
                     ('a-unsupported', 'unsupported', 1, 'PLANNING'),
                     ('a-disabled', 'disabled', 0, 'EXECUTION'),
                     ('a-unbound', 'unbound', 1, 'EXECUTION');
                  INSERT INTO active_agent_bindings VALUES
                     ('a-exec'), ('a-disc'), ('a-review'), ('a-verify'),
                     ('a-unsupported'), ('a-disabled');",
            )
            .unwrap();
        let excluded_scope = std::env::temp_dir()
            .join(format!("cas-assessment-excluded-{unique}"))
            .to_string_lossy()
            .into_owned();
        connection
            .execute(
                "INSERT INTO project_orchestration_exclusions VALUES (?1)",
                [&excluded_scope],
            )
            .unwrap();
        drop(connection);
        let original_database = std::fs::read(&database_path).unwrap();

        let cases = [
            (
                "task-1a",
                "executor",
                json!({"work_units":1,"estimated_minutes":5}),
                "PRIMARY",
                "ROLE_GATE_NOT_MET",
            ),
            (
                "task-1b",
                "executor",
                json!({"work_units":1,"estimated_minutes":8}),
                "PRIMARY",
                "ROLE_GATE_NOT_MET",
            ),
            (
                "task-qualified-as-two-units",
                "executor",
                json!({"work_units":2,"estimated_minutes":10}),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
            (
                "task-qualified-as-one-unit",
                "executor",
                json!({"work_units":1,"estimated_minutes":10}),
                "PRIMARY",
                "ROLE_GATE_NOT_MET",
            ),
            (
                "large-execution",
                "executor",
                json!({"work_units":2,"modules":2,"tests_defined":true,"estimated_minutes":30}),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
            (
                "long-single-unit",
                "executor",
                json!({"work_units":1,"estimated_minutes":120}),
                "PRIMARY",
                "ROLE_GATE_NOT_MET",
            ),
            (
                "user-forbidden",
                "executor",
                json!({"work_units":2,"estimated_minutes":30,"delegation_forbidden":true}),
                "PRIMARY",
                "USER_FORBIDDEN",
            ),
            (
                "unbounded",
                "executor",
                json!({"work_units":2,"bounded":false}),
                "PRIMARY",
                "TASK_NOT_INDEPENDENT",
            ),
            (
                "no-acceptance",
                "executor",
                json!({"work_units":2,"acceptance_defined":false}),
                "PRIMARY",
                "TASK_NOT_INDEPENDENT",
            ),
            (
                "dependent",
                "executor",
                json!({"work_units":2,"independent":false}),
                "PRIMARY",
                "TASK_NOT_INDEPENDENT",
            ),
            (
                "high-handoff",
                "executor",
                json!({"work_units":2,"estimated_minutes":30,"handoff_small":false}),
                "PRIMARY",
                "HANDOFF_COST_HIGH",
            ),
            (
                "wrong-role",
                "discoverer",
                json!({"work_units":2,"modules":2,"tests_defined":true,"estimated_minutes":30}),
                "PRIMARY",
                "ROLE_GATE_NOT_MET",
            ),
            (
                "discovery",
                "discoverer",
                json!({"call_chains":2,"estimated_minutes":20}),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
            (
                "discovery-at-benefit-threshold",
                "discoverer",
                json!({"call_chains":2,"estimated_minutes":15}),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
            (
                "high-risk-review",
                "reviewer",
                json!({"high_risk":true,"estimated_minutes":5}),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
            (
                "short-verification",
                "verifier",
                json!({"environments":2,"estimated_minutes":5}),
                "PRIMARY",
                "BENEFIT_NOT_PROVEN",
            ),
            (
                "large-verification",
                "verifier",
                json!({"environments":2,"estimated_minutes":20}),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
            (
                "disabled",
                "disabled",
                json!({"work_units":2,"estimated_minutes":30}),
                "UNAVAILABLE",
                "AGENT_NOT_EXECUTABLE",
            ),
            (
                "unbound",
                "unbound",
                json!({"work_units":2,"estimated_minutes":30}),
                "UNAVAILABLE",
                "AGENT_NOT_EXECUTABLE",
            ),
            (
                "missing",
                "missing",
                json!({"work_units":2,"estimated_minutes":30}),
                "UNAVAILABLE",
                "AGENT_NOT_EXECUTABLE",
            ),
            (
                "unsupported-phase",
                "unsupported",
                json!({"work_units":2,"estimated_minutes":30}),
                "UNAVAILABLE",
                "PHASE_UNSUPPORTED",
            ),
        ];
        let exclusion_cases = [
            (
                "excluded-root",
                excluded_scope.clone(),
                "PRIMARY",
                "PROJECT_EXCLUDED",
            ),
            (
                "excluded-child",
                format!("{excluded_scope}/child"),
                "PRIMARY",
                "PROJECT_EXCLUDED",
            ),
            (
                "excluded-case-variant",
                excluded_scope.to_uppercase(),
                "PRIMARY",
                "PROJECT_EXCLUDED",
            ),
            (
                "excluded-prefix-sibling",
                format!("{excluded_scope}-sibling"),
                "SUGGEST",
                "CANDIDATE_RECOMMENDED",
            ),
        ];
        let workspace_scope = std::env::temp_dir().to_string_lossy().into_owned();
        let mut input = format!(
            "{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"initialize"})
        );
        for (index, (_, agent_key, overrides, _, _)) in cases.iter().enumerate() {
            let mut assessment = json!({
                "bounded":true, "acceptance_defined":true,
                "independent":true, "handoff_small":true
            });
            assessment
                .as_object_mut()
                .unwrap()
                .extend(overrides.as_object().unwrap().clone());
            input.push_str(&format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0", "id":index+2, "method":"tools/call",
                    "params":{"name":TOOL_NAME,"arguments":{
                        "agent_key":agent_key,
                        "workspace_scope":workspace_scope,
                        "assessment":assessment
                    }}
                })
            ));
        }
        for (index, (_, workspace_scope, _, _)) in exclusion_cases.iter().enumerate() {
            input.push_str(&format!(
                "{}\n",
                json!({
                    "jsonrpc":"2.0", "id":cases.len()+index+2, "method":"tools/call",
                    "params":{"name":TOOL_NAME,"arguments":{
                        "agent_key":"executor",
                        "workspace_scope":workspace_scope,
                        "assessment":{
                            "bounded":true, "acceptance_defined":true,
                            "independent":true, "handoff_small":true, "work_units":2
                        }
                    }}
                })
            ));
        }
        let mut output = Vec::new();
        serve_io(&database_path, Cursor::new(input), &mut output).unwrap();
        let responses = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(responses.len(), cases.len() + exclusion_cases.len() + 1);
        let expected = cases
            .iter()
            .map(|(name, _, _, action, reason)| (name, action, reason))
            .chain(
                exclusion_cases
                    .iter()
                    .map(|(name, _, action, reason)| (name, action, reason)),
            );
        for (index, (name, action, reason)) in expected.enumerate() {
            let result = &responses[index + 1]["result"];
            assert_eq!(result["isError"], false, "{name}");
            assert_eq!(result["structuredContent"]["action"], *action, "{name}");
            assert_eq!(
                result["structuredContent"]["reason_code"], *reason,
                "{name}"
            );
        }
        assert_eq!(
            std::fs::read(&database_path).unwrap(),
            original_database,
            "只读评估不得修改数据库或创建 Job/Lease"
        );
        std::fs::remove_file(database_path).unwrap();
    }

    #[test]
    fn assessment_rejects_invalid_facts_before_opening_database() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let database_path = std::env::temp_dir().join(format!(
            "cas-invalid-assessment-{}-{unique}.db",
            std::process::id()
        ));
        let cases = [
            (
                "explicit-false",
                json!({"explicit_request":false}),
                "explicit_request",
            ),
            (
                "unknown-field",
                json!({"score":100}),
                "ASSESSMENT_INPUT_INVALID",
            ),
            (
                "negative-units",
                json!({"work_units":-1}),
                "ASSESSMENT_INPUT_INVALID",
            ),
            (
                "overflow-units",
                json!({"work_units":256}),
                "ASSESSMENT_INPUT_INVALID",
            ),
            (
                "fractional-time",
                json!({"estimated_minutes":15.5}),
                "ASSESSMENT_INPUT_INVALID",
            ),
            (
                "wrong-gate-type",
                json!({"independent":"true"}),
                "ASSESSMENT_INPUT_INVALID",
            ),
        ];
        for (name, overrides, reason) in cases {
            let mut assessment = json!({
                "bounded":true, "acceptance_defined":true,
                "independent":true, "handoff_small":true, "work_units":2
            });
            assessment
                .as_object_mut()
                .unwrap()
                .extend(overrides.as_object().unwrap().clone());
            let result = call_tool(
                &database_path,
                Some(&json!({
                    "name":TOOL_NAME, "arguments":{
                        "agent_key":"executor",
                        "workspace_scope":std::env::temp_dir().to_string_lossy(),
                        "assessment":assessment
                    }
                })),
            )
            .unwrap_err();
            assert!(result.contains(reason), "{name}: {result}");
            assert!(!database_path.exists(), "{name}");
        }
    }

    #[test]
    fn runtime_hook_tool_rejects_unknown_events_before_starting_helper() {
        let result = call_tool(
            Path::new("unused.db"),
            Some(&json!({
                "name": RUNTIME_HOOK_TOOL,
                "arguments": {"hook_event_name": "NotARealEvent"}
            })),
        );
        assert_eq!(result.unwrap_err(), "不支持的 Hook 事件");
    }
}
