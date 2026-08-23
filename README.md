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
> **v0.4.1 当前仍不推荐作为稳定生产工具安装使用。** 项目仍处于快速迭代阶段，Apply 配置会改写 Codex 全局配置及 `AGENTS.md` 编排资源；第三方 Provider 的兼容性会因 Provider 和模型的工具协议而存在差异；安装包也尚未进行代码签名。建议仅在测试环境尝鲜，使用前备份现有配置，并在 Apply 前后仔细核对 Preview 与 Snapshot。

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
- **编排、调度、复用三位一体**：多 Agent 按阶段自动委派；Thread 复用（REUSE / SPAWN）按运行时指纹、Workspace Scope 与当前上下文健康度客观判定，避免重复烧钱。
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
4. 在 Models 与 Agents 页面绑定模型，并在运行模式中启用编排配置；Preview 后 Apply。
5. 在用量页面查看原生子 Agent Thread 的生命周期（基于 rollout 事实）、当前上下文与 Token 统计。

安装包当前未进行代码签名；Windows SmartScreen 可能显示警告，请按组织安全策略核验 Release 的 SHA-256。

## 测试状态与客观数据

以下结果对应 v0.4.1 发布前的实际验证，可用仓库中的验证命令复查。

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
| Codex Native `gpt-5.6-terra` 子 Agent | 已人工验证 | 已完成 SPAWN → 执行 → 结果返回，并写入目标文件 |
| DeepSeek Responses 子 Agent | 已有成功实测 | 仅说明该实测配置可运行，不外推至其他 Provider 或模型 |
| 外部配置 E2E 自动化 | 未纳入默认测试 | 仍依赖实际 Provider、模型与工具协议配置 |
| 阿里及其他 Provider | 待测试 | 不声明已通过 |

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

## 多 Agent 编排

CAS 将 Agent 分为 Primary、Discovery、Execution、Verification、Review 等 Role/Phase。编排模式下每种 Role 只能启用一个 Agent，避免职责和模型绑定发生歧义。Primary 负责读取、规划、审查和收束；所有实现命令与文件写入必须委派给 Execution Agent。

失败策略可选：

- **Strict Stop**：原 Thread 无法续接时，先由同职责 replacement Agent 接棒；只有 replacement 仍不可用、连续替换没有可验证进展或结果不可验证时才停止并报告，Primary 不静默接管。
- **Primary Fallback**：同样优先续接或替换子 Agent；恢复失败后才允许 Primary 在明确提示后接管，并保留回退原因。

项目可被排除在 CAS 编排之外；项目级配置与全局配置冲突时，CAS 会检测冲突并要求查看后中止或显式重新 Apply，而不会覆盖外部修改。

## 调度与 Thread 复用

每次独立任务在委派前由 `cas-helper schedule <agent-key>` 预检，输出唯一的 `CAS1|SPAWN|...` 或 `CAS1|REUSE|...` 决策。`SPAWN` 创建新原生子 Agent Thread；`REUSE` 将完整任务交给既有 Thread。

0.4.0 起，调度直接感知 Codex 原生运行时，不再依赖用量页面是否打开：

- **原生状态直读**：`schedule` 在决策前以只读方式重新打开 Codex 的 `state_*.sqlite`，合并原生候选线程后再计算，新 Spawn 的子 Agent 无需经过 CAS 同步即可进入候选。
- **生命周期以 rollout 为准**：线程状态由 Codex rollout 尾部最后一个明确事件判定（`task_complete`/`turn_aborted` 为 `IDLE`，`task_started` 为 `RUNNING`，无法证明为 `UNKNOWN`）；writer lock 文件存在不等于线程正在运行，残留锁不会永久阻断复用。
- **SPAWN 后必须 bind**：`spawn_agent` 成功返回 child Thread ID 后，Primary 须立即执行 `cas-helper bind <agent-key> <child-thread-id>` 固化线程归属与运行时指纹；bind 只读核验原生身份后才事务写入，身份缺失或指纹冲突拒绝覆盖。

复用是一个客观判定：**同一 Agent（含运行时指纹）、同一 Primary、Exact Workspace Scope、IDLE 状态且上下文健康**。

- **显式 Task Scope**：`cas-helper schedule <agent-key> [task-key]` 支持由 Primary 从任务描述提取的稳定任务键（如 `auth-oauth2`）。只有 `task-key` 完全一致的空闲 Thread 才会被复用；未传任务键的预检不会复用任何绑定了任务键的 Thread（fail-closed），CAS 不做任何模糊分类或历史猜测。`bind` 同样接受并固化任务键，既有键不被覆盖。

- **运行时指纹**：每个 Agent 配置生成稳定 `runtime_fingerprint`（纳入 Provider 身份、Base URL、模型、指令、推理与沙箱策略、能力集合），配置变更后旧 Thread 立即失配并 `SPAWN`，不会复用旧配置的线程。
- **Workspace Scope**：当前工作目录的规范化值（UNIX/UNC 统一归一），不是逻辑任务或模块匹配；执行入口会拒绝 Scope 与实际 `cwd` 不一致的请求。未提供 Task Scope 时，同一工作区内的不同任务不会被自动区分；提供显式 Task Scope 后才按任务键精确隔离。
- **上下文健康以当前上下文为准**：仅使用 `current_context_tokens`（来自 App Server `lastTokenUsage` 或 rollout 尾部解析），累计 `totalTokenUsage`/`tokens_used` 只用于用量统计；当前上下文或运行时窗口未知时 fail closed 为 `CONTEXT_UNKNOWN` 并 `SPAWN`，不会把「无法证明健康」当作健康。

调度器还结合 Agent 的 `AUTO` / `HOT` / `COLD` 策略、Provider 的缓存能力和缓存保留提示，避免复用上下文压力过高或已超出缓存窗口的 Thread。

成功的子 Agent Thread 会保留并同步为 `IDLE`，不得自动调用 `close_agent`；仅在用户明确要求、Agent 被停用或移除、Thread 异常不可用，或 CAS 判定不再可复用时才允许关闭。

## Provider、Model 与用量

- **Codex Native**：可将当前 Codex 登录中的原生 Provider/Model 绑定为子 Agent，包括 gpt-5.6 Terra 等可用模型。
- **第三方 Provider**：支持 Responses API Provider、模型发现和能力校验；凭据不写入 Codex 配置文件。
- **原生 Thread 同步**：同步子 Agent Thread 及其生命周期状态（基于 rollout 事实，而非 writer lock），便于确认运行、完成和空闲状态。
- **Token 监控**：采集输入、缓存输入、输出、推理输出与总 Token，并单独记录当前上下文 Token（`current_context_tokens`）；CAS 只统计 Token，不计算或展示费用，也不保存 Prompt、Response 正文或 API Key。
- **重启提示**：配置同步后，只要检测到新的 Codex 实例，红色重启提示即可自动清除。

## 配置与安全

配置采用 Preview → Apply 流程，含冲突检测、快照、回读校验和失败回滚。`cas-helper` 负责凭据交付与调度预检；Provider 密钥存入 Windows 凭据管理器，Codex 配置仅引用凭据标识。

编排投影的清理边界：只有 Apply / 运行模式切换会改写 `.codex`；切回 Default 时按 baseline 精确还原 `config.toml` 相关片段与全局 `AGENTS.md`，并删除 `agents/cas-*.toml` 等 CAS 托管资源（不触碰用户自有内容）；关闭应用时自动执行同一清理路径。

## Roadmap（按可发布门槛排序）

- **并发正确性**：SPAWN 幂等去重——在显式 Task Key 基础上补齐「同任务重复预检返回既有结果」的回写与重试语义。
- **配置与版本防线**：禁止停用活动 Agent 造成双重事实源；Codex 兼容性由最低版本判断升级为能力级探测，无法证明能力时 Fail Closed。
- **证据与回归**：UI 展示状态来源、数据新鲜度、父子 Thread 与真实决策日志；补齐真实 Primary 委派 → 复用 → 上下文决策的端到端回归。

## 开发验证

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo test --manifest-path src-tauri/Cargo.toml --workspace
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
