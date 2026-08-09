# ADR 0014: V0.1 采用 Responses-first、Provider-neutral、Windows-first 与 PoC 门禁

## 状态

Accepted

## 决策

- V0.1 的 Direct Apply 链路只支持已通过真实端到端验证、原生提供 Codex `responses` wire protocol 的 Provider。
- `deepseek-v4-flash` 是 V0.1 首个官方 Direct Provider Preset 和旗舰验收路径；Custom Responses Provider 是同级 P0 通用入口。Provider、Model、Agent、配置编译器与 IPC 均不得依赖 DeepSeek 品牌类型。
- DeepSeek 官方当前只确认 `deepseek-v4-flash` 支持 Codex Responses；`deepseek-v4-pro` 不继承该结论，保持 `UNKNOWN / NOT_READY`，直到官方文档与真实 PoC 均通过。CAS V0.1 不内置 Chat Completions → Responses Gateway。
- V0.1 正式发布平台为 Windows。macOS / Linux 保留平台抽象与未来设计，但不计入 V0.1 Definition of Done。
- Profiles、Provider Model Discovery、Model Runtime Probe、Import / Adopt 和复杂 Capability Evidence 历史移入 P1；V0.1 使用单层 Base Binding。
- 完整产品开发前必须通过四个可复现 PoC Gate：真实 Subagent 的 per-agent provider/model；`auth.command` + Windows Credential Manager；保留外部配置的语义 patch / conflict / restore；安装与升级后稳定的 `cas-helper` 绝对路径。

## 背景

Codex Custom Provider 当前要求 `wire_api = "responses"`。DeepSeek 于 2026-07-31 正式宣布 V4 Flash 原生支持 Responses API 并适配 Codex，官方配置使用 `base_url = "https://api.deepseek.com/"`、`wire_api = "responses"` 和独立 Model Catalog。该能力使 DeepSeek V4 Flash 可以成为真实首发场景，但不能成为核心架构中的品牌特例。同时，三平台 Secret Store、安装器和升级路径会显著扩大首版验证面。

证据来源：[DeepSeek Change Log](https://api-docs.deepseek.com/updates/)、[DeepSeek Codex Integration](https://api-docs.deepseek.com/quick_start/agent_integrations/codex/)、[Codex Config Reference](https://learn.chatgpt.com/docs/config-file/config-reference)、[Codex Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)。

## 影响

- 品牌 Provider 只有在 Responses contract 与真实 Subagent 场景均通过验证后，才可提升为 Direct Preset；DeepSeek V4 Flash 是第一例，不代表 V4 Pro 或其他 DeepSeek 模型自动通过。
- 非 Responses Provider 未来可由外部/独立 Gateway 接入；Gateway 自身的部署、鉴权、可靠性与协议转换不属于 V0.1。
- ADR 0001 / 0011 的 Profile 语义仍有效，但从 V0.1 实现计划延后到 P1；ADR 0013 的 `activeProfile` 与 Profile CLI 命令同样延后。
- 未通过任一 PoC Gate 时，不得以 Mock、静态配置生成或单独的 Provider Ping 宣称 V0.1 核心链路可行。
