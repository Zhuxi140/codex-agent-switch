# Codex Agent Switch 产品需求文档（PRD）

> 文档类型：Product Requirements Document  
> 项目暂定名称：Codex Agent Switch  
> 简称：CAS  
> 当前目标版本：V0.1  
> 文档职责：定义产品目标、目标用户、问题范围、功能需求、用户流程、版本边界及产品验收标准。

---

# 0. V0.1 决策基线

本基线与第 67—69、82—83 节共同定义 V0.1；其他章节保留的扩展设计若与本节冲突，以本节为准。

```text
协议路线：Responses-first
首个官方 Preset：DeepSeek V4 Flash（deepseek-v4-flash）
通用入口：Custom Responses Provider
架构约束：Provider-neutral，品牌差异只存在于 Preset / Adapter 边界
正式发布平台：Windows
绑定范围：单层 Base Binding
延后到 P1：Profiles、Model Discovery、Runtime Probe、Import / Adopt
```

DeepSeek 官方于 2026-07-31 宣布 V4 Flash 原生支持 Responses API 并适配 Codex，因此它是 V0.1 旗舰子 Agent 场景。该结论**只适用于 `deepseek-v4-flash`**；V4 Pro 或其他模型必须独立取得官方证据并通过真实 PoC，不能按品牌继承兼容性。

证据来源：[DeepSeek Change Log](https://api-docs.deepseek.com/updates/)；[DeepSeek Integrate with Codex](https://api-docs.deepseek.com/quick_start/agent_integrations/codex/)。

V0.1 的价值不是“只管理 DeepSeek”，而是用 DeepSeek V4 Flash 跑通第一个真实 Preset，同时让任何满足同一 Responses contract 的 Provider 复用相同的 Provider、Model、Agent、Apply、Credential 与 Diagnostics 链路。

---

# 1. 产品概述

Codex Agent Switch 是一个面向 Codex Multi-Agent 使用场景的 Agent 模型管理工具。

CAS 的核心目标是：

> 让用户无需手动编辑 Codex 配置文件，即可为不同 Codex Agent 角色配置、切换和管理不同模型。

典型使用方式：

```text
Codex Primary Agent
        │
        ├── Executor → DeepSeek V4 Flash
        ├── Explorer → Model A
        └── Reviewer → Model B
```

用户可以在 CAS 中修改：

```text
Executor
DeepSeek V4 Flash
        ↓
Model X
```

而无需重新设计 Agent Role，也无需手动修改：

```text
config.toml
agents/*.toml
model provider configuration
model metadata
```

---

# 2. 产品背景

Codex 已具备 Multi-Agent、Custom Agent、Custom Model Provider 等能力。

理论上用户可以自行完成：

```text
注册 Provider
    ↓
配置模型
    ↓
创建 Custom Agent
    ↓
配置 model_provider
    ↓
配置 model
    ↓
维护 Agent TOML
    ↓
管理模型元数据
```

但这一过程存在较高使用门槛。

普通开发者需要理解：

```text
Codex 配置结构
Provider 配置
Agent 配置
Model Catalog
Responses API
模型兼容能力
配置文件位置
认证方式
```

当用户需要管理多个：

```text
Provider
Model
Agent
Profile
```

后，配置复杂度会快速提高。

现有 Provider 切换工具主要解决：

> 当前 Codex 使用哪个 Provider / Model。

CAS 重点解决：

> Codex 中不同 Agent Role 分别应该使用哪个 Model。

因此 CAS 不定位为单纯的 Codex Provider Switch，而是：

> Codex Agent Team Configuration Manager。

---

# 3. 产品问题

CAS 主要解决以下问题。

## 3.1 手动配置成本高

用户需要自行寻找并编辑 Codex 配置文件。

例如：

```text
~/.codex/config.toml
~/.codex/agents/*.toml
```

容易出现：

```text
字段写错
模型 ID 写错
Provider 配置错误
文件位置错误
Agent 配置不生效
```

---

## 3.2 Agent 和模型容易绑定过死

用户常见配置方式可能变成：

```text
deepseek_worker
gemini_reviewer
qwen_explorer
```

一旦更换模型，就需要：

```text
修改 Agent
修改 Prompt
修改配置
修改调用习惯
```

CAS 希望将：

```text
Agent Role
```

和：

```text
Model
```

分开管理。

例如：

```text
executor
```

始终表示执行 Agent。

它今天可以使用：

```text
DeepSeek
```

明天也可以使用：

```text
Qwen
Kimi
Gemini
其他兼容模型
```

---

## 3.3 多 Agent 配置难以整体切换

用户可能存在不同使用策略：

```text
Budget
Balanced
Quality
```

当前如果完全依赖手工配置，需要反复修改多个 Agent。

CAS 通过 Profile 提供整套配置切换能力。

---

## 3.4 用户难以判断配置是否真正可用

即使配置文件语法正确，也可能存在：

```text
Credential 缺失
Provider 不可访问
Model 不支持 Tool Calling
Model 不支持 Codex Multi-Agent
配置未 Apply
配置被外部修改
Codex 版本不兼容
```

CAS 需要向用户提供明确状态和诊断能力。

---

## 3.5 手工修改存在破坏已有配置风险

用户可能已经在 Codex 中配置：

```text
MCP
Projects
Permissions
其他 Providers
其他 Agents
TUI 配置
```

单纯覆盖：

```text
config.toml
```

可能破坏原有环境。

CAS 必须提供：

```text
非破坏式配置管理
+
备份
+
恢复
```

能力。

---

# 4. 产品目标

V0.1 的核心产品目标如下。

## 4.1 降低 Codex Subagent 多模型配置门槛

用户无需理解底层 TOML，即可完成：

```text
添加 Provider
    ↓
添加 / 选择 Model
    ↓
绑定 Agent
    ↓
Apply
```

---

## 4.2 实现 Agent 与 Model 解耦

用户主要操作对象是：

```text
Executor
Explorer
Reviewer
Custom Agent
```

而不是：

```text
DeepSeek Worker
Gemini Worker
```

允许随时替换 Agent 使用的模型。

---

## 4.3 支持多 Provider

V0.1 P0 至少支持：

```text
DeepSeek V4 Flash Preset
Custom Responses-Compatible Provider
```

系统产品设计不得把 DeepSeek 写死为唯一 Provider；DeepSeek Preset 与 Custom Responses Provider 必须经过同一领域与配置编译链路。

---

## 4.4 提供安全配置应用能力

CAS 必须能够：

```text
读取已有 Codex 配置
    ↓
识别 CAS 管理内容
    ↓
计算需要修改的内容
    ↓
备份
    ↓
应用配置
    ↓
验证
```

并尽可能避免破坏用户原有 Codex 配置。

---

## 4.5 提供可诊断能力

用户遇到：

```text
为什么 Executor 没生效？
为什么 Provider 不可用？
为什么模型不能绑定？
```

时，应优先能够通过 CAS 自身定位问题。

---

# 5. 非目标

CAS V0.1 不试图解决所有 AI Agent 问题。

以下内容不属于 V0.1 产品目标：

```text
自研 Agent Runtime
替代 Codex
自研 Coding Agent
LLM 聊天客户端
代码编辑器
IDE
Agent Workflow Engine
可视化节点编排
模型 Benchmark 平台
API 聚合计费平台
模型代理平台
云端 Agent 平台
团队协作平台
```

CAS 的基本边界始终是：

> 管理 Codex Agent 配置，而不是执行 Codex Agent。

---

# 6. 目标用户

## 6.1 Codex Multi-Agent 用户

已经使用 Codex，希望尝试：

```text
不同 Subagent 使用不同模型
```

的开发者。

---

## 6.2 多模型用户

同时拥有多个模型 API：

```text
DeepSeek
OpenRouter
其他 Responses-Compatible Provider
```

希望根据任务类型分配不同模型。

---

## 6.3 成本敏感型开发者

希望：

```text
高价值推理
→ 高能力模型

大量机械编码
→ 更低成本模型
```

从而减少主要模型消耗。

---

## 6.4 Agent Workflow 爱好者

希望构建：

```text
Architect
Executor
Explorer
Reviewer
Tester
```

这种模型异构 Agent Team。

---

# 7. 核心用户价值

CAS 提供的核心价值不是：

> 帮用户修改 TOML。

而是：

> 将 Codex Multi-Agent 的模型配置变成一个可管理的产品能力。

用户最终关注的是：

```text
我的 Executor 用什么模型？

我的 Reviewer 用什么模型？

当前整个 Agent Team 是什么配置？

配置是否已经生效？

哪个模型现在不可用？
```

而不是底层配置字段。

---

# 8. 核心产品对象

用户直接理解和操作以下四种核心对象：

```text
Agent
Model
Provider
Profile
```

其产品含义如下。

## Agent

回答：

> 谁负责做这件事？

例如：

```text
Executor
Explorer
Reviewer
Tester
```

---

## Model

回答：

> Agent 实际使用哪个模型？

---

## Provider

回答：

> 这个模型通过哪个服务调用？

---

## Profile

回答：

> 当前整套 Agent Team 应该使用怎样的模型组合？

---

# 9. V0.1 核心场景

V0.1 必须优先满足以下真实场景。

---

# 10. 场景一：给 Executor 配置 DeepSeek

用户已经使用 Codex。

用户希望：

```text
Primary Agent
继续使用 Codex 官方模型

Executor
改为 DeepSeek V4 Flash
```

目标流程：

```text
安装 CAS

↓

CAS 检测 Codex

↓

添加 DeepSeek

↓

输入 API Key

↓

选择 DeepSeek V4 Flash

↓

绑定 Executor

↓

Apply

↓

Codex 可以 Spawn Executor
```

这是 V0.1 最重要的验证场景。

---

# 11. 场景二：更换 Executor 模型

用户当前：

```text
Executor
→ DeepSeek V4 Flash
```

后来增加：

```text
Model X
```

用户只需要：

```text
Executor

Model:
DeepSeek V4 Flash
        ↓
Model X

Save
Apply
```

用户不应该：

```text
重新创建 Agent
重新修改 Instructions
重新修改 AGENTS.md
```

---

# 12. 场景三：添加自定义 Provider

用户拥有一个兼容 Responses API 的第三方模型服务。

需要支持：

```text
Add Provider
    ↓
Custom
    ↓
Name
Base URL
Credential
    ↓
Model
    ↓
Agent Binding
```

CAS 不应要求该 Provider 必须属于内置品牌列表。

---

# 13. 场景四：切换整套 Agent Team（P1）

用户已经配置：

```text
Balanced

Executor → DeepSeek
Explorer → Model A
Reviewer → Model B
```

同时存在：

```text
Quality
```

点击：

```text
Activate Quality
```

然后：

```text
Apply
```

即可整体调整 Agent Team。

---

# 14. 场景五：诊断错误

例如：

```text
Executor
→ DeepSeek V4 Flash
```

但 API Key 已失效。

用户打开 Diagnostics：

```text
DeepSeek Credential
✕ Authentication failed

Executor
✕ Provider unavailable
```

用户能够明确知道问题来源。

---

# 15. 场景六：用户已有 Codex 配置

用户在安装 CAS 前已经有：

```text
MCP Server
Projects
Custom Provider
Custom Agent
```

CAS 首次运行必须：

```text
检测
读取
展示
保留
```

不能因为 CAS Apply 就把这些配置删除。

---

# 16. V0.1 功能范围

V0.1 功能划分为：

```text
Codex Environment
Provider Management
Model Management
Agent Management
Configuration Apply
Credential Management
Diagnostics
Backup & Restore
Basic Settings
```

---

# 17. Codex Environment

CAS 启动后必须自动检测：

```text
Codex 是否安装
Codex Home
Codex 配置是否存在
Codex 版本
配置是否可访问
```

DeepSeek 官方 V4 Flash Catalog 当前声明 `minimal_client_version = 0.144.0`。CAS 不仅比较版本号，还必须执行所需字段 / 行为的 Feature Probe；不满足时只阻断受影响的 DeepSeek Apply，并给出升级指引。

用户不得必须手工填写路径才能进入应用。

如果无法检测，允许：

```text
Manual configuration
```

作为兜底。

---

# 18. Codex 状态

CAS 至少需要向用户表达：

```text
Detected
Not detected
Unsupported
Configuration unavailable
Ready
```

当 Codex 不可用时，CAS 仍可启动，但与 Codex 配置相关的 Apply 功能必须受到限制。

---

# 19. Provider Management

用户必须可以：

```text
添加 Provider
查看 Provider
编辑 Provider
启用 Provider
禁用 Provider
删除 Provider
测试 Provider
```

V0.1 Provider 类型至少包括：

```text
DeepSeek V4 Flash Preset
Custom Responses Provider
```

---

# 20. Provider Preset

对于已知 Provider，CAS 应提供预配置模板。

V0.1 首个实例：

```text
DeepSeek V4 Flash
```

用户不需要自己填写所有默认技术参数。

通常只需要：

```text
Credential
```

及必要的少量配置。

---

# 21. Custom Provider

用户必须可以创建未被 CAS 内置识别的 Provider。

V0.1 的 Custom Provider 重点支持：

```text
Responses-Compatible
```

Provider。

产品必须明确告诉用户：

> 自定义 Provider 是否已经验证为 Codex Compatible。

---

# 22. Provider Credential

用户可以：

```text
添加 Credential
替换 Credential
删除 Credential
```

CAS 不应要求用户自己维护环境变量作为正常使用流程。

---

# 23. Provider 测试

CAS 提供：

```text
Test Connection
```

至少判断：

```text
Provider 是否可访问
认证是否有效
```

模型发现不属于 V0.1 P0；Provider 支持时可以在 P1 的独立 Discovery 操作中返回候选模型。

---

# 24. Provider 禁用

用户可以暂时：

```text
Disable Provider
```

禁用后：

```text
Provider 保留
Credential 保留
Models 保留
Bindings 保留
```

但依赖该 Provider 的 Agent 显示不可用。

---

# 25. Provider 删除

如果 Provider 当前仍被使用，CAS 不允许静默删除。

例如：

```text
DeepSeek

used by:
Executor
Balanced Profile
```

必须要求用户先解除相关引用。

---

# 26. Model Management

用户必须可以：

```text
查看模型
添加模型
启用模型
禁用模型
查看兼容性
查看主要能力
```

模型可以来源于：

```text
Provider Preset
Provider Discovery
Manual Input
Model Catalog
```

---

# 27. Model Discovery（P1）

如果 Provider 支持模型发现：

```text
Discover Models
```

CAS 可以向用户展示候选模型。

发现结果不得在未经规则允许时自动绑定任何 Agent。

---

# 28. 手动添加 Model

Custom Provider 必须允许：

```text
手动填写 Model ID
```

因为不是所有 Provider 都提供标准模型列表接口。

---

# 29. Model Compatibility

CAS 至少需要表达：

```text
Native
Compatible
Gateway Required
Unsupported
Unknown
```

V0.1 不要求 CAS 自动证明所有模型可用。

如果无法判断：

```text
Unknown
```

比错误标记为 Compatible 更合理。

---

# 30. Model Capability

用户至少可以看到与 Codex Agent 使用相关的关键能力：

```text
Responses
Tool Calling
Parallel Tool Calling
Reasoning
Multi-Agent
Context Window
```

不要求 V0.1 成为完整模型参数数据库。

---

# 31. Agent Management

用户必须可以：

```text
查看 Agent
创建 Agent
编辑 Agent
启用 Agent
禁用 Agent
删除 Agent
绑定 Model
修改 Instructions
配置 Reasoning
配置 Sandbox
```

---

# 32. 默认 Agent 模板

V0.1 推荐至少内置：

```text
Executor
Explorer
Reviewer
Tester
```

其中首次启动可以重点推荐：

```text
Executor
```

但不得自动修改 Codex 行为。

---

# 33. Executor

Executor 的产品定位：

> 负责在主要技术方向已经明确后进行代码实现、修改、测试和实现级修复。

CAS 不规定 Executor 一定使用哪一个模型。

---

# 34. Explorer

Explorer 的产品定位：

> 负责代码库探索、信息检索和上下文收集。

---

# 35. Reviewer

Reviewer 的产品定位：

> 负责对实现结果进行独立检查和问题发现。

---

# 36. Tester

Tester 的产品定位：

> 负责测试相关任务和实现验证。

V0.1 不要求用户必须同时启用所有这些 Agent。

---

# 37. Custom Agent

用户可以创建：

```text
database_expert
java_worker
frontend_worker
security_reviewer
```

等自定义 Agent。

自定义 Agent 不得与 Provider 绑定为固定概念。

---

# 38. Agent Model Binding

每个启用 Agent 在 V0.1 最多绑定：

```text
一个 Active Model
```

即：

```text
Executor
→ DeepSeek V4 Flash
```

V0.1 不实现：

```text
自动模型路由
Fallback Model
按请求动态选模型
加权负载均衡
```

---

# 39. Agent Compatibility Validation

绑定模型时 CAS 必须检查已知兼容性。

至少区分：

```text
Compatible
Warning
Incompatible
Unknown
```

完全不兼容时：

```text
不允许正常 Apply
```

如果只是：

```text
Unknown
```

可以允许高级用户继续，但必须明确提示风险。

---

# 40. Agent Instructions

用户允许编辑 Agent Instructions。

同时 CAS 应提供：

```text
Reset to Template
```

避免修改后无法恢复。

V0.1 不实现：

```text
Prompt Version Control
Prompt Marketplace
AI Prompt Optimizer
```

---

# 41. Profile Management（P1）

本节及第 42—44 节保留 Profile 产品语义，但不进入 V0.1 实现、导航、IPC 与 Schema。

用户必须可以：

```text
创建 Profile
查看 Profile
修改 Profile
复制 Profile
激活 Profile
删除 Profile
```

---

# 42. Profile 内容

Profile 主要保存：

```text
Agent
→
Model
```

组合。

例如：

```text
Balanced

Executor
→ DeepSeek V4 Flash

Explorer
→ Model A

Reviewer
→ Model B
```

---

# 43. 默认 Profile（P1）

Profile 功能进入 P1 后按需 Lazy Create：

```text
Default
```

Profile。

如果用户只有：

```text
Executor
```

同样可以使用 Default Profile。

---

# 44. Profile 激活（P1）

激活 Profile 只改变：

```text
CAS 当前目标配置
```

不代表已经写入 Codex。

必须经过：

```text
Apply
```

后才表示 Codex 配置更新。

---

# 45. Configuration State

CAS 产品层必须明确区分：

```text
Draft
Saved
Applied
```

三种概念。

## Draft

页面中的编辑尚未保存。

## Saved

CAS 已经保存业务状态，但尚未同步到 Codex。

## Applied

CAS 当前目标状态已经成功同步到 Codex。

---

# 46. Pending Changes

当：

```text
CAS State
≠
Applied Codex State
```

时，用户必须看到：

```text
Changes Pending
```

不能让用户误以为修改已经生效。

---

# 47. Apply

Apply 是 V0.1 的核心操作。

用户触发 Apply 后，CAS 负责尝试：

```text
将当前 CAS 状态同步到 Codex
```

Apply 结果必须明确为：

```text
Success
Failed
Partially Recovered
Conflict
```

具体配置写入实现由其他文档定义。

---

# 48. Apply Review

用户在 Apply 前应能够查看：

```text
将新增什么
将修改什么
将删除什么 CAS-owned 配置
```

普通用户看到逻辑变化。

高级用户可以查看更底层变化。

---

# 49. Apply 冲突

当 CAS 发现：

```text
配置在外部发生修改
```

不能自动静默覆盖。

用户至少应被告知：

```text
External changes detected.
```

并中止或进入显式冲突处理流程。

---

# 50. Backup

CAS 在修改 Codex 配置前必须支持自动 Backup。

V0.1 默认建议：

```text
Auto Backup = Enabled
```

用户可以查看历史备份。

---

# 51. Restore

用户必须可以将 Codex 配置恢复到此前的 CAS Backup。

Restore 本身也应该先保护当前状态。

用户必须明确触发 Restore。

---

# 52. Diagnostics

V0.1 必须提供：

```text
Run Diagnostics
```

至少检查：

```text
Codex Environment
Codex Version
Configuration Access
Configuration Validity
Provider Credential
Provider Connectivity
Model Compatibility
Agent Binding
Agent Configuration
CAS Managed Resources
Pending Configuration
```

---

# 53. Diagnostics 原则

Diagnostics：

```text
默认只检查
```

不能：

```text
打开页面就自动修改配置
```

如果可以修复：

```text
Repair
```

必须独立、显式触发。

---

# 54. Error Guidance

产品错误不能只提供：

```text
Error 500
Invalid configuration
```

应尽可能说明：

```text
问题是什么
影响什么
用户下一步可以做什么
```

例如：

```text
DeepSeek authentication failed.

Executor cannot currently use DeepSeek V4 Flash.

Replace the configured credential and try again.
```

---

# 55. External Codex Resource

CAS 首次检测到已有：

```text
Custom Agent
Custom Provider
```

时，不得默认认定为 CAS 创建。

必须标识：

```text
External
```

用户可以：

```text
查看
忽略
选择纳入 CAS 管理
```

V0.1 可以限制 Import 能力，但不能自动接管。

---

# 56. Import

如果 V0.1 实现已有资源导入，则要求：

```text
用户显式触发
```

导入之后才允许 CAS 对该资源进行管理。

如果 V0.1 不完整实现 Import，则至少：

```text
正确识别
不覆盖
```

已有外部资源。

---

# 57. CLI

V0.1 应提供基础 CLI。

CLI 与 Desktop 的产品能力共享同一个 CAS 状态。

最低命令集建议：

```text
cas status
cas doctor
cas apply
```

Profile CLI 随 P1 Profile Management 一并引入，V0.1 不注册占位命令或别名。

Provider / Agent 完整编辑可以首先由 Desktop 承担。

V0.1 不要求所有 GUI 操作都有完整 CLI 对应。

---

# 58. `cas status`

至少展示：

```text
Codex 状态
Agent → Model
Pending Changes
Provider 可用性摘要
```

目标：

> 用户在终端里几秒看懂当前 Agent Team。

---

# 59. `cas doctor`

执行诊断。

退出状态应能让自动化脚本判断：

```text
健康
存在 Warning
存在 Error
```

具体退出码规范由 CLI 文档定义。

---

# 60. `cas apply`

将当前保存的 CAS 配置应用到 Codex。

必须遵守与 Desktop Apply 相同的：

```text
校验
Backup
Conflict Detection
Apply
```

规则。

---

# 61. Credential Helper

为了使 Codex 可以使用 CAS 安全保存的 Secret，V0.1 可以附带：

```text
cas-helper
```

用户不直接操作它。

它属于产品安装内容的一部分。

用户体验目标：

```text
用户在 CAS 输入一次 API Key
        ↓
后续 Codex 正常使用
```

而不是要求用户另外手工执行系统 Credential 命令。

---

# 62. Settings

V0.1 设置仅保留真正必要的产品配置：

```text
Codex Location
CAS Data Location
Auto Backup
Appearance
Update Channel
Advanced Operations
```

不把核心业务对象塞进 Settings。

---

# 63. First Run

第一次启动主要解决：

```text
CAS 能否找到 Codex？
```

以及：

```text
用户下一步做什么？
```

基本流程：

```text
启动
  ↓
检测 Codex
  ↓
检测成功
  ↓
建议 Add Provider
```

用户允许：

```text
Skip
```

进入应用。

---

# 64. 首次 Provider 配置

最重要的首次路径：

```text
Add Provider
    ↓
DeepSeek V4 Flash
    ↓
Credential
    ↓
Test Connection
    ↓
Save
```

保存成功后，可以建议：

```text
Assign to Executor
```

减少用户寻找下一步。

---

# 65. 产品成功路径

V0.1 发布后最重要的用户故事是：

> 用户安装 CAS，在不知道 Codex Subagent TOML 如何配置的情况下，能够在几分钟内把 DeepSeek V4 Flash 设置为 Executor 的模型，并成功让 Codex 主 Agent 调用它；随后也能用相同流程换成另一个已验证的 Responses Provider / Model，而无需重建 Executor。

只要这一条不稳定：

```text
V0.1 就不应该视为完成。
```

---

# 66. 用户故事

## US-001

作为 Codex 用户，

我希望 CAS 自动找到本机 Codex，

从而无需手工寻找配置目录。

验收：

```text
正常安装 Codex 的情况下
CAS 可以自动识别环境。
```

---

## US-002

作为用户，

我希望添加 DeepSeek V4 Flash Provider Preset，

从而可以在 Codex Subagent 中使用 `deepseek-v4-flash`。

验收：

```text
可以配置 Credential
可以测试连接
可以保存 Provider
可以看到 Provider 状态
```

---

## US-003

作为高级用户，

我希望添加自定义 Responses Provider，

从而不被 CAS 内置 Provider 列表限制。

验收：

```text
可填写自定义 Provider
可添加 Model ID
可进行兼容性检查
```

---

## US-004

作为用户，

我希望为 Executor 选择模型，

从而让具体编码工作使用我选择的模型。

验收：

```text
Agent Detail 可以选择已启用模型
保存后产生 Pending Changes
Apply 后配置生效
```

---

## US-005

作为用户，

我希望以后替换 Executor 模型时不需要重新创建 Agent，

从而能够快速尝试不同模型。

验收：

```text
更换绑定 Model 后
Agent key
Description
Instructions
保持不变
```

---

## US-006

> P1 用户故事，不属于 V0.1 Definition of Done。

作为用户，

我希望建立多个 Profile，

从而快速切换不同成本和质量策略。

验收：

```text
可以建立至少两个 Profile
激活不同 Profile
Apply 后使用新的 Agent → Model 组合
```

---

## US-007

作为用户，

我希望 CAS 不破坏自己原来的 Codex 配置。

验收：

```text
CAS Apply 后
非 CAS 管理的配置仍然存在
且内容未被无关修改
```

---

## US-008

作为用户，

我希望配置修改前自动备份，

从而在出现问题时可以恢复。

验收：

```text
Apply 前生成有效 Snapshot
用户可执行 Restore
```

---

## US-009

作为用户，

我希望知道为什么某个 Agent 不可用。

验收：

```text
CAS 能定位到至少以下原因：

Provider disabled
Credential missing
Provider unavailable
Model incompatible
Binding missing
Configuration invalid
```

---

## US-010

作为用户，

我希望 CAS 不在普通数据库中保存我的 API Key 明文，

降低凭据泄露风险。

验收：

```text
普通 CAS 数据库和业务配置中
不存在完整 Secret 明文。
```

---

# 67. V0.1 P0 功能

进入完整功能开发前，必须先通过以下 Gate 0；任一项失败都不得用 Mock 或单独的 Provider Ping 宣称核心链路可行：

```text
PoC-1：真实 Codex Primary Agent 能 spawn 使用 DeepSeek V4 Flash 的 Executor，且 per-agent model/provider 生效
PoC-2：auth.command + Windows Credential Manager 可端到端取用 DeepSeek API Key，配置文件不出现 Secret 明文
PoC-3：config.toml / models.json / agents/*.toml 能非破坏式 patch、检测并发外部修改、失败回滚与 Restore
PoC-4：Windows 安装与升级后 cas-helper 绝对路径保持稳定，旧配置继续可调用
```

Gate 0 通过后必须实现：

```text
Codex 自动检测

DeepSeek V4 Flash Provider Preset

Custom Responses Provider

Credential 安全保存

Provider Test Connection

Model 管理

Model Compatibility 基础判断

Executor / Explorer / Reviewer / Tester 模板

Custom Agent

Agent → Model Binding

Pending Changes

Review Changes

Apply

非破坏式 Codex 配置修改

Backup

Restore

Diagnostics

External Resource Preserve

Desktop GUI

基础 CLI
```

缺少其中影响核心链路的能力，不建议发布正式 V0.1。

---

# 68. V0.1 P1 功能

不阻塞 V0.1 发布，后续按验证结果进入 P1：

```text
Provider Model Discovery

Profile Management / Profile Activation

Agent Duplicate

Profile Duplicate

Model Capability Probe

Existing Agent Import

Existing Provider Import

Developer Details

Configuration Diff

Automatic Update
```

这些功能缺失不影响最核心产品闭环。

---

# 69. V0.1 明确不做

以下能力推迟：

```text
Chat Completions → Responses Gateway

Anthropic Messages Gateway

Gemini Protocol Adapter

自动模型 Routing

Fallback Model

Load Balancing

按 Token 成本自动切模型

模型 Benchmark

调用量统计

Provider Billing

Agent 执行历史

Agent 实时运行监控

Agent 对话记录

远程 CAS Sync

账号登录

团队功能

云端 Profile

Marketplace

插件体系

复杂 Provider Script

Workflow Graph

Agent Pipeline Designer

DeepSeek V4 Pro Direct Preset（在官方 Codex 支持与真实 PoC 完成前）
```

---

# 70. 后续版本方向

## V0.2

重点：

```text
Profile 增强
Provider Preset 增强
Import 增强
CLI 增强
模型 Catalog 更新机制
```

---

## V0.3

重点：

```text
更多 Responses-Compatible Provider
Provider Preset Registry
更多 Agent Template
兼容性检测增强
```

---

## V0.4+

重点评估：

```text
CAS Local Gateway

Chat Completions
Anthropic Messages
其他协议
        ↓
Responses
        ↓
Codex
```

从而扩大模型兼容范围。

---

# 71. 产品安全要求

V0.1 必须满足以下产品级安全要求：

```text
Secret 不明文持久化到普通业务存储

Secret 默认不回显

Apply 前自动备份

外部修改不得静默覆盖

非 CAS-owned 配置不得无条件修改

危险操作需要显式确认

诊断默认不得修改配置
```

具体技术实现由安全与配置规范定义。

---

# 72. 可靠性要求

核心配置操作必须优先考虑：

```text
可验证
可恢复
可诊断
```

如果 CAS 无法确认某次 Apply 是否成功：

```text
不得显示 Applied。
```

应显示：

```text
Unknown / Needs attention
```

并引导诊断。

---

# 73. 兼容性要求

CAS 必须假设：

```text
Codex 会更新
Provider 会更新
Model 会更新
```

因此产品不得向用户承诺：

> 所有 OpenAI-Compatible Model 都一定可以作为 Codex Subagent。

CAS 应明确区分：

```text
Native
Compatible
Unknown
Unsupported
```

---

# 74. 易用性要求

核心路径：

```text
Add Provider
→ Assign Model
→ Apply
```

不应该要求用户：

```text
手工编辑 TOML
手工设置系统环境变量
手工复制 Model Catalog
手工创建 Agent 文件
```

高级用户仍可以查看底层信息，但不应成为正常使用前提。

---

# 75. 可移植性目标

V0.1 正式发布与质量承诺平台：

```text
Windows
```

架构继续保留：

```text
macOS / Linux 平台抽象
```

但它们不进入 V0.1 安装、升级、Secret Store 与 Definition of Done。

但产品模型和功能定义不得写死 Windows 专用概念。

---

# 76. 离线能力

CAS 本身的：

```text
Agent 编辑
查看本地状态
查看历史 Backup
```

应尽可能不依赖 CAS 云服务。

CAS V0.1 不要求拥有任何官方后端服务。

只有：

```text
Provider Test
Model Discovery
Update Check
```

等外部能力需要网络。

---

# 77. 无 CAS 运行依赖

完成 Apply 后：

```text
CAS 退出
```

不能导致：

```text
Codex 无法正常运行
```

除非用户明确使用了依赖 `cas-helper` 的 Credential 功能。

即使存在 Helper，它也应该是：

```text
独立轻量运行组件
```

而不是要求 CAS Desktop 常驻。

---

# 78. 可分享性要求

CAS 的目标是：

```text
别人安装后直接使用
```

而不是：

```text
作者自己的个人配置脚本
```

因此 V0.1 发布必须具备：

```text
安装包
初次启动流程
默认 Provider Preset
默认 Agent Templates
错误提示
Diagnostics
配置恢复
基础文档
```

不允许依赖用户：

```text
修改源代码
自行改 TOML
手动构建程序
```

才能完成核心使用路径。

---

# 79. 用户配置导出

V0.1 可以支持导出：

```text
CAS Configuration
```

但导出结果不得默认包含：

```text
Secret
```

目标是未来能够分享：

```text
Profile
Agent Templates
Provider Definitions
```

而不用分享 API Key。

如果 V0.1 时间不足，可以作为 P1。

---

# 80. 产品状态定义

全局状态必须至少区分：

```text
Ready

Changes Pending

Applying

Applied

Warning

Error

External Change Detected
```

不能将：

```text
保存成功
```

等价于：

```text
Codex 已经生效。
```

---

# 81. 核心产品指标

V0.1 不需要复杂埋点系统。

开发阶段主要使用可人工验证的成功指标。

### 指标一

新用户可以从：

```text
空配置
```

完成：

```text
DeepSeek → Executor
```

全过程。

---

### 指标二

模型切换不要求重新创建 Agent。

---

### 指标三

CAS Apply 不破坏预先存在的非 CAS Codex 配置。

---

### 指标四

配置失败后存在有效恢复路径。

---

### 指标五

常见故障可以通过 Diagnostics 定位。

---

### 指标六

用户核心流程无需直接编辑 Codex TOML。

---

# 82. V0.1 发布验收场景

正式发布前至少通过以下端到端场景。

---

## Scenario A：全新安装

前置：

```text
Codex 已安装
CAS 未配置
```

操作：

```text
启动 CAS
添加 DeepSeek
配置 Credential
创建 / 启用 Executor
绑定 deepseek-v4-flash
Apply
启动 Codex
调用 Executor
```

预期：

```text
Executor 正常使用配置模型。
```

---

## Scenario B：切换模型

前置：

```text
Executor → Model A
Model B 来自另一个已验证的 Custom Responses Provider
```

操作：

```text
改为 Model B
Apply
```

预期：

```text
Executor Role 保留
只有目标模型 / Provider 发生变化，Executor Role 与 Instructions 保持不变。
```

---

## Scenario C：已有用户配置

前置：

```text
config.toml 已包含：
MCP
Projects
Custom Settings
```

操作：

```text
CAS Apply
```

预期：

```text
已有非 CAS 配置完整保留。
```

---

## Scenario D：Provider Credential 失效

前置：

```text
Executor → DeepSeek
错误 Credential
```

操作：

```text
Run Diagnostics
```

预期：

```text
明确显示 Provider Authentication Failure
以及 Executor 受到影响。
```

---

## Scenario E：外部配置发生改变

前置：

```text
CAS 已读取配置
之后用户手动修改 Codex Config
```

操作：

```text
CAS Apply
```

预期：

```text
检测外部变化
不静默覆盖。
```

---

## Scenario F：Apply 失败

制造：

```text
文件不可写
或其他配置写入失败
```

预期：

```text
用户获得明确错误
不存在错误 Applied 状态
可以恢复或保持原配置。
```

---

## Scenario G：Restore

操作：

```text
Apply 新配置
Restore 上一个 Snapshot
```

预期：

```text
Codex 配置恢复到快照状态。
```

---

# 83. V0.1 Definition of Done

只有同时满足以下条件，V0.1 才视为完成：

```text
1. 用户能够正常安装 CAS。

2. CAS 可以检测本机 Codex。

3. 用户可以通过 Preset 添加 DeepSeek V4 Flash Provider。

4. 用户可以添加 Custom Responses Provider。

5. Credential 可以安全保存。

6. 用户可以管理 Model。

7. 用户可以创建和管理 Agent。

8. 用户可以自由修改 Agent → Model Binding。

9. V0.1 使用 Base Binding 生成目标配置，不依赖 Profile。

10. 用户修改后可以明确看到 Pending Changes。

11. Apply 能正确生成 Codex 所需配置。

12. Apply 不破坏非 CAS-owned 配置。

13. Apply 前存在有效 Backup。

14. 用户可以 Restore。

15. Diagnostics 可以发现核心配置问题。

16. 外部配置修改不会被 CAS 静默覆盖。

17. 用户不需要手动编辑 TOML 完成核心流程。

18. DeepSeek V4 Flash Executor 端到端场景真实跑通，且 Provider 特定数据未进入核心领域类型。

19. 同一 Executor 可真实切换到独立配置的 Custom Responses Provider / Model，无需增加品牌专用代码。

20. Desktop 关闭后 Codex 仍可正常使用生成配置。

21. 项目可以被第三方用户安装和使用，而不是只能在开发机器运行。
```

---

# 84. 产品原则

整个 CAS 产品开发过程中保持以下原则。

### 原则一

```text
Agent First。
```

用户配置的是 Agent Team，不是堆积 Provider 参数。

---

### 原则二

```text
Role 与 Model 分离。
```

切模型不应导致 Agent 身份改变。

---

### 原则三

```text
默认简单，高级能力渐进暴露。
```

---

### 原则四

```text
配置安全优先于自动化便利。
```

CAS 宁可停止 Apply，也不要在无法判断时覆盖用户配置。

---

### 原则五

```text
未知必须明确为 Unknown。
```

不能为了界面好看虚构兼容性。

---

### 原则六

```text
CAS 不替代 Codex。
```

Codex 负责运行 Agent，CAS 负责管理 Agent 配置。

---

### 原则七

```text
本地优先。
```

V0.1 不依赖 CAS 云服务即可完成核心功能。

---

### 原则八

```text
可恢复。
```

所有高影响配置操作都必须考虑恢复路径。

---

### 原则九

```text
可分享。
```

最终产品必须服务普通第三方用户，而不是只服务项目作者。

---

### 原则十

```text
V0.1 保持聚焦。
```

不为了“以后也许需要”提前把项目扩展成：

```text
IDE
LLM Gateway Platform
Agent Runtime
Workflow Builder
模型交易平台
```

---

# 85. 产品最终形态

CAS 的最终核心体验应保持简单：

```text
                  Codex
                    │
             Primary Agent
                    │
           ┌────────┼────────┐
           ▼        ▼        ▼
       Executor  Explorer  Reviewer
           │        │        │
           ▼        ▼        ▼
        Model A   Model B   Model C
           │        │        │
           ▼        ▼        ▼
       Provider  Provider  Provider
```

用户通过 CAS 管理：

```text
谁负责什么
        ↓
使用哪个模型
        ↓
模型来自哪里
```

而无需直接处理底层 Codex 配置。

V0.1 的产品成功标准并不是支持最多模型、最多 Provider 或最多功能。

而是把下面这件事做到稳定：

> 用户安装 CAS 后，可以安全、直观地管理 Codex Subagent 使用的模型，并且能够随时替换模型而不破坏 Agent Role 和已有 Codex 环境。
