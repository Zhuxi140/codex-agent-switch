# Codex Agent Switch（CAS）

一个 Tauri 2 桌面工具，用于管理 Codex（multi-agent）各个 subagent 角色使用的模型——**不手改 Codex 配置**，在 GUI 中完成 Provider / Model / Agent 的绑定、应用与诊断。

当前版本：**0.2.0**（首个 Windows 安装包版本）

---

## 目录

- [为什么需要它](#为什么需要它)
- [核心功能](#核心功能)
- [工作原理](#工作原理)
- [安全模型](#安全模型)
- [配置应用机制](#配置应用机制)
- [环境要求](#环境要求)
- [Roadmap 与当前进度](#roadmap-与当前进度)

---

## 为什么需要它

Codex 支持多 Agent 协作（`agents/*.toml` + `[model_providers.*]`），但配置全部手写 TOML，常见痛点：

| 痛点 | CAS 的解法 |
|---|---|
| 手写 `config.toml` 和 `agents/*.toml`，容易出错 | 表单化管理，Apply 前有 Preview |
| Agent 和模型绑定过死，换模型要改多个文件 | Agent 与 Model 解耦，绑定关系在 GUI 中维护 |
| 整套 Agent Team 无法整体切换 | 运行模式一键切换（编排化 Subagent） |
| 难以判断配置是否真的可用 | 内置 Diagnostics、Provider 连通性测试、模型能力校验 |
| 手工改配置可能破坏已有内容 | 全量备份 + Snapshot + Apply 后校验 + 失败自动回滚 |

---

## 核心功能

**已完成（V0.1 P0）：**

- **Provider 管理**：创建 / 编辑 / 禁用 / 删除；官方 Preset（DeepSeek 等）；API Key 走 `cas-helper` 凭据链，不落盘到 Codex 配置；连通性测试
- **Model 管理**：随 Provider 自动发现或手动添加；`responses` / `chat` wire API；能力校验；启用/禁用
- **Agent 管理**：内置模板（Executor / Explorer / Reviewer / Tester）与自定义 Agent；模型绑定；Role Key + Phase 编排元数据；启用/禁用
- **配置应用**：Preview → Apply 两段式；Apply 后回读校验；失败自动回滚；冲突检测；Snapshot 列表 / 详情 / 恢复
- **运行模式**：Default（Codex 全权）↔ 编排化 Subagent（按 Role 启用多个 CAS 管理子 Agent）；切换自动建 Snapshot、失败回滚；项目排除
- **Strict Stop 编排**：Primary 只读，Discovery → Execution → Verification → Review 分阶段职责；Executor 缺失或委派失败即停止并报告，严禁静默兜底
- **诊断服务**：只读检查 Codex 环境、配置可读可写性、Agent 就绪度
- **设置**：自定义 `CODEX_HOME`、codex 可执行路径等

---

## 工作原理

```
┌─────────────────────────────────────────────────────────┐
│  React + TypeScript 前端（Vite）                         │
│  └─ 通过 @tauri-apps/api 调用 IPC command                │
├─────────────────────────────────────────────────────────┤
│  Tauri 2 主进程（Rust，crate: codex-agent-switch）       │
│  ├─ Provider / Model / Agent / Configuration 服务层      │
│  ├─ SQLite 持久化（cas.db，rusqlite bundled）            │
│  └─ 写 Codex 配置时调用 cas-helper（同目录 exe）          │
├─────────────────────────────────────────────────────────┤
│  cas-helper（Rust，独立 crate）                          │
│  │  只做一件事：凭据存取（OS 凭据库 / 加密文件）             │
│  │  协议：命令 + JSON，退出码 0/2/3/4/5/6                 │
│  └─ 被 Codex 以 auth.command 方式调用，返回 API Key       │
└─────────────────────────────────────────────────────────┘
```

核心链路：**CAS 维护 Agent ↔ Model 绑定 → Apply 时把 `cas_deepseek` 等 Provider（含 `auth.command` 指向绝对路径的 `cas-helper`）写入 Codex `config.toml`，把 Agent 写入 `agents/*.toml` → Codex 运行子 Agent 时通过 `cas-helper` 取凭据**。

`cas-helper` 必须与主程序同目录，路径在运行时以 `current_exe()` 推断，兼容 debug / release / 安装目录三种部署形态。

---

## 安全模型

- **凭据不落盘**：`config.toml` 的 `auth.command` 指向 `cas-helper token <uuid>`，文件中只有 UUID，永无 API Key 明文
- **forbid-list**：检测配置中已知明文密钥模式；外部明文仅作诊断提示，不阻塞无关 Apply
- **最小权限**：`cas-helper` 只做凭据存取，无网络能力，不做配置写入
- **互斥与事务**：OS 文件锁（`LockFileEx`）为权威锁源；Apply 事务化，失败自动回滚
- **数据位置**：`cas.db` 位于系统 `app_local_data_dir`，不写入 Codex 目录

---

## 配置应用机制

1. **Preview**：编译期望状态 → 变更清单 + blocker（Agent 未就绪、冲突、helper 不可用等）
2. **Apply**：语义指纹检测外部改动 → 写入前备份 → 写入后回读 + hash 校验 → 失败按 Journal 回滚
3. **冲突处理**只有两条路：查看 + 中止，或显式重新 Apply（携带新快照）
4. **Snapshot**：每次成功应用生成，可列出 / 查看 / 恢复

配置状态机：`Applied → PendingChanges → Drift → Conflict → RecoveryRequired`（启动检测到未完成事务时进入恢复引导）。

---

## 环境要求

- **Windows 10/11**（当前目标平台）
- **Rust**（2024 edition）+ **Node.js ≥ 22**
- **Codex CLI ≥ 0.144.0**：Store 版需将 `%USERPROFILE%\.codex\.sandbox-bin\codex.exe` 复制到 `%USERPROFILE%\.local\bin\`（WindowsApps 无执行别名）
- 其余（WebView2 Runtime）Windows 11 自带；全部 Rust 依赖离线可构建

## 安装与构建

- 普通用户：运行发布产物中的 `Codex Agent Switch_0.2.0_x64-setup.exe`，按向导安装。
- 开发者生成安装包：在仓库根目录运行 `npm.cmd run bundle:windows`。
- 安装包会同时携带 `cas-helper.exe`，无需单独复制凭据助手。
- 默认按当前用户安装，不要求管理员权限；首次运行后再在设置页检测 Codex 环境。

---

## Roadmap 与当前进度

### V0.1（当前，0.2.0）

**已完成：**

- P0 全量功能（见「核心功能」），含编排基础三阶段（`多Agent自动编排设计方案.md` 阶段一~三）：
  - **阶段一 Agent 角色元数据**：`role_key` + `orchestration_phase` 字段、四个 Preset 迁移、Agent CRUD/DTO 校验、页面编辑
  - **阶段二 多 Agent 后端**：`active_agent_bindings` 与兼容迁移、运行模式 IPC（保留旧单 Agent 入口）、多 Agent/Provider/Catalog 依赖闭包编译、去重与过期投影清理、Snapshot 与回滚
  - **阶段三 Strict Stop 自动编排**：Overview 多角色选择、CAS `developer_instructions` 编排片段生成与维护、Primary 只读、缺 Execution Agent 诊断、切回 Default 精确恢复

**剩余事项：**

- E2E 第 5 步：真实委派（隔离 `CODEX_HOME` + `codex login` + executor 真实调用 DeepSeek，验收 `/agent` 出现 executor、返回 `CAS_DEEPSEEK_E2E_OK`）

**明确不做（当前阶段）：** Token 监控、复杂调度算法、关键词路由引擎、可视化编排器、多个写 Agent 并行改同一工作区、Provider 专用调度、自动修改项目级 `AGENTS.md`。

---

### 路线一：多 Agent 自动编排

**V0.1 已完成阶段一~三**（见上）。剩余工作集中在阶段四，进入 P1：

**阶段四：Primary Fallback 与完整验收**

- 增加 `Primary Fallback` 设置与风险提示（当前实现只允许 Strict Stop，编排片段明确「严禁静默 fallback」）
- 实现回退记录、权限恢复与配套 Diagnostics
- 完善 Agent 状态、警告与新建会话提示
- 真实 Codex E2E：自动创建、等待、结果收束、失败关闭全链路

验收标准：子 Agent 失败后 Primary 可接管并显式警告；Strict Stop 与 Primary Fallback 行为稳定可区分；启动参数覆盖磁盘权限时能检测并提示；自定义 Role Key 可参与真实自动委派。

**编排规则基线（已定）：** `Default` 保留原生行为；`Orchestrated` 同一 Role Key 只启用一个 Agent；Phase 使用四个稳定枚举；Role Key 可扩展；Primary 负责规划/调度/审查/收束，执行 Agent 负责写入。

---

### 路线二：Token 使用监控

**状态：设计方案已定稿，尚未开工。**

架构结论：CAS 内置 App Server 代理上游，通过 `collabToolCall` 建立父子 Thread 映射，采集 `thread/tokenUsage/updated` 事件；**不做 Transcript 解析**（不稳定，不得冒充正式实现）。

实施顺序（每阶段单独验收）：

- **Phase 0 POC**：App Server 生命周期管理；父子 Thread 映射；Usage 事件打印开发日志；不写库不做 UI。通过 10 项验收门槛后才允许进入 Phase 1，关键门槛：父子 Thread 能收到可区分的 `tokenUsage` 事件、Streaming/取消/断网/重试不重复累计、会话恢复后累计值单调不重复、统计总量与 `codex exec --json` 可接受一致、不保存任何正文与 Key。若「子线程可区分」失败，停止主路线，重新评估 Responses Proxy
- **Phase 1 可靠采集**：SQLite Migration；累计值幂等 Upsert；会话恢复与异常状态；Repository 与 IPC 测试
- **Phase 2 Agents Usage UI**：Agent 汇总、时间范围、Provider/Model 明细、`LIVE / FINAL / PARTIAL / UNKNOWN` 状态说明
- **Phase 3 费用估算（可选）**：Model Pricing、价格快照、币种与缓存计费、Estimated Cost UI

安全约束：只保存计数/标识符/模型快照/时间；不保存 Prompt、Response、Reasoning、工具参数输出；采集失败 Fail Open（不中断 Codex 工作）；用户可停止监控并清除历史。

---

### 路线三：Subagent 生命周期与缓存感知调度

**状态：设计方案已定稿（含实验 Proof 1/2），V0.1 仅实现基础 Reuse/Spawn 决策，Cache 信息模型尚未预留到数据层。**

核心结论：

- **Agent Definition 与 Agent Instance/Thread 必须分离**：复用的是 Thread 生命周期，不是 Definition
- Thread 复用与 Provider Cache 是**两个不同的生命周期**；Provider Cache **不得作为正确性依赖**
- 缓存命中前旧 Agent 可能更贵（上下文污染），V0.1 不实现复杂评分模型

版本边界：

- **V0.1（已含）**：Agent Definition、Provider/Model 配置、Subagent 基础运行、同 Thread 继续调用、基础 Reuse/Spawn 决策
- **P1**：AUTO / BALANCED / HOT / COLD 策略选择；Provider Cache Profile（`retentionType`: UNKNOWN / APPROXIMATE / GUARANTEED）；Cache Retention Hint；Agent Scope；Context Size 监控；最近使用时间；基础 Agent Reuse Policy
- **P2**：HOT / WARM / COLD Runtime State；Reuse Score；Cost-aware Scheduling（含 Cached / Uncached 定价）；Context Bloat 与 Topic Drift 检测；Agent Thread Pool；Agent 自动退休/重建；真实运行数据驱动调度

**后续验证项（已定，待执行）：**

- Proof 3：重新 Spawn 相同 Agent Definition，比较 Thread Continuity vs Prompt Prefix Cache
- 缓存时间实验（立即 / 1h / 6h / 24h），记录 Cached Input / Uncached Input / Output / Cache Hit Rate / Execution Time，仅用于形成启发式（Heuristic），不代表 Provider 正式 SLA

---

### 里程碑视图

| 版本 | 编排 | Token 监控 | 调度/缓存 |
|---|---|---|---|
| V0.1（当前） | 阶段一~三（Strict Stop）✅ | — | 基础 Reuse/Spawn ✅，Cache 模型未预留 |
| P1 | 阶段四（Primary Fallback） | Phase 0~2（POC→采集→UI） | AUTO/HOT/COLD、Cache Profile、Reuse Policy |
| P2 | 编排深度演进 | Phase 3（费用估算，可选） | Runtime State、Cost-aware、Thread Pool |
