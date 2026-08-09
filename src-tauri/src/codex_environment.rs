use std::env;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const MINIMUM_MULTI_AGENT_VERSION: ClientVersion = ClientVersion(0, 144, 0);

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
    fn explicit_codex_home_wins() {
        let resolved = resolve_codex_home(
            Some(OsString::from(r"D:\Codex")),
            Some(OsString::from(r"C:\Users\tester")),
            None,
        );

        assert_eq!(resolved, Some(PathBuf::from(r"D:\Codex")));
    }
}
