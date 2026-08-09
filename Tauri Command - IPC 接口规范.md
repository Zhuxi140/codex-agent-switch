# Codex Agent Switch Tauri Command / IPC 接口规范

> 文档类型：Tauri Command / IPC Interface Specification  
> 项目暂定名称：Codex Agent Switch  
> 简称：CAS  
> 当前目标版本：V0.1  
> 文档职责：定义 React UI 与 Rust Native Layer 之间的 Tauri IPC 调用契约，包括 Command 命名、请求/响应 DTO、统一错误模型、查询与修改边界、敏感数据传输规则、长任务状态、事件通知及接口演进原则。

---

# 0. V0.1 IPC 基线

V0.1 IPC 只暴露 Responses-first 通用 Use Case。DeepSeek V4 Flash Preset 与 Custom Responses Provider 共用 `provider_* / model_* / agent_* / configuration_*` 命令，不增加 `deepseek_*` Command。Profiles、Discovery、Runtime Probe / `model_verify`、Import / Adopt 在 P1 前不注册；其详细 DTO 章节保留为未来契约草案。

`app_get_bootstrap` 不含 `activeProfile`，明细仍按 list 命令按需加载。Windows 平台细节不泄漏到 DTO。

---

# 1. 文档定位

CAS 是：

```text
本地桌面应用
```

不存在：

```text
Spring Boot Backend
Node.js Server
localhost REST API
Remote CAS Server
```

因此本文中的“接口”专指：

```text
React UI
   │
   │ Tauri IPC
   ▼
Rust Native Layer
```

完整调用关系：

```text
React Component
      ↓
Frontend API Wrapper
      ↓
Tauri invoke()
      ↓
Tauri Command Handler
      ↓
Application Use Case
      ↓
Domain / Infrastructure
```

---

# 2. 接口层职责

Tauri Command 只负责：

```text
接收 IPC Request
        ↓
基础 DTO 解析
        ↓
调用 Application Layer
        ↓
转换 Response DTO
        ↓
转换 ApiError
```

不得负责：

```text
业务规则

SQLite SQL

Codex TOML Patch

Provider HTTP 实现

Secret Store 实现

Agent Compatibility 计算
```

---

# 3. Tauri Command 不是业务层

错误：

```rust
#[tauri::command]
async fn provider_create(request: CreateProviderRequest) {
    // validate provider
    // write sqlite
    // save api key
    // call deepseek
    // update config.toml
}
```

正确：

```text
Tauri Command
      ↓
CreateProviderUseCase
      ↓
Application / Domain
```

Command Handler 应保持：

```text
薄
稳定
无业务决策
```

---

# 4. IPC 不是 REST API

CAS 不使用：

```text
GET /providers
POST /agents
HTTP 200
HTTP 404
```

这样的服务端语义。

使用：

```text
provider_list
provider_create
agent_update
configuration_apply
```

等 Tauri Command。

因此本文不定义：

```text
HTTP Path
HTTP Method
HTTP Status Code
```

---

# 5. Command 命名规则

Rust Command 名称统一：

```text
<domain>_<action>
```

使用：

```text
snake_case
```

例如：

```text
provider_list

provider_get

provider_create

provider_update

provider_delete

agent_set_model_binding

profile_activate

configuration_apply
```

---

# 6. 禁止模糊 Command

禁止：

```text
execute

process

handle

save

update_data

do_action

manage_provider
```

必须从名称看出：

```text
操作哪个领域对象
+
执行什么动作
```

---

# 7. Query / Command 语义

IPC 分成两类。

## Query

只读取状态：

```text
provider_list

model_get

agent_list

configuration_get_status
```

Query 不产生业务副作用。

---

## Command

改变状态：

```text
provider_create

agent_update

profile_activate

configuration_apply
```

---

# 8. Query 不得偷偷修改状态

例如：

```text
diagnostics_run
```

虽然可能执行：

```text
文件读取
环境探测
网络检查
```

但默认不得：

```text
修复配置
修改数据库业务状态
修改 Codex 文件
```

Repair 必须拥有独立 Command。

---

# 9. Save 与 Apply 分离

CAS 必须保持：

```text
保存 CAS Desired State
≠
写入 Codex Configuration
```

例如：

```text
agent_set_model_binding
```

只修改 CAS 状态。

真正修改 Codex：

```text
configuration_apply
```

禁止：

```text
agent_update
   ↓
顺便写 agents/*.toml
```

---

# 10. Frontend API Wrapper

React 组件不得直接：

```typescript
invoke("provider_list")
```

Tauri 调用统一封装于：

```text
src/api/
```

例如：

```text
api/
├── appApi.ts
├── providerApi.ts
├── modelApi.ts
├── agentApi.ts
├── profileApi.ts
├── configurationApi.ts
├── diagnosticsApi.ts
├── snapshotApi.ts
└── settingsApi.ts
```

---

# 11. 前端调用示例

推荐：

```typescript
const providers = await providerApi.list();
```

而不是：

```typescript
const providers = await invoke("provider_list");
```

这样：

```text
组件
```

不依赖：

```text
Tauri Command 字符串
```

---

# 12. IPC DTO 与 Domain Entity 分离

Tauri IPC 使用专门：

```text
Request DTO
Response DTO
```

禁止直接把：

```text
Provider Domain Entity
Agent Domain Entity
SQLite Row
```

作为长期 IPC Contract。

关系：

```text
IPC Request
     ↓
Application Command
     ↓
Domain
     ↓
Application Result
     ↓
IPC Response
```

---

# 13. JSON 字段命名

IPC JSON 统一：

```text
camelCase
```

Rust DTO：

```rust
#[serde(rename_all = "camelCase")]
```

例如：

```json
{
  "providerId": "...",
  "modelId": "...",
  "createdAt": "..."
}
```

---

# 14. ID 类型

IPC 边界 UUID 使用：

```text
string
```

例如：

```typescript
type ProviderId = string;
```

Rust 收到后必须转换：

```text
String
↓
ProviderId
```

并进行格式验证。

---

# 15. 时间格式

IPC 时间统一：

```text
RFC3339 UTC
```

例如：

```text
2026-08-07T10:42:31Z
```

前端负责转换为：

```text
用户本地时间
```

---

# 16. Nullable

不存在或未知字段：

```text
null
```

不得通过：

```text
""
0
"unknown-value"
```

代替。

---

# 17. Enum

IPC 枚举使用稳定大写字符串。

例如：

```text
NATIVE
COMPATIBLE
GATEWAY_REQUIRED
UNSUPPORTED
UNKNOWN
```

TypeScript：

```typescript
type CompatibilityLevel =
  | "NATIVE"
  | "COMPATIBLE"
  | "GATEWAY_REQUIRED"
  | "UNSUPPORTED"
  | "UNKNOWN";
```

---

# 18. Enum 不依赖 Rust Variant 名称

Rust：

```rust
#[serde(rename = "GATEWAY_REQUIRED")]
GatewayRequired,
```

这样内部重构：

```text
GatewayRequired
```

不自动破坏 IPC 格式。

---

# 19. 通用响应原则

成功 Command：

```text
直接返回业务 Response DTO
```

不统一包：

```json
{
  "success": true,
  "data": {}
}
```

Tauri `Result` 本身已经表达：

```text
Success / Error
```

避免无意义 Envelope。

---

# 20. 成功示例

```json
{
  "id": "0198...",
  "providerKey": "deepseek",
  "name": "DeepSeek",
  "status": "READY"
}
```

---

# 21. 错误统一使用 ApiError

所有 Command Error 必须转换成：

```text
ApiError
```

逻辑结构：

```text
ApiError
├── code
├── message
├── details
├── retryable
└── correlationId
```

---

# 22. ApiError DTO

推荐：

```typescript
interface ApiError {
  code: string;
  message: string;
  details?: Record<string, unknown> | null;
  retryable: boolean;
  correlationId?: string | null;
}
```

---

# 23. `code`

机器可读稳定错误码。

例如：

```text
PROVIDER_NOT_FOUND

PROVIDER_AUTH_FAILED

MODEL_INCOMPATIBLE

AGENT_NAME_CONFLICT

CODEX_CONFIG_PARSE_ERROR

APPLY_CONFLICT
```

前端通过：

```text
code
```

判断错误类型。

不得通过匹配：

```text
message
```

实现业务判断。

---

# 24. `message`

提供：

```text
安全
可理解
可直接展示
```

的默认消息。

例如：

```text
The provider credential is missing.
```

如果应用实现 i18n，前端可以根据：

```text
code
```

使用本地化消息。

---

# 25. `details`

只允许放：

```text
安全的结构化诊断信息
```

例如：

```json
{
  "providerId": "0198...",
  "httpStatus": 401
}
```

禁止：

```text
API Key
Authorization Header
完整 Stack Trace
完整 Request
Secret Store Value
```

---

# 26. `retryable`

用于告诉 UI：

```text
当前错误是否可能通过重试解决
```

例如：

```text
PROVIDER_RATE_LIMITED
→ true

AGENT_NAME_CONFLICT
→ false
```

仅用于 UX。

不得用来：

```text
自动无限重试
```

---

# 27. `correlationId`

复杂操作：

```text
Apply
Restore
Provider Probe
```

可以返回：

```text
correlationId
```

用于日志关联。

不得包含 Secret。

---

# 28. Rust Command 返回类型

统一：

```rust
Result<T, ApiError>
```

例如：

```rust
#[tauri::command]
async fn provider_get(
    ...
) -> Result<ProviderDetailResponse, ApiError>
```

---

# 29. 禁止 Command 返回裸 String Error

禁止：

```rust
Result<T, String>
```

正式接口。

否则：

```text
前端无法稳定识别错误类别。
```

---

# 30. Bootstrap 接口

应用启动需要一个轻量：

```text
app_get_bootstrap
```

用于获得全局基础状态。

不得返回：

```text
完整 Provider
完整 Agent
完整 Model
全部数据库内容
```

---

# 31. `app_get_bootstrap`

Request：

```text
None
```

Response：

```typescript
interface AppBootstrapResponse {
  appVersion: string;
  ipcSchemaVersion: number;

  codex: CodexEnvironmentSummary;

  configurationStatus: ConfigurationStatus;

  runningOperationId: string | null;

  recoveryRequired: boolean;
}
```

---

# 32. Bootstrap 职责

只解决：

```text
应用能否正常进入

Codex 是否检测到

当前有没有 Recovery 阻塞

当前配置是否 Pending
```

其他页面自行查询。

---

# 33. IPC Schema Version

返回：

```text
ipcSchemaVersion
```

用于开发和诊断。

V0.1：

```text
1
```

但由于：

```text
React 与 Rust 随同一个 Desktop 安装包发布
```

不需要每一个 Request 都携带 API Version。

---

# 34. Codex Environment Command

提供：

```text
codex_get_environment
```

用于查看更完整环境信息。

Response：

```typescript
interface CodexEnvironmentResponse {
  detected: boolean;
  executablePath: string | null;
  codexHome: string | null;
  version: string | null;
  supported: boolean;
  configurationReadable: boolean;
  configurationWritable: boolean;
  multiAgentAvailable: boolean;
  issues: DiagnosticIssue[];
}
```

---

# 35. Codex 重新检测

显式：

```text
codex_redetect
```

用于用户修改：

```text
Codex 安装
CODEX_HOME
```

后重新识别。

不得使用：

```text
settings_save
```

暗中执行复杂检测。

---

# 36. Provider Command 集合

V0.1：

```text
provider_list

provider_get

provider_create

provider_update

provider_set_enabled

provider_delete

provider_test_connection

provider_replace_credential

provider_remove_credential
```

---

# 37. `provider_list`

Request：

```typescript
interface ProviderListRequest {
  search?: string | null;
  enabled?: boolean | null;
}
```

Response：

```typescript
interface ProviderSummary {
  id: string;
  providerKey: string;
  name: string;
  providerType: string;
  protocol: string;
  enabled: boolean;
  status: ProviderStatus;
  credentialStatus: CredentialStatus;
  modelCount: number;
}
```

---

# 38. Provider List 不返回

禁止返回：

```text
Credential Value
完整 Model 数组
完整 Metadata
完整 Check History
```

避免列表接口膨胀。

---

# 39. `provider_get`

Request：

```typescript
interface ProviderGetRequest {
  providerId: string;
}
```

Response：

```typescript
interface ProviderDetailResponse {
  id: string;
  providerKey: string;
  name: string;
  providerType: string;

  baseUrl: string;
  protocol: string;
  authStrategy: AuthStrategy;

  enabled: boolean;
  source: string;
  presetId: string | null;

  credentialStatus: CredentialStatus;

  modelCount: number;

  lastCheck: ProviderCheckSummary | null;

  createdAt: string;
  updatedAt: string;
}
```

---

# 40. Provider Detail 不返回 Secret

即使 Provider Credential 已配置，也只能返回：

```text
CONFIGURED
MISSING
STORE_UNAVAILABLE
```

不能返回：

```text
secret
maskedSecret
lastFour
```

---

# 41. `provider_create`

Request：

```typescript
interface ProviderCreateRequest {
  providerKey: string;
  name: string;

  presetId?: string | null;

  baseUrl: string;
  protocol: ProviderProtocol;

  auth: ProviderAuthInput;

  enabled: boolean;
}
```

---

# 42. ProviderAuthInput

V0.1：

```typescript
type ProviderAuthInput =
  | {
      strategy: "OS_SECRET_HELPER";
      secret: string;
    }
  | {
      strategy: "EXTERNAL_ENV";
      envKey: string;
    }
  | {
      strategy: "NONE";
    };
```

---

# 43. Sensitive Request

`provider_create` 在：

```text
OS_SECRET_HELPER
```

模式下包含 Secret。

因此该 Command：

```text
不得记录完整 request
不得 Debug DTO
不得持久化 IPC payload
```

---

# 44. Provider Create 响应

返回：

```text
ProviderDetailResponse
```

不得回显：

```text
request.auth.secret
```

---

# 45. `provider_update`

只修改非 Secret Provider 属性。

Request：

```typescript
interface ProviderUpdateRequest {
  providerId: string;
  name: string;
  baseUrl: string;
  enabled: boolean;
}
```

Credential 修改不得混入普通：

```text
provider_update
```

---

# 46. Provider Endpoint Origin Change

如果：

```text
baseUrl Origin
```

发生变化，这是安全敏感修改。

Application Layer 可以返回：

```text
PROVIDER_ORIGIN_CONFIRMATION_REQUIRED
```

或者要求 Request 明确：

```typescript
confirmOriginChange: true
```

具体业务规则由安全 / 业务规则文档控制。

---

# 47. `provider_set_enabled`

Request：

```typescript
interface ProviderSetEnabledRequest {
  providerId: string;
  enabled: boolean;
}
```

Response：

```text
ProviderDetailResponse
```

---

# 48. `provider_delete`

Request：

```typescript
interface ProviderDeleteRequest {
  providerId: string;
}
```

Response：

```typescript
interface DeleteResult {
  deleted: boolean;
}
```

如果存在引用：

返回：

```text
PROVIDER_IN_USE
```

及安全 details：

```json
{
  "agentCount": 1,
  "profileCount": 2
}
```

---

# 49. 删除接口不使用 `force`

V0.1 禁止：

```typescript
force: true
```

这种绕过业务关系的通用参数。

必须先解决引用。

---

# 50. `provider_test_connection`

Request：

```typescript
interface ProviderTestConnectionRequest {
  providerId: string;
}
```

Response：

```typescript
interface ProviderConnectionTestResponse {
  status:
    | "SUCCESS"
    | "AUTH_FAILED"
    | "UNREACHABLE"
    | "PROTOCOL_ERROR"
    | "RATE_LIMITED"
    | "SERVER_ERROR";

  latencyMs: number | null;

  providerRequestId: string | null;

  modelDiscoveryAvailable: boolean;
}
```

---

# 51. Connection Test 使用正式 Credential

`provider_test_connection` 必须使用：

```text
当前 Provider 已保存 Credential Strategy
```

不能接收：

```text
apiKey
secret
```

作为普通测试参数。

这样：

```text
测试配置
=
实际运行配置
```

---

# 52. `provider_replace_credential`

Request：

```typescript
interface ProviderReplaceCredentialRequest {
  providerId: string;
  secret: string;
}
```

Response：

```typescript
interface CredentialMutationResponse {
  credentialStatus: CredentialStatus;
  updatedAt: string;
}
```

---

# 53. Replace Credential 不返回 Secret

任何结果不得包含：

```text
secret
oldSecret
newSecret
```

---

# 54. `provider_remove_credential`

Request：

```typescript
interface ProviderRemoveCredentialRequest {
  providerId: string;
}
```

Response：

```typescript
CredentialMutationResponse
```

Provider 本身可以继续存在：

```text
Credential Missing
```

---

# 55. `provider_discover_models`（P1 草案）

Request：

```typescript
interface ProviderDiscoverModelsRequest {
  providerId: string;
}
```

Response：

```typescript
interface ModelDiscoveryResponse {
  providerId: string;
  models: DiscoveredModel[];
  discoveredAt: string;
}
```

---

# 56. DiscoveredModel

```typescript
interface DiscoveredModel {
  modelId: string;
  displayName: string | null;
  knownToCas: boolean;
  compatibility: CompatibilityLevel;
}
```

Discovery 不自动创建正式 Model Entity。

---

# 57. Model Command 集合

V0.1：

```text
model_list

model_get

model_add

model_update

model_set_enabled

model_delete

```

---

# 58. `model_list`

Request：

```typescript
interface ModelListRequest {
  search?: string | null;
  providerId?: string | null;
  enabled?: boolean | null;
  compatibility?: CompatibilityLevel | null;
}
```

Response：

```typescript
interface ModelSummary {
  id: string;
  providerId: string;
  providerName: string;

  modelId: string;
  displayName: string;

  enabled: boolean;
  lifecycle: ModelLifecycle;

  compatibility: CompatibilityLevel;
  contextWindow: number | null;
}
```

---

# 59. `model_get`

Response 应包含：

```text
核心 Metadata
Capability
Compatibility
Verification Summary
```

但不包含完整 Provider Credential。

---

# 60. Model Detail DTO

```typescript
interface ModelDetailResponse {
  id: string;

  provider: {
    id: string;
    name: string;
  };

  modelId: string;
  displayName: string;

  enabled: boolean;
  lifecycle: ModelLifecycle;

  contextWindow: number | null;
  maxOutputTokens: number | null;

  reasoning: {
    status: CapabilityStatus;
    supportedEfforts: string[];
    defaultEffort: string | null;
  };

  capabilities: CapabilityResponse[];

  compatibility: ModelCompatibilityResponse;

  createdAt: string;
  updatedAt: string;
}
```

---

# 61. `model_add`

用于：

```text
手工把发现模型 / Custom Model
纳入 CAS 管理
```

Request：

```typescript
interface ModelAddRequest {
  providerId: string;
  modelId: string;
  displayName?: string | null;
}
```

默认：

```text
unknown metadata
→ UNKNOWN
```

不能由 UI 随意声明：

```text
NATIVE
```

---

# 62. `model_update`

V0.1 只允许更新用户可编辑字段。

例如：

```typescript
interface ModelUpdateRequest {
  modelId: string;
  displayName: string;
}
```

兼容性等系统字段必须通过：

```text
Verification / Metadata Resolution
```

更新。

---

# 63. `model_set_enabled`

Request：

```typescript
interface ModelSetEnabledRequest {
  modelId: string;
  enabled: boolean;
}
```

---

# 64. `model_delete`

存在：

```text
Agent Binding
Profile Binding
```

时：

返回：

```text
MODEL_IN_USE
```

不得提供通用 Force Delete。

---

# 65. `model_verify`（P1 草案）

Request：

```typescript
interface ModelVerifyRequest {
  modelId: string;
  level:
    | "BASIC"
    | "TOOL_CALL";
}
```

V0.1 不让前端任意构造：

```text
Probe Prompt
Probe Tool
```

验证方案由 Rust Provider Integration 控制。

---

# 66. Model Verify Response

```typescript
interface ModelVerifyResponse {
  modelId: string;
  status: "PASSED" | "FAILED" | "PARTIAL";

  compatibility: CompatibilityLevel;

  capabilities: CapabilityResponse[];

  verifiedAt: string;

  issues: DiagnosticIssue[];
}
```

---

# 67. Agent Command 集合

V0.1：

```text
agent_list

agent_get

agent_create

agent_update

agent_set_enabled

agent_set_model_binding

agent_remove_model_binding

agent_delete
```

---

# 68. `agent_list`

Response：

```typescript
interface AgentSummary {
  id: string;
  agentKey: string;
  name: string;
  description: string;

  enabled: boolean;

  model: AgentModelReference | null;

  availability: AgentAvailability;
  reasoningPolicy: string;
}
```

---

# 69. Agent List 不返回完整 Instructions

Agent Instructions 可能较长。

只在：

```text
agent_get
```

返回。

---

# 70. `agent_get`

Response：

```typescript
interface AgentDetailResponse {
  id: string;
  agentKey: string;
  name: string;

  description: string;
  instruction: string;

  agentType: string;
  enabled: boolean;

  sandboxPolicy: string;
  reasoningPolicy: string;

  requiredCapabilities: string[];
  preferredCapabilities: string[];

  modelBinding: AgentModelReference | null;

  compatibility: AgentBindingCompatibility;

  source: string;
  managed: boolean;

  createdAt: string;
  updatedAt: string;
}
```

---

# 71. `agent_create`

Request：

```typescript
interface AgentCreateRequest {
  agentKey: string;
  name: string;
  description: string;
  instruction: string;

  templateKey?: string | null;

  enabled: boolean;

  sandboxPolicy: string;
  reasoningPolicy: string;

  modelId?: string | null;
}
```

---

# 72. Template 与请求

如果使用：

```text
templateKey
```

Application Layer 根据 Template 生成默认值。

如果用户已经明确覆盖：

```text
description
instruction
```

以用户最终提交值为准。

不要让 Tauri Handler 自己读取 Preset。

---

# 73. `agent_update`

允许修改：

```text
name
description
instruction
sandboxPolicy
reasoningPolicy
```

如果修改：

```text
agentKey
```

属于 Identity Change。

V0.1 推荐不通过普通：

```text
agent_update
```

修改 Agent Key。

---

# 74. Agent Key Rename

如果 V0.1 暂不支持：

返回：

```text
AGENT_KEY_IMMUTABLE
```

比假装普通字段更新更安全。

未来如支持，应独立：

```text
agent_rename_key
```

---

# 75. `agent_set_model_binding`

Request：

```typescript
interface AgentSetModelBindingRequest {
  agentId: string;
  modelId: string;
}
```

Response：

```typescript
interface AgentBindingResponse {
  agentId: string;
  model: AgentModelReference;
  compatibility: AgentBindingCompatibility;
}
```

---

# 76. Binding Compatibility

```typescript
interface AgentBindingCompatibility {
  status:
    | "COMPATIBLE"
    | "WARNING"
    | "INCOMPATIBLE"
    | "UNKNOWN";

  issues: CompatibilityIssueResponse[];
}
```

---

# 77. Incompatible Binding

如果模型明确：

```text
INCOMPATIBLE
```

Application 应拒绝保存或者按业务规则处理。

UI 不负责自行判断。

---

# 78. `agent_remove_model_binding`

Request：

```typescript
interface AgentRemoveModelBindingRequest {
  agentId: string;
}
```

用于让 Agent 进入：

```text
Needs model
```

状态。

---

# 79. `agent_delete`

存在：

```text
Profile 引用
```

时返回：

```text
AGENT_IN_USE
```

不允许 Force。

---

# 80. Profile Command 集合（P1 草案）

Profile 功能进入 P1 后：

```text
profile_list

profile_get

profile_create

profile_update

profile_duplicate

profile_replace_bindings

profile_activate

profile_delete
```

---

# 81. `profile_list`

Response：

```typescript
interface ProfileSummary {
  id: string;
  profileKey: string;
  name: string;
  description: string | null;

  active: boolean;

  status: ProfileStatus;

  agentCount: number;
}
```

---

# 82. `profile_get`

Response：

```typescript
interface ProfileDetailResponse {
  id: string;
  profileKey: string;
  name: string;
  description: string | null;
  active: boolean;

  bindings: ProfileAgentBindingResponse[];

  status: ProfileStatus;

  createdAt: string;
  updatedAt: string;
}
```

---

# 83. Profile Binding DTO

```typescript
interface ProfileAgentBindingResponse {
  agentId: string;
  agentKey: string;
  agentName: string;

  modelId: string;
  modelIdentifier: string;
  modelDisplayName: string;

  providerId: string;
  providerName: string;

  enabled: boolean;

  reasoningOverride: string | null;
  sandboxOverride: string | null;

  compatibility: AgentBindingCompatibility;
}
```

---

# 84. `profile_create`

Request：

```typescript
interface ProfileCreateRequest {
  profileKey: string;
  name: string;
  description?: string | null;

  bindings: ProfileAgentBindingInput[];
}
```

---

# 85. Profile Binding Input

```typescript
interface ProfileAgentBindingInput {
  agentId: string;
  modelId: string;

  enabled: boolean;

  reasoningOverride?: string | null;
  sandboxOverride?: string | null;
}
```

---

# 86. `profile_replace_bindings`

Profile Binding 作为一个集合进行：

```text
Replace
```

比暴露：

```text
profile_binding_create
profile_binding_update
profile_binding_delete
```

更符合：

```text
Profile Aggregate
```

的一致性边界。

---

# 87. Replace Bindings Request

```typescript
interface ProfileReplaceBindingsRequest {
  profileId: string;
  bindings: ProfileAgentBindingInput[];
}
```

必须在一个数据库事务中完成。

---

# 88. `profile_activate`

Request：

```typescript
interface ProfileActivateRequest {
  profileId: string;
}
```

Response：

```typescript
interface ProfileActivateResponse {
  activeProfileId: string;
  configurationStatus: ConfigurationStatus;
}
```

---

# 89. Profile Activate 不 Apply

必须保持：

```text
profile_activate
```

只改变：

```text
CAS Desired State
```

响应通常：

```text
configurationStatus = PENDING_CHANGES
```

不能内部调用：

```text
configuration_apply
```

---

# 90. `profile_duplicate`

Request：

```typescript
interface ProfileDuplicateRequest {
  profileId: string;
  newProfileKey: string;
  newName: string;
}
```

Application 层复制：

```text
Profile
+
Bindings
```

---

# 91. Configuration Command 集合

V0.1：

```text
configuration_get_status

configuration_preview_apply

configuration_apply
```

以及：

```text
snapshot_list
snapshot_get
snapshot_restore
```

---

# 92. `configuration_get_status`

Response：

```typescript
type ConfigurationStatus =
  | "APPLIED"
  | "PENDING_CHANGES"
  | "DRIFT"
  | "CONFLICT"
  | "RECOVERY_REQUIRED"
  | "UNAVAILABLE";
```

详细：

```typescript
interface ConfigurationStatusResponse {
  status: ConfigurationStatus;

  desiredStateHash: string | null;
  lastAppliedAt: string | null;

  driftCount: number;
  conflictCount: number;

  restartRecommended: boolean;

  issues: DiagnosticIssue[];
}
```

---

# 93. 不返回敏感 Hash 内容

Hash：

```text
只是 fingerprint
```

可以返回给 Developer UI。

不得通过它反推出：

```text
配置正文。
```

---

# 94. `configuration_preview_apply`

目的：

```text
展示逻辑变化
+
验证当前是否可 Apply
```

Request：

```text
None
```

Response：

```typescript
interface ConfigurationApplyPreview {
  desiredStateHash: string;

  changes: ConfigurationChange[];

  blockers: DiagnosticIssue[];
  warnings: DiagnosticIssue[];

  hasChanges: boolean;
}
```

---

# 95. ConfigurationChange

```typescript
interface ConfigurationChange {
  operation: "CREATE" | "UPDATE" | "DELETE";

  resourceType:
    | "CODEX_PROVIDER"
    | "CODEX_AGENT"
    | "MODEL_CATALOG";

  logicalKey: string;

  summary: string;
}
```

普通 Preview 不要求返回完整 TOML。

---

# 96. Developer Diff

如果未来需要：

```text
Show configuration diff
```

应使用独立：

```text
configuration_get_redacted_diff
```

并确保：

```text
Sensitive Data Redacted
```

不要让普通 Preview 自动返回完整文件内容。

---

# 97. Preview 不是 Apply Plan Token

重要：

```text
Preview
```

只是：

```text
预览
```

不得把它直接保存下来然后执行。

Apply 必须：

```text
重新读取当前 Codex 磁盘状态
重新做 Conflict Detection
重新 Compile Plan
```

---

# 98. `configuration_apply`

Request：

```typescript
interface ConfigurationApplyRequest {
  expectedDesiredStateHash?: string | null;
}
```

其中：

```text
expectedDesiredStateHash
```

用于避免：

```text
用户 Preview 后
CAS Desired State 又发生变化
```

却仍然以为自己 Apply 的是刚刚看到的方案。

---

# 99. Apply 不接收 Plan

禁止：

```typescript
configuration_apply({
  operations: [...]
})
```

让 UI 告诉 Rust：

```text
写哪些 TOML
```

Configuration Plan 必须由 Rust：

```text
根据正式 Desired State
```

自行编译。

---

# 100. Apply Response

```typescript
interface ConfigurationApplyResponse {
  transactionId: string;

  status:
    | "APPLIED"
    | "NO_CHANGES"
    | "FAILED_ROLLED_BACK"
    | "RECOVERY_REQUIRED";

  snapshotId: string | null;

  appliedAt: string | null;

  changedResourceCount: number;

  restartRecommended: boolean;

  warnings: DiagnosticIssue[];
}
```

---

# 101. Apply Conflict

如果磁盘状态冲突：

Command 应返回：

```text
ApiError
code = APPLY_CONFLICT
```

而不是：

```text
status = FAILED
```

将结构性冲突混入普通执行失败。

---

# 102. Apply 失败但 Rollback 成功

可以返回正常业务结果：

```text
FAILED_ROLLED_BACK
```

或者统一 ApiError。

V0.1 推荐：

```text
操作未达到用户目标
→ ApiError
```

即：

```text
APPLY_FAILED_ROLLED_BACK
```

details 可以包含：

```text
snapshotId
transactionId
```

---

# 103. Recovery Required

返回：

```text
RECOVERY_REQUIRED
```

属于高优先级错误。

之后所有新的：

```text
configuration_apply
snapshot_restore
```

是否允许执行，应由 Recovery 规则控制。

---

# 104. 全局 Discard Pending

V0.1 暂不定义：

```text
configuration_discard_pending
```

原因：

当前设计中：

```text
CAS Database
```

保存的是当前 Desired State，

但没有保存一整套：

```text
Last Applied Domain State
```

用于完整反向恢复 Provider / Agent / Profile。

因此 UI 如果保留：

```text
Discard Pending Changes
```

必须先在业务规则和数据模型中定义：

```text
到底恢复哪些 CAS Domain 数据。
```

在此之前不得实现一个语义模糊的：

```text
discard()
```

接口。

---

# 105. Snapshot Command

V0.1：

```text
snapshot_list

snapshot_get

snapshot_restore
```

---

# 106. `snapshot_list`

Request：

```typescript
interface SnapshotListRequest {
  limit?: number;
  cursor?: string | null;
}
```

Response：

```typescript
interface SnapshotListResponse {
  items: SnapshotSummary[];
  nextCursor: string | null;
}
```

---

# 107. Snapshot Summary

```typescript
interface SnapshotSummary {
  id: string;
  reason: string;
  codexVersion: string | null;

  status: string;

  createdAt: string;

  resourceCount: number;
}
```

---

# 108. `snapshot_get`

返回：

```text
Manifest
```

但默认不返回：

```text
完整文件内容
```

Response：

```typescript
interface SnapshotDetailResponse {
  id: string;
  reason: string;
  status: string;

  codexHome: string;
  codexVersion: string | null;

  resources: SnapshotResourceResponse[];

  createdAt: string;
}
```

---

# 109. Snapshot Path

普通 UI Response 尽量不需要暴露：

```text
snapshot filesystem path
```

Developer Details 如确有需要再提供。

---

# 110. `snapshot_restore`

Request：

```typescript
interface SnapshotRestoreRequest {
  snapshotId: string;
}
```

Restore 不接收：

```text
文件路径
```

防止 UI 指定任意文件系统目标。

---

# 111. Restore Response

```typescript
interface SnapshotRestoreResponse {
  transactionId: string;

  restoredSnapshotId: string;

  restoredAt: string;

  configurationStatus: ConfigurationStatus;

  warnings: DiagnosticIssue[];
}
```

---

# 112. Diagnostics Command

V0.1：

```text
diagnostics_run
```

Request：

```typescript
interface DiagnosticsRunRequest {
  includeNetworkChecks: boolean;
}
```

---

# 113. 网络诊断显式控制

```text
includeNetworkChecks = false
```

时：

不得读取 Provider Secret 发起远程请求。

只检查：

```text
Codex
Filesystem
Database
ManagedResource
Credential existence
Configuration
Compatibility metadata
```

---

# 114. Diagnostics Response

```typescript
interface DiagnosticsResponse {
  overall:
    | "HEALTHY"
    | "WARNING"
    | "ERROR";

  sections: DiagnosticSection[];

  checkedAt: string;
}
```

---

# 115. DiagnosticSection

```typescript
interface DiagnosticSection {
  key: string;
  title: string;
  issues: DiagnosticIssue[];
}
```

---

# 116. DiagnosticIssue

统一：

```typescript
interface DiagnosticIssue {
  code: string;

  severity:
    | "INFO"
    | "WARNING"
    | "ERROR";

  message: string;

  entityType?: string | null;
  entityId?: string | null;

  action?: DiagnosticAction | null;
}
```

---

# 117. Diagnostic Action

Diagnostics 可以告诉 UI：

```text
问题应该跳到哪里修复
```

例如：

```typescript
interface DiagnosticAction {
  type:
    | "OPEN_PROVIDER"
    | "OPEN_AGENT"
    | "OPEN_SETTINGS"
    | "RUN_REPAIR";

  targetId?: string | null;
}
```

---

# 118. Repair 不通过 Diagnostics 隐式执行

如果未来存在：

```text
configuration_repair_resource
```

必须独立调用。

不能：

```text
diagnostics_run()
```

内部修复。

---

# 119. Settings Command

V0.1：

```text
settings_get

settings_update
```

---

# 120. `settings_get`

Response 示例：

```typescript
interface SettingsResponse {
  appearance:
    | "SYSTEM"
    | "LIGHT"
    | "DARK";

  autoBackupEnabled: boolean;

  updateChannel: string;

  customCodexHome: string | null;
}
```

注意：

```text
Appearance
```

如果最终由 WebView / OS 设置独立管理，可以不通过业务 SQLite。

以实际应用设计为准。

---

# 121. `settings_update`

只允许更新白名单字段。

禁止：

```typescript
Record<string, unknown>
```

让前端任意写：

```text
application_settings
```

---

# 122. Settings Request

```typescript
interface SettingsUpdateRequest {
  appearance?: "SYSTEM" | "LIGHT" | "DARK";

  autoBackupEnabled?: boolean;

  updateChannel?: string;

  customCodexHome?: string | null;
}
```

---

# 123. Raw Setting Command 禁止

禁止：

```text
setting_set(key, value)
```

作为前端公共接口。

这样会绕过：

```text
类型
校验
业务边界
```

---

# 124. Preset Query

UI 添加 Provider / Agent 时需要 Preset。

提供：

```text
provider_preset_list

agent_preset_list
```

这两个都是：

```text
Read Only
```

---

# 125. `provider_preset_list`

不返回：

```text
任意 executable
Secret
内部 Adapter implementation detail
```

只返回 UI 创建 Provider 所需数据。

---

# 126. `agent_preset_list`

返回：

```typescript
interface AgentPresetResponse {
  key: string;
  name: string;
  description: string;

  defaultSandboxPolicy: string;
  defaultReasoningPolicy: string;

  requiredCapabilities: string[];
}
```

完整 Instructions 是否展示取决于 UI。

创建时后端仍以正式 Preset 为源。

---

# 127. 不让 UI 成为 Preset Source of Truth

即使 UI 已拿到：

```text
Agent Preset
```

创建 Agent 时 Rust 仍需根据：

```text
templateKey
```

读取正式 Preset。

不能完全信任：

```text
UI 回传的 Preset 内容。
```

---

# 128. Sensitive IPC

以下 Command 属于：

```text
Sensitive IPC
```

至少：

```text
provider_create
provider_replace_credential
```

只要 Request 中可能存在 Secret，就必须：

```text
skip request tracing
禁止 Debug dump
禁止 persistence
```

---

# 129. 不提供 Credential Get

V0.1 明确不存在：

```text
credential_get

provider_get_api_key

secret_read
```

这样的 Tauri Command。

---

# 130. 不提供通用 Filesystem Command

禁止：

```text
file_read(path)

file_write(path, content)

directory_delete(path)
```

暴露给 React。

所有文件操作必须属于：

```text
明确 Application Use Case。
```

---

# 131. 不提供通用 Shell Command

禁止：

```text
shell_execute(command)
```

暴露给 UI。

如果未来需要：

```text
Open Config Directory
```

实现专用：

```text
system_open_codex_directory
```

而不是开放任意 Shell。

---

# 132. Native Dialog

文件选择器等系统 Dialog 可以由 Tauri 能力提供。

但选择结果仍需：

```text
后端业务校验
```

不能因为路径来自系统 Dialog 就默认安全。

---

# 133. Command Registration

所有 Command 必须：

```text
显式注册
```

禁止动态：

```text
command name → reflection execution
```

---

# 134. Command Module

推荐：

```text
src-tauri/src/commands/
├── app.rs
├── codex.rs
├── provider.rs
├── model.rs
├── agent.rs
├── profile.rs
├── configuration.rs
├── diagnostics.rs
├── snapshot.rs
└── settings.rs
```

---

# 135. Command Handler 示例

```rust
#[tauri::command]
pub async fn agent_set_model_binding(
    state: State<'_, AppState>,
    request: AgentSetModelBindingRequest,
) -> Result<AgentBindingResponse, ApiError> {
    let command = request.try_into()?;

    let result = state
        .agent_service
        .set_model_binding(command)
        .await
        .map_err(ApiError::from)?;

    Ok(result.into())
}
```

Handler 本身不得增加：

```text
模型兼容判断
数据库操作
```

---

# 136. AppState

Tauri State 可以保存：

```text
Application Services
Query Services
```

例如：

```rust
struct AppState {
    provider_service: Arc<ProviderApplicationService>,
    model_service: Arc<ModelApplicationService>,
    agent_service: Arc<AgentApplicationService>,
    profile_service: Arc<ProfileApplicationService>,
    configuration_service: Arc<ConfigurationApplicationService>,
}
```

---

# 137. AppState 不保存 UI State

禁止：

```text
currentSelectedAgent
currentForm
activeDialog
```

放进 Rust AppState。

---

# 138. Command 生命周期

Command 必须：

```text
单次调用
单次结果
```

不要设计：

```text
provider_create_begin
provider_create_step2
provider_create_commit
```

依赖 UI 维持隐式服务器 Session。

---

# 139. Long-running 操作

可能较长：

```text
configuration_apply

diagnostics_run(includeNetworkChecks=true)

model_verify

snapshot_restore
```

允许通过：

```text
Tauri Event
```

提供进度。

---

# 140. Operation ID

长任务开始时必须存在：

```text
operationId
```

用于：

```text
进度事件
日志
最终结果
```

关联。

---

# 141. Progress Event

统一事件：

```text
cas://operation-progress
```

Payload：

```typescript
interface OperationProgressEvent {
  operationId: string;

  operationType:
    | "CONFIGURATION_APPLY"
    | "DIAGNOSTICS"
    | "MODEL_VERIFY"
    | "SNAPSHOT_RESTORE";

  phase: string;

  completed: number;
  total: number | null;

  message: string | null;
}
```

---

# 142. Event 不承载业务最终结果

最终 Success / Failure：

仍然由：

```text
Command Result
```

返回。

Event 只用于：

```text
Progress
```

不能让 UI：

```text
只监听 Event 才知道 Apply 是否成功。
```

---

# 143. Progress 数字

如果无法准确知道总量：

```text
total = null
```

不要伪造：

```text
42%
```

---

# 144. State Changed Event

V0.1 默认不需要复杂全局 Event Bus。

因为：

```text
UI 发出 Mutation
→ Mutation 成功
→ 前端 invalidate 对应 Query
```

已经足够。

---

# 145. 多窗口

如果未来支持多个 Window，需要同步状态时可以增加：

```text
cas://state-changed
```

V0.1 不提前建立复杂事件系统。

---

# 146. Command 并发

普通 Query 可以并发。

Mutation 根据领域约束处理。

例如：

```text
provider_update
+
model_list
```

可以同时。

---

# 147. Apply 并发

同时只能存在：

```text
一个 Configuration Apply
```

第二个调用返回：

```text
APPLY_ALREADY_RUNNING
```

---

# 148. Restore 与 Apply

以下不能并发：

```text
configuration_apply

snapshot_restore
```

两者都属于：

```text
Codex Configuration Mutation
```

必须共享：

```text
Apply / Configuration Lock。
```

---

# 149. Diagnostics 与 Apply

只读 Diagnostics 可以与普通业务操作并发。

但如果：

```text
Diagnostics
```

需要扫描 Codex 配置，

可能看到：

```text
Apply 中间状态
```

因此 Configuration Layer 必须使用一致性锁或 Snapshot。

不要让 UI 自行解决竞态。

---

# 150. Provider Test 并发

允许不同 Provider 并行 Test。

同一 Provider 不需要强制阻止多个 Test，但应避免：

```text
无限并发请求。
```

Provider Layer 可以采用合理并发限制。

---

# 151. Double Click

所有 Mutation Command 必须能够处理：

```text
用户双击按钮
```

等重复调用。

UI 应 Disable 正在提交按钮，

Rust 侧仍不能完全依赖 UI。

---

# 152. Create 幂等

`provider_create` 不需要天然幂等。

重复 Provider Key：

返回：

```text
PROVIDER_KEY_ALREADY_EXISTS
```

而不是创建两个。

---

# 153. Enable / Disable 幂等

```text
provider_set_enabled(enabled=true)
```

Provider 已经 Enabled：

可以返回当前状态。

不需要 Error。

---

# 154. Apply 幂等

如果当前：

```text
Desired State
=
Applied State
```

`configuration_apply` 返回：

```text
NO_CHANGES
```

不得再次写文件或创建无意义 Snapshot。

---

# 155. Request Size

IPC Request 必须保持合理大小。

尤其：

```text
Agent Instructions
```

应设置防御性最大长度。

不得允许：

```text
数百 MB 字符串
```

通过 IPC。

---

# 156. String Validation

后端必须验证：

```text
Key
Name
URL
Instruction
Model ID
Environment Variable Name
```

长度和格式。

不能仅依赖前端。

---

# 157. Pagination

V0.1：

```text
Provider
Agent
Profile
```

数量少，可以一次返回。

Models 数量可能较多。

`model_list` 应支持：

```text
limit
cursor
```

或在确认数据规模后使用全部查询。

推荐预留：

```typescript
interface PageRequest {
  limit?: number;
  cursor?: string | null;
}
```

---

# 158. 不使用 Offset 作为长期外部契约

如果列表未来变大：

```text
Cursor Pagination
```

优于：

```text
offset = 10000
```

但 V0.1 本地模型数量有限，不需过度设计。

---

# 159. Search

搜索字符串：

```text
trim for query
```

但不要修改：

```text
Model ID
Provider Key
```

等真正业务输入。

---

# 160. Empty Search

```text
search = ""
```

等价：

```text
null
```

可以在 Query DTO 解析时统一处理。

---

# 161. Sorting

前端不得传任意 SQL Column。

如果需要排序：

```typescript
type ModelSort =
  | "DISPLAY_NAME"
  | "PROVIDER"
  | "COMPATIBILITY";
```

后端映射白名单。

---

# 162. Frontend Response 类型

前端不手写重复 DTO 多份。

建议：

```text
src/api/types/
```

统一维护 IPC Types。

如果项目后续采用自动生成 TypeScript Binding，可替换人工同步。

---

# 163. 自动生成 Binding

如果选用：

```text
specta
tauri-specta
```

或其他 Rust → TypeScript Binding 工具，

必须满足：

```text
成熟
与 Tauri 2 兼容
不会扩大 Runtime 权限
```

才引入。

V0.1 不强制。

---

# 164. 手工 DTO 同步

如果不自动生成：

任何 Rust IPC DTO 变化必须同步：

```text
TypeScript type
```

并通过：

```text
Type Check
Integration Test
```

验证。

---

# 165. Command 契约测试

每个重要 Command 至少测试：

```text
合法 Request

非法 Request

Not Found

Conflict

Infrastructure Failure

正确 Response Mapping
```

完整测试策略由测试规范定义。

---

# 166. Sensitive Command 测试

至少验证：

```text
Provider Create
Credential Replace
```

发生错误时：

```text
ApiError
logs
tracing
```

都不出现 Secret。

---

# 167. Command 不暴露 Stack Trace

Release：

```text
ApiError
```

不返回：

```text
Rust backtrace
SQL
OS raw stack
```

Developer Logs 可以保留内部错误链，但必须脱敏。

---

# 168. Not Found

例如：

```text
provider_get
```

Provider 不存在：

```text
PROVIDER_NOT_FOUND
```

而不是返回：

```text
null
```

因为：

```text
Get by explicit ID
```

语义要求存在。

---

# 169. List Empty

```text
provider_list
```

无结果：

返回：

```json
[]
```

不是：

```text
PROVIDER_NOT_FOUND
```

---

# 170. Delete Missing

V0.1 推荐显式：

```text
PROVIDER_NOT_FOUND
```

以避免 UI 误认为删除了其他状态。

如果未来需要幂等 CLI Delete，再在 Application Use Case 中明确设计。

---

# 171. Validation Error

统一：

```text
VALIDATION_ERROR
```

details 可以包含：

```json
{
  "fields": {
    "providerKey": "INVALID_FORMAT",
    "baseUrl": "INVALID_URL"
  }
}
```

---

# 172. Field Error 不返回任意英文

推荐结构：

```typescript
interface ValidationDetails {
  fields: Record<string, string>;
}
```

前端根据：

```text
error code
```

映射具体提示。

---

# 173. Conflict Error

例如：

```text
AGENT_NAME_CONFLICT
PROVIDER_KEY_ALREADY_EXISTS
APPLY_CONFLICT
```

必须独立于：

```text
VALIDATION_ERROR
```

---

# 174. Infrastructure Error

例如：

```text
DATABASE_UNAVAILABLE
SECRET_STORE_UNAVAILABLE
CODEX_CONFIG_NOT_WRITABLE
```

前端应能区分：

```text
业务输入问题
```

和：

```text
系统环境问题。
```

---

# 175. Error Retry

例如：

```text
PROVIDER_RATE_LIMITED
→ retryable = true

DATABASE_CORRUPTED
→ retryable = false
```

前端仍不得自动无限 Retry。

---

# 176. Cancellation

V0.1 不强制所有 Command 可取消。

可以优先支持：

```text
provider test
model verify
diagnostics network checks
```

如果需要取消，应独立设计：

```text
operation_cancel
```

---

# 177. 不通过关闭 Promise 实现取消

前端不再等待：

```text
Promise
```

不代表 Rust Task 已停止。

真正 Cancellation 必须由：

```text
CancellationToken
```

等 Native 机制实现。

---

# 178. V0.1 `operation_cancel`

如果实现：

```text
operation_cancel
```

Request：

```typescript
interface OperationCancelRequest {
  operationId: string;
}
```

只允许取消：

```text
明确可安全取消的操作。
```

---

# 179. Apply 不随意取消

一旦：

```text
configuration_apply
```

已经进入：

```text
Filesystem Mutation
```

不能简单终止。

必须：

```text
继续完成
或
进入 Rollback
```

因此 Apply 的 Cancel 支持不作为 V0.1 目标。

---

# 180. Timeout

Tauri IPC 本身不定义业务 Timeout。

外部操作由对应 Application / Infrastructure 设置：

```text
Provider HTTP Timeout

Helper Timeout

Process Timeout
```

UI 可以显示：

```text
Testing...
```

但不自行判定后端已经失败。

---

# 181. Provider Test Request 不携带 Timeout

不要让 UI：

```typescript
timeoutMs: 99999999
```

任意控制安全策略。

Timeout 属于：

```text
Native Configuration。
```

---

# 182. File Path 暴露原则

普通 Response 不暴露：

```text
所有内部路径
```

只有：

```text
Settings / Developer Details / Diagnostics
```

真正需要时返回。

---

# 183. Secret Store Backend

普通 Provider Response 可以返回：

```text
credentialStatus
```

不必返回：

```text
Windows Credential Manager
Keychain storage locator
```

这些属于高级诊断信息。

---

# 184. Internal IDs

`ManagedResourceId`

`ApplyOperationId`

等内部实现 ID：

只有实际 UI / Diagnostics 需要时才进入 IPC。

不因为数据库存在就全部暴露。

---

# 185. Raw Metadata

Model `metadata_json` 不直接：

```text
原样通过 IPC
```

普通 UI 需要什么字段就定义什么 Response。

Developer Details 如果需要 Raw Metadata，应使用独立接口或安全字段。

---

# 186. API 稳定性

由于：

```text
React UI
+
Rust Native
```

随同一版本发布，

IPC 不属于第三方公开 API。

因此允许：

```text
同一个 CAS Release 开发周期内
同步调整 Command 和 DTO。
```

---

# 187. 正式版本兼容

已发布版本升级时：

数据库负责 Migration。

IPC 因前后端一起升级：

```text
无需长期同时兼容多个旧 UI。
```

除非未来引入：

```text
独立 Web UI
第三方插件
外部 IPC Client
```

届时再正式 Version API。

---

# 188. 不提前设计 REST-like V1/V2

V0.1 不需要：

```text
provider_v1_create
provider_v2_create
```

等复杂版本体系。

保持：

```text
ipcSchemaVersion
+
同版本前后端同步
```

即可。

---

# 189. CLI 不通过 Tauri IPC

CLI：

```text
cas
```

应直接复用：

```text
Application Layer
```

而不是：

```text
CLI
→ 启动 Desktop
→ Tauri IPC
```

---

# 190. CLI 与 UI 一致性

即使调用入口不同：

```text
Desktop Tauri Command
```

和：

```text
CLI Command
```

最终都调用：

```text
同一个 Application Use Case。
```

因此：

```text
cas apply
```

和：

```text
GUI Apply
```

业务规则必须一致。

---

# 191. Helper 不通过 Tauri IPC

`cas-helper` 独立：

```text
Secret Store Reader
```

不启动 Desktop，也不调用 Tauri。

---

# 192. Rust Native 层最终接口关系

```text
                    React UI
                       │
                       ▼
                 Frontend API
                       │
                       ▼
                 Tauri Commands
                       │
                       ▼
              Application Services
                │       │       │
          ┌─────┘       │       └─────┐
          ▼             ▼             ▼
       Domain       Query Services   Use Cases
          │
          ▼
        Ports
          ▲
    ┌─────┼───────────────┐
    │     │               │
    ▼     ▼               ▼
 SQLite  Codex        Provider / Secret
```

---

# 193. V0.1 Command 总览

```text
Application
───────────
app_get_bootstrap


Codex
─────
codex_get_environment
codex_redetect


Provider
────────
provider_list
provider_get
provider_create
provider_update
provider_set_enabled
provider_delete
provider_test_connection
provider_replace_credential
provider_remove_credential


Provider Preset
───────────────
provider_preset_list


Model
─────
model_list
model_get
model_add
model_update
model_set_enabled
model_delete


Agent
─────
agent_list
agent_get
agent_create
agent_update
agent_set_enabled
agent_set_model_binding
agent_remove_model_binding
agent_delete


Agent Preset
────────────
agent_preset_list


Configuration
─────────────
configuration_get_status
configuration_preview_apply
configuration_apply


Snapshot
────────
snapshot_list
snapshot_get
snapshot_restore


Diagnostics
───────────
diagnostics_run


Settings
────────
settings_get
settings_update
```

---

# 194. V0.1 明确不提供的 Command

禁止：

```text
database_query

database_execute

file_read

file_write

shell_execute

command_execute

secret_get

secret_list

credential_get_value

config_write_raw

config_replace_all

toml_execute_patch

provider_request_raw

http_request
```

这些接口都会破坏：

```text
层级
安全
Ownership
业务约束
```

---

# 195. 前端职责不变量

React UI：

```text
可以：

调用业务 Command
展示状态
保存 Form Draft
执行前端格式校验
```

React UI：

```text
不能：

决定 Codex TOML 如何写
决定 Agent 是否真正 Compatible
读取 Secret
执行 SQL
执行 Shell
直接调用 Provider API
```

---

# 196. Tauri Command 不变量

## 不变量一

```text
Command Handler 必须保持薄。
```

## 不变量二

```text
Command 不直接执行 SQL。
```

## 不变量三

```text
Command 不直接修改 Codex 文件。
```

## 不变量四

```text
Command 不实现 Provider-specific HTTP。
```

## 不变量五

```text
Command 不保存业务规则。
```

## 不变量六

```text
所有外部 Request 都必须后端重新校验。
```

## 不变量七

```text
所有 Error 必须转换为稳定 ApiError。
```

## 不变量八

```text
Frontend 不通过 message 文本判断错误类型。
```

## 不变量九

```text
Secret 不得通过任何 Query Command 返回。
```

## 不变量十

```text
Sensitive Command payload 不得进入 tracing/logging。
```

## 不变量十一

```text
Save 与 Apply 必须保持独立 Command。
```

## 不变量十二

```text
Profile Activate 不得隐式 Apply。
```

## 不变量十三

```text
Apply 必须由 Rust 重新生成最新 Configuration Plan。
```

## 不变量十四

```text
UI 不得向 Apply 传 TOML 或具体文件操作。
```

## 不变量十五

```text
Diagnostics 默认只读。
```

## 不变量十六

```text
Restore 只能引用 Snapshot ID，
不能由 UI 指定任意目标文件。
```

## 不变量十七

```text
不得暴露通用 Filesystem / SQL / Shell IPC。
```

## 不变量十八

```text
CLI 和 GUI 必须复用同一 Application Layer。
```

---

# 197. Command 开发模板

新增一个 Command 时按以下顺序实现：

```text
1. 明确 Use Case

2. 判断它是 Query 还是 Mutation

3. 定义 Application Command / Query

4. 定义 IPC Request DTO

5. 定义 IPC Response DTO

6. 实现 DTO → Application 类型转换

7. 调用 Application Service

8. Application Result → Response DTO

9. Error → ApiError

10. 注册 Tauri Command

11. 创建 Frontend API Wrapper

12. 添加 TypeScript 类型

13. 添加契约测试

14. 检查 Sensitive Data

15. 检查是否绕过现有领域边界
```

---

# 198. 新增 Command 审核问题

每个新 Command 必须回答：

```text
这个操作为什么需要暴露给 UI？

现有 Command 是否已经能够表达？

这是业务操作还是底层实现泄漏？

UI 是否获得了不必要的文件系统能力？

UI 是否获得了不必要的 Secret 能力？

它是否绕过 Application Layer？

它是否应该是 Query？

它是否有隐藏副作用？

它是否需要幂等？

它是否可能与 Apply / Restore 冲突？
```

---

# 199. 典型错误设计一

错误：

```text
config_update_agent_file
```

因为它暴露：

```text
底层文件实现。
```

正确：

```text
agent_update
```

然后：

```text
configuration_apply
```

---

# 200. 典型错误设计二

错误：

```text
provider_call_api
```

让 UI 传：

```text
URL
Method
Headers
Body
```

正确：

```text
provider_test_connection

provider_discover_models

model_verify
```

暴露真实业务行为。

---

# 201. 典型错误设计三

错误：

```text
database_save_profile
```

正确：

```text
profile_create
profile_update
profile_activate
```

UI 不需要知道：

```text
Profile 存在 SQLite。
```

---

# 202. 典型错误设计四

错误：

```text
credential_get
```

然后前端把 API Key：

```text
再发给 Rust
```

正确：

```text
Provider 只返回 credentialStatus

真正 Secret
只在 Rust / helper 安全边界内使用。
```

---

# 203. 典型错误设计五

错误：

```text
configuration_apply(preview.operations)
```

正确：

```text
configuration_preview_apply()

↓

用户确认

↓

configuration_apply(expectedDesiredStateHash)

↓

Rust 重新编译最新 Plan
```

---

# 204. 典型错误设计六

错误：

```text
settings_set("anything", arbitraryJson)
```

正确：

```text
settings_update(TypedRequest)
```

---

# 205. 最终接口设计原则

CAS Tauri IPC 的目标不是：

```text
把 Rust 所有能力暴露给 React
```

而是：

> 只暴露 UI 完成产品行为真正需要的 Application Use Case。

因此最终应保持：

```text
UI
只知道：

Provider
Model
Agent
Configuration
Diagnostics
Settings
```

而不知道：

```text
SQLite Table

TOML AST

Windows Credential API

Atomic File Rename

SQLx Transaction

reqwest Client

Secret Store Locator

Codex Internal Config Mapper
```

接口层应该形成稳定边界：

```text
React
  │
  │ Typed Request
  ▼
Tauri Command
  │
  ▼
Application Use Case
  │
  ▼
Domain / Infrastructure
  │
  ▼
Typed Result
  │
  ▼
Tauri Command
  │
  │ Typed Response / ApiError
  ▼
React
```

即使未来内部：

```text
SQLite 实现改变

Codex 配置格式变化

Provider Adapter 重构

Secret Store 实现变化
```

只要产品语义没有改变：

```text
provider_create

agent_set_model_binding

configuration_apply
```

这些 IPC Use Case 就不应该因为基础设施变化而被迫整体重写。

这就是 CAS Tauri Command 层最核心的职责边界。
