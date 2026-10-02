//! 手动运行：cargo test -p codex-agent-switch chat_gateway::e2e -- --ignored --nocapture
//! 使用隔离数据库和假的上游密钥，不访问用户的 Provider 或真实模型。

use std::env;
use std::fs;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use cas_secret_store::{CredentialId, SecretValue, delete, store};
use rusqlite::params;
use serde_json::{Value, json};
use tiny_http::{Response, Server};
use uuid::Uuid;

use super::{provider_base_url, start};
use crate::persistence::open_database;

struct TestResources {
    directory: std::path::PathBuf,
    credential: CredentialId,
}

impl Drop for TestResources {
    fn drop(&mut self) {
        let _ = delete(self.credential);
        let _ = fs::remove_dir_all(&self.directory);
    }
}

#[test]
#[ignore = "需要本机 Codex CLI，且 43187 端口空闲"]
fn codex_exec_command_round_trip_through_chat_gateway() {
    let directory = env::temp_dir().join(format!("cas-chat-e2e-{}", Uuid::new_v4()));
    fs::create_dir(&directory).unwrap();
    let credential_text = Uuid::new_v4().to_string();
    let credential = CredentialId::from_str(&credential_text).unwrap();
    let resources = TestResources {
        directory,
        credential,
    };
    fs::create_dir(resources.directory.join("codex-home")).unwrap();
    store(
        credential,
        &SecretValue::from_string("cas-fake-chat-key".to_owned()).unwrap(),
    )
    .unwrap();

    let upstream = Server::http("127.0.0.1:0").unwrap();
    let upstream_port = upstream.server_addr().to_ip().unwrap().port();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = serve_mock_chat(upstream);
        let _ = sender.send(result);
    });

    let provider_id = Uuid::new_v4().to_string();
    let database_path = resources.directory.join("cas.db");
    let connection = open_database(&database_path).unwrap();
    connection
        .execute(
            "INSERT INTO providers
         (id, provider_key, name, provider_type, base_url, protocol,
          auth_type, enabled, source, created_at, updated_at)
         VALUES (?1, ?2, 'CAS mock', 'CUSTOM', ?3, 'CHAT_COMPLETIONS',
                 'BEARER_TOKEN', 1, 'USER', '2026-01-01', '2026-01-01')",
            params![
                provider_id,
                format!("chat-e2e-{provider_id}"),
                format!("http://127.0.0.1:{upstream_port}/v1")
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO credentials
         (id, provider_id, credential_key, secret_type, storage_backend,
          storage_key, created_at, updated_at)
         VALUES (?1, ?2, 'primary', 'BEARER_TOKEN',
                 'WINDOWS_CREDENTIAL_MANAGER', ?1, '2026-01-01', '2026-01-01')",
            params![credential_text, provider_id],
        )
        .unwrap();
    drop(connection);
    start(database_path).unwrap();

    let base_url = provider_base_url(&provider_id);
    let mut child = Command::new("codex")
        .args([
            "exec",
            "--ignore-user-config",
            "--skip-git-repo-check",
            "--ephemeral",
            "--sandbox",
            "read-only",
            "-C",
        ])
        .arg(&resources.directory)
        .args([
            "-m",
            "cas-mock-chat",
            "-c",
            "model_provider=cas_mock",
            "-c",
            "model_providers.cas_mock.name=CASMock",
            "-c",
            &format!("model_providers.cas_mock.base_url={base_url}"),
            "-c",
            "model_providers.cas_mock.wire_api=responses",
            "-c",
            "model_providers.cas_mock.env_key=CAS_TEST_GATEWAY_TOKEN",
            "-c",
            "model_reasoning_effort=none",
            "Call exec_command once to run Write-Output CAS_CHILD_TOOL_OK; then report the result.",
        ])
        .env("CODEX_HOME", resources.directory.join("codex-home"))
        .env(
            "CAS_TEST_GATEWAY_TOKEN",
            format!("cas-gateway-{credential_text}"),
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("Codex CLI 在 90 秒内未结束");
        }
        thread::sleep(Duration::from_millis(100));
    }
    let output = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "Codex CLI failed: {stderr}");
    assert!(stdout.contains("CAS_CHILD_LOOP_OK"), "{stdout}\n{stderr}");
    receiver
        .recv_timeout(Duration::from_secs(3))
        .unwrap()
        .unwrap();
}

fn serve_mock_chat(server: Server) -> Result<(), String> {
    for turn in 0..2 {
        let mut request = server
            .recv_timeout(Duration::from_secs(70))
            .map_err(|error| error.to_string())?
            .ok_or("未收到 Chat 请求")?;
        if request.url() != "/v1/chat/completions" {
            return Err(format!("错误的上游路径：{}", request.url()));
        }
        let mut body = String::new();
        request
            .as_reader()
            .read_to_string(&mut body)
            .map_err(|error| error.to_string())?;
        let body: Value = serde_json::from_str(&body).map_err(|error| error.to_string())?;
        let messages = body["messages"].as_array().ok_or("缺少 Chat messages")?;
        let has_exec = body["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool["function"]["name"] == "exec_command")
        });
        if !has_exec {
            return Err("Codex exec_command 未映射到 Chat tools".to_owned());
        }
        let message = if turn == 0 {
            json!({"role":"assistant","content":null,"tool_calls":[{
                "id":"call_cas_e2e","type":"function",
                "function":{"name":"exec_command","arguments":
                    "{\"cmd\":\"Write-Output CAS_CHILD_TOOL_OK\"}"}
            }]})
        } else {
            let tool_result = messages.iter().find(|message| {
                message["role"] == "tool" && message["tool_call_id"] == "call_cas_e2e"
            });
            if !tool_result.is_some_and(|message| {
                message["content"]
                    .as_str()
                    .is_some_and(|content| content.contains("CAS_CHILD_TOOL_OK"))
            }) {
                return Err("Codex 工具结果未回传到 Chat messages".to_owned());
            }
            json!({"role":"assistant","content":"CAS_CHILD_LOOP_OK"})
        };
        let completion = json!({
            "id":format!("chatcmpl-cas-e2e-{turn}"),"object":"chat.completion",
            "created":1_757_000_000,"model":"cas-mock-chat",
            "choices":[{"index":0,"finish_reason":if turn == 0 {"tool_calls"} else {"stop"},
                "message":message}],
            "usage":{"prompt_tokens":20,"completion_tokens":5,"total_tokens":25}
        });
        request
            .respond(Response::from_string(completion.to_string()))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}
