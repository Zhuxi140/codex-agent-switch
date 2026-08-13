use std::env;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use serde::Deserialize;
use serde::Serialize;

#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const RUNTIME_OVERRIDE_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const MINIMUM_MULTI_AGENT_VERSION: ClientVersion = ClientVersion(0, 144, 0);
#[cfg(windows)]
const WINDOWS_TO_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CodexEnvironment {
    pub(crate) detected: bool,
    pub(crate) executable_path: Option<String>,
    pub(crate) codex_home: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) supported: bool,
    pub(crate) configuration_readable: bool,
    pub(crate) configuration_writable: bool,
    pub(crate) multi_agent_available: bool,
    pub(crate) issues: Vec<DiagnosticIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimePermissionOverride {
    pub(crate) process_id: u32,
    pub(crate) flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticIssue {
    code: &'static str,
    severity: DiagnosticSeverity,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum DiagnosticSeverity {
    Warning,
    Error,
}

pub(crate) fn detect_with_codex_home(custom_codex_home: Option<PathBuf>) -> CodexEnvironment {
    let codex_home = custom_codex_home.or_else(|| {
        resolve_codex_home(
            env::var_os("CODEX_HOME"),
            env::var_os("USERPROFILE"),
            env::var_os("HOME"),
        )
    });
    let executable = find_codex_executable();
    let config_path = codex_home.as_ref().map(|path| path.join("config.toml"));
    let configuration_readable = config_path
        .as_ref()
        .is_some_and(|path| File::open(path).is_ok());
    let configuration_writable = config_path
        .as_ref()
        .is_some_and(|path| OpenOptions::new().write(true).open(path).is_ok());
    let mut issues = Vec::new();

    if codex_home.is_none() {
        issues.push(issue(
            "CODEX_HOME_UNRESOLVED",
            DiagnosticSeverity::Error,
            "无法定位 CODEX_HOME。",
        ));
    } else if !codex_home.as_ref().is_some_and(|path| path.is_dir()) {
        issues.push(issue(
            "CODEX_HOME_NOT_FOUND",
            DiagnosticSeverity::Error,
            "CODEX_HOME 目录不存在。",
        ));
    } else if !config_path.as_ref().is_some_and(|path| path.is_file()) {
        issues.push(issue(
            "CODEX_CONFIG_NOT_FOUND",
            DiagnosticSeverity::Warning,
            "未找到 Codex config.toml。",
        ));
    } else if !configuration_readable {
        issues.push(issue(
            "CODEX_CONFIG_NOT_READABLE",
            DiagnosticSeverity::Error,
            "Codex config.toml 不可读取。",
        ));
    } else if !configuration_writable {
        issues.push(issue(
            "CODEX_CONFIG_NOT_WRITABLE",
            DiagnosticSeverity::Warning,
            "Codex config.toml 当前不可写。",
        ));
    }

    let version = match executable.as_ref() {
        Some(path) => match probe_codex_version(path) {
            Ok(version) => Some(version),
            Err(message) => {
                issues.push(issue(
                    "CODEX_VERSION_PROBE_FAILED",
                    DiagnosticSeverity::Warning,
                    message,
                ));
                None
            }
        },
        None => {
            issues.push(issue(
                "CODEX_EXECUTABLE_NOT_FOUND",
                DiagnosticSeverity::Error,
                "未在 PATH 或 Windows App Execution Aliases 中找到 codex。",
            ));
            None
        }
    };

    let supported = version
        .as_deref()
        .and_then(parse_client_version)
        .is_some_and(|version| version >= MINIMUM_MULTI_AGENT_VERSION);

    if version.is_some() && !supported {
        issues.push(issue(
            "CODEX_VERSION_UNSUPPORTED",
            DiagnosticSeverity::Error,
            "Codex 客户端低于 0.144.0。",
        ));
    }

    CodexEnvironment {
        detected: executable.is_some(),
        executable_path: executable.map(path_to_string),
        codex_home: codex_home.map(path_to_string),
        version,
        supported,
        configuration_readable,
        configuration_writable,
        multi_agent_available: supported,
        issues,
    }
}

pub(crate) fn restart_required(last_applied_at_ms: i64) -> bool {
    #[cfg(windows)]
    {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return false;
            }

            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut process_start_times = Vec::new();
            let mut has_entry = Process32FirstW(snapshot, &mut entry) != 0;
            while has_entry {
                if is_codex_process_name(&entry.szExeFile) {
                    let process =
                        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                    if !process.is_null() {
                        let mut created = FILETIME::default();
                        let mut exited = FILETIME::default();
                        let mut kernel = FILETIME::default();
                        let mut user = FILETIME::default();
                        if GetProcessTimes(
                            process,
                            &mut created,
                            &mut exited,
                            &mut kernel,
                            &mut user,
                        ) != 0
                        {
                            process_start_times.push(filetime_to_unix_ms(created));
                        }
                        CloseHandle(process);
                    }
                }
                has_entry = Process32NextW(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            restart_required_for_process_start_times(last_applied_at_ms, process_start_times)
        }
    }
    #[cfg(not(windows))]
    {
        let _ = last_applied_at_ms;
        false
    }
}

fn restart_required_for_process_start_times(
    last_applied_at_ms: i64,
    process_start_times: impl IntoIterator<Item = i64>,
) -> bool {
    let mut found_process = false;
    for process_start_time in process_start_times {
        found_process = true;
        if process_start_time >= last_applied_at_ms {
            return false;
        }
    }
    found_process
}

pub(crate) fn detect_runtime_permission_overrides() -> Result<Vec<RuntimePermissionOverride>, String>
{
    #[cfg(windows)]
    {
        let mut child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                r#"ConvertTo-Json -InputObject @(Get-CimInstance Win32_Process -Filter "Name='codex.exe' OR Name='ChatGPT.exe'" -ErrorAction Stop | Select-Object ProcessId,Name,CommandLine) -Compress"#,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("无法启动运行时权限覆盖检测：{error}"))?;
        let deadline = Instant::now() + RUNTIME_OVERRIDE_PROBE_TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("运行时权限覆盖检测超过 3 秒。".to_owned());
                }
                Err(error) => return Err(format!("运行时权限覆盖检测失败：{error}")),
            }
        };
        let stdout = read_pipe(child.stdout.take());
        let stderr = read_pipe(child.stderr.take());
        if !status.success() {
            return Err(if stderr.trim().is_empty() {
                format!("运行时权限覆盖检测退出码：{status}")
            } else {
                format!("运行时权限覆盖检测失败：{}", stderr.trim())
            });
        }
        if stdout.trim().is_empty() {
            return Err(if stderr.trim().is_empty() {
                "运行时权限覆盖检测没有返回结果。".to_owned()
            } else {
                format!("运行时权限覆盖检测失败：{}", stderr.trim())
            });
        }
        let processes = serde_json::from_str::<Vec<WindowsProcessCommand>>(stdout.trim())
            .map_err(|error| format!("运行时权限覆盖检测结果无法解析：{error}"))?;
        return Ok(collect_permission_overrides(
            processes.into_iter().filter_map(|process| {
                process
                    .command_line
                    .map(|command_line| (process.process_id, process.name, command_line))
            }),
        ));
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("ps")
            .args(["-axo", "pid=,comm=,args="])
            .output()
            .map_err(|error| format!("无法启动运行时权限覆盖检测：{error}"))?;
        if !output.status.success() {
            return Err(format!("运行时权限覆盖检测退出码：{}", output.status));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let processes = stdout.lines().filter_map(parse_ps_process);
        Ok(collect_permission_overrides(processes))
    }
}

#[cfg(windows)]
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct WindowsProcessCommand {
    process_id: u32,
    name: String,
    command_line: Option<String>,
}

fn collect_permission_overrides(
    processes: impl IntoIterator<Item = (u32, String, String)>,
) -> Vec<RuntimePermissionOverride> {
    processes
        .into_iter()
        .filter(|(_, name, _)| is_codex_process_label(name))
        .filter_map(|(process_id, _, command_line)| {
            let flags = permission_override_flags(&command_line);
            (!flags.is_empty()).then_some(RuntimePermissionOverride { process_id, flags })
        })
        .collect()
}

fn is_codex_process_label(name: &str) -> bool {
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_ascii_lowercase();
    matches!(
        name.as_str(),
        "codex" | "codex.exe" | "chatgpt" | "chatgpt.exe"
    )
}

fn permission_override_flags(command_line: &str) -> Vec<String> {
    let tokens = command_line
        .split_whitespace()
        .map(|token| token.trim_matches(['"', '\'']).to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut flags = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if matches!(
            token.as_str(),
            "--approve-for-me"
                | "--dangerously-bypass-approvals-and-sandbox"
                | "--full-auto"
                | "--yolo"
        ) {
            flags.push(token.clone());
        } else if token == "--sandbox" || token == "-s" {
            let value = tokens.get(index + 1).map(String::as_str).unwrap_or("?");
            flags.push(format!("{token}={value}"));
            index += 1;
        } else if let Some(value) = token.strip_prefix("--sandbox=") {
            flags.push(format!("--sandbox={value}"));
        } else if matches!(
            token.as_str(),
            "--ask-for-approval" | "-a" | "--profile" | "-p" | "--add-dir"
        ) {
            let value = tokens.get(index + 1).map(String::as_str).unwrap_or("?");
            flags.push(format!("{token}={value}"));
            index += 1;
        } else if ["--ask-for-approval=", "--profile=", "--add-dir="]
            .iter()
            .any(|prefix| token.starts_with(prefix))
        {
            flags.push(token.clone());
        } else if token == "-c" || token == "--config" {
            if let Some(value) = tokens.get(index + 1)
                && is_permission_config_override(value)
            {
                flags.push(format!("{token}={value}"));
            }
            index += 1;
        } else if let Some(value) = token.strip_prefix("--config=")
            && is_permission_config_override(value)
        {
            flags.push(format!("--config={value}"));
        }
        index += 1;
    }
    flags.sort();
    flags.dedup();
    flags
}

fn is_permission_config_override(value: &str) -> bool {
    ["approval_policy=", "default_permissions=", "sandbox_mode="]
        .iter()
        .any(|key| value.starts_with(key))
}

#[cfg(not(windows))]
fn parse_ps_process(line: &str) -> Option<(u32, String, String)> {
    let mut fields = line.split_whitespace();
    let process_id_text = fields.next()?;
    let process_id = process_id_text.parse().ok()?;
    let name = fields.next()?.to_owned();
    let command_line = fields.collect::<Vec<_>>().join(" ");
    Some((process_id, name, command_line))
}

#[cfg(windows)]
fn is_codex_process_name(buffer: &[u16]) -> bool {
    let length = buffer
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(buffer.len());
    matches!(
        String::from_utf16_lossy(&buffer[..length])
            .to_ascii_lowercase()
            .as_str(),
        "chatgpt.exe" | "codex.exe"
    )
}

#[cfg(windows)]
fn filetime_to_unix_ms(value: FILETIME) -> i64 {
    let ticks = (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime);
    (ticks.saturating_sub(WINDOWS_TO_UNIX_EPOCH_100NS) / 10_000) as i64
}

fn resolve_codex_home(
    codex_home: Option<OsString>,
    user_profile: Option<OsString>,
    home: Option<OsString>,
) -> Option<PathBuf> {
    non_empty_path(codex_home).or_else(|| {
        non_empty_path(user_profile)
            .or_else(|| non_empty_path(home))
            .map(|path| path.join(".codex"))
    })
}

fn non_empty_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

fn find_codex_executable() -> Option<PathBuf> {
    let mut directories = env::var_os("PATH")
        .map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default();

    if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
        directories.push(
            PathBuf::from(local_app_data)
                .join("Microsoft")
                .join("WindowsApps"),
        );
    }

    let names: &[&str] = if cfg!(windows) {
        &["codex.exe", "codex.cmd", "codex.bat"]
    } else {
        &["codex"]
    };

    directories
        .iter()
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

fn probe_codex_version(executable: &Path) -> Result<String, String> {
    let mut child = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动 Codex 版本探测：{error}"))?;
    let deadline = Instant::now() + VERSION_PROBE_TIMEOUT;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Codex 版本探测超过 2 秒。".to_owned());
            }
            Err(error) => return Err(format!("Codex 版本探测失败：{error}")),
        }
    };

    let stdout = read_pipe(child.stdout.take());
    let stderr = read_pipe(child.stderr.take());
    if !status.success() {
        let detail = stderr.trim();
        return Err(if detail.is_empty() {
            format!("Codex 版本探测退出码：{status}")
        } else {
            format!("Codex 版本探测失败：{detail}")
        });
    }

    let output = if stdout.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };

    extract_version(output).ok_or_else(|| "Codex 版本输出无法解析。".to_owned())
}

fn read_pipe(pipe: Option<impl Read>) -> String {
    let mut output = String::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_string(&mut output);
    }
    output
}

fn extract_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let token = token
            .trim_matches(|character: char| !character.is_ascii_alphanumeric() && character != '.');
        let token = token.strip_prefix('v').unwrap_or(token);
        let core = token.split(['-', '+']).next()?;
        parse_client_version(core).map(|_| core.to_owned())
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct ClientVersion(u64, u64, u64);

fn parse_client_version(value: &str) -> Option<ClientVersion> {
    let core = value
        .trim()
        .strip_prefix('v')
        .unwrap_or(value.trim())
        .split(['-', '+'])
        .next()?;
    let mut parts = core.split('.');
    let version = ClientVersion(
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    parts.next().is_none().then_some(version)
}

fn issue(
    code: &'static str,
    severity: DiagnosticSeverity,
    message: impl Into<String>,
) -> DiagnosticIssue {
    DiagnosticIssue {
        code,
        severity,
        message: message.into(),
    }
}

fn path_to_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_codex_cli_version() {
        assert_eq!(
            extract_version("codex-cli 0.144.0"),
            Some("0.144.0".to_owned())
        );
        assert_eq!(
            extract_version("codex v0.153.1-beta.2"),
            Some("0.153.1".to_owned())
        );
        assert_eq!(extract_version("unknown"), None);
    }

    #[test]
    fn compares_multi_agent_baseline() {
        assert!(parse_client_version("0.144.0").unwrap() >= MINIMUM_MULTI_AGENT_VERSION);
        assert!(parse_client_version("0.143.9").unwrap() < MINIMUM_MULTI_AGENT_VERSION);
    }

    #[test]
    fn restart_is_not_required_without_codex_processes() {
        assert!(!restart_required_for_process_start_times(
            100,
            std::iter::empty(),
        ));
    }

    #[test]
    fn restart_is_required_when_all_codex_processes_precede_sync() {
        assert!(restart_required_for_process_start_times(100, [99]));
    }

    #[test]
    fn restart_is_not_required_when_a_codex_process_started_after_sync() {
        assert!(!restart_required_for_process_start_times(100, [101]));
    }

    #[test]
    fn restart_is_not_required_when_old_and_new_codex_processes_coexist() {
        assert!(!restart_required_for_process_start_times(100, [99, 101]));
    }

    #[test]
    fn explicit_codex_home_wins() {
        let resolved = resolve_codex_home(
            Some(OsString::from(r"D:\Codex")),
            Some(OsString::from(r"C:\Users\tester")),
            None,
        );

        assert_eq!(resolved, Some(PathBuf::from(r"D:\Codex")));
    }

    #[cfg(windows)]
    #[test]
    fn identifies_codex_processes_and_converts_filetime() {
        let mut name = [0u16; 260];
        for (index, character) in "ChatGPT.exe".encode_utf16().enumerate() {
            name[index] = character;
        }
        assert!(is_codex_process_name(&name));

        let unix_epoch = FILETIME {
            dwLowDateTime: WINDOWS_TO_UNIX_EPOCH_100NS as u32,
            dwHighDateTime: (WINDOWS_TO_UNIX_EPOCH_100NS >> 32) as u32,
        };
        assert_eq!(filetime_to_unix_ms(unix_epoch), 0);
    }

    #[test]
    fn detects_only_permission_related_runtime_overrides() {
        let overrides = collect_permission_overrides([
            (
                42,
                "codex.exe".to_owned(),
                "codex exec --sandbox read-only -a never -c approval_policy=never prompt"
                    .to_owned(),
            ),
            (
                43,
                "codex.exe".to_owned(),
                "codex exec -m gpt-5.6-sol prompt".to_owned(),
            ),
            (
                44,
                "other.exe".to_owned(),
                "other.exe --sandbox danger-full-access".to_owned(),
            ),
        ]);

        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].process_id, 42);
        assert_eq!(
            overrides[0].flags,
            vec![
                "--sandbox=read-only".to_owned(),
                "-a=never".to_owned(),
                "-c=approval_policy=never".to_owned()
            ]
        );
    }
}
