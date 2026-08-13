# Codex Agent Switch (CAS)

<p align="center">
  <a href="https://img.shields.io/badge/版本-0.3.0-blue"><img src="https://img.shields.io/badge/版本-0.3.0-blue" alt="版本 0.3.0"></a>
  <a href="https://img.shields.io/badge/平台-Windows%2010%2F11-0078D6"><img src="https://img.shields.io/badge/平台-Windows%2010%2F11-0078D6" alt="平台 Windows 10/11"></a>
  <a href="https://img.shields.io/badge/License-MIT-yellow"><img src="https://img.shields.io/badge/License-MIT-yellow" alt="License MIT"></a>
  <a href="https://github.com/Zhuxi140/codex-agent-switch/actions"><img src="https://img.shields.io/github/actions/workflow/status/Zhuxi140/codex-agent-switch/ci.yml?label=CI" alt="CI"></a>
</p>

CAS 是面向 Codex CLI 的 Windows 桌面应用：用图形界面管理 Provider、Model 与 Agent 绑定，并将多 Agent 编排、原生 Thread 生命周期和 Token 用量集中到同一处。它通过官方 `codex app-server` 接口工作，无需手改 Codex TOML。

## v0.3.0 快速开始

1. 从 GitHub Release 下载 `Codex Agent Switch_0.3.0_x64-setup.exe` 并运行安装。
2. 启动应用后检查 Codex 可执行文件与 `CODEX_HOME`；Windows Store 版如无法解析命令，可将 `%USERPROFILE%\.codex\.sandbox-bin\codex.exe` 复制到 `%USERPROFILE%\.local\bin\`。
3. 在 Provider 页面选择 **Codex Native (ChatGPT)**，或添加第三方 Responses Provider；Native Provider 使用当前 Codex 登录，第三方密钥由 Windows 凭据管理器保存。
4. 在 Models 与 Agents 页面绑定模型，并在运行模式中启用编排配置；Preview 后 Apply。
5. 在用量页面同步原生子 Agent Thread，查看状态和 Token 统计。

安装包当前未进行代码签名；Windows SmartScreen 可能显示警告，请按组织安全策略核验 Release 的 SHA-256。

## 多 Agent 编排

CAS 将 Agent 分为 Primary、Discovery、Execution、Verification、Review 等 Role/Phase。编排模式下每种 Role 只能启用一个 Agent，避免职责和模型绑定发生歧义。Primary 负责读取、规划、审查和收束；所有实现命令与文件写入必须委派给 Execution Agent。

失败策略可选：

- **Strict Stop**：缺少对应 Agent、委派失败或结果不可验证时立即停止并报告，不静默接管。
- **Primary Fallback**：明确提示后允许 Primary 接管失败任务，并保留回退原因。

项目可被排除在 CAS 编排之外；项目级配置与全局配置冲突时，CAS 会检测冲突并要求查看后中止或显式重新 Apply，而不会覆盖外部修改。

## 调度与 Thread 复用

每次独立任务在委派前由 `cas-helper schedule <agent-key>` 预检，输出唯一的 `CAS1|SPAWN|...` 或 `CAS1|REUSE|...` 决策。`SPAWN` 创建新原生子 Agent Thread；`REUSE` 将完整任务交给既有 Thread。

复用是一个客观判定：**同一 Agent、同一 Primary、Exact Scope、IDLE 状态且上下文健康**。调度器还结合 Agent 的 `AUTO` / `HOT` / `COLD` 策略、Provider 的缓存能力和缓存保留提示，避免复用上下文压力过高或已超出缓存窗口的 Thread。

成功的子 Agent Thread 会保留并同步为 `IDLE`，不得自动调用 `close_agent`；仅在用户明确要求、Agent 被停用或移除、Thread 异常不可用，或 CAS 判定不再可复用时才允许关闭。

## Provider、Model 与用量

- **Codex Native**：可将当前 Codex 登录中的原生 Provider/Model 绑定为子 Agent，包括 gpt-5.6 Terra 等可用模型。
- **第三方 Provider**：支持 Responses API Provider、模型发现和能力校验；凭据不写入 Codex 配置文件。
- **原生 Thread 同步**：同步子 Agent Thread 及其生命周期状态，便于确认运行、完成和空闲状态。
- **Token 监控**：采集输入、缓存输入、输出、推理输出与总 Token；CAS 只统计 Token，不计算或展示费用，也不保存 Prompt、Response 正文或 API Key。
- **重启提示**：配置同步后，只要检测到新的 Codex 实例，红色重启提示即可自动清除。

## 配置与安全

配置采用 Preview → Apply 流程，含冲突检测、快照、回读校验和失败回滚。`cas-helper` 负责凭据交付与调度预检；Provider 密钥存入 Windows 凭据管理器，Codex 配置仅引用凭据标识。

## 开发验证

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml
npm.cmd run build
git diff --check
npm.cmd run bundle:windows
```

`bundle:windows` 会生成 NSIS x64 安装包，并携带 `cas-helper.exe`。

## 系统要求

- Windows 10/11
- Node.js 22+
- Rust（2024 edition）
- Codex CLI 0.144.0+

## License

[MIT](LICENSE) © 2026 [ZhuXi](https://github.com/Zhuxi140)
