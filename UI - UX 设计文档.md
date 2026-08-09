# Codex Agent Switch UI / UX 设计文档

> 文档类型：UI / UX Design Specification  
> 项目暂定名称：Codex Agent Switch  
> 简称：CAS  
> 文档职责：定义桌面应用的信息架构、页面结构、组件规范、交互行为、视觉风格及状态反馈。

---

# 0. V0.1 UI 基线

V0.1 UI 围绕 `Agents / Providers / Models / Diagnostics / Settings` 与 Apply / Restore 闭环。DeepSeek V4 Flash 是首个推荐 Preset，但 Add Provider 必须同时提供 Custom Responses 路径；除 Preset 标签、说明和模型元数据外，不出现 DeepSeek 专属页面或交互分支。

Profiles、Model Discovery、Runtime Probe、Import / Adopt 页面为 P1，V0.1 不出现在导航。正式桌面交付仅承诺 Windows；平台差异只在 Settings / Diagnostics 中作为能力状态呈现。

---

# 1. UI 设计目标

Codex Agent Switch 是面向开发者的 Codex Agent 管理工具。

UI 的核心目标不是展示复杂配置，而是将：

```text
Provider
Model
Agent
Profile
Codex Configuration
```

这些底层概念转化为清晰、低认知负担的操作流程。

用户应该能够在不知道：

```text
config.toml
agents/*.toml
model_provider
model_catalog_json
Responses API
```

等底层细节的情况下完成主要配置。

核心体验应达到：

```text
添加 Provider
    ↓
选择模型
    ↓
绑定 Agent
    ↓
Apply
    ↓
使用 Codex
```

---

# 2. UI 产品定位

整体视觉定位：

> 现代、专业、开发者工具、克制、可靠。

参考方向可以吸收：

```text
Linear
Raycast
GitHub Desktop
VS Code Settings
CC Switch
Vercel
OpenAI / Codex
```

但不得直接复制任何产品界面。

不采用典型 AI 产品常见的：

- 大面积紫蓝渐变；
- 发光边框；
- 大量玻璃拟态；
- 夸张圆角；
- 大量卡片套卡片；
- 无意义渐变按钮；
- 巨大 Hero 区域；
- 聊天机器人式主页。

CAS 是：

> Developer Utility。

不是：

> AI Landing Page。

---

# 3. 核心设计原则

## 3.1 Agent First

CAS 的核心不是 Provider。

因此主导航和首页必须优先表达：

```text
Agent Team
```

而不是：

```text
API Provider List
```

用户进入软件后首先应该看到：

> 当前 Codex 的 Agent Team 是怎么组成的。

---

## 3.2 配置细节渐进暴露

普通用户默认只看到：

```text
Provider
Model
Agent
Profile
```

高级配置例如：

```text
Base URL
Protocol
Reasoning
Sandbox
Custom Headers
Model Metadata
```

通过：

```text
Advanced
```

区域展开。

不得让用户首次打开软件就面对大量技术参数。

---

## 3.3 状态优先于配置

界面需要始终让用户知道：

```text
现在是什么配置？
是否已经 Apply？
Codex 是否正常？
Provider 是否可用？
有没有未保存修改？
```

优先展示状态，不让用户猜测。

---

# 4. 信息架构

主导航建议：

```text
Agents

Profiles

Providers

Models

Diagnostics

Settings
```

不建议把：

```text
Dashboard
Home
Overview
```

单独做成一个没有实际操作价值的首页。

默认进入：

```text
Agents
```

---

# 5. 主界面整体布局

采用桌面开发者工具常见的：

```text
Sidebar + Content
```

结构。

```text
┌────────────────────────────────────────────────────────────┐
│ Codex Agent Switch                                         │
├──────────────┬─────────────────────────────────────────────┤
│              │                                             │
│ Agents       │                                             │
│ Profiles     │                                             │
│ Providers    │                Content                      │
│ Models       │                                             │
│ Diagnostics │                                             │
│              │                                             │
│              │                                             │
│ Settings     │                                             │
│              │                                             │
├──────────────┴─────────────────────────────────────────────┤
│ Codex ● Ready          Balanced           ✓ Applied        │
└────────────────────────────────────────────────────────────┘
```

---

# 6. 左侧导航栏

建议宽度：

```text
220px ~ 248px
```

导航顺序：

```text
Agents
Profiles

Providers
Models

Diagnostics

Settings
```

通过间距分组，不需要大量分割线。

导航项包含：

```text
Icon
Label
Active State
Optional Badge
```

例如：

```text
Agents             4

Providers          3

Diagnostics        1
```

Badge 只用于有实际意义的信息。

不得把所有导航项都加数字。

---

# 7. 顶部区域

页面内容区顶部统一：

```text
Page Title
Page Description
Primary Action
```

例如：

```text
Agents

Configure the model and behavior of each Codex subagent.

                                    + Create Agent
```

不要设计全局巨大 Header。

---

# 8. 全局底部状态栏

建议提供非常轻量的底部状态栏：

```text
Codex ● Ready      Profile: Balanced      Configuration: Applied
```

如果存在未应用修改：

```text
Codex ● Ready      Profile: Balanced      ● Changes pending
```

用户在任何页面都可以知道当前系统状态。

---

# 9. Agents 页面

Agents 是整个产品的核心页面。

页面标题：

```text
Agents
```

副标题：

```text
Configure the roles and models used by Codex subagents.
```

主要操作：

```text
+ Create Agent
```

---

# 10. Agent 列表布局

推荐使用：

```text
Compact List / Row
```

而不是巨大卡片瀑布流。

示例：

```text
┌──────────────────────────────────────────────────────┐
│ Executor                                      Active │
│ Implementation, refactoring and test execution      │
│                                                      │
│ DeepSeek   deepseek-v4-flash        High      ›     │
├──────────────────────────────────────────────────────┤
│ Explorer                                      Active │
│ Repository exploration and code investigation       │
│                                                      │
│ OpenAI     gpt-5.6-luna             Medium    ›     │
├──────────────────────────────────────────────────────┤
│ Reviewer                                      Active │
│ Final implementation and correctness review         │
│                                                      │
│ OpenAI     gpt-5.6-terra            High      ›     │
└──────────────────────────────────────────────────────┘
```

每行只展示关键数据：

```text
Agent Name
Description
Provider
Model
Reasoning
Status
```

详细配置进入 Agent Detail。

---

# 11. Agent 状态

状态推荐：

```text
Active
Disabled
Needs model
Unavailable
Incompatible
```

正常状态不需要强烈颜色强调。

异常状态才使用：

```text
Warning
Error
```

---

# 12. Agent Detail

点击 Agent 后进入详情页。

布局建议：

```text
← Agents

Executor                                    Enabled ●

Implementation-focused worker used after architecture
and task boundaries have been determined.


Model
────────────────────────────────────────────

Provider
[ DeepSeek                              ▾ ]

Model
[ DeepSeek V4 Flash                    ▾ ]

Reasoning
[ High                                  ▾ ]


Permissions
────────────────────────────────────────────

Sandbox
[ Workspace Write                       ▾ ]


Instructions
────────────────────────────────────────────

[ Edit instructions ]


Compatibility
────────────────────────────────────────────

✓ Responses API
✓ Tool calling
✓ Codex Multi-Agent
✓ Parallel tool calls


                         [ Save Changes ]
```

---

# 13. Agent Model Selector

Model 选择不得只是一个包含几十个字符串的普通 Select。

建议点击后打开：

```text
Model Picker
```

示例：

```text
Select Model

Search models...

DeepSeek

● DeepSeek V4 Flash
  1M context · Tool calling · Multi-Agent

  DeepSeek V4 Pro
  Codex readiness unknown · unavailable for binding


OpenRouter

  Qwen ...
  ...


────────────────────────────────
+ Add Provider
```

模型列表按：

```text
Provider
```

分组。

---

# 14. Model Picker 信息密度

每个模型条目展示：

```text
Display Name

Provider

关键能力
```

例如：

```text
DeepSeek V4 Flash

DeepSeek
1M context · Reasoning · Multi-Agent
```

不要展示：

```text
完整 metadata JSON
全部 capability
完整 model ID
```

这些属于详情。

---

# 15. 模型兼容提示

如果用户选择的模型不满足 Agent 要求：

```text
Executor
requires Tool Calling + Multi-Agent
```

而模型缺少能力：

```text
⚠ This model may not support this agent.

Missing:
• Multi-Agent support
```

默认不应该直接静默接受。

如果属于完全不兼容：

```text
This model cannot be assigned to Executor.
```

按钮禁用。

---

# 16. Create Agent

创建 Agent 使用：

```text
Modal / Sheet
```

不需要独立多步骤向导。

结构：

```text
Create Agent

Name
[ Database Expert                      ]

Key
[ database_expert                      ]

Description
[ Reviews schema design and queries... ]


Template

○ Blank
● Executor
○ Explorer
○ Reviewer
○ Tester


Model

Provider
[ DeepSeek                          ▾ ]

Model
[ deepseek-v4-flash                ▾ ]


                               Cancel
                         Create Agent
```

创建完成后进入 Agent Detail。

---

# 17. Agent Templates

新建 Agent 时提供模板：

```text
Blank

Executor
Explorer
Reviewer
Tester

Security Reviewer
Database Expert
Frontend Worker
```

模板展示简短解释即可：

```text
Executor
Implementation and task execution.

Reviewer
Correctness and final review.

Explorer
Repository exploration and investigation.
```

不要一次展示完整 Prompt。

---

# 18. Agent Instructions 编辑器

Instructions 属于高级但重要能力。

建议使用专门编辑区域：

```text
Instructions

┌──────────────────────────────────────┐
│ You are an implementation worker... │
│                                      │
│                                      │
└──────────────────────────────────────┘

Reset to template
```

使用：

```text
Monospace
```

字体。

支持：

```text
Undo
Reset
```

但 V0.1 不需要复杂 Prompt IDE。

---

# 19. Profiles 页面（P1）

Profiles 用于快速切换整套 Agent Team 配置。

页面：

```text
Profiles

Switch between complete agent/model configurations.

                                    + Create Profile
```

---

# 20. Profile 列表（P1）

推荐：

```text
┌────────────────────────────────────────────────┐
│ Balanced                               Active  │
│                                                │
│ Executor   DeepSeek V4 Flash                   │
│ Explorer   GPT-5.6 Luna                        │
│ Reviewer   GPT-5.6 Terra                       │
│                                                │
│                                   Manage ›     │
├────────────────────────────────────────────────┤
│ Budget                                         │
│                                                │
│ Executor   DeepSeek V4 Flash                   │
│ Explorer   Qwen ...                            │
│ Reviewer   DeepSeek ...                        │
│                                                │
│                       Activate      Manage ›    │
└────────────────────────────────────────────────┘
```

Profile 可以使用卡片形式，因为 Profile 数量通常有限。

---

# 21. Profile 激活（P1）

点击：

```text
Activate
```

不立即隐式 Apply。

状态变为：

```text
Balanced → Budget
```

底部显示：

```text
● Changes pending
```

并出现全局操作：

```text
Apply Changes
```

这样：

```text
业务状态修改
```

和：

```text
写入 Codex
```

在 UI 上也明确区分。

---

# 22. Profile Detail（P1）

Profile Detail 不修改 Agent 本身定义。

只修改：

```text
Agent → Model
```

映射。

例如：

```text
Profile: Balanced


Executor
[ DeepSeek / V4 Flash                   ▾ ]

Explorer
[ OpenAI / GPT-5.6 Luna                 ▾ ]

Reviewer
[ OpenAI / GPT-5.6 Terra                ▾ ]


+ Add Agent


                           Save Profile
```

不要在 Profile 页面出现：

```text
Agent Instructions
Provider API Key
Provider Base URL
```

这些属于其他页面。

---

# 23. Providers 页面

Providers 页面负责：

> 管理模型服务来源。

页面：

```text
Providers

Manage model API providers used by Codex agents.

                                   + Add Provider
```

列表：

```text
DeepSeek

https://api.deepseek.com/
Responses API

3 models                                  ● Ready


OpenRouter

https://openrouter.ai/...
Responses Compatible

12 models                                 ● Ready
```

---

# 24. Provider 状态表现

推荐：

```text
Ready
Credential missing
Connection failed
Disabled
Configuration error
Unknown
```

Provider 的主页列表不展示 API Key。

---

# 25. Add Provider

点击：

```text
+ Add Provider
```

首先进入 Provider Preset Selector。

```text
Add Provider

Popular

DeepSeek
OpenRouter
Custom Responses Provider


Other

Local Provider
Custom
```

V0.1 可以只显示实际支持的选项。

不要展示尚未实现但无法使用的 Provider。

---

# 26. Provider Preset Card

Preset 应保持简洁：

```text
┌─────────────────────────────┐
│ DeepSeek                    │
│                             │
│ Official DeepSeek API       │
│ Responses API               │
│                             │
│                        ›    │
└─────────────────────────────┘
```

不需要品牌大图或复杂视觉装饰。

---

# 27. Provider Configuration

以 DeepSeek 为例：

```text
Add DeepSeek

Name
[ DeepSeek                         ]

Base URL
[ https://api.deepseek.com/       ]

API Key
[ •••••••••••••••••••••••       ]

Protocol
Responses API


                     Test Connection
```

高级项：

```text
Advanced
```

展开：

```text
Custom Headers

Timeout

Compatibility Settings
```

普通用户默认无需接触。

---

# 28. Test Connection

点击：

```text
Test Connection
```

按钮进入：

```text
Testing...
```

成功：

```text
✓ Connected successfully

Models discovered: 3
```

失败：

```text
Connection failed

401 Unauthorized

Check your API key and try again.
```

不要只显示：

```text
Error
```

也不要直接把完整 Rust/HTTP Stack Trace 展示给普通用户。

---

# 29. Provider 保存流程

建议：

```text
填写 Provider
      ↓
Test Connection
      ↓
Save
```

但：

```text
Test Connection
```

不必成为强制前置条件。

用户可以保存一个暂时不可访问的 Provider。

此时状态：

```text
Unverified
```

---

# 30. Provider Detail

结构：

```text
← Providers

DeepSeek                                    ● Ready

Connection
────────────────────────────────────────────

Base URL
https://api.deepseek.com/

Protocol
Responses API

Credential
Configured                         Replace


Models
────────────────────────────────────────────

DeepSeek V4 Flash                  Enabled
DeepSeek V4 Pro                    Not ready for Codex

                                    Manage Models


Advanced
────────────────────────────────────────────

Custom Headers
Model Catalog
Compatibility
```

---

# 31. Credential UI

API Key 默认永远不回显。

界面：

```text
Credential

API Key
Configured

Last updated
2026-08-07

[ Replace Credential ]
[ Remove Credential ]
```

不得：

```text
点击眼睛 → 显示完整 API Key
```

CAS 没有必要提供该能力。

---

# 32. Models 页面

Models 页面是跨 Provider 模型总览。

目标不是让用户手动维护所有 metadata，而是快速知道：

```text
有哪些模型
从哪里来
能不能用于 Codex Agent
```

---

# 33. Model Table

Models 数量可能很多，因此优先使用表格。

```text
Models

Search models...

Provider     Model                  Compatibility   Status
──────────────────────────────────────────────────────────
DeepSeek     V4 Flash               Native          Ready
DeepSeek     V4 Pro                 Compatible      Ready
OpenRouter   Qwen ...               Compatible      Ready
Custom       Model X                Unknown         Disabled
```

支持过滤：

```text
Provider
Compatibility
Status
```

---

# 34. Model Detail

点击模型：

```text
DeepSeek V4 Flash

Provider
DeepSeek

Model ID
deepseek-v4-flash

Compatibility
Native


Capabilities
────────────────────────────────────

✓ Responses API
✓ Tool Calling
✓ Parallel Tool Calling
✓ Reasoning
✓ Codex Multi-Agent


Context
────────────────────────────────────

Context Window
1,000,000

Reasoning
Low / Medium / High
```

详细 metadata 进入：

```text
Advanced
```

---

# 35. Unknown Model

用户手动添加模型但 CAS 不知道完整能力时：

```text
Compatibility

? Unknown

This model has not been verified for Codex Multi-Agent use.
```

用户可以：

```text
Run compatibility check
```

但不能伪装成：

```text
✓ Compatible
```

---

# 36. Diagnostics 页面

Diagnostics 是开发者工具非常重要的页面。

结构：

```text
Diagnostics

Check whether Codex Agent Switch is correctly configured.

                                Run Diagnostics
```

运行后：

```text
Codex Environment
─────────────────────────────────────
✓ Codex detected
✓ Supported version
✓ CODEX_HOME writable


Configuration
─────────────────────────────────────
✓ config.toml valid
✓ CAS managed resources found
✓ Agent configuration valid


Providers
─────────────────────────────────────
✓ DeepSeek reachable
⚠ OpenRouter credential missing


Agents
─────────────────────────────────────
✓ Executor ready
✓ Explorer ready
✕ Reviewer model unavailable
```

---

# 37. Diagnostic Severity

统一使用：

```text
Success
Warning
Error
Info
```

视觉不要只依赖颜色。

每一种状态同时使用：

```text
Icon + Text
```

例如：

```text
✓ Ready
⚠ Warning
✕ Error
ⓘ Info
```

保证可访问性。

---

# 38. Repair 行为

Diagnostics 不应自动修复问题。

如果某问题支持修复：

```text
⚠ Agent configuration is out of sync

                         Repair
```

用户点击 Repair 后再执行。

不得进入 Diagnostics 页面就自动改 Codex 文件。

---

# 39. Settings 页面

Settings 只管理 CAS 自身设置。

推荐分组：

```text
General

Codex

Appearance

Updates

Advanced
```

不要把 Provider / Agent 配置塞到 Settings。

---

# 40. Settings / General

例如：

```text
Launch behavior

□ Start minimized


Configuration

☑ Create backup before applying changes
```

---

# 41. Settings / Codex

展示：

```text
Codex Installation

Executable
C:\...

CODEX_HOME
C:\Users\...\ .codex

Version
0.xxx

                              Detect Again
```

允许高级用户手动覆盖：

```text
CODEX_HOME
```

但默认自动检测。

---

# 42. Settings / Appearance

V0.1 推荐：

```text
Theme

○ System
○ Light
○ Dark
```

不要第一版就增加：

```text
几十种主题
自定义 CSS
主题市场
```

---

# 43. Settings / Advanced

高级区域可以包含：

```text
Open CAS Data Directory

Open Codex Config Directory

Export CAS Configuration

Import CAS Configuration

Reset Application
```

危险操作使用独立 Danger Zone。

---

# 44. Apply Changes

这是全局最重要的交互之一。

当 CAS 内部状态和 Codex 实际配置不一致时：

底部出现：

```text
● 3 unapplied changes

                      Discard
                      Review
                      Apply Changes
```

避免：

> 用户改一个 Select 就立刻修改 Codex 配置。

---

# 45. Review Changes

点击：

```text
Review
```

打开：

```text
Configuration Changes

Agent
Executor

Model
DeepSeek V4 Flash
→ Model X


Provider
+ Provider X


Codex Resources
1 file will be created
2 managed sections will be updated


                         Cancel
                         Apply
```

这是逻辑层级的 Diff。

不是直接把 TOML Diff 扔给用户。

高级用户可以展开：

```text
Show configuration diff
```

---

# 46. Apply 成功

成功后：

```text
✓ Configuration applied

Codex Agent configuration has been updated.
```

如果需要重启 Codex：

```text
Restart Codex to ensure all changes take effect.
```

底部：

```text
✓ Applied
```

---

# 47. Apply 失败

例如：

```text
Configuration could not be applied.

The existing Codex configuration changed after it
was loaded by CAS.

No files were overwritten.

Review the latest configuration and try again.
```

必须明确：

```text
有没有写入
有没有恢复
是否存在风险
```

而不是：

```text
Apply failed.
```

---

# 48. Backup / Restore UI

Backup 不需要成为主导航一级页面。

建议位于：

```text
Settings → Codex → Configuration History
```

例如：

```text
Configuration History

Today 17:42
Before applying Balanced profile

Yesterday 21:13
Before updating DeepSeek

2026-08-05
Initial CAS configuration
```

操作：

```text
View
Restore
```

---

# 49. Restore 确认

Restore 属于高影响操作。

需要确认：

```text
Restore configuration?

This will restore Codex configuration files to
their state from August 7, 17:42.

Current configuration will be backed up first.

                    Cancel
                    Restore
```

---

# 50. 未保存状态

编辑实体但尚未保存：

```text
● Unsaved changes
```

离开页面：

```text
You have unsaved changes.

Discard changes?
```

如果只是：

```text
Saved to CAS
but not applied to Codex
```

则状态应明确不同：

```text
Saved
● Not applied
```

这两个状态不得混淆。

---

# 51. 状态层级

CAS UI 应明确区分三种状态：

```text
Draft State
    ↓ Save

CAS State
    ↓ Apply

Codex State
```

对应 UI：

```text
Unsaved
Saved / Changes pending
Applied
```

这一点必须全局统一。

---

# 52. Empty States

首次启动时，不展示空白表格。

Agents：

```text
No agents configured

Create your first Codex subagent or start from
a recommended template.

[ Create Agent ]
```

Providers：

```text
No providers added

Add a model provider to start assigning external
models to Codex agents.

[ Add Provider ]
```

Profiles：

```text
No profiles yet

Profiles let you switch an entire agent team at once.

[ Create Profile ]
```

---

# 53. 首次启动体验

首次启动不建议做长达 6~8 步的 onboarding wizard。

推荐：

```text
Welcome to Codex Agent Switch

Manage the models used by Codex subagents.

Codex
✓ Detected

No external provider configured.


[ Add Provider ]

or

[ Explore App ]
```

用户可以跳过。

---

# 54. 推荐首次配置流程

用户选择：

```text
Add Provider
```

完成 Provider 后：

```text
Provider added successfully.

What would you like to do next?

[ Assign to Executor ]
[ Add Another Provider ]
[ Done ]
```

点击：

```text
Assign to Executor
```

直接进入 Agent 配置。

减少用户自己寻找下一步。

---

# 55. Search

Models、Providers、Agents 数量增加后均应支持搜索。

统一搜索交互：

```text
Search...
```

支持：

```text
名称
key
modelId
Provider
```

但普通列表数量很少时不强制显示搜索框。

---

# 56. Context Menu

列表条目可提供：

```text
...
```

内容例如：

Agent：

```text
Edit
Duplicate
Disable
Delete
```

Provider：

```text
Edit
Test Connection
Disable
Delete
```

Profile：

```text
Activate
Duplicate
Rename
Delete
```

主要动作仍应直接可见。

不要所有操作都藏入 Context Menu。

---

# 57. 删除确认

普通非破坏性对象采用轻确认。

存在引用关系时必须明确影响：

```text
Delete DeepSeek?

This provider is currently used by:

• Executor
• Balanced profile

Reassign these models before deleting the provider.

                         Close
```

此时删除按钮直接禁用。

---

# 58. Danger Zone

仅用于：

```text
Remove Credential

Delete Provider

Delete Agent

Reset CAS

Restore Configuration
```

危险按钮不得到处使用红色。

只有真正不可逆或高影响操作使用危险视觉。

---

# 59. Toast

Toast 用于：

```text
Saved
Copied
Connection successful
Profile activated
```

例如：

```text
✓ Agent saved
```

不要用 Toast 承载：

```text
长错误信息
复杂冲突
需要用户选择的操作
```

这些使用 Inline Error 或 Dialog。

---

# 60. Loading State

避免整页 Spinner。

局部操作使用局部状态：

```text
Provider

DeepSeek

Testing connection...
```

模型列表：

```text
Discovering models...
```

页面首次加载可使用轻量 Skeleton。

---

# 61. Error State

错误信息格式：

```text
发生了什么
+
为什么可能发生
+
用户接下来能做什么
```

例如：

```text
Unable to connect to DeepSeek.

The API returned 401 Unauthorized.

Check the configured API key and try again.

                     Replace Credential
```

---

# 62. 色彩系统

整体以中性灰阶为主。

建议：

```text
Background
Surface
Border
Primary Text
Secondary Text
Muted Text
```

构成主要界面。

品牌 Accent 仅用于：

```text
Active navigation
Primary button
Selected state
Focus
```

不应大面积铺色。

---

# 63. Light Theme

Light 模式建议：

```text
主背景：
接近白色但避免过度纯白层层堆叠

Surface：
与背景形成非常轻微层级

Border：
浅灰中性色

正文：
接近黑色

Secondary：
中性灰
```

通过：

```text
间距
边框
排版
背景层级
```

区分区域。

而不是大量阴影。

---

# 64. Dark Theme

Dark 模式避免：

```text
纯黑背景 + 高饱和蓝紫
```

推荐：

```text
深中性背景
略亮 Surface
低对比 Border
高可读正文
克制 Accent
```

开发工具应适合长时间打开。

---

# 65. 圆角

建议：

```text
Input / Button:
6px ~ 8px

Dialog / Panel:
8px ~ 12px
```

不要：

```text
16px
20px
24px
```

大量使用。

CAS 应保持工具感，而不是移动端消费 App 感。

---

# 66. 阴影

默认界面：

```text
几乎不依赖阴影
```

允许 Dialog / Popover 使用轻阴影。

层级主要依赖：

```text
背景
边框
间距
```

---

# 67. 字体

优先系统字体。

UI：

```text
Inter
或者系统 Sans
```

代码、Model ID、路径：

```text
JetBrains Mono
SFMono
Cascadia Code
系统 Monospace
```

无需强制内置大型字体包。

---

# 68. 字号层级

建议：

```text
Page Title
20–24px

Section Title
14–16px / Semibold

Body
13–14px

Secondary
12–13px

Code / Metadata
12–13px
```

开发者工具不需要非常大的字体。

---

# 69. 间距体系

采用统一 4px 基础单位：

```text
4
8
12
16
20
24
32
```

禁止出现大量：

```text
13px
19px
27px
```

随机 spacing。

---

# 70. Button 体系

只保留主要类型：

```text
Primary

Secondary

Ghost

Danger
```

例如：

```text
Apply Changes
→ Primary

Cancel
→ Secondary

Edit
→ Ghost

Delete Provider
→ Danger
```

---

# 71. Primary Button 使用限制

一个独立操作区域通常只允许一个明显 Primary Action。

错误：

```text
[ Save ]
[ Apply ]
[ Test ]
[ Add ]
```

全部高亮。

正确：

```text
Test Connection     Save Provider
secondary           primary
```

---

# 72. Input

Input 默认：

```text
Label
Control
Optional description
Validation
```

例如：

```text
Base URL

[ https://api.example.com/v1 ]

The endpoint used for model requests.
```

Placeholder 不得替代 Label。

---

# 73. Select

Provider / Model / Reasoning 等重要 Selector：

```text
Label

[ Current Value                          ▾ ]
```

如果选项较多，必须使用：

```text
Searchable Combobox
```

而不是浏览器原生超长 Select。

---

# 74. Switch

Switch 只用于真实即时布尔状态：

```text
Enabled
Auto Backup
```

不要把：

```text
Profile
Model
Reasoning
```

做成 Switch。

---

# 75. Table

Models 等高密度信息使用 Table。

Table 避免：

```text
竖线网格
强边框
每格背景色
```

采用：

```text
行分隔
Hover
清晰列对齐
```

即可。

---

# 76. Card

Card 只用于天然独立的内容，例如：

```text
Profile
Provider Preset
Diagnostic Summary
```

Agent 主列表优先 Row。

禁止：

```text
整个页面每一个字段都是 Card
Card 里再套 Card
```

---

# 77. Badge

Badge 用于短状态：

```text
Native
Ready
Active
Unknown
```

不用于长文本。

颜色保持低饱和。

---

# 78. Icon

图标风格统一使用：

```text
线性图标
```

例如：

```text
Lucide
```

不要混用：

```text
emoji
filled icons
outline icons
品牌图标
```

品牌 Logo 仅 Provider 列表可以使用。

---

# 79. 动画

允许：

```text
Popover
Dialog
展开
状态切换
```

使用短动画。

推荐：

```text
120–200ms
```

不加入：

```text
页面飞入
卡片漂浮
光效
背景粒子
```

---

# 80. 窗口尺寸

桌面应用推荐最小尺寸：

```text
960 × 640
```

推荐舒适尺寸：

```text
1100 × 720
+
```

页面必须支持：

```text
窗口缩小
```

但不需要针对手机布局。

---

# 81. 响应式原则

这是桌面应用，不按照移动优先设计。

宽度不足时优先：

```text
内容区收缩
表格横向合理隐藏次要列
Detail 页面由双列降为单列
```

Sidebar 可在较小窗口进入：

```text
Compact Mode
```

---

# 82. 可访问性

所有交互控件必须支持：

```text
Keyboard Navigation
Focus State
Accessible Label
```

状态不得只依赖颜色。

例如：

```text
红色圆点
```

不能单独表达错误。

必须同时：

```text
✕ Connection failed
```

---

# 83. 快捷键

V0.1 可以提供少量真正有价值的快捷键：

```text
Ctrl/Cmd + K
Search / Command Palette

Ctrl/Cmd + S
Save current entity

Ctrl/Cmd + ,
Settings
```

不要第一版设计几十个快捷键。

---

# 84. Command Palette

Command Palette 可作为增强功能。

例如：

```text
Switch Profile: Balanced

Open Executor

Add Provider

Run Diagnostics

Apply Changes
```

不作为 V0.1 必须能力。

---

# 85. Provider Logo

已知 Provider 可以展示 Logo。

自定义 Provider：

```text
使用首字母或通用 Provider 图标
```

Logo 只作为识别辅助。

不能因为缺少品牌 Logo 影响使用。

---

# 86. 主 Agent 的展示

CAS 重点管理 Subagent，但 Agents 页面顶部可以保留 Primary Agent 概览：

```text
Primary Agent

Codex Official
GPT-5.6 Sol

Managed by Codex
```

Primary Agent 如果不是 CAS 管理范围：

```text
Managed by Codex
```

而不是给用户造成：

> CAS 正在控制主 Agent

的误解。

---

# 87. Agent Team 总览

Agents 页面顶部可以提供紧凑 Team Overview：

```text
Agent Team

Primary
Codex Official

Executor
DeepSeek V4 Flash

Explorer
GPT-5.6 Luna

Reviewer
GPT-5.6 Terra
```

但不要重复下面所有 Agent Detail。

其用途是：

> 5 秒看懂当前配置。

---

# 88. 推荐 Agents 首页最终结构

```text
┌──────────────────────────────────────────────────────────┐
│ Agents                              + Create Agent        │
│ Configure the roles used by Codex subagents.             │
│                                                          │
│ Current Team                                             │
│                                                          │
│ Primary        Codex Official / GPT-5.6 Sol              │
│                                                          │
├──────────────────────────────────────────────────────────┤
│ Executor                                      Active     │
│ Implementation and test execution                        │
│ DeepSeek / deepseek-v4-flash        High             ›   │
├──────────────────────────────────────────────────────────┤
│ Explorer                                      Active     │
│ Repository exploration                                  │
│ OpenAI / gpt-5.6-luna               Medium           ›   │
├──────────────────────────────────────────────────────────┤
│ Reviewer                                      Active     │
│ Final correctness review                                │
│ OpenAI / gpt-5.6-terra              High             ›   │
└──────────────────────────────────────────────────────────┘
```

---

# 89. 全局 Apply Bar

存在 Pending Changes 时，在窗口底部显示固定操作区：

```text
┌───────────────────────────────────────────────────────┐
│ ● 3 changes have not been applied                    │
│                                                       │
│                         Review   Discard   Apply       │
└───────────────────────────────────────────────────────┘
```

无修改时隐藏。

这比在每个页面到处放：

```text
Apply
```

更清晰。

---

# 90. 视觉优先级

全系统优先级：

```text
一级：
用户当前 Agent Team

二级：
Agent → Model Binding

三级：
Provider / Model 管理

四级：
高级兼容配置
```

不能让：

```text
API URL
Protocol
Catalog
Header
```

视觉上比 Agent 更重要。

---

# 91. 文案原则

界面文案要求：

```text
短
明确
技术准确
不过度解释
```

避免典型 AI 产品文案：

```text
Unlock the full potential of AI agents

Supercharge your coding workflow

Experience next-generation intelligence
```

CAS 使用：

```text
Configure the models used by Codex subagents.

Add a model provider.

This model is not compatible with Codex Multi-Agent.

Configuration applied successfully.
```

---

# 92. 技术词保留原则

开发者明确熟悉的词不要强行翻译或包装：

```text
Provider
Model
Agent
Profile
Responses API
Reasoning
Sandbox
Codex
```

可以直接使用。

不创造：

```text
智能大脑
执行引擎
模型魔法
AI Core
```

这类营销词。

---

# 93. 初始默认 Agent

首次安装可以提供推荐模板：

```text
Executor
Explorer
Reviewer
```

但：

```text
不自动绑定外部模型
```

没有 Provider 时：

```text
Executor
No model assigned
```

避免 CAS 擅自改变 Codex 行为。

---

# 94. 外部配置识别

如果 CAS 检测到已有 Codex Agent：

```text
Existing Codex agents detected

executor
reviewer
custom-agent
```

UI 必须明确标记：

```text
External
```

操作：

```text
View

Manage with CAS
```

不得自动接管。

---

# 95. 外部修改提示

如果 CAS 管理的配置在外部被修改：

```text
Configuration changed outside CAS

executor.toml was modified after the last apply.

[ Review Changes ]
```

不得静默覆盖。

---

# 96. Disabled Provider

Provider 被 Disable 后：

```text
DeepSeek
Disabled
```

所有依赖模型应在 UI 中派生显示：

```text
Executor

DeepSeek V4 Flash
Provider disabled
```

而不是直接删除 Binding。

---

# 97. 缺失 Credential

表现：

```text
DeepSeek

⚠ Credential required

                              Add Credential
```

关联 Agent：

```text
Executor

DeepSeek V4 Flash

⚠ Provider credential missing
```

用户可以从 Agent 页面直接跳 Provider 修复。

---

# 98. Developer Details

对于高级用户，部分页面允许：

```text
Developer Details
```

例如：

```text
Model ID
Provider Key
CAS Resource ID
Codex File
Compatibility Metadata
```

默认折叠。

避免普通界面充满内部字段。

---

# 99. V0.1 页面范围

V0.1 正式实现以下页面：

```text
Agents
Agent Detail
Create Agent

Providers
Provider Detail
Add Provider

Models
Model Detail

Diagnostics

Settings
```

以及：

```text
First Run
Apply Changes
Review Changes
Restore Confirmation
Common Dialogs
```

---

# 100. V0.1 不实现的 UI

以下能力不进入 V0.1：

```text
云端账号
团队空间
多人协作
市场
插件商城
Prompt Marketplace
复杂 Agent Workflow 编辑器
节点拖拽编排
Token Cost Dashboard
历史 Agent 对话
实时 Agent 执行监控
LLM Chat
Provider Billing
复杂数据分析
Profiles / Profile Detail / Create Profile
Provider Model Discovery
Model Runtime Probe
Import / Adopt
```

避免产品演变成：

> 又一个 AI IDE。

---

# 101. 页面职责边界

必须长期保持：

```text
Agents
负责 Agent Role 和模型绑定

Profiles
负责整套 Agent Binding 组合

Providers
负责服务来源

Models
负责模型能力和状态

Diagnostics
负责检查系统是否健康

Settings
负责 CAS 自身设置
```

禁止出现：

```text
Providers 页面编辑 Agent Prompt

Profiles 页面修改 API Key

Models 页面配置 Codex 路径

Settings 页面管理 Agent
```

---

# 102. UI 核心用户路径

最核心路径必须控制在非常少的步骤。

首次用户：

```text
启动 CAS
   ↓
Add Provider
   ↓
输入 Credential
   ↓
Test / Save
   ↓
Assign to Executor
   ↓
Apply Changes
   ↓
完成
```

日常切模型：

```text
Agents
   ↓
Executor
   ↓
Select Model
   ↓
Save
   ↓
Apply
```

切整套方案：

```text
Profiles
   ↓
Balanced / Quality / Budget
   ↓
Activate
   ↓
Apply
```

故障：

```text
Diagnostics
   ↓
发现问题
   ↓
进入具体修复页面
```

---

# 103. 最终 UI 结构

```text
Codex Agent Switch
│
├── Agents
│   ├── Agent Team Overview
│   ├── Agent List
│   ├── Agent Detail
│   └── Create Agent
│
├── Profiles
│   ├── Profile List
│   ├── Profile Detail
│   └── Create Profile
│
├── Providers
│   ├── Provider List
│   ├── Provider Presets
│   ├── Provider Detail
│   └── Add Provider
│
├── Models
│   ├── Model Table
│   └── Model Detail
│
├── Diagnostics
│
└── Settings
    ├── General
    ├── Codex
    ├── Appearance
    └── Advanced
```

全局能力：

```text
Apply Bar
Dialogs
Toast
Model Picker
Provider Picker
Loading
Error Handling
Backup / Restore
```

---

# 104. UI 核心不变量

### 不变量一

```text
Agent 是 UI 的核心对象，
Provider 不是首页核心对象。
```

### 不变量二

```text
修改 CAS 状态与 Apply Codex 配置
必须是两个可识别状态。
```

### 不变量三

```text
普通用户不需要直接编辑 TOML。
```

### 不变量四

```text
Credential 永不明文回显。
```

### 不变量五

```text
高级配置默认折叠。
```

### 不变量六

```text
状态不能只依赖颜色表达。
```

### 不变量七

```text
外部 Codex 配置不得被 UI 表现为 CAS-owned。
```

### 不变量八

```text
Provider、Model、Agent、Profile
必须保持清晰页面职责。
```

### 不变量九

```text
所有危险修改必须显式触发。
```

### 不变量十

```text
UI 不实现自己的 Agent Runtime。
```

---

# 105. 最终设计目标

用户打开 Codex Agent Switch 后，应当在数秒内理解：

```text
我的 Codex 现在有哪些 Agent？

每个 Agent 在用什么模型？

这些模型来自哪个 Provider？

当前配置有没有生效？

系统有没有问题？
```

同时，一个完全不了解：

```text
Codex config.toml
Custom Model Provider
Subagent TOML
Model Catalog
```

的用户，也应该能够通过 GUI 完成：

```text
添加模型服务
    ↓
选择模型
    ↓
绑定到 Executor
    ↓
Apply
```

高级用户仍然可以查看：

```text
Model ID
Protocol
Compatibility
Capabilities
Codex Resource
```

但这些信息不能妨碍普通配置流程。

CAS UI 最终应呈现为：

> 一个轻量、稳定、克制的 Codex Agent Team 配置工具。

而不是：

> 一个复杂的 AI 控制台。
