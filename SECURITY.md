# 安全政策（Security Policy）

## 支持的版本（Supported Versions）

当前仅维护最新发布的版本（见 [Releases](https://github.com/Zhuxi140/codex-agent-switch/releases)）。
旧版本不提供安全修复，建议始终升级到最新版。

## 报告漏洞（Reporting a Vulnerability）

请**不要**通过公开 Issue 报告安全问题（尤其是凭证、配置写入、代码执行相关漏洞）。

安全报告渠道：

- **首选**：发送邮件至 `xiaofei6626@126.com`，主题以 `[SECURITY]` 开头
- 或通过 GitHub Security Advisory 的「Report a vulnerability」入口（仓库 → Security → Report a vulnerability）

请在报告中包含：

1. 影响版本与复现步骤（尽量最小化）
2. 漏洞类型与潜在影响（如：配置篡改、凭据泄露、任意命令执行、注入）
3. 可选的 PoC 与修复建议

## 处理流程

1. 维护者在 72 小时内确认收到报告
2. 确认漏洞后优先修复，并在修复版本发布后公开披露（约 30 天内）
3. 修复发布前不公开细节，避免被利用

## 本项目关注的高风险区域

- 凭据链：`cas-helper`、`auth.command` 配置、Windows 凭据管理器交互
- 配置应用：对 Codex `config.toml` / `agents/*.toml` 的写入与回滚
- Runtime Bridge：`codex app-server` 子进程、JSON-RPC 事件解析、托管 Agent 执行
- 诊断服务：PowerShell 进程枚举等本机命令调用

## 安全承诺

- 凭据明文永不写入 Codex 配置或仓库文件
- Token 监控不保存任何 Prompt、Response、正文内容
- 有意引入破坏性行为（如删除、覆盖外部文件）前必须显式确认
