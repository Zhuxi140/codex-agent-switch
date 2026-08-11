# Codex Agent Switch（CAS）

### 让旗舰模型负责思考，让高性价比模型负责执行

<p align="center">
  <a href="https://img.shields.io/badge/版本-0.2.0-blue"><img src="https://img.shields.io/badge/版本-0.2.0-blue" alt="版本 0.2.0"></a>
  <a href="https://img.shields.io/badge/平台-Windows%2010%2F11-0078D6"><img src="https://img.shields.io/badge/平台-Windows%2010%2F11-0078D6" alt="平台 Windows 10/11"></a>
  <a href="https://img.shields.io/badge/License-MIT-yellow"><img src="https://img.shields.io/badge/License-MIT-yellow" alt="License MIT"></a>
  <a href="https://img.shields.io/badge/Codex%20CLI-%E2%89%A5%200.144.0-green"><img src="https://img.shields.io/badge/Codex%20CLI-%E2%89%A5%200.144.0-green" alt="Codex CLI ≥ 0.144.0"></a>
</p>

<p align="center">
  <strong>
    🧠 <a href="#核心理念">核心理念</a> ·
    🚀 <a href="#快速开始">快速开始</a> ·
    📋 <a href="#核心功能">核心功能</a> ·
    ⚙️ <a href="#工作原理">工作原理</a> ·
    🔒 <a href="#安全模型">安全模型</a> ·
    🗺️ <a href="#roadmap-与当前进度">Roadmap</a>
  </strong>
</p>

一个 Tauri 2 桌面工具：在 GUI 中管理 Codex 多 Agent 的 Provider / Model / 绑定、编排、调度与 Token 监控——**不手改 Codex 配置**。

<div style="border: 2px solid #d93025; border-left: 6px solid #d93025; background: #fdf0ef; border-radius: 8px; padding: 12px 16px; margin: 16px 0 24px;">

<strong>⚠️ 当前并不推荐安装使用。</strong><br><br>

v0.2.0 是第一个可用的发布版本：大部分核心功能已实现，但仍有<strong>诸多 Bug 与不完善之处</strong>，且<strong>尚未完全通过测试</strong>。<br><br>

第二个发布版本正在打磨中，将更加完善与优秀。欢迎尝鲜体验，但请知悉当前状态并谨慎使用。

</div>

---

## 核心理念

我们不追求打造一支「全面、专业、强大」的子 Agent 团队。我们相信，效果与成本的最优解来自**脑力与体力的分工**：

```
┌─────────────────────────────────────────────────────────────┐
│  主 Agent（旗舰高智商模型，如 ChatGPT 5.6 Sol）                │
│  ── 编排 · 规划 · 审查 · 收束                                 │
│  「想清楚怎么做，并判断做得对不对」                             │
└────────────────────────────────┬────────────────────────────┘
                                 │ 委派（Discovery → Execution → Verification → Review）
            ┌────────────────────┴────────────────────┐
            │              子 Agent 团队               │
            │  轻量 · 可定制 · 按需启用                  │
            │  ── 执行 · 测试 · 探索 · 细节审查          │
            │  每个角色独立绑定高性价比模型               │
            │  「把活干完，把量跑满」                    │
            └─────────────────────────────────────────┘
```

**价值主张：**

- **轻量、可定制**：子 Agent 是薄薄一层「角色 + 职责 + 模型绑定」，不用堆砌全能 Agent，按需创建、随时调整
- **主脑做精，手脚做量**：把编排、规划、审查交给旗舰模型（贵，但决定成败）；把执行、测试、探索交给高性价比模型（便宜，但量大管饱）
- **编排、调度、缓存三位一体**：多 Agent 按阶段自动委派；Thread 复用决策（Reuse / Spawn）避免重复烧钱；Provider 缓存能力建模为后续调度留底
- **结果**：以更合理的价格，实现更好的效果

一句话：**用旗舰模型的判断力，配上高性价比模型的执行力。**

---

## 效率对比（待实测）

> 以下数据均为**占位，尚未完成真实基准测试**。测试计划：对同一任务，分别用「纯旗舰模型单 Agent」与「CAS 主脑 + 高性价比子 Agent 团队」各跑若干轮，取平均值回填下表。

| 指标 | 不用 CAS（纯旗舰单 Agent） | 用 CAS（主脑 + 子 Agent 团队） |
|---|---|---|
| 任务总 Token 消耗 | 待测试 | 待测试 |
| 总费用（价格） | 待测试 | 待测试 |
| 单任务耗时 | 待测试 | 待测试 |
| 产出 / 投入比 | 基准 1.0 | 待测试 |
| 性价比 | 基准 1.0 | 待测试 |

预期方向（待实测验证）：子 Agent 承担的大批量执行消耗由高性价比模型吃掉，总费用显著下降；主 Agent 专注规划与审查，产出质量不降反升。

---

## 推荐方案

「旗舰主脑 + 高性价比子 Agent」的现成组合，在 CAS 中一键落地（运行模式 → 编排化 Subagent → 为各角色绑定模型）：

| 方案 | 主 Agent（规划 / 编排 / 审查） | 子 Agent（执行 / 测试 / 探索） | 适用场景 |
|---|---|---|---|
| 方案一 | ChatGPT 5.6 Sol | DeepSeek V4 Flash（正式版） | 日常开发主力，性价比优先 |
| 方案二 | ChatGPT 5.6 Sol | DeepSeek V4 Pro（即将上线） | 需要更强子 Agent 执行质量 |
| 方案三 | ChatGPT 5.6 Sol | ChatGPT 5.6 Terra / Luan | 同一模型生态内搭配，切换无缝 |

子 Agent 内部仍可分级：Executor / Explorer 用高性价比模型跑量，Reviewer 可上调一档换更强模型——所有角色绑定都可在 GUI 中随时调整。

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
| 看不出子 Agent 花了多少 Token、复用还是重建 | Token 监控 + 子 Agent 实例追踪 + Reuse 决策预览 |

---

## 快速开始

1. **安装**：运行发布产物中的 `Codex Agent Switch_0.2.0_x64-setup.exe`，按向导安装（默认当前用户，无需管理员权限）
2. **环境检测**：首次启动后进入设置页，确认 Codex 可执行文件与 `CODEX_HOME` 已正确解析（Store 版需将 `%USERPROFILE%\.codex\.sandbox-bin\codex.exe` 复制到 `%USERPROFILE%\.local\bin\`）
3. **创建 Provider**：Provider 管理页添加服务商，填入 API Key（明文只进 Windows 凭据管理器，不落盘）
4. **绑定 Agent**：在 Agents 页为各角色选择模型，用「运行模式」一键切换 Default ↔ 编排化 Subagent
5. **应用配置**：Preview → Apply，确认变更清单后应用；出问题可随时从 Snapshot 恢复

想观察效果：用量监控页开启 Token 监控，运行一次 Codex 任务，即可看到各子 Agent Thread 的 Token 消耗与 Reuse 决策。

---

## 核心功能

### 配置管理（V0.1 P0，已发布）

| 模块 | 能力 |
|---|---|
| **Provider 管理** | 创建 / 编辑 / 禁用 / 删除；官方 Preset；API Key 走 `cas-helper` 凭据链，不落盘到 Codex 配置；连通性测试 |
| **Model 管理** | 随 Provider 自动发现或手动添加；`responses` / `chat` wire API；能力校验；启用/禁用 |
| **Agent 管理** | 内置模板（Executor / Explorer / Reviewer / Tester）与自定义 Agent；模型绑定；Role Key + Phase 编排元数据 |
| **配置应用** | Preview → Apply 两段式；Apply 后回读校验；失败自动回滚；冲突检测；Snapshot 列表 / 详情 / 恢复 |
| **诊断服务** | 只读检查 Codex 环境、配置可读可写性、Agent 就绪度 |

### 多 Agent 编排（阶段一~三，已发布）

- **运行模式**：Default（Codex 全权）↔ 编排化 Subagent（按 Role 启用多个 CAS 管理子 Agent）；切换自动建 Snapshot、失败回滚；项目排除
- **Strict Stop 编排**：Primary 只读，Discovery → Execution → Verification → Review 分阶段职责；Executor 缺失或委派失败即停止并报告，严禁静默兜底

### 用量与调度（0.2.0，已发布）

- **Token 用量监控**：CAS 托管启动 `codex app-server`，实时采集 `thread/tokenUsage/updated` 事件，自动归因到子 Agent 与 Model；状态 `LIVE / FINAL / PARTIAL / UNKNOWN` 全程可见
- **子 Agent 实例**：Agent Definition 与 Thread 实例分离，Scope 标记、实例列表、状态追踪
- **Reuse 决策**：提交任务前预览「复用既有 Thread / 新建 Thread」决策；显式托管执行（REUSE 走 `thread/resume`，SPAWN 走 `thread/start`）
- **缓存信息模型**：Provider Cache Profile（`retentionType`: UNKNOWN / APPROXIMATE / GUARANTEED）与 Agent `reuse_strategy`（AUTO / HOT / COLD）已入数据层，为调度评分预留

> **注意**：托管执行会产生真实模型调用与费用，执行前 UI 有显式确认。、

---

## 工作原理

```
┌─────────────────────────────────────────────────────────────┐
│  React + TypeScript 前端（Vite）                              │
│  Overview / 用量监控 / Agents / Providers / Models / 诊断…   │
│  └─ 通过 @tauri-apps/api 调用 IPC command                    │
├─────────────────────────────────────────────────────────────┤
│  Tauri 2 主进程（Rust，crate: codex-agent-switch）            │
│  ├─ Provider / Model / Agent / Configuration 服务层          │
│  ├─ UsageService（Token 幂等入库、调度画像、Reuse 决策）        │
│  ├─ RuntimeBridgeService（托管 codex app-server 子进程）      │
│  ├─ SQLite 持久化（cas.db，rusqlite bundled）                │
│  └─ 写 Codex 配置时调用 cas-helper（同目录 exe）              │
├─────────────────────────────────────────────────────────────┤
│  cas-helper（Rust，独立 crate）                               │
│  │  只做一件事：凭据存取（Windows 凭据管理器）                 │
│  │  协议：命令 + JSON，退出码 0/2/3/4/5/6                     │
│  └─ 被 Codex 以 auth.command 方式调用，返回 API Key           │
└─────────────────────────────────────────────────────────────┘

监控开启时（可选）：
  主进程 ──spawn──► codex app-server（--listen stdio://，JSON-RPC）
                        ├─ thread/start · thread/resume · turn/start（托管执行）
                        └─ thread/tokenUsage/updated 事件回流 → UsageService 幂等入库
```

**核心链路**：CAS 维护 Agent ↔ Model 绑定 → Apply 时把 Provider（含 `auth.command` 指向绝对路径的 `cas-helper`）写入 Codex `config.toml`，把 Agent 写入 `agents/*.toml` → Codex 运行子 Agent 时通过 `cas-helper` 取凭据。

`cas-helper` 必须与主程序同目录，路径在运行时以 `current_exe()` 推断，兼容 debug / release / 安装目录三种部署形态。

### 配置应用机制

1. **Preview**：编译期望状态 → 变更清单 + blocker（Agent 未就绪、冲突、helper 不可用等）
2. **Apply**：语义指纹检测外部改动 → 写入前备份 → 写入后回读 + hash 校验 → 失败按 Journal 回滚
3. **冲突处理**只有两条路：查看 + 中止，或显式重新 Apply（携带新快照）
4. **Snapshot**：每次成功应用生成，可列出 / 查看 / 恢复

配置状态机：`Applied → PendingChanges → Drift → Conflict → RecoveryRequired`（启动检测到未完成事务时进入恢复引导）。

---

## 安全模型

- **凭据不落盘**：`config.toml` 的 `auth.command` 指向 `cas-helper token <uuid>`，文件中只有 UUID，永无 API Key 明文；明文只存 Windows 凭据管理器
- **forbid-list**：检测配置中已知明文密钥模式；外部明文仅作诊断提示，不阻塞无关 Apply
- **最小权限**：`cas-helper` 只做凭据存取，无网络能力，不做配置写入
- **互斥与事务**：OS 文件锁（`LockFileEx`）为权威锁源；Apply 事务化，失败自动回滚
- **监控隐私**：Token 监控只保存计数 / Thread 标识符 / Agent 与模型快照 / 时间戳，**不保存任何 Prompt、Response、正文与 Key**；采集失败 Fail Open（不中断 Codex 工作）；可随时停止监控并清除历史
- **数据位置**：`cas.db` 位于系统 `app_local_data_dir`，不写入 Codex 目录

---

## 环境要求与构建发布

### 环境要求

- **Windows 10/11**（当前目标平台）
- **Rust**（2024 edition）+ **Node.js ≥ 22**
- **Codex CLI ≥ 0.144.0**：Store 版需将 `%USERPROFILE%\.codex\.sandbox-bin\codex.exe` 复制到 `%USERPROFILE%\.local\bin\`（WindowsApps 无执行别名）
- 其余（WebView2 Runtime）Windows 11 自带；全部 Rust 依赖离线可构建

### 构建与发布

| 目的 | 命令 |
|---|---|
| 前端构建验证 | `npm.cmd run build` |
| Rust 测试 | `cargo test --manifest-path src-tauri/Cargo.toml` |
| 生成 Windows 安装包 | `npm.cmd run bundle:windows` |
| App Server POC 自测 | `npm.cmd run poc:app-server:self-test` |

安装包会同时携带 `cas-helper.exe`，无需单独复制凭据助手。

---

## Roadmap 与当前进度

### 三路线状态总览

| 版本 | 编排 | Token 监控 | 调度 / 缓存 |
|---|---|---|---|
| **0.2.0（当前）** | 阶段一~三（Strict Stop）✅ | Phase 0~2（POC → 可靠采集 → UI）✅ | 数据模型 + Reuse/Spawn 决策 + 托管执行 ✅ |
| **P1** | 阶段四（Primary Fallback） | — | 运行时评分、缓存时间实验、AUTO/HOT/COLD 策略深化 |
| **P2** | 编排深度演进 | — | Cost-aware Scheduling、Thread Pool、自动退休/重建 |

### 编排（路线一）

- 阶段一~三已完成：Agent 角色元数据（`role_key` + `orchestration_phase`）、多 Agent 绑定与运行模式、Strict Stop 自动编排
- **剩余（阶段四，P1）**：Primary Fallback 设置与风险提示、回退记录、权限恢复与配套 Diagnostics、真实 Codex E2E（自动创建、等待、结果收束、失败关闭全链路）
- 编排规则基线：`Default` 保留原生行为；`Orchestrated` 同一 Role Key 只启用一个 Agent；Phase 使用四个稳定枚举；Role Key 可扩展；Primary 负责规划/调度/审查/收束，执行 Agent 负责写入

### Token 使用监控（路线二）

- **Phase 0 POC** ✅ 生命周期管理、父子 Thread 映射、Usage 事件采集（参考 `scripts/cas-app-server-poc.mjs`）
- **Phase 1 可靠采集** ✅ 幂等 Upsert、会话恢复、异常状态、Repository 与 IPC 测试
- **Phase 2 UI** ✅ 监控开关、Agent 汇总、时间范围、`LIVE / FINAL / PARTIAL / UNKNOWN` 状态说明
- **Phase 3 费用估算（可选，未开工）**：Model Pricing、价格快照、币种与缓存计费

### Subagent 生命周期与缓存感知调度（路线三）

- **已落地**：Agent Definition 与 Thread 实例分离（迁移 0013）、Provider Cache Profile 与 `reuse_strategy` 数据模型（0014/0015）、Reuse/Spawn 决策与托管执行
- **待执行**：Proof 3（Thread Continuity vs Prompt Prefix Cache 对照）、缓存时间实验（立即 / 1h / 6h / 24h）、HOT/WARM/COLD Runtime State、Cost-aware Scheduling、Thread Pool
- 设计底线：Thread 复用与 Provider Cache 是两个不同的生命周期；Provider Cache **不得作为正确性依赖**

### 明确不做（当前阶段）

Token 定价预测、复杂调度算法、关键词路由引擎、可视化编排器、多个写 Agent 并行改同一工作区、Provider 专用调度、自动修改项目级 `AGENTS.md`。

---

## License

[MIT](LICENSE) © 2026 [ZhuXi](https://github.com/Zhuxi140)
