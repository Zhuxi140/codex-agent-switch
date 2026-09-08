use std::collections::HashSet;
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::codex_config::{RUNTIME_HOOK_EVENTS, RUNTIME_HOOK_MARKER};

const HOOK_STATUS_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum RuntimeHookStatus {
    NotRequired,
    Unsupported,
    NotInstalled,
    Incomplete,
    PendingTrust,
    Modified,
    Disabled,
    Active,
    Unavailable,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeHookStatusResponse {
    pub(crate) status: RuntimeHookStatus,
    pub(crate) expected_hook_count: usize,
    pub(crate) installed_hook_count: usize,
    pub(crate) trusted_hook_count: usize,
    pub(crate) message: String,
}

impl RuntimeHookStatusResponse {
    pub(crate) fn not_required() -> Self {
        Self::new(
            RuntimeHookStatus::NotRequired,
            0,
            0,
            "当前为 Default 模式，不需要 CAS Runtime Hook。".to_owned(),
        )
    }

    pub(crate) fn unsupported() -> Self {
        Self::new(
            RuntimeHookStatus::Unsupported,
            0,
            0,
            "当前 Codex 未声明可用的 hooks 能力；CAS 无法启用本地工具调用 Guard。".to_owned(),
        )
    }

    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(RuntimeHookStatus::Unavailable, 0, 0, message.into())
    }

    fn new(
        status: RuntimeHookStatus,
        installed_hook_count: usize,
        trusted_hook_count: usize,
        message: String,
    ) -> Self {
        Self {
            status,
            expected_hook_count: RUNTIME_HOOK_EVENTS.len(),
            installed_hook_count,
            trusted_hook_count,
            message,
        }
    }
}

pub(crate) fn probe_runtime_hook_status(
    executable: &Path,
    codex_home: &Path,
) -> RuntimeHookStatusResponse {
    match list_hooks(executable, codex_home) {
        Ok(response) => classify_hooks(response, &codex_home.join("config.toml")),
        Err(error) => RuntimeHookStatusResponse::unavailable(format!(
            "无法通过当前 Codex 的 hooks/list 核验 CAS Hook 状态：{error}"
        )),
    }
}

#[derive(Debug, Deserialize)]
struct HooksListResponse {
    data: Vec<HooksListEntry>,
}

#[derive(Debug, Deserialize)]
struct HooksListEntry {
    hooks: Vec<HookMetadata>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookMetadata {
    command: Option<String>,
    enabled: bool,
    event_name: String,
    source_path: Option<String>,
    trust_status: String,
}

fn classify_hooks(
    response: HooksListResponse,
    expected_source_path: &Path,
) -> RuntimeHookStatusResponse {
    let hooks = response
        .data
        .into_iter()
        .flat_map(|entry| entry.hooks)
        .filter(|hook| {
            hook.command
                .as_deref()
                .is_some_and(|command| command.contains(RUNTIME_HOOK_MARKER))
        })
        .collect::<Vec<_>>();
    let installed = hooks.len();
    let trusted = hooks
        .iter()
        .filter(|hook| matches!(hook.trust_status.as_str(), "trusted" | "managed"))
        .count();
    if installed == 0 {
        return RuntimeHookStatusResponse::new(
            RuntimeHookStatus::NotInstalled,
            0,
            0,
            "当前 Codex 未列出 CAS Runtime Hook；请重新同步子 Agent 模式。".to_owned(),
        );
    }

    if hooks.iter().any(|hook| {
        hook.source_path
            .as_deref()
            .is_none_or(|source| !same_path(Path::new(source), expected_source_path))
    }) {
        return RuntimeHookStatusResponse::new(
            RuntimeHookStatus::Incomplete,
            installed,
            trusted,
            format!(
                "Codex 列出的 CAS Runtime Hook 并非全部来自当前 CODEX_HOME 的 {}；已按配置不完整处理。",
                expected_source_path.display()
            ),
        );
    }

    if hooks.iter().any(|hook| !hook.enabled) {
        return RuntimeHookStatusResponse::new(
            RuntimeHookStatus::Disabled,
            installed,
            trusted,
            format!("CAS Runtime Hook 已安装，但至少一项被禁用（已信任 {trusted}/{installed}）。"),
        );
    }

    let expected_events = RUNTIME_HOOK_EVENTS
        .iter()
        .map(|(event, _)| event_name_for_api(event))
        .collect::<HashSet<_>>();
    let installed_events = hooks
        .iter()
        .map(|hook| hook.event_name.as_str())
        .collect::<HashSet<_>>();
    if installed != expected_events.len()
        || installed_events.len() != expected_events.len()
        || !expected_events
            .iter()
            .all(|event| installed_events.contains(event.as_str()))
    {
        return RuntimeHookStatusResponse::new(
            RuntimeHookStatus::Incomplete,
            installed,
            trusted,
            format!(
                "CAS Runtime Hook 配置不完整：当前 Codex 列出 {installed} 项，期望 {} 项；请重新同步子 Agent 模式。",
                expected_events.len()
            ),
        );
    }

    if hooks.iter().any(|hook| hook.trust_status == "modified") {
        return RuntimeHookStatusResponse::new(
            RuntimeHookStatus::Modified,
            installed,
            trusted,
            "CAS Runtime Hook 在上次信任后发生变化；必须在 Codex 的 /hooks 中重新审核。".to_owned(),
        );
    }
    if hooks.iter().any(|hook| hook.trust_status == "untrusted") {
        return RuntimeHookStatusResponse::new(
            RuntimeHookStatus::PendingTrust,
            installed,
            trusted,
            format!(
                "CAS Runtime Hook 已写入，但尚未全部受信任（已信任 {trusted}/{installed}）；未完成审核前 Codex 不会执行这些 Hook。"
            ),
        );
    }
    if hooks
        .iter()
        .any(|hook| !matches!(hook.trust_status.as_str(), "trusted" | "managed"))
    {
        return RuntimeHookStatusResponse::unavailable(
            "当前 Codex 返回了 CAS 尚不认识的 Hook 信任状态；已按未核验处理。",
        );
    }

    RuntimeHookStatusResponse::new(
        RuntimeHookStatus::Active,
        installed,
        trusted,
        format!("当前 Codex 报告全部 {installed} 项 CAS Runtime Hook 已启用并受信任。"),
    )
}

fn event_name_for_api(event: &str) -> String {
    let mut chars = event.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn list_hooks(executable: &Path, codex_home: &Path) -> Result<HooksListResponse, HookProbeError> {
    let mut child = Command::new(executable)
        .args(["app-server", "--listen", "stdio://"])
        .env("CODEX_HOME", codex_home)
        .current_dir(codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(HookProbeError::Spawn)?;
    let result = probe_child(&mut child, codex_home);
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn probe_child(child: &mut Child, codex_home: &Path) -> Result<HooksListResponse, HookProbeError> {
    let mut stdin = child
        .stdin
        .take()
        .ok_or(HookProbeError::MissingPipe("stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or(HookProbeError::MissingPipe("stdout"))?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let item = line.map_err(|error| error.to_string()).and_then(|line| {
                serde_json::from_str::<Value>(&line).map_err(|error| error.to_string())
            });
            if sender.send(item).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + HOOK_STATUS_PROBE_TIMEOUT;
    write_message(
        &mut stdin,
        &json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "codex_agent_switch",
                    "title": "Codex Agent Switch",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
    )?;
    let initialized = wait_for_response(&receiver, 1, deadline)?;
    let loaded_home = initialized.get("codexHome").and_then(Value::as_str).ok_or(
        HookProbeError::InvalidResponse("initialize 未返回 codexHome".to_owned()),
    )?;
    if !same_path(Path::new(loaded_home), codex_home) {
        return Err(HookProbeError::InvalidResponse(format!(
            "Codex 实际加载了 {loaded_home}，与 CAS 当前 CODEX_HOME {} 不一致",
            codex_home.display()
        )));
    }

    write_message(&mut stdin, &json!({"method": "initialized", "params": {}}))?;
    write_message(
        &mut stdin,
        &json!({
            "id": 2,
            "method": "hooks/list",
            "params": {"cwds": [codex_home.to_string_lossy()]}
        }),
    )?;
    let hooks = wait_for_response(&receiver, 2, deadline)?;
    serde_json::from_value(hooks).map_err(|error| {
        HookProbeError::InvalidResponse(format!("hooks/list 返回结构不兼容：{error}"))
    })
}

fn write_message(stdin: &mut impl Write, message: &Value) -> Result<(), HookProbeError> {
    serde_json::to_writer(&mut *stdin, message)
        .map_err(|error| HookProbeError::InvalidResponse(error.to_string()))?;
    stdin.write_all(b"\n").map_err(HookProbeError::Io)?;
    stdin.flush().map_err(HookProbeError::Io)
}

fn wait_for_response(
    receiver: &mpsc::Receiver<Result<Value, String>>,
    request_id: i64,
    deadline: Instant,
) -> Result<Value, HookProbeError> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(HookProbeError::Timeout)?;
        let message = receiver
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => HookProbeError::Timeout,
                mpsc::RecvTimeoutError::Disconnected => HookProbeError::StreamClosed,
            })?
            .map_err(HookProbeError::InvalidResponse)?;
        if message.get("id").and_then(Value::as_i64) != Some(request_id) {
            continue;
        }
        if let Some(error) = message.get("error") {
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("未知协议错误");
            return Err(HookProbeError::Rejected(message.to_owned()));
        }
        return message
            .get("result")
            .cloned()
            .ok_or_else(|| HookProbeError::InvalidResponse("响应缺少 result".to_owned()));
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = left
        .canonicalize()
        .unwrap_or_else(|_| left.to_path_buf())
        .to_string_lossy()
        .into_owned();
    let right = right
        .canonicalize()
        .unwrap_or_else(|_| right.to_path_buf())
        .to_string_lossy()
        .into_owned();
    if cfg!(windows) {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

#[derive(Debug)]
enum HookProbeError {
    Spawn(std::io::Error),
    MissingPipe(&'static str),
    Io(std::io::Error),
    Timeout,
    StreamClosed,
    Rejected(String),
    InvalidResponse(String),
}

impl fmt::Display for HookProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "无法启动 app-server：{error}"),
            Self::MissingPipe(pipe) => write!(formatter, "app-server 缺少 {pipe} 管道"),
            Self::Io(error) => write!(formatter, "app-server 通信失败：{error}"),
            Self::Timeout => formatter.write_str("hooks/list 探测超过 5 秒"),
            Self::StreamClosed => formatter.write_str("app-server 在返回 hooks/list 前退出"),
            Self::Rejected(message) => write!(formatter, "hooks/list 被拒绝：{message}"),
            Self::InvalidResponse(message) => write!(formatter, "{message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event_name: &str, enabled: bool, trust_status: &str) -> HookMetadata {
        HookMetadata {
            command: Some(format!(
                "\"cas-helper\" hook \"cas.db\" {RUNTIME_HOOK_MARKER}"
            )),
            enabled,
            event_name: event_name.to_owned(),
            source_path: Some("C:\\fixture\\.codex\\config.toml".to_owned()),
            trust_status: trust_status.to_owned(),
        }
    }

    fn response(hooks: Vec<HookMetadata>) -> HooksListResponse {
        HooksListResponse {
            data: vec![HooksListEntry { hooks }],
        }
    }

    fn complete_hooks(trust_status: &str) -> Vec<HookMetadata> {
        RUNTIME_HOOK_EVENTS
            .iter()
            .map(|(event, _)| hook(&event_name_for_api(event), true, trust_status))
            .collect()
    }

    fn classify_fixture(hooks: Vec<HookMetadata>) -> RuntimeHookStatusResponse {
        classify_hooks(
            response(hooks),
            Path::new("C:\\fixture\\.codex\\config.toml"),
        )
    }

    #[test]
    fn hook_status_requires_all_expected_events_to_be_trusted() {
        let active = classify_fixture(complete_hooks("trusted"));
        assert_eq!(active.status, RuntimeHookStatus::Active);
        assert_eq!(active.installed_hook_count, RUNTIME_HOOK_EVENTS.len());
        assert_eq!(active.trusted_hook_count, RUNTIME_HOOK_EVENTS.len());

        let pending = classify_fixture(complete_hooks("untrusted"));
        assert_eq!(pending.status, RuntimeHookStatus::PendingTrust);
        assert_eq!(pending.trusted_hook_count, 0);
    }

    #[test]
    fn hook_status_fails_closed_for_modified_disabled_and_partial_config() {
        let modified = classify_fixture(complete_hooks("modified"));
        assert_eq!(modified.status, RuntimeHookStatus::Modified);

        let mut disabled_hooks = complete_hooks("trusted");
        disabled_hooks[0].enabled = false;
        let disabled = classify_fixture(disabled_hooks);
        assert_eq!(disabled.status, RuntimeHookStatus::Disabled);

        let mut partial_hooks = complete_hooks("trusted");
        partial_hooks.pop();
        let partial = classify_fixture(partial_hooks);
        assert_eq!(partial.status, RuntimeHookStatus::Incomplete);
    }

    #[test]
    fn hook_status_ignores_non_cas_hooks() {
        let unrelated = HookMetadata {
            command: Some("other-hook".to_owned()),
            enabled: true,
            event_name: "stop".to_owned(),
            source_path: Some("C:\\fixture\\.codex\\config.toml".to_owned()),
            trust_status: "trusted".to_owned(),
        };
        let status = classify_fixture(vec![unrelated]);
        assert_eq!(status.status, RuntimeHookStatus::NotInstalled);
    }

    #[test]
    fn hook_status_requires_the_active_codex_home_config_source() {
        let mut hooks = complete_hooks("trusted");
        hooks[0].source_path = Some("C:\\other\\.codex\\config.toml".to_owned());
        let wrong_source = classify_fixture(hooks);
        assert_eq!(wrong_source.status, RuntimeHookStatus::Incomplete);

        let mut hooks = complete_hooks("trusted");
        hooks[0].source_path = None;
        let missing_source = classify_fixture(hooks);
        assert_eq!(missing_source.status, RuntimeHookStatus::Incomplete);
    }
}
