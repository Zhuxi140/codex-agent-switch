# Codex Agent Switch（CAS）上下文

CAS 是管理 Codex Multi-Agent 模型配置的桌面工具：用户在「谁负责什么、用哪个模型、模型来自哪个 Provider」三个层面管理 Agent Team，而不直接编辑 Codex 的 TOML 配置。

## Language

**Agent**：
Codex 中的一个子代理角色（Executor、Explorer、Reviewer、Tester、Custom Agent），回答"谁负责做这件事"。`Agent.key` 与 Codex agent 的 `name` 恒等：生成 `cas-<key>.toml` 时强制 `name="<key>"`。
_Avoid_：Worker、DeepSeek Worker、gemini_reviewer（把模型写进角色名的叫法）

**displayName（展示名）**：
Agent 面向 UI 的名字，随用户修改，只是展示用途，不写入 Codex。定位词：显示名。
_Avoid_: 把展示名混同 Codex name 写入配置

**Model**：
Agent 实际使用的模型（如 DeepSeek V4 Flash）。
_Avoid_: 大模型、LLM（在需要精确指代时）

**Provider**：
提供模型调用的服务（如 DeepSeek、自定义 Responses 兼容服务），一个 Model 只属于一个 Provider。
_Avoid_: 供应商、服务商

**Direct Responses Provider（直连 Responses Provider）**：
原生提供 Codex Custom Provider 所需 `responses` wire protocol，并已通过 CAS 端到端验证的 Provider；V0.1 只允许此类 Provider 进入 Direct Apply 链路。
_Avoid_: 仅因接口风格接近 OpenAI 就称为 Responses-compatible、把 Chat Completions 兼容等同于 Responses 兼容

**Gateway Required（需要网关）**：
Provider / Model 当前不能直接满足 Codex `responses` wire protocol，必须经过明确的协议转换网关才能进入 Codex。DeepSeek V4 Flash 已原生支持 Responses，不属于此类；未获得相同官方证据的其他模型仍需独立判断。
_Avoid_: 用 `Compatible` 掩盖协议缺口、让 CAS 桌面端静默承担协议转换

**Profile**：
整套 Agent → Model 绑定组合（如 Balanced / Quality），回答"当前整套 Agent Team 使用什么模型组合"。
Profile 是 P1 领域能力；V0.1 只实现 Base Binding，不创建、激活或编译 Profile。

**Binding（绑定）**：
Agent 与 Model 之间的关联关系。

**Base Binding（基础绑定）**：
Agent 自带、独立于任何 Profile 的默认绑定。它是 Agent 的默认值，激活 Profile 时**不会被改写**。
_Avoid_: Current binding、生效绑定（会造成"激活 Profile 会改基础绑定"的误解）

**Profile Binding（Profile 绑定）**：
Profile 激活时对某个 Agent 基础绑定的覆盖。激活 Profile 只改变"当前生效的目标配置"，不代表写入 Codex。

**Draft**：
页面中的编辑尚未保存。

**Saved**：
CAS 已保存业务状态，但与 Codex 配置尚未同步。

**Applied State（已应用状态）**：
CAS 的目标状态已成功同步到 Codex。

**Desired State（目标状态）**：
CAS 内保存的"我们希望 Codex 变成的样子"。Codex 配置文件是它的投影（Projection），两者可能不一致。

**Pending Changes（待应用修改）**：
当 CAS 目标状态与 Codex 已应用状态不一致时存在的修改。

**Managed（受管）**：
由 CAS 创建并拥有所有权的资源（Provider、Agent、绑定等）。CAS 拥有基于指纹识别。
_Avoid_: 管理了的（歧义），CAS-owned（用于片段级所有权时）

**External（外部）**：
Codex 中已存在、但非 CAS 创建的资源。CAS 不得自动接管：只能查看、忽略、或用户显式 Import 纳入管理。（V0.1 不实现 Import 时，至少正确识别且不覆盖。）

**Externally Modified（外部修改）**：
已登记为 CAS-owned 的资源（Agent/Provider/文件）在 Codex 侧被用户改过。它不是 External——仍是 CAS 的资源，但进入冲突/拦截状态（如 `CAS_RESOURCE_EXTERNALLY_MODIFIED`），绝不自动覆盖。

**Semantic Fingerprint（语义指纹）**：
冲突判定的依据，按 CAS-owned 片段/文档的语义内容计算；空白、顺序无关、注释变化不构成冲突。字节级 `contentHash` 只用于诊断展示，不参与 Ownership 判定。注释 `# Managed by Codex Agent Switch` 只是人类提示，不参与 hash。

**CAS-owned**：
指 Codex 配置中属于 CAS 管理的片段（如 `model_providers.cas_xxx`、`agents/cas-*.toml`）。外部修改 CAS-owned 片段在全时视为冲突（Conflict），不静默覆盖。
_Avoid_: 我们的配置（太含糊）、CAS 的配置（与 External 混淆）

**Preset**：
随 CAS 分发的预置模板（Provider Preset、Agent Template、Model Definition），只读资源，实例化后才写入 CAS 数据。**模板不是实体**：Agent 模板始终留在 resources，用户显式创建时才实例化为 Agent；首启不会自动产生 Agent 实体。
_Avoid_：默认配置（容易与 Default Profile 混淆）、把模板当已有 Agent 展示

**CodexEnvironment（Codex 环境）**：
解析后的明确运行环境，含 `executablePath` 与 `codexHome`（CODEX_HOME）。executable 与配置环境是两个独立维度，最终收敛成一个 CodexEnvironment；多候选或歧义时由用户显式选择并持久化 override，不每次询问。

**Credential**：
Provider 的认证凭据（如 API Key）。明文密钥只存 OS 凭据库，只保留引用。

**cas-helper**：
随 CAS 安装的轻量二进制，Codex 按需调用以取 Token（用户不直接操作）。
**forbid-list（禁用清单）**：
- 禁止给 Provider 配置任意 `auth.command`（凭据命令）
- 禁止 `experimental_bearer_token` 明文
- 禁止明文 `.env` vault
- 禁止在 Secret Store 不可用时静默降级为明文（fail closed）
- 不碰 `auth.json` / ChatGPT OAuth

**Apply**：
将当前 CAS 目标状态同步到 Codex 的核心操作。Apply 前必须读盘、备份、冲突检测；结果为 Success / Failed / Partially Recovered / Conflict。

**Compatibility（兼容性）**：
Model 与 Codex 使用的匹配程度，取值有限枚举（Native / Compatible / Gateway Required / Unsupported / Unknown）；未知值必须保持 Unknown，不得为美观编造。**Provider 层不定义 Compatibility**——Provider 层只有接入方式（source / adapter / protocol）与集成就绪情况（Integration Readiness）；Codex Compatibility 主要属于 Model。

**Capability（能力）**：
模型能力（Responses、Tool Calling、Reasoning 等）。三态：Supported / Unsupported / Unknown，Unknown 不得升级。证据来源为单一 Domain 枚举：`OFFICIAL_PROVIDER / OFFICIAL_CODEX / CAS_BUILT_IN / RUNTIME_PROBE / USER_OVERRIDE / UNKNOWN`。Discovery 只描述 Model 的「存在/获取方式」，不是能力证据来源。

**Diagnostics（诊断）**：
检查并定位配置问题的能力集，默认只检查、不自动修改配置；若支持修复（Repair），必须独立、显式触发。

**Backup（备份）**：
Apply 前自动生成的全量配置快照，用户可显式触发 Restore（恢复）。恢复操作本身也应先保护当前状态再执行。恢复结果不强制回滚 Domain 层状态。
