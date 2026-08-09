# Codex Agent Switch Provider / Model 接入规范

> 文档类型：Provider / Model Integration Specification  
> 项目暂定名称：Codex Agent Switch  
> 简称：CAS  
> 当前目标版本：V0.1  
> 文档职责：定义第三方 Provider 与 Model 如何接入 CAS，包括 Provider 类型、Preset、协议要求、模型发现、模型能力声明、兼容性验证、Catalog 来源、Adapter 边界及新增 Provider / Model 的标准流程。

---

# 0. V0.1 接入基线

V0.1 采用 Responses-first、Provider-neutral：DeepSeek V4 Flash 是首个官方 Direct Preset，Custom Responses Provider 是同级通用入口。两者复用相同 `GenericResponsesAdapter`、领域实体、IPC 与 Apply 链路；品牌信息只能存在于只读 Preset / Model Definition。

DeepSeek 官方于 2026-07-31 明确 `deepseek-v4-flash` 原生支持 Responses API 并适配 Codex。该证据不自动覆盖 `deepseek-v4-pro`；在官方 Codex 支持与真实 PoC 通过前，V4 Pro 保持 `UNKNOWN / NOT_READY` 且不可绑定。Discovery、Runtime Probe 与 Gateway 实现均不属于 V0.1 P0。

官方依据：

- <https://api-docs.deepseek.com/updates/>
- <https://api-docs.deepseek.com/quick_start/agent_integrations/codex/>

---

# 1. 文档目标

CAS 必须能够持续接入新的：

```text
Provider
Model
```

而不要求修改：

```text
Agent Core
Profile Core
Configuration Core
UI Core
```

本规范主要回答：

```text
一个 Provider 怎样接入 CAS？

一个 Model 怎样接入 CAS？

什么 Provider 可以直接连接 Codex？

什么 Provider 需要 Adapter / Gateway？

怎样判断 Model 是否适合作为 Codex Subagent？

Provider Preset 应包含什么？

Model Definition 应包含什么？

模型能力信息从哪里获取？

官方声明、自动探测和用户覆盖冲突时相信谁？

怎样新增 Provider 而不污染核心代码？

怎样处理 Provider API 能连接但模型并不兼容 Codex 的情况？
```

---

# 2. 核心接入关系

CAS 必须保持：

```text
Provider
   │
   ├── Connection
   ├── Authentication
   ├── Protocol
   └── Provider Capabilities
          │
          ▼
        Model
          │
          ├── Model Identity
          ├── Model Capabilities
          ├── Model Metadata
          └── Codex Compatibility
```

其中：

```text
Provider 可用
≠
Model 可用
```

同时：

```text
Model API 可调用
≠
可以作为 Codex Subagent
```

---

# 3. Provider 与 Model 必须分离接入

错误设计：

```text
DeepSeekV4FlashProvider

QwenCoderProvider

GeminiProProvider
```

这会把：

```text
Provider
+
Model
```

绑死。

正确设计：

```text
Provider
DeepSeek

Models
├── deepseek-v4-flash
└── deepseek-v4-pro
```

或者：

```text
Provider
OpenRouter

Models
├── Model A
├── Model B
└── Model C
```

---

# 4. Provider 接入不等于 Model 接入

Provider 接入成功表示：

```text
CAS 知道：

如何访问服务
如何认证
使用什么协议
如何检查连接
如何发现模型（如果支持）
```

Model 接入成功表示：

```text
CAS 知道：

模型 ID
模型能力
Codex 兼容性
必要的运行元数据
```

二者必须独立完成。

---

# 5. Provider 接入级别

CAS 将 Provider 接入分成四类：

```text
PRESET

GENERIC_RESPONSES

ADAPTER

GATEWAY
```

---

# 6. PRESET Provider

表示：

> CAS 已知并维护默认配置的 Provider。

例如：

```text
DeepSeek
```

Preset 可以提供：

```text
默认名称
默认 Base URL
协议
认证方式
已知模型
已知能力
官方文档来源
兼容性信息
```

用户仍然可以根据允许范围修改：

```text
Name
Base URL
Credential
```

---

# 7. GENERIC_RESPONSES Provider

表示：

> 用户提供一个 CAS 未内置识别，但兼容 Codex 所需 Responses API 的服务。

例如：

```text
企业内部代理
自建 Responses Gateway
第三方兼容服务
```

CAS 不需要为每个此类 Provider 编写代码。

用户主要提供：

```text
Base URL
Authentication
Model ID
```

CAS 使用：

```text
Generic Responses Adapter
```

处理。

---

# 8. ADAPTER Provider

如果 Provider 存在：

```text
特殊认证
特殊模型发现
特殊请求语义
特殊 Capability Mapping
```

但仍可以直接输出 Codex 需要的协议，则可以实现：

```text
Provider Adapter
```

例如：

```text
Provider
    ↓
Custom Adapter
    ↓
Responses-compatible endpoint
```

---

# 9. GATEWAY Provider

如果 Provider 原生不提供 Codex 所需协议：

```text
Chat Completions

Anthropic Messages

其他专有协议
```

则必须通过：

```text
Gateway
```

进行转换。

逻辑：

```text
Codex
   ↓
Responses
   ↓
CAS Gateway
   ↓
Target Protocol
   ↓
Provider
```

V0.1 只保留此接入等级定义。

不要求实现完整 Gateway。

---

# 10. V0.1 Provider 支持范围

V0.1 正式支持：

```text
PRESET
    └── DeepSeek V4 Flash

GENERIC_RESPONSES
    └── Custom Responses Provider
```

架构允许：

```text
ADAPTER
GATEWAY
```

但不因为预留接口而提前实现复杂转换系统。

---

# 11. Provider 接入核心原则

新增 Provider 时必须优先判断：

```text
能否使用现有 Generic Adapter？
```

如果可以：

```text
只新增 Preset
```

不要：

```text
新增专用 Provider 类
```

只有存在真实特殊行为时才增加 Adapter。

---

# 12. 禁止 Provider Class 爆炸

错误：

```text
DeepSeekProvider
OpenRouterProvider
KimiProvider
QwenProvider
SiliconFlowProvider
CompanyProvider
FooProvider
BarProvider
```

如果它们实际上都只是：

```text
Responses API
+
Bearer Token
+
不同 Base URL
```

则应全部使用：

```text
GenericResponsesAdapter
```

---

# 13. Preset 与 Adapter 的区别

```text
Preset
=
数据
```

```text
Adapter
=
行为
```

例如 DeepSeek：

如果只需要：

```text
base_url
auth
Responses
known models
```

那么：

```text
DeepSeek
=
Preset
+
GenericResponsesAdapter
```

无需创建：

```text
DeepSeekAdapter
```

---

# 14. Provider Preset 的职责

Provider Preset 用于提供：

```text
开箱即用的默认接入描述
```

它不负责：

```text
保存用户 Credential
持久化 Provider
修改 Codex 配置
执行 Agent
```

---

# 15. Provider Preset 推荐目录

项目资源建议：

```text
resources/
└── provider-presets/
    ├── deepseek.json
    ├── provider-x.json
    └── ...
```

或者：

```text
resources/
└── provider-presets/
    ├── deepseek.yaml
    └── ...
```

格式必须：

```text
Declarative
Versioned
Schema Validated
```

---

# 16. Preset 顶层结构

推荐逻辑结构：

```text
ProviderPreset
├── schemaVersion
├── id
├── name
├── adapter
├── protocol
├── endpoint
├── authentication
├── discovery
├── capabilities
├── models
├── compatibility
├── documentation
└── metadata
```

这里描述的是：

```text
接入声明格式
```

不是数据库表结构。

---

# 17. 示例 Provider Preset

示意：

```json
{
  "schemaVersion": 1,
  "id": "deepseek",
  "name": "DeepSeek",
  "adapter": "responses",
  "protocol": "responses",

  "endpoint": {
    "defaultBaseUrl": "https://api.deepseek.com/"
  },

  "authentication": {
    "type": "bearer"
  },

  "discovery": {
    "type": "openai-models",
    "path": "/models"
  },

  "models": [
    "deepseek-v4-flash",
    "deepseek-v4-pro"
  ]
}
```

具体 Capability 在 Model Definition 中进一步定义。

---

# 18. Preset Schema Version

每个 Preset 必须拥有：

```text
schemaVersion
```

用于未来 Preset 格式升级。

禁止通过：

```text
CAS App Version
```

猜测 Preset 格式。

---

# 19. Preset ID

Preset ID 必须稳定。

例如：

```text
deepseek
```

不能因为：

```text
DeepSeek API
DeepSeek Official
DeepSeek V4
```

展示名称变化而修改。

---

# 20. Provider Adapter ID

Preset 必须声明使用：

```text
adapter
```

例如：

```text
responses
```

未来可能：

```text
openrouter
custom-oauth
gateway
```

但只有存在特殊行为时才提供专用 Adapter。

---

# 21. Protocol

当前直接 Codex Provider 的首要协议：

```text
RESPONSES
```

在当前 Codex Custom Provider 配置中，`wire_api` 正式支持的值为 `responses`。因此 CAS V0.1 的 Direct Provider Compatibility 也必须以 Responses 为基础。

未来 CAS 可以认识：

```text
CHAT_COMPLETIONS
ANTHROPIC_MESSAGES
GEMINI
```

但这些并不自动意味着：

```text
Direct Codex Compatible
```

---

# 22. Protocol 与 Compatibility 分离

禁止：

```text
protocol = responses
    ↓
直接认定 Native
```

因为：

```text
存在 /responses
```

并不能证明：

```text
所有 Codex Tool Semantics
Multi-Agent Behavior
Streaming
Reasoning
Tool Calls
```

都正确工作。

因此：

```text
Protocol Compatibility
```

和：

```text
Codex Compatibility
```

是两个维度。

---

# 23. Provider Endpoint

Provider Preset 可以声明：

```text
defaultBaseUrl
```

例如：

```text
https://api.example.com/
```

但必须允许某些 Preset：

```text
Base URL Override
```

因为用户可能通过：

```text
企业代理
Regional Endpoint
自建 Gateway
```

访问相同 Provider。

---

# 24. Endpoint 不承载 Model

禁止：

```text
每个模型单独定义一个 Provider URL
```

除非目标服务确实如此设计。

一般关系：

```text
Provider Endpoint
    ↓
多个 Model
```

---

# 25. Authentication 声明

Provider Preset 只声明：

```text
需要哪种认证
```

例如：

```text
NONE
BEARER
ENV
CUSTOM_HEADER
COMMAND
```

具体 Credential 存储不属于本文档。

---

# 26. Authentication Requirement

Preset 可以声明：

```text
required
optional
none
```

例如公开本地 Provider：

```text
authentication = none
```

而普通云 API：

```text
authentication = required
```

---

# 27. Provider Model Discovery

Provider 可以声明：

```text
Discovery Capability
```

推荐类型：

```text
NONE

OPENAI_MODELS

STATIC

CUSTOM
```

---

# 28. OPENAI_MODELS Discovery

表示 Provider 提供兼容：

```text
GET /models
```

能力。

CAS 可以读取：

```text
model.id
```

作为候选模型。

例如当前 DeepSeek 官方提供 `/models`，返回可用模型 ID。

但：

```text
Discovery Result
```

只说明模型存在。

不说明：

```text
该模型适合 Codex。
```

---

# 29. STATIC Discovery

部分 Provider 无模型发现接口。

Preset 可以提供：

```text
Known Models
```

例如：

```text
model-a
model-b
```

CAS 使用内建 Definition 展示。

---

# 30. NONE Discovery

用户只能：

```text
Manual Model ID
```

添加模型。

不得因为 Provider 没有 `/models`：

```text
判定 Provider 不兼容。
```

---

# 31. CUSTOM Discovery

仅用于：

```text
模型发现机制确实特殊
```

的 Provider Adapter。

避免为了不同 JSON 字段写大量专用 Adapter。

---

# 32. Model Discovery 不是自动导入

Discovery：

```text
Provider
    ↓
发现 100 个模型
```

不等于：

```text
CAS 自动创建 100 个正式模型。
```

必须区分：

```text
Discovered
```

与：

```text
Managed / Enabled
```

---

# 33. Model 接入层级

Model 接入分成：

```text
KNOWN

DISCOVERED

MANUAL

VERIFIED
```

这些不是互斥的最终状态，而是描述模型信息来源。

---

# 34. KNOWN Model

由：

```text
CAS Preset
官方 Catalog
```

明确提供定义。

例如：

```text
deepseek-v4-flash
```

---

# 35. DISCOVERED Model

从：

```text
Provider /models
```

获取。

CAS 可能只知道：

```text
modelId
provider
```

其他能力仍未知。

---

# 36. MANUAL Model

用户手工填写：

```text
modelId
```

CAS 不应假设它的 Capability。

默认：

```text
Compatibility = UNKNOWN
```

---

# 37. VERIFIED Model

模型经过：

```text
官方声明
CAS 内建验证
兼容测试
```

之一获得较高可信度。

Verified 不意味着：

```text
未来永久兼容。
```

必须结合：

```text
verification version
Codex version
model/provider version
```

理解。

---

# 38. Model Definition

已知 Model 使用：

```text
ModelDefinition
```

声明。

推荐逻辑结构：

```text
ModelDefinition
├── schemaVersion
├── modelId
├── displayName
├── aliases
├── context
├── reasoning
├── capabilities
├── codex
├── lifecycle
├── sources
└── metadata
```

---

# 39. Model ID

`modelId` 必须是真实发送给 Provider 的模型标识。

例如：

```text
deepseek-v4-flash
```

不能使用：

```text
DeepSeek V4 Flash
```

作为请求 ID。

---

# 40. Display Name

Display Name 仅用于：

```text
人类阅读
```

例如：

```text
DeepSeek V4 Flash
```

其变化不能影响：

```text
模型绑定。
```

---

# 41. Model Alias

允许模型存在：

```text
alias
```

例如历史兼容名称。

但 Alias 不应自动覆盖 Canonical Model ID。

应明确：

```text
alias
→
canonical
```

关系。

---

# 42. Deprecated Alias

如果 Provider 已宣布某 Alias 弃用：

必须记录：

```text
deprecated
```

CAS 不应继续将其作为新配置默认值。

---

# 43. Model Lifecycle

推荐：

```text
ACTIVE

PREVIEW

DEPRECATED

RETIRED

UNKNOWN
```

---

# 44. ACTIVE

当前正式可用模型。

---

# 45. PREVIEW

可以使用，但：

```text
接口
行为
模型身份
```

可能发生变化。

CAS 应降低兼容性承诺级别。

---

# 46. DEPRECATED

仍可调用，但 Provider 已宣布未来移除。

新 Agent Binding 应：

```text
Warning
```

而不是默认推荐。

---

# 47. RETIRED

已无法正常调用。

不得继续作为可选择的 Active Model。

---

# 48. Lifecycle 与 Compatibility 分离

一个模型可以：

```text
Compatibility = NATIVE
Lifecycle = DEPRECATED
```

两者表达完全不同概念。

---

# 49. Context Metadata

Model Definition 可以声明：

```text
contextWindow
maxOutputTokens
```

来源不确定时：

```text
unknown
```

不得猜测。

---

# 50. Reasoning Capability

Model Definition 应能够描述：

```text
supportsReasoning

supportedEfforts

defaultEffort
```

例如：

```text
low
medium
high
max
```

但不得假设所有 Provider 对相同 effort 名称具有相同语义。

---

# 51. Reasoning Effort Mapping

某些 Provider 可能：

```text
不接受 Codex 的全部 reasoning effort
```

此时允许定义：

```text
ReasoningEffortMap
```

例如：

```text
minimal → high
low     → high
medium  → high
high    → high
xhigh   → max
```

这种映射必须来源于：

```text
Provider / Model 官方兼容信息
```

或者 CAS Adapter。

不能凭经验随意添加。

---

# 52. Capability 分类

Capability 建议分成：

```text
Protocol Capabilities

Model Capabilities

Codex Runtime Capabilities

Optional Capabilities
```

---

# 53. Protocol Capabilities

例如：

```text
RESPONSES_API
STREAMING
```

表达传输层能力。

---

# 54. Model Capabilities

例如：

```text
REASONING
TOOL_CALLING
STRUCTURED_OUTPUT
IMAGE_INPUT
LARGE_CONTEXT
```

表达模型本身能力。

---

# 55. Codex Runtime Capabilities

例如：

```text
CODEX_TOOL_CALLING
PARALLEL_TOOL_CALLING
CODEX_MULTI_AGENT
APPLY_PATCH
SHELL_TOOL
```

它们强调：

> 模型在 Codex Runtime 中的兼容表现。

---

# 56. Capability 不得只用单个 `compatible=true`

错误：

```json
{
  "codexCompatible": true
}
```

信息太少。

正确：

```text
Responses           ✓
Tool Calling        ✓
Parallel Tools      ✓
Codex Multi-Agent   ✓
Reasoning           ✓
```

然后再根据这些能力推导：

```text
Compatibility Level
```

---

# 57. Capability 状态

每个 Capability 不建议只有：

```text
true / false
```

而应至少支持：

```text
SUPPORTED

UNSUPPORTED

UNKNOWN
```

原因：

```text
不知道
```

不能等同于：

```text
不支持。
```

---

# 58. 可选 Capability Evidence

重要 Capability 可以携带 Evidence：

```text
source
verifiedAt
version
```

例如：

```text
TOOL_CALLING

status:
SUPPORTED

source:
OFFICIAL_PROVIDER

verifiedAt:
...
```

这样未来能力冲突可以追踪来源。

---

# 59. Capability 来源

标准来源：

```text
OFFICIAL_CODEX

OFFICIAL_PROVIDER

CAS_BUILT_IN

RUNTIME_PROBE

USER_OVERRIDE
UNKNOWN
```

---

# 60. 能力信息可信度

建议内部定义：

```text
Evidence Confidence
```

例如：

```text
AUTHORITATIVE
VERIFIED
INFERRED
USER_DECLARED
UNKNOWN
```

---

# 61. 官方 Provider 声明

如果 Provider 官方明确提供：

```text
Codex Integration Guide
```

并给出模型配置，应视为高价值 Evidence。

但仍不代表：

```text
CAS 当前支持的 Codex 版本
```

一定完全兼容。

---

# 62. Codex Compatibility Level

统一使用：

```text
NATIVE

COMPATIBLE

GATEWAY_REQUIRED

UNSUPPORTED

UNKNOWN
```

---

# 63. NATIVE

满足：

```text
Provider / Model 明确针对 Codex 或完整所需行为适配
```

并有较强 Evidence。

例如：

```text
官方明确提供 Codex 集成
+
Responses API
+
必要 Tool / Agent 能力
```

---

# 64. COMPATIBLE

表示：

```text
不是官方专门针对 Codex 适配
```

但 CAS 已验证：

```text
所需核心能力可工作。
```

---

# 65. GATEWAY_REQUIRED

表示：

```text
Provider / Model 本身不能直接作为 Codex Provider
```

但理论上可以通过：

```text
CAS Gateway
```

工作。

V0.1 应展示：

```text
当前不可直接使用
```

---

# 66. UNSUPPORTED

存在已知明确的不兼容问题。

例如：

```text
缺少 Responses
无法 Tool Call
协议行为与 Codex 不兼容
```

---

# 67. UNKNOWN

CAS 尚无足够证据判断。

这是新增 Custom Model 的默认状态。

---

# 68. Compatibility 不是模型质量评级

CAS 不应该：

```text
NATIVE = 更聪明
COMPATIBLE = 更差
```

Compatibility 只表示：

```text
能否可靠地运行在 Codex Agent 环境。
```

---

# 69. Agent Compatibility

模型是否适合某 Agent，由：

```text
Model Effective Capability
        ↓
Agent Requirements
```

计算。

因此：

```text
Model compatible with Codex
```

不代表：

```text
适合所有 Agent Role。
```

---

# 70. 典型 Executor Requirement

例如：

```text
Tool Calling
Multi-Agent compatibility
Reliable file/shell tool behavior
```

这些 Requirement 由 Agent 定义。

本文档只要求 Model Capability 能够提供判断依据。

---

# 71. Provider Capability

Provider Capability 与 Model Capability 必须分开。

例如：

```text
Provider
支持 Responses
```

但：

```text
Model X
不支持 Tool Calls
```

最终该模型仍不能满足 Codex Executor 要求。

---

# 72. Effective Capability

最终：

```text
Effective Capability
=
Provider
∩
Model
∩
Protocol Adapter
∩
Codex Compatibility
```

任何一层不支持某关键能力：

```text
最终不支持。
```

---

# 73. `UNKNOWN` 的传播

例如：

```text
Provider Tool Calling = SUPPORTED

Model Tool Calling = UNKNOWN
```

最终：

```text
Effective Tool Calling = UNKNOWN
```

不得提升为：

```text
SUPPORTED
```

---

# 74. Capability Override

允许高级用户声明：

```text
User Override
```

例如：

```text
我已经验证 Model X 支持 Tool Calling。
```

但 CAS 必须保留：

```text
source = USER_OVERRIDE
```

不能将其伪装成官方兼容。

---

# 75. User Override 不改变 Preset

用户 Override 只作用于：

```text
用户自己的 Model Instance
```

不得修改：

```text
内置 Provider Preset
Model Definition
```

---

# 76. Provider Connection Check

Provider Adapter 必须提供标准：

```text
Connection Check
```

检查目标：

```text
DNS / network reachability
authentication
basic API validity
```

不得把：

```text
完整 Agent Compatibility Test
```

塞入 Connection Check。

---

# 77. Connection Check 结果

推荐：

```text
SUCCESS

AUTH_FAILED

UNREACHABLE

PROTOCOL_ERROR

RATE_LIMITED

SERVER_ERROR

UNKNOWN_ERROR
```

---

# 78. Connection 成功不等于 Model 成功

必须保持：

```text
Provider Test ✓
```

和：

```text
Model Verification ?
```

独立。

---

# 79. Model Availability Check

可以针对某模型执行轻量：

```text
Model Availability Check
```

用于确认：

```text
Model ID 是否存在
账户是否有权限
Provider 是否接受该 Model
```

不负责验证完整 Tool Loop。

---

# 80. Compatibility Probe

更高级的：

```text
Compatibility Probe
```

用于验证 Codex Agent 所需行为。

建议逻辑分层：

```text
Level 0
Static Metadata

Level 1
Endpoint Check

Level 2
Basic Model Request

Level 3
Tool Call Probe

Level 4
Codex Runtime Probe
```

---

# 81. P1 Product Probe 范围

V0.1 产品只实现 Provider connection / authentication 与静态 Metadata 校验，不注册 `model_verify` 或 Runtime Probe。ADR 0014 的真实 Codex E2E 是发布前开发门禁，不是面向用户的 Probe 功能，也不产生业务历史记录。

P1 同样不应一开始实现昂贵复杂的完整 Benchmark。

优先：

```text
Static Metadata
+
Connection
+
Basic Request
+
必要 Tool Capability Check
```

即可。

---

# 82. Static Verification

从以下来源判断：

```text
CAS Built-in Metadata
Provider Official Metadata
Model Catalog
```

不发送实际模型请求。

---

# 83. Basic Request Probe

向 Model 发送最小请求：

```text
验证模型是否可正常响应
```

Probe 必须：

```text
小 Token
无副作用
明确成本
```

---

# 84. Tool Call Probe

如果需要验证：

```text
TOOL_CALLING
```

使用一个完全无副作用的虚拟 Tool。

例如：

```text
echo(value)
```

模型只需生成 Tool Call。

不得：

```text
执行 shell
写文件
访问网络资源
```

作为 Provider Compatibility Test。

---

# 85. Parallel Tool Probe

如果声明：

```text
PARALLEL_TOOL_CALLING
```

可以设计两个互不依赖的虚拟 Tool。

只有真实返回符合预期时才标记：

```text
VERIFIED
```

---

# 86. Multi-Agent Probe

真正的 Codex Multi-Agent 兼容性可能不仅是 Provider API Capability。

因此：

```text
CODEX_MULTI_AGENT
```

不能仅通过普通 API Tool Call Probe 推断。

必须来源于：

```text
官方明确声明
```

或者：

```text
Codex Runtime Integration Probe
```

---

# 87. Runtime Probe

Runtime Probe 指：

```text
通过实际 Codex Runtime
生成一个隔离测试 Agent
执行受控任务
```

用于验证：

```text
Spawn
Tool Call
Agent Message
Return
```

完整路径。

这是：

```text
最高等级兼容性验证
```

从 P1 起再评估为独立产品能力。

---

# 88. Probe 不修改用户 Agent

Compatibility Probe 禁止复用：

```text
executor
reviewer
```

等真实 Agent。

应使用：

```text
CAS Temporary Probe Agent
```

测试完成后清理。

具体 Codex 配置事务由配置集成规范处理。

---

# 89. Probe 结果需要作用域

验证结果必须至少与：

```text
Provider
Model
Codex Version
CAS Probe Version
```

关联。

不能：

```text
Model X 验证一次
以后所有版本永久 Compatible
```

---

# 90. Probe Expiration

Compatibility Evidence 可以过期。

例如：

```text
Provider Model 更新
Codex major compatibility change
CAS Probe 规则升级
```

后：

```text
需要重新验证。
```

具体失效策略由业务规则文档定义。

---

# 91. Model Catalog

Codex Model Catalog 与 CAS Model Definition 不应混为一体。

CAS Model Definition：

```text
面向 CAS
```

Codex Model Catalog：

```text
面向 Codex Runtime
```

---

# 92. Model Catalog Source

CAS 可以从以下来源获得 Codex Catalog：

```text
OFFICIAL_PROVIDER

CAS_BUILT_IN

GENERATED

USER_SUPPLIED
```

---

# 93. OFFICIAL_PROVIDER Catalog

如果 Provider 官方发布：

```text
专用 Codex models.json
```

优先使用：

```text
官方版本
```

前提：

```text
格式合法
来源可信
```

---

# 94. CAS_BUILT_IN Catalog

如果官方没有提供，但 CAS 项目经过验证后维护：

```text
CAS Catalog
```

则必须：

```text
明确来源是 CAS
```

不能伪装为 Provider 官方文件。

---

# 95. GENERATED Catalog

CAS 根据：

```text
Model Definition
```

生成 Codex Model Catalog。

只有：

```text
CAS 明确知道 Codex 当前 Catalog Schema
```

时才能生成。

V0.1 为第三方 Responses Provider 生成 Catalog 时，`multi_agent_version` 固定为 `v1`。当前 V2 会把委派任务放入 OpenAI 专用的 `agent_message.encrypted_content`，第三方 Provider 无法解密；只有 Codex 提供并验证 provider-aware V2 兼容路径后才能升级。

跟踪依据：<https://github.com/openai/codex/issues/33551>

---

# 96. USER_SUPPLIED Catalog

高级用户可以提供：

```text
自定义 Model Catalog
```

CAS 可以：

```text
校验
引用
```

但不应该默认为官方兼容。

---

# 97. Catalog Validation

至少验证：

```text
JSON Syntax

Required Fields

Model ID Match

Supported Schema

Duplicated Model Entries
```

---

# 98. Catalog 与 Model ID 必须一致

如果 Agent 绑定：

```text
model-x
```

Catalog 却只包含：

```text
model-y
```

则：

```text
Configuration Invalid
```

---

# 99. Catalog 不得污染其他 Model

一个 Provider Catalog 中可以包含多个模型。

但 CAS 如果只管理：

```text
Model A
```

不得无理由修改：

```text
用户手动维护的其他 Model Metadata。
```

CAS-generated Catalog 应尽量使用：

```text
CAS-exclusive resource
```

---

# 100. Provider Preset Model Definition

一个 Preset 可以附带：

```text
Known Model Definitions
```

例如：

```text
DeepSeek
├── deepseek-v4-flash
└── deepseek-v4-pro
```

但：

```text
Provider Preset 更新
```

不等于：

```text
强制修改用户现有 Model。
```

---

# 101. Preset 更新原则

Preset 属于：

```text
Template / Knowledge
```

用户创建后的 Provider / Model 属于：

```text
User Configuration
```

Preset 更新时：

```text
可以提示更新
```

不能：

```text
静默覆盖用户设置。
```

---

# 102. Provider Preset Registry

长期可以维护：

```text
Provider Preset Registry
```

例如：

```text
DeepSeek
Provider A
Provider B
Custom Responses
```

V0.1 Registry 可以直接随应用打包。

不要求在线 Marketplace。

---

# 103. Registry Entry

Registry Entry 只提供：

```text
Preset Metadata
Version
Compatibility Metadata
```

不包含：

```text
Credential
用户配置
```

---

# 104. Preset Distribution

V0.1：

```text
Bundled with CAS Release
```

后续可以：

```text
Signed Remote Registry
```

但不属于 V0.1 必须能力。

---

# 105. Preset Trust

如果未来支持远程 Preset：

必须区分：

```text
OFFICIAL_CAS

COMMUNITY

LOCAL
```

因为 Provider Preset 最终可能影响：

```text
API Endpoint
Authentication behavior
```

不能把所有远程配置视为同等可信。

---

# 106. V0.1 不执行 Preset Script

Provider Preset V0.1 必须保持：

```text
Data Only
```

禁止：

```text
shell command
JavaScript
Lua
arbitrary executable
```

作为 Provider 安装脚本。

---

# 107. 为什么禁止 Preset Script

否则：

```text
添加 Provider
```

实际上变成：

```text
执行第三方代码。
```

会显著扩大：

```text
供应链风险
安全边界
维护复杂度
```

---

# 108. Adapter 才允许代码行为

如果确实需要特殊行为：

```text
必须实现为 CAS 代码中的 Adapter
```

经过：

```text
Review
Test
Release
```

而不是藏进 Preset。

---

# 109. Generic Responses Adapter

V0.1 最重要的 Adapter：

```text
GenericResponsesAdapter
```

负责：

```text
连接测试
标准 Bearer Auth
/models discovery
Responses compatibility
基础请求
```

---

# 110. Generic Adapter 不包含 Provider 名称判断

禁止：

```text
if provider.name == "DeepSeek"
```

出现在 Generic Adapter。

DeepSeek 特有信息应该来自：

```text
Preset
Model Definition
```

---

# 111. Adapter Capability

每个 Adapter 可以声明：

```text
supportedProtocols
supportedAuthTypes
supportsDiscovery
supportsCompatibilityProbe
```

CAS 在接入前先判断：

```text
Preset Requirement
⊆
Adapter Capability
```

---

# 112. Adapter Version

Adapter 行为发生重要变化时应有：

```text
Adapter Version
```

用于兼容性验证和测试。

不一定暴露给普通用户。

---

# 113. Provider Adapter Interface

概念接口：

```text
ProviderAdapter

validateConfiguration()

testConnection()

discoverModels()

checkModelAvailability()

probeCapabilities()
```

不得包含：

```text
saveProvider()

bindAgent()

applyCodexConfiguration()
```

这些属于其他模块职责。

---

# 114. ProviderAdapter 输入

Adapter 接收：

```text
Resolved Provider Connection
Credential Handle
Requested Operation
```

而不是：

```text
数据库实体
UI Form
Codex TOML AST
```

---

# 115. Adapter 输出

输出统一 Result：

```text
Success
Failure
Capability Evidence
Discovered Models
```

不能向上层直接暴露：

```text
Provider-specific exception stack
```

---

# 116. HTTP Status Mapping

Provider Adapter 应将常见 HTTP 状态标准化。

例如：

```text
401 / 403
→ AUTH_FAILED

404 Model
→ MODEL_NOT_FOUND

429
→ RATE_LIMITED

5xx
→ PROVIDER_SERVER_ERROR
```

具体 Error Model 由错误处理规范负责。

---

# 117. Provider-specific Error

原始错误可以保留为：

```text
diagnostic details
```

但业务层必须首先获得：

```text
标准错误类别。
```

---

# 118. Timeout

所有 Provider Probe 必须存在：

```text
明确 Timeout
```

不得无限等待外部 Provider。

Timeout 策略属于接入 Adapter 配置，不属于 Agent。

---

# 119. Retry

连接检测默认只允许：

```text
有限重试
```

不得因为：

```text
Provider unavailable
```

让 CAS GUI 或 Apply 长时间阻塞。

具体 Retry 参数由实现规范确定。

---

# 120. Provider Rate Limit

Connection Test 或 Compatibility Probe 必须：

```text
低频
用户触发
可缓存结果
```

不得：

```text
后台持续请求 Provider
```

作为普通状态刷新方式。

---

# 121. Provider Status 不做实时监控

CAS V0.1 不是：

```text
Provider Monitoring Platform
```

因此：

```text
Ready
```

只表示：

```text
最近一次检查成功
+
当前配置完整
```

不能保证 Provider 此刻永久可用。

---

# 122. 模型价格

CAS Model Definition 可以未来提供：

```text
pricing metadata
```

但：

```text
价格
```

不是 V0.1 Codex Compatibility 的必要字段。

不要因为价格变化导致：

```text
Model Integration 失效。
```

---

# 123. Model Quality

CAS V0.1 不定义：

```text
Intelligence Score
Coding Score
Agent Score
```

除非未来建立独立 Benchmark 系统。

Provider 官方宣传数据不应直接转化成 CAS 排名。

---

# 124. 推荐模型

Preset 可以标记：

```text
recommendedForCodex
```

但必须基于：

```text
Compatibility
```

而不是：

```text
商业偏好。
```

---

# 125. Default Model

Provider Preset 可以指定：

```text
suggestedDefaultModel
```

例如：

```text
deepseek-v4-flash
```

但只影响：

```text
首次选择建议
```

不得自动绑定已有 Agent。

---

# 126. Model Deprecation 更新

CAS 发现：

```text
当前绑定 Model 已 Deprecated
```

应：

```text
保留 Binding
+
产生 Warning
```

不得自动替换。

---

# 127. Model Retirement 更新

如果模型已：

```text
RETIRED
```

当前 Binding 变成：

```text
Unavailable
```

但 CAS 仍不应：

```text
擅自切换到新模型。
```

---

# 128. Provider Preset Removal

CAS 新版本删除某 Preset 时：

```text
已有 Provider 配置
```

不得随之消失。

Preset 只是：

```text
创建/知识来源。
```

---

# 129. Unknown Provider

CAS 读取到：

```text
自己不认识的 Provider
```

应视为：

```text
External / Unknown
```

不能：

```text
尝试套用最相似 Preset。
```

---

# 130. Unknown Model

CAS 发现 Provider 返回：

```text
new-model-xyz
```

默认：

```text
Lifecycle = UNKNOWN
Compatibility = UNKNOWN
Capabilities = UNKNOWN
```

直到获取有效 Metadata。

---

# 131. 名称相似不能推断能力

例如：

```text
model-v4-pro
model-v4-pro-new
```

不能因为名字相似：

```text
自动复制 Capability。
```

---

# 132. Alias 必须显式

只有：

```text
Provider 官方
或 CAS 明确 Metadata
```

声明：

```text
A aliases B
```

才能共享兼容信息。

---

# 133. Model Version 变化

对于滚动 Alias：

```text
model-latest
```

必须认识到：

```text
底层模型可以变化。
```

因此 Compatibility Evidence 的可靠性可能低于：

```text
固定 Snapshot
```

---

# 134. Model Snapshot

如果 Provider 支持固定 Snapshot：

可以记录：

```text
snapshot
```

用于更稳定的兼容验证。

CAS V0.1 不要求模型都必须有 Snapshot。

---

# 135. Provider Capability Discovery

不应假设 Provider 的：

```text
/models
```

返回完整 Capability。

通常它只提供：

```text
id
owned_by
```

因此 Capability 仍需要：

```text
Metadata
Preset
Probe
```

补充。

---

# 136. Metadata Merge

Model Metadata 可能来自多个来源：

```text
Preset
Official Provider
Discovery
Probe
User Override
```

必须经过：

```text
Metadata Resolution
```

不能简单：

```text
后加载覆盖先加载。
```

---

# 137. Metadata 冲突

例如：

```text
Official:
Tool Calling = true

Probe:
Tool Calling = false
```

不能直接选一个静默覆盖。

应该形成：

```text
Compatibility Issue
```

并降低：

```text
Effective Compatibility
```

---

# 138. Metadata Resolution 原则

优先：

```text
可验证的实际兼容行为
```

但需要考虑：

```text
Probe 时间
Provider 临时故障
Codex 版本
```

因此：

```text
Probe Failure
```

不一定立即等同：

```text
官方声明错误。
```

---

# 139. Hard Capability

部分能力属于直接运行前提：

```text
RESPONSES
必要 Tool Semantics
```

缺失时：

```text
UNSUPPORTED
```

---

# 140. Soft Capability

例如：

```text
PARALLEL_TOOL_CALLING
LARGE_CONTEXT
```

缺失通常只影响：

```text
性能 / 使用体验
```

不一定阻止 Agent 使用。

---

# 141. Capability Requirement 不应写在 Provider Preset

例如：

```text
Executor 必须 Tool Calling
```

属于：

```text
Agent Requirement
```

不是：

```text
DeepSeek Preset
```

Provider / Model 接入层只声明：

```text
它有什么能力。
```

---

# 142. Custom Responses Provider

V0.1 Generic Custom Provider 必须允许：

```text
Name
Base URL
Authentication
Model IDs
```

但不能让用户必须编写：

```text
Provider Plugin
```

---

# 143. Custom Provider 默认值

对于 Custom Responses：

```text
Protocol = RESPONSES

Compatibility = UNKNOWN
```

直到验证。

---

# 144. Custom Provider Model

用户输入：

```text
model-x
```

创建后：

```text
Capabilities = UNKNOWN
Compatibility = UNKNOWN
```

允许：

```text
Run Check
```

进一步确定。

---

# 145. Custom Provider 不能通过名称变成 Preset

如果用户命名：

```text
DeepSeek
```

但 Base URL：

```text
https://example.com/
```

不能自动使用：

```text
DeepSeek Official Metadata
```

除非用户明确选择：

```text
DeepSeek Preset。
```

---

# 146. Provider Identity 与 Branding 分离

Provider：

```text
displayName = DeepSeek Proxy
```

并不意味着：

```text
providerType = DeepSeek Official。
```

---

# 147. DeepSeek Preset V0.1

当前官方可作为首个正式 Preset。

逻辑：

```text
Preset:
deepseek

Adapter:
responses

Default Base URL:
https://api.deepseek.com/

Authentication:
Bearer

Known Models:
deepseek-v4-flash
deepseek-v4-pro
```

DeepSeek 当前官方 Catalog 列出上述两种模型，但 V0.1 仅允许 `deepseek-v4-flash` 进入 Codex Binding。`GET /models` Discovery 属于 P1；V0.1 Preset 使用随 CAS 发布并带版本的官方 Model Definition 快照。

安全差异：DeepSeek 官方手工示例使用 `experimental_bearer_token`，CAS 不复制明文方案；Preset 只声明 Bearer 认证需求，Configuration Compiler 注入由 Windows 安装层解析的 `cas-helper` 绝对路径。

---

# 148. DeepSeek Preset 不写死 V4

未来：

```text
DeepSeek V5
```

发布时应该：

```text
增加 Model Definition
```

而不是：

```text
修改 Provider Core。
```

---

# 149. DeepSeek V4 Flash Definition

可以维护类似：

```text
modelId:
deepseek-v4-flash

displayName:
DeepSeek V4 Flash

lifecycle:
ACTIVE

contextWindow:
1000000

reasoning:
SUPPORTED

toolCalling:
SUPPORTED

codexCompatibility:
NATIVE

minimumCodexClientVersion:
0.144.0
```

Codex-specific Metadata 必须根据官方 Codex 集成信息和实际验证维护。

---

# 150. DeepSeek V4 Pro

同样作为：

```text
独立 Model Definition
```

不能因为：

```text
同属 V4
```

直接复制全部 Codex Compatibility。

V0.1 固定：

```text
integrationReadiness: NOT_READY
compatibility: UNKNOWN
bindingSelectable: false
```

只有 DeepSeek 官方 Codex 文档明确支持且 CAS 真实 Subagent PoC 通过后，才可用 Preset 数据更新提升；不得修改 Provider Core。

---

# 151. Provider 接入开发流程

新增 Provider 时执行：

```text
1. 判断 Provider 类型

2. 确认协议

3. 判断 Generic Adapter 是否足够

4. 收集官方 Endpoint

5. 收集 Authentication 方式

6. 确定 Model Discovery

7. 创建 Provider Preset

8. 添加 Known Model Definitions

9. 声明 Capability Evidence

10. 验证 Preset Schema

11. 执行 Provider Integration Tests

12. 执行 Model Compatibility Tests

13. 更新 Compatibility Matrix
```

---

# 152. 第一步：协议确认

必须先确认：

```text
Provider 实际支持什么 API。
```

禁止因为宣传写：

```text
OpenAI-compatible
```

就默认：

```text
Responses-compatible。
```

---

# 153. 第二步：Generic Adapter 判断

如果：

```text
Responses endpoint
Bearer Auth
标准模型 ID
```

已经足够：

```text
使用 Generic Adapter。
```

不要新增代码。

---

# 154. 第三步：特殊行为识别

只有以下情况才考虑 Adapter：

```text
特殊签名认证

特殊 token 获取

特殊模型发现

非标准 Responses 行为

必要请求 Header

特殊 reasoning mapping

特殊 streaming compatibility
```

---

# 155. 第四步：Model Definitions

至少为：

```text
官方推荐用于 Codex 的模型
```

提供 Definition。

不要为了：

```text
Provider 有 200 个模型
```

就手工维护全部模型。

---

# 156. 第五步：Capability Evidence

任何：

```text
SUPPORTED
```

声明都必须知道来源。

尤其：

```text
MULTI_AGENT
PARALLEL_TOOL_CALLING
REASONING
```

---

# 157. 第六步：兼容验证

至少验证：

```text
Provider Connection

Model Availability

Basic Responses

Tool Calling
```

如果声明：

```text
Native Codex Multi-Agent
```

还必须具有：

```text
官方 Codex Evidence
或 Runtime Probe。
```

---

# 158. Model-only 接入流程

如果：

```text
Provider 已存在
```

只增加新 Model：

```text
1. 确认 Model ID

2. 确认 Lifecycle

3. 获取官方 Metadata

4. 创建 Model Definition

5. 声明 Capability

6. 验证基本调用

7. 验证 Tool Capability

8. 判断 Codex Compatibility

9. 发布
```

无需修改：

```text
Provider Adapter。
```

---

# 159. Provider Preset 测试

每个内置 Preset 必须测试：

```text
Schema valid

ID unique

Adapter exists

Protocol supported

Endpoint valid format

Auth type supported

Discovery type supported

Known Model IDs unique
```

---

# 160. Model Definition 测试

至少：

```text
Schema valid

Model ID valid

Capability values valid

Lifecycle valid

Reasoning mapping valid

Compatibility references valid
```

---

# 161. Integration Test

有真实 API Key 的 CI 环境可运行：

```text
Provider Connection Test

Model List Test

Basic Responses Test

Tool Call Test
```

但 Secret 不进入普通 CI 日志。

---

# 162. Offline Test

绝大多数测试必须可以：

```text
不调用真实 Provider
```

通过：

```text
Mock HTTP Server
Fixtures
Recorded compatible responses
```

完成。

---

# 163. Provider Fixtures

每个 Adapter 应维护：

```text
success response

auth failure

model list

rate limit

invalid response

tool call response

stream response
```

等必要 Fixtures。

---

# 164. Contract Test

Generic Responses Adapter 应拥有统一：

```text
Responses Provider Contract Test
```

所有声明：

```text
adapter = responses
```

的 Preset 都应该能复用。

---

# 165. Preset-specific Test

Preset 只测试：

```text
数据是否满足 Generic Adapter Contract
```

而不是复制 Generic Adapter 的测试逻辑。

---

# 166. Provider 文档来源

每个内置 Provider Preset 必须保存：

```text
Documentation Source
```

用于维护者检查更新。

优先：

```text
Provider 官方文档。
```

---

# 167. Model Metadata 来源

同样：

```text
Official Provider Documentation

Official Codex Integration Documentation

CAS Verification
```

应能够追踪。

---

# 168. 不依赖第三方博客作为权威 Capability

第三方资料可以用于：

```text
发现线索
```

但不能作为内置 Compatibility 的唯一依据。

---

# 169. Provider 更新维护

当 Provider API 发生：

```text
Base URL Change
Model Rename
Deprecation
Authentication Change
Protocol Change
```

优先更新：

```text
Preset / Model Definition
```

只有行为变化才更新：

```text
Adapter。
```

---

# 170. Model 更新维护

例如：

```text
deepseek-v4-flash
```

的：

```text
context
reasoning
tool behavior
```

发生变化：

只更新：

```text
Model Metadata
Compatibility Evidence
```

不修改：

```text
Provider Core。
```

---

# 171. Capability Regression

如果已知兼容模型后来发生回归：

```text
Compatibility
NATIVE
    ↓
UNKNOWN / UNSUPPORTED
```

是允许的。

CAS 不能为了：

```text
保持绿色状态
```

而忽视新证据。

---

# 172. Compatibility Matrix

Provider / Model 接入最终需要输出到独立：

```text
Compatibility Matrix
```

例如：

```text
Provider   Model       Responses  Tool  Multi-Agent  Status

DeepSeek   V4 Flash    ✓          ✓     ✓            Native
...
```

矩阵只是：

```text
结果展示
```

不是 Capability Source of Truth。

---

# 173. Gateway 边界

未来 Provider：

```text
不支持 Responses
```

时，不允许 Generic Adapter：

```text
偷偷进行复杂协议转换。
```

应该明确：

```text
Gateway Required。
```

---

# 174. 为什么 Gateway 独立

因为协议转换涉及：

```text
message format

streaming

reasoning items

tool calls

tool results

error semantics

continuation state
```

复杂度远高于：

```text
Provider Preset。
```

必须独立治理。

---

# 175. Gateway 后的 Provider

未来：

```text
Native Provider
     ↓
CAS Gateway
     ↓
Codex Responses
```

从 CAS Core 看，可以表现为：

```text
Gateway-backed Provider
```

其 Model 仍使用相同：

```text
Model Capability
Compatibility
```

体系。

---

# 176. Gateway 不改变 Model Identity

例如：

```text
Kimi model X
```

通过 CAS Gateway：

```text
model identity
```

仍是：

```text
Kimi model X
```

而不是：

```text
cas-proxy-model-1。
```

除非协议实现确实需要内部映射。

---

# 177. Provider Plugin 体系

V0.1 不做动态：

```text
Third-party Native Plugin
```

即不允许用户下载：

```text
.dll
.so
.dylib
```

直接注入 CAS。

先使用：

```text
Preset
+
Built-in Adapter
```

保证安全和可维护性。

---

# 178. Community Provider

未来社区贡献新 Provider 的首选方式：

```text
提交 Provider Preset
+
Model Metadata
+
Tests
```

而不是：

```text
提交大量新 Provider 代码。
```

---

# 179. Provider Contribution 最小内容

Contributor 至少提供：

```text
Provider Preset

Official Documentation Reference

Known Models

Compatibility Evidence

Tests
```

如果需要新 Adapter：

还必须解释：

```text
为什么 Generic Adapter 不足。
```

---

# 180. Adapter Contribution 要求

新增 Adapter 必须说明：

```text
Provider 特殊行为

为什么无法数据化

接口差异

安全影响

测试策略
```

否则不接受。

---

# 181. Provider 接入审核原则

维护者审核优先问：

```text
这个 Provider 真需要代码吗？

能不能只增加 Preset？

Capability 有证据吗？

Model ID 是官方的吗？

有没有错误把 Chat Completions 当 Responses？

有没有把 Provider 名称写进 Core Logic？
```

---

# 182. Model 接入审核原则

审核：

```text
Model 是否真实存在？

ID 是否准确？

Lifecycle 是否准确？

Capabilities 是否有依据？

是否错误继承同系列模型能力？

Compatibility 是否被夸大？
```

---

# 183. V0.1 Provider 不变量

### 不变量一

```text
Provider Preset 是数据，不执行代码。
```

### 不变量二

```text
同协议 Provider 优先共享 Generic Adapter。
```

### 不变量三

```text
Provider Name 不得进入核心条件判断。
```

### 不变量四

```text
Provider 连接成功不代表 Model Codex Compatible。
```

### 不变量五

```text
Provider Discovery 不代表 Model 已正式接入。
```

### 不变量六

```text
Custom Provider 默认 Compatibility = UNKNOWN。
```

### 不变量七

```text
未知 Capability 不得自动提升为 Supported。
```

---

# 184. V0.1 Model 不变量

### 不变量一

```text
Model 必须属于 Provider。
```

### 不变量二

```text
Model ID 与 Display Name 分离。
```

### 不变量三

```text
模型名称相似不能继承能力。
```

### 不变量四

```text
Capability 必须有明确来源。
```

### 不变量五

```text
Compatibility 与 Model Quality 分离。
```

### 不变量六

```text
Lifecycle 与 Compatibility 分离。
```

### 不变量七

```text
Deprecated Model 不得自动迁移。
```

### 不变量八

```text
User Override 必须保留其来源身份。
```

---

# 185. V0.1 接入矩阵

正式支持：

```text
                    Direct CAS    Codex Direct
                    Integration   Compatibility

DeepSeek V4 Flash      Yes           Yes*

Custom Responses       Yes           Depends

Chat Completions       No            No

Anthropic Messages     No            No

Gateway Provider       Reserved       Future
```

其中：

```text
Yes*
```

表示：

```text
具体 Model 仍以 Model Compatibility
和当前 Codex 能力验证为准。
```

---

# 186. Provider / Model 接入总体流程

完整流程：

```text
             Provider
                 │
                 ▼
        Protocol Identification
                 │
        ┌────────┴─────────┐
        │                  │
   Direct Responses      Other
        │                  │
        ▼                  ▼
 Generic Adapter     Gateway Required
        │
        ▼
 Provider Preset
        │
        ▼
 Connection Validation
        │
        ▼
 Model Discovery / Definition
        │
        ▼
 Capability Resolution
        │
        ▼
 Compatibility Verification
        │
        ▼
      Model Ready
        │
        ▼
   Available for Agent Binding
```

---

# 187. Provider 最终职责

Provider 接入层只回答：

```text
这个模型服务在哪里？

怎样连接？

怎样认证？

使用什么协议？

能发现哪些模型？

Provider 本身支持哪些能力？
```

它不回答：

```text
哪个 Agent 应该使用它？
```

---

# 188. Model 最终职责

Model 接入层只回答：

```text
模型是什么？

Model ID 是什么？

模型有什么能力？

生命周期是什么？

与 Codex 的兼容程度如何？

需要什么运行元数据？
```

它不回答：

```text
应该作为 Executor 还是 Reviewer。
```

---

# 189. Agent 绑定前的最终判断

只有：

```text
Provider Valid
        +
Model Available
        +
Protocol Compatible
        +
Required Capability Satisfied
        +
Codex Compatibility Acceptable
```

模型才可以进入：

```text
Ready for Agent Binding
```

状态。

---

# 190. 最终扩展目标

系统必须最终允许从：

```text
DeepSeek
    ↓
deepseek-v4-flash
```

自然扩展：

```text
Provider A
    ├── Model A
    └── Model B

Provider B
    ├── Model C
    └── Model D

Custom Responses
    └── Model X

CAS Gateway
    ├── Model Y
    └── Model Z
```

而核心调用逻辑始终保持：

```text
Agent
   ↓
Model
   ↓
Provider
   ↓
Adapter / Gateway
```

新增 Provider 的理想成本应该是：

```text
新增 Preset
+
新增 Model Definition
+
新增 Tests
```

而不是：

```text
修改 Agent
修改 Profile
修改 Configuration Core
修改 UI Core
增加大量 if/else
```

只有 Provider 存在真正特殊的运行行为时，才允许增加新的 Adapter。

这就是 CAS Provider / Model 接入体系长期可扩展性的核心。
