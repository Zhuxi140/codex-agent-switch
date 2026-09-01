# Codex Agent Switch (CAS)

### 让旗舰模型负责思考，让高性价比模型负责执行

<p align="center">
  <a href="https://img.shields.io/badge/版本-0.4.1-blue"><img src="https://img.shields.io/badge/版本-0.4.1-blue" alt="版本 0.4.1"></a>
  <a href="https://img.shields.io/badge/平台-Windows%2010%2F11-0078D6"><img src="https://img.shields.io/badge/平台-Windows%2010%2F11-0078D6" alt="平台 Windows 10/11"></a>
  <a href="https://img.shields.io/badge/License-MIT-yellow"><img src="https://img.shields.io/badge/License-MIT-yellow" alt="License MIT"></a>
  <a href="https://github.com/Zhuxi140/codex-agent-switch/actions"><img src="https://img.shields.io/github/actions/workflow/status/Zhuxi140/codex-agent-switch/ci.yml?label=CI" alt="CI"></a>
</p>

CAS 是面向 Codex CLI 的 Windows 桌面应用：用图形界面管理 Provider、Model 与 Agent 绑定，并将多 Agent 编排、原生 Thread 生命周期和 Token 用量集中到同一处。它通过官方 `codex app-server` 接口工作，无需手改 Codex TOML。

> [!WARNING]
> **v0.4.1 及当前主分支开发快照仍暂不推荐安装使用，更不应作为稳定生产工具部署。** 项目仍处于快速迭代阶段，Apply 会改写 Codex 的 `config.toml` 相关片段并投影 Agent、模型目录与 Skill 资源；第三方 Provider 的兼容性会因 Provider 和模型的工具协议而存在差异；安装包也尚未进行代码签名。建议仅在隔离的测试环境尝鲜，使用前备份现有配置，并在 Apply 前后仔细核对 Preview 与 Snapshot。

## 核心理念

我们不追求打造一支「全面、专业、强大」的万能 Agent 团队。我们相信，效果与成本的最优解来自**脑力与体力的分工**：

| 主脑 Primary | 手脚子 Agent |
| --- | --- |
| 旗舰高智商模型（如 GPT-5.6 Sol） | 轻量 · 可定制 · 按需启用 |
| 编排 · 规划 · 审查 · 收束 | 执行 · 测试 · 探索 · 细节审查 |
| 「想清楚怎么做，并判断做得对不对」 | 每个角色独立绑定高性价比模型 |
| | 「把活干完，把量跑满」 |

**价值主张：**

- **轻量、可定制**：子 Agent 是一层「角色 + 职责 + 模型绑定」，按需创建、随时调换，不堆砌全能 Agent。
- **主脑做精、手脚做量**：把编排、规划、审查交给旗舰模型（贵，但决定成败）；把执行、测试、探索交给高性价比模型（便宜，但量大管饱）。
- **编排、调度、复用三位一体**：多 Agent 按阶段自动委派；Thread 调度（REUSE / SPAWN / WAIT）按运行时指纹、Primary、Workspace Scope、Task Scope 与当前上下文健康度客观判定，避免错误复用和重复创建。
- **结果**：以更合理的价格，实现更好的效果。

一句话：**用旗舰模型的判断力，配上高性价比模型的执行力。**

## 为什么需要它

Codex 原生支持子 Agent 协作（`agents/*.toml` + `[model_providers.*]`），但配置全部手写 TOML，常见痛点：

| 痛点 | CAS 的解法 |
| --- | --- |
| 手写 `config.toml` 与 `agents/*.toml`，容易出错 | 表单化管理，Apply 前有 Preview |
| Agent 和模型绑定过死，换模型要改多个文件 | Agent 与 Model 解耦，绑定关系在 GUI 中维护 |
| 整套 Agent Team 无法整体切换 | 运行模式一键切换（Default ↔ 编排子 Agent） |
| 难以判断配置是否真的可用 | Diagnostics、Provider 连通性测试、模型能力校验与兼容状态 |
| 手工改配置可能破坏已有内容 | 快照 + 回读校验 + 失败自动回滚 + 冲突检测 |
| 看不出子 Agent 花了多少 Token、复用还是重建 | Token 监控 + 子 Agent 实例追踪 + REUSE / SPAWN 决策 |

## 推荐方案

「旗舰主脑 + 高性价比子 Agent」的现成组合，在 CAS 中一键落地（运行模式 → 编排子 Agent → 为各角色绑定模型）：

| 方案 | 主脑（规划 / 编排 / 审查） | 子 Agent（执行 / 测试 / 探索） | 适用场景 |
| --- | --- | --- | --- |
| 方案一 | GPT-5.6 Sol（Codex Native） | DeepSeek V4 Flash | 日常开发主力，性价比优先 |
| 方案二 | GPT-5.6 Sol（Codex Native） | DeepSeek V4 Pro（如已开放） | 需要更强子 Agent 执行质量 |
| 方案三 | GPT-5.6 Sol（Codex Native） | GPT-5.6 Terra / GPT-5.6 Luna | 同一模型生态内搭配，切换无感 |

子 Agent 内部仍可分级：Executor / Explorer 用高性价比模型跑量，Reviewer 可上调一档换更强模型——所有角色绑定都可在 GUI 中随时调整。

## v0.4.1 快速开始

1. 从 GitHub Release 下载 `Codex.Agent.Switch_0.4.1_x64-setup.exe` 并运行安装。
2. 启动应用后检查 Codex 可执行文件与 `CODEX_HOME`；Windows Store 版如无法解析命令，可将 `%USERPROFILE%\.codex\.sandbox-bin\codex.exe` 复制到 `%USERPROFILE%\.local\bin\`。
3. 在 Provider 页面选择 **Codex Native (ChatGPT)**，或添加第三方 Responses Provider；Native Provider 使用当前 Codex 登录，第三方密钥由 Windows 凭据管理器保存。
4. 在 Models 与 Agents 页面绑定模型；可为每个 Agent 选择 CAS 内置 Skill、完整禁用指定 MCP Server，或按工具名配置“仅允许 / 禁用”规则；随后在运行模式中启用编排配置并 Apply。
5. 在用量页面查看原生子 Agent Thread 的生命周期（基于 rollout 事实）、当前上下文与 Token 统计。

安装包当前未进行代码签名；Windows SmartScreen 可能显示警告，请按组织安全策略核验 Release 的 SHA-256。

## 测试状态与客观数据

以下结果区分已发布的 v0.4.1 与当前主分支开发快照；主分支新增能力尚未进入 v0.4.1 安装包。

### 当前主分支开发验证（2026-09-01）

| 验证项 | 真实结果 |
| --- | --- |
| Rust Workspace 测试 | 201 passed、0 failed、8 ignored |
| 前端生产构建 | 通过 |
| Diff 检查 | 通过 |
| Codex Native RC-1：Primary → SPAWN → bind → IDLE → REUSE | 通过（`gpt-5.6-terra`） |
| Codex Native RC-2：并发与失配调度矩阵 | 通过（`gpt-5.6-terra`） |
| Codex Native Phase 12：空闲/运行中断流 → 同 Primary 恢复 | 通过（`gpt-5.6-terra`，原 Turn 未重放） |
| Phase 12：启动失败 / 连续恢复失败上限 | 通过（`FAILED` 状态闭环、3 次硬上限） |
| Windows 项目监控浮窗：打开 → 隐藏 → 重新打开 → 状态恢复 | 真实桌面端验证通过，且保持单实例 |

当前快照增加了 Provider 凭据删除恢复、用量按项目分组、Task Scope、SPAWN Reservation、`WAIT` 决策、`bind` 身份固化、Thread 复用池管理、角色感知的 `AUTO` 复用策略、Agent 级 Skill 与 MCP Server / 工具权限、紧凑编排提示词，以及可重复的 RC-1 / RC-2 / Phase 12 原生 E2E。Runtime Bridge 断流后最多自动恢复 3 次，恢复时 Resume 原 Primary；不确定 Turn 不会被自动重放。启动失败不会残留伪 `STARTING` 状态；无法证明 Schema 能力时统一 Fail Closed。原生 Thread 观察现在由应用级服务持续同步，不再依赖用户停留在用量页面；项目监控浮窗可独立展示所选项目的编排状态、活跃 Thread、累计 Token 与观察增量。它仍是开发快照，不应当作新的 Release 安装包分发。

当前 workspace 的 8 个 ignored 测试包括 1 个会写入当前 Windows 用户凭据库的合成凭据测试、2 个按需输出 Phase 12 结构化证据的确定性场景，以及 5 个依赖 Codex 登录、真实 Provider 或外部配置的 E2E；它们均不计入默认测试通过结论。

### v0.4.1 发布验证

| 验证项 | 命令 | 真实结果 |
| --- | --- | --- |
| Rust 格式 | `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` | 通过 |
| Rust 单测 | `cargo test --manifest-path src-tauri/Cargo.toml --workspace` | 163 passed、0 failed、2 ignored |
| 前端构建 | `npm.cmd run build` | 通过 |
| Diff 检查 | `git diff --check` | 通过 |
| NSIS 打包 | `npm.cmd run bundle:windows` | 通过 |

其中 2 个 ignored 测试分别是依赖外部配置的真实 E2E，以及会写入当前 Windows 用户凭据库的合成凭据测试；二者均不应视为已通过。

### 真实 E2E 覆盖

| 链路 | 状态 | 覆盖边界 |
| --- | --- | --- |
| Codex Native `gpt-5.6-terra` 子 Agent | RC-1、RC-2、Phase 12 自动化通过 | 同一 Primary 下完成 SPAWN → bind → IDLE → REUSE；并发与失配矩阵通过；空闲及运行中断流均恢复同一 Primary，原 Turn 未重放，显式停止后不自动拉起 |
| DeepSeek Responses 子 Agent | 已有成功实测 | 仅说明该实测配置可运行，不外推至其他 Provider 或模型 |
| 外部配置 E2E 自动化 | 已提供独立命令，未纳入默认测试 | 依赖当前 Codex 登录、活动 Agent、真实 Provider 与模型；失败会保留 JSON 证据 |
| 阿里及其他 Provider | 待测试 | 不声明已通过 |

2026-08-23 的 Codex Native RC-1 运行使用 `gpt-5.6-terra`：首次决策为 `SPAWN`，第二次为 `REUSE`，两个任务复用同一 Child Thread，最终生命周期为 `IDLE`，重复 Child 数为 0，累计归属 Token 为 178,060。两个 Primary Turn 均因 Codex App Server 未原生结束而使用 `turn/interrupt` 收束，并在证据中标记为 `UPSTREAM_STALL_RECOVERY`；该兼容结果不等同于原生 `turn/completed`。单次样本仅证明链路正确，不用于宣称性能或 Token 节省。

同日 RC-2 在另一条真实 Codex Native 父子链路上通过：两个相同 Task Scope 的并发预检严格得到 1 个 `SPAWN` 与 1 个 `WAIT / SPAWN_RESERVED`；更换 Workspace、Runtime Fingerprint 以及把临时原生 rollout 的当前 Context 合成到 100% 时，分别稳定得到 `NO_WORKSPACE_SCOPE_MATCH`、`RUNTIME_FINGERPRINT_MISMATCH` 与 `CONTEXT_PRESSURE`。矩阵探针只执行预检，结束后 Child 记录仍为 1。Context 项明确是隔离 rollout 的合成状态探针，不是额外消耗 258,400 Token 的模型运行。

2026-09-01 的 Phase 12 使用原生 `gpt-5.6-terra` 分别验证空闲和运行中断流。空闲样本恢复了相同的 Primary Thread ID，Session 回到 `IDLE`，显式停止后保持停止；运行中样本先通过 `thread/read` 确认原 Turn 已持久化，再强制终止 App Server，恢复后仍是同一 Primary，原生 Thread 只有 1 个 Turn，原 Turn ID 只出现 1 次，因此没有自动重放。确定性场景还验证了启动失败进入 `FAILED` 且不保留伪 Launch，以及连续恢复失败稳定停在 3 次上限。Schema Fixture 覆盖当前字段、未来可选字段、缺失字段和新增必填字段；真实多版本仍需使用用户本地已有的不同 Codex 可执行文件逐个运行，不据单版本外推。

### 效率对比（待持续实测）

CAS 只统计 Token，不统计费用。下表仅列出可客观采集的指标；当前尚未形成可比较的完整样本，因此不虚构数字，也不宣称 CAS 更省 Token 或更快。

| 指标 | 不用 CAS | 使用 CAS |
| --- | --- | --- |
| 任务总 Token | 待测试 | 待测试 |
| Primary / 子 Agent Token 分布 | 待测试 | 待测试 |
| 缓存输入 Token | 待测试 | 待测试 |
| 任务耗时 | 待测试 | 待测试 |
| SPAWN / REUSE 次数或命中率 | 待测试 | 待测试 |
| 任务成功率 / 人工接管次数 | 待测试 | 待测试 |

基准方法：使用同一版本、同一任务集、相同权限和验收标准，对各方案进行多轮运行；先公布每轮原始 Token、耗时与成功结果，再计算汇总指标。在样本足够前，不以任何费用、性能或效率结论进行宣传。

仓库提供了零第三方依赖的 [`benchmarks/efficiency-fixture`](benchmarks/efficiency-fixture) Pilot 夹具，用于先校准测试流程，而不是预先证明 CAS 更高效。Task 1A 要求实现闭区间归一化，Task 1B 在同一个 Primary 任务中继续实现区间扣除；CAS ON 组应记录第一次 `SPAWN`、第二次 `REUSE` 的调度证据。固定运行顺序为 `OFF-01 → ON-01 → ON-02 → OFF-02`，以减小预热和顺序偏差；每轮都必须保留原始 Token、耗时、验收结果和人工接管记录。

验收文件是基准契约，不得在测试过程中修改。夹具中的初始实现会故意失败，参测方案需要完成实现后再运行：

```powershell
node --test benchmarks/efficiency-fixture/acceptance/task-1a.test.mjs
node --test benchmarks/efficiency-fixture/acceptance/task-1a.test.mjs benchmarks/efficiency-fixture/acceptance/task-1b.test.mjs
```

只有四轮均按相同权限、相同验收标准完成，并公开逐轮原始数据后，才会把上表中的“待测试”替换为汇总值。

## 内置 Skill 与精简预设

CAS 已把 `caveman`、`ponytail` 的正常版，以及面向子 Agent 的 CAS 精简版和对应 MIT License 编译进应用，不依赖用户电脑预先安装。Agent 页面中每个 Skill 家族可选择“未启用 / CAS 精简版 / 正常版”，同一家族不能同时启用两个版本：

- **Caveman 正常版**：保留上游完整模式、强度选项、示例和规则。
- **Caveman 精简版**：只保留压缩进度与结果的核心约束，同时要求保留错误、安全信息和验收证据。
- **Ponytail 正常版**：保留上游完整工作流、模式、示例和规则。
- **Ponytail 精简版**：只保留复用现有实现、最小范围改动、停止边界和必要验证。
- **精简预设**：一次选择两份精简版。新建 Executor 默认启用两项；新建 Explorer、Reviewer、Tester 默认只启用 Caveman 精简版。已有 Agent 的选择不会被自动改写。

Apply 时，CAS 将所选版本投影到当前 `CODEX_HOME/cas/bundled-skills/`，并在对应 `agents/cas-*.toml` 中写入 `[[skills.config]]` 与匹配的版本约束。切回 Default、删除绑定或切换 `CODEX_HOME` 时，这些文件与其他 CAS 托管资源走同一套 Preview、冲突检测、快照、清理和恢复流程。

Skill 的键和内容修订号都是 Agent 运行时身份的一部分：改变版本会让旧 Thread 退出复用池，并改变桌面端和 `cas-helper` 使用的 `runtime_fingerprint`，防止新旧指令配置错误复用。精简版用于减少 Skill 自身与输出带来的上下文占用，但 CAS 不承诺固定 Token 节省比例，应以用量监控中的真实数据判断。

## 多 Agent 编排

CAS 将 Agent 分为 Primary、Discovery、Execution、Verification、Review 等 Role/Phase。编排模式下每种 Role 只能启用一个 Agent，避免职责和模型绑定发生歧义。Primary 负责读取、规划、审查和收束；所有实现命令与文件写入必须委派给 Execution Agent。

委派以“一个可独立验收的工作单元”为边界。Primary 只发送由 `GOAL / DECISIONS / ALLOW / DENY / TOOLS / CWD / ACCEPT / STOP` 组成的紧凑 TASK 包，不附带完整对话历史或工具说明。Child 首行以 `DONE / NEEDS_DECISION / PARTIAL / BLOCKED` 返回，后续字段由阶段决定：Execution 报告改动与验证，Discovery 区分证据、推断和未知项，Review 输出带严重度与位置的 Findings，Verification 输出命令、结果、失败和产物。Primary 审查结果后，才接受结果、作出缺失决定，或向同一 Thread 交付下一单元，避免执行器自行扩大任务。

CAS 不复制或改写用户全局 MCP 的命令、环境变量、凭据与 OAuth 状态；自定义 Agent 未覆盖的会话设置继续按 Codex 原生规则继承。Agents 页面会只读解析当前 `CODEX_HOME/config.toml`，仅向前端返回 MCP Server ID、传输类型和全局启用状态，供用户勾选；连接地址、命令、环境变量和认证信息不会离开 Rust 后端。项目级、Profile 或插件内未被发现的 Server 仍可手工补充 ID。

用户选择的静态 MCP 禁用列表只在对应 `agents/cas-*.toml` 中写入 `[mcp_servers.<id>] enabled = false`，未列出的 Server 继续继承 Primary。工具级权限有两种互斥模式：**仅允许列出的工具**投影为 `enabled_tools`，**禁用列出的工具**投影为 `disabled_tools`；同一 Server 只能选择一种工具模式，也不能同时被完整禁用。工具名由用户按 Provider 实际暴露名称手工填写，CAS 不连接或启动 MCP Server 来枚举工具。

完整禁用列表或工具权限变化都会使旧 Thread 退出复用池，并同时改变桌面端与 `cas-helper` 使用的运行时指纹，防止沿用旧工具权限。插件与 App 权限不在本机制范围内。

静态配置仍不替代单次任务授权：CAS 写入 Child 指令的 `TOOLS` 契约，只允许调用 TASK `TOOLS` 明列且完成验收必需的外部工具；`TOOLS: -` 表示禁用，外部写入还必须同时获得 `ALLOW` 明确许可。Discovery 与 Review 的外部工具保持只读，Verification 也不得制造未授权外部状态。CAS 选中的内置 Skill 通过 `skills.config` 投影，并按任务匹配和 Agent 显式绑定规则渐进加载；Primary 不把完整 Skill 内容复制进委派 prompt。

Primary 专属的调度协议只写入 `config.toml` 的 `developer_instructions`；CAS 不再把完整协议写入全局 `AGENTS.md`。用户已有的全局和项目 `AGENTS.md` 仍由 Codex 正常加载，对 Primary 与 Child 共同生效；Child 另由 `agents/cas-*.toml` 接收职责边界。升级后首次 Apply 会移除旧版本留下的 CAS 全局编排块，并保留用户原有内容。

失败策略可选：

- **Strict Stop**：原 Thread 无法续接时，先由同职责 replacement Agent 接棒；只有 replacement 仍不可用、连续替换没有可验证进展或结果不可验证时才停止并报告，Primary 不静默接管。
- **Primary Fallback**：同样优先续接或替换子 Agent；恢复失败后才允许 Primary 在明确提示后接管，并保留回退原因。

项目可被排除在 CAS 编排之外；项目级配置与全局配置冲突时，CAS 会检测冲突并要求查看后中止或显式重新 Apply，而不会覆盖外部修改。

## 调度与 Thread 复用

每次独立任务在委派前由 `cas-helper schedule <agent-key> [task-key]` 预检，输出唯一的 `CAS1|REUSE|...`、`CAS1|SPAWN|...` 或 `CAS1|WAIT|...`。`REUSE` 将完整任务交给既有 Thread；`SPAWN` 创建并绑定新 Thread；`WAIT` 表示同一任务已有未完成的 SPAWN Reservation，Primary 必须等待并重试，不能重复创建。

0.4.0 起，调度直接感知 Codex 原生运行时，不再依赖用量页面是否打开：

- **原生状态直读**：`schedule` 在决策前以只读方式重新打开 Codex 的 `state_*.sqlite`，合并原生候选线程后再计算，新 Spawn 的子 Agent 无需经过 CAS 同步即可进入候选。
- **生命周期以 rollout 为准**：线程状态由 Codex rollout 尾部最后一个明确事件判定（`task_complete`/`turn_aborted` 为 `IDLE`，`task_started` 为 `RUNNING`，无法证明为 `UNKNOWN`）；writer lock 文件存在不等于线程正在运行，残留锁不会永久阻断复用。
- **SPAWN 后必须 bind**：`spawn_agent` 成功返回 child Thread ID 后，Primary 须立即执行 `cas-helper bind <agent-key> <child-thread-id>` 固化线程归属与运行时指纹；bind 只读核验原生身份后才事务写入，身份缺失或指纹冲突拒绝覆盖。
- **并发去重**：带 `task-key` 的 SPAWN 使用 CAS 数据库中的可过期 Reservation 原子预留；相同 Agent、Primary、Workspace 和 Task Scope 的重复预检返回 `WAIT`，避免断流或模型重试制造重复 Thread。

复用是一个客观判定：**同一 Agent（含运行时指纹）、同一 Primary、Exact Workspace Scope、IDLE 状态且上下文健康**。

- **显式 Task Scope**：`cas-helper schedule <agent-key> [task-key]` 支持由 Primary 从任务描述提取的稳定任务键（如 `auth-oauth2`）。只有 `task-key` 完全一致的空闲 Thread 才会被复用；未传任务键的预检不会复用任何绑定了任务键的 Thread（fail-closed），CAS 不做任何模糊分类或历史猜测。`bind` 同样接受并固化任务键，既有键不被覆盖。

- **运行时指纹**：每个 Agent 配置生成稳定 `runtime_fingerprint`（纳入 Provider 身份、Base URL、模型、指令、推理与沙箱策略、能力集合、内置 Skill 选择、MCP Server 禁用列表及工具级权限），配置变更后旧 Thread 立即失配并 `SPAWN`，不会复用旧配置或旧工具权限的线程。
- **Workspace Scope**：当前工作目录的规范化值（UNIX/UNC 统一归一），不是逻辑任务或模块匹配；执行入口会拒绝 Scope 与实际 `cwd` 不一致的请求。未提供 Task Scope 时，同一工作区内的不同任务不会被自动区分；提供显式 Task Scope 后才按任务键精确隔离。
- **上下文健康以当前上下文为准**：仅使用 `current_context_tokens`（来自 App Server `lastTokenUsage` 或 rollout 尾部解析），累计 `totalTokenUsage`/`tokens_used` 只用于用量统计；当前上下文或运行时窗口未知时 fail closed 为 `CONTEXT_UNKNOWN` 并 `SPAWN`，不会把「无法证明健康」当作健康。

调度器还结合 Agent 的 `AUTO` / `HOT` / `COLD` 策略、Provider 的缓存能力和缓存保留提示，避免复用上下文压力过高或已超出缓存窗口的 Thread。`AUTO` 会按 Phase 解析软偏好：Discovery 为 `HOT`（90% Context 阈值）、Execution 保持平衡（80%）、Verification 与 Review 为 `COLD`（60%）；用户显式选择 `HOT/COLD` 时覆盖角色默认。缓存窗口只会影响软阈值与提示，不会绕过 Agent、Primary、Scope、IDLE、Fingerprint 或 Context 可观测性等安全条件，实际缓存命中仍以 Provider 返回的 Token 用量为准。

成功的子 Agent Thread 会保留并同步为 `IDLE`，不得自动调用 `close_agent`。CAS 以独立的 `ACTIVE / RETIRE_PENDING / RETIRED` 状态管理复用资格：运行中的 Thread 只能标记为“完成后退休”，不会被中断；退休只将其移出候选池，不删除 Codex Thread、rollout 或 Token 历史。恢复到复用池前会重新校验 Agent、运行时指纹、Workspace 和上下文健康；批量清理由这些客观条件判定，不依赖模型阅读历史。

## Provider、Model 与用量

- **Codex Native**：可将当前 Codex 登录中的原生 Provider/Model 绑定为子 Agent，包括 gpt-5.6 Terra 等可用模型。
- **第三方 Provider**：支持 Responses API Provider、模型发现和能力校验；凭据不写入 Codex 配置文件。
- **原生 Thread 同步**：同步子 Agent Thread 及其生命周期状态（基于 rollout 事实，而非 writer lock），便于确认运行、完成和空闲状态。
- **Token 监控**：采集输入、缓存输入、输出、推理输出与总 Token，并单独记录当前上下文 Token（`current_context_tokens`）；页面按项目进入二层查看其子 Agent、Thread 与调度决策。CAS 只统计 Token，不计算或展示费用，也不保存 Prompt、Response 正文或 API Key。
- **项目监控浮窗（当前主分支开发快照）**：从用量监控页面打开一个 Windows 单实例浮窗，选择并记住所关注项目，查看项目是否被排除、已启用 Agent 数、运行/恢复/复用状态、项目累计 Token、当前观察增量、活跃 Thread 累计 Token 与 Top 3 Thread。浮窗支持置顶、返回主窗口和隐藏；隐藏后可从主窗口恢复，不会重复创建窗口。
- **重启提示**：配置同步后，只要检测到新的 Codex 实例，红色重启提示即可自动清除。

项目监控数据默认每 3 秒刷新一次。这里展示的是 CAS 从 Codex 原生 Thread/rollout 观测到的 Token 与生命周期事实，不是实时计费器；窗口关闭按钮会执行隐藏，以保留项目选择和置顶偏好。2026-08-23 已在真实 Windows 桌面端完成“打开 → 隐藏 → 从用量监控重新打开”的闭环验证，重新出现后仍只有一个浮窗，并恢复原项目、Thread、Token 和同步状态。

## 配置与安全

配置采用 Preview → Apply 流程，含冲突检测、快照、回读校验和失败回滚。`cas-helper` 负责凭据交付与调度预检；Provider 密钥存入 Windows 凭据管理器，Codex 配置仅引用凭据标识。删除 Provider 时先在 CAS 数据库记录待清理凭据，再删除并回查 Windows 凭据；清理未完成会保留队列并在后续启动重试。

编排投影的清理边界：只有 Apply / 运行模式切换会改写 `.codex`；切回 Default 时按 baseline 精确还原 `config.toml` 相关片段，并删除 `agents/cas-*.toml`、`cas/bundled-skills/*` 等 CAS 托管资源。新版不会向全局 `AGENTS.md` 写入 Primary 协议；若检测到旧版 CAS 托管块，则只移除该块并保留用户内容。关闭应用时自动执行同一清理路径。

## Roadmap（下一阶段：v0.5 RC）

- **真实编排闭环（RC-1 已完成）**：Codex Native 已固定验证 Primary → SPAWN → bind → IDLE → REUSE → follow-up，并核对父子 Thread、Token 与决策日志。
- **并发与失配矩阵（RC-2 已完成）**：同任务并发返回唯一 SPAWN 与 WAIT；Task Scope、Runtime Fingerprint、Workspace 或 Context 变化时稳定拒绝旧 Thread，并给出 SPAWN 建议。
- **恢复能力（Runtime Bridge 当前版本矩阵已完成）**：空闲/运行中断流、启动失败与恢复上限已闭环；继续覆盖 Default 往返、项目排除、配置冲突、凭据清理重试、旧数据库升级和本地多 Codex 版本样本。
- **复用池生命周期（已完成）**：支持按 Thread 移出、完成后退休、受控恢复和客观条件批量清理；退休记录继续保留 Token 与调度证据。
- **发布候选**：在干净环境完成 NSIS 全新安装、0.4.1 升级、卸载边界和 sidecar 校验；CI 产出可核验的安装包。

在上述门槛通过前，不继续增加 Reuse Score、AI 任务分类或费用估算。

## 开发验证

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace
npm.cmd run build
git diff --check
npm.cmd run bundle:windows
```

`bundle:windows` 会生成 NSIS x64 安装包，并携带 `cas-helper.exe`。

需要真实 Codex 登录和活动 Agent 时，可单独执行 RC-1、RC-2 或 Phase 6；它们不会进入默认测试：

```powershell
npm.cmd run e2e:orchestration -- -AgentKey <agent-key> -TimeoutSeconds 180
npm.cmd run e2e:orchestration:matrix -- -AgentKey <codex-native-agent-key> -TimeoutSeconds 180
npm.cmd run e2e:runtime-recovery -- -Scenario Idle -TimeoutSeconds 120
npm.cmd run e2e:runtime-recovery -- -Scenario Running -TimeoutSeconds 120
npm.cmd run e2e:runtime-recovery -- -Scenario Storm
npm.cmd run e2e:runtime-recovery -- -Scenario StartupFailure
```

多版本验证不自动下载 Codex。对用户已经安装的其他版本重复传入
`-CodexExecutable <path-to-codex.exe>`；CAS 以实际 Schema 能力判定支持、未声明、不兼容或未能证明，
不按版本号猜测功能。

## 系统要求

- Windows 10/11
- Node.js 22+
- Rust（2024 edition）
- Codex CLI 0.144.0+

## License

[MIT](LICENSE) © 2026 [ZhuXi](https://github.com/Zhuxi140)

内置第三方纯 Skill 保留各自 MIT License：[`caveman`](https://github.com/JuliusBrussee/caveman) © 2026 Julius Brussee；[`ponytail`](https://github.com/DietrichGebert/ponytail) © 2026 Dietrich Gebert。CAS 未捆绑 `caveman` 仓库中采用其他许可证的 Proxy / Engine。
