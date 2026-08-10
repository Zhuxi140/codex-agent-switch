# Codex Agent Switch（CAS）

一个 Tauri 2 桌面工具，用于管理 Codex（multi-agent）各个 subagent 角色使用的模型——**不手改 Codex 配置**，在 GUI 中完成 Provider / Model / Agent 的绑定、应用与诊断。

当前版本：**0.2.0**（开发中，未发布正式产物）

---

## 目录

- [为什么需要它](#为什么需要它)
- [核心功能](#核心功能)
- [界面说明](#界面说明)
- [工作原理](#工作原理)
- [技术栈](#技术栈)
- [目录结构](#目录结构)
- [环境要求](#环境要求)
- [开发](#开发)
- [构建与发布](#构建与发布)
- [安全模型](#安全模型)
- [配置应用机制](#配置应用机制)
- [状态模型](#状态模型)
- [端到端测试（E2E）](#端到端测试e2e)
- [文档索引](#文档索引)
- [Roadmap 与当前进度](#roadmap-与当前进度)

---

## 为什么需要它

Codex 支持多 Agent 协作（`agents/*.toml` + `[model_providers.*]`），但配置全部手写 TOML，常见痛点：

| 痛点 | CAS 的解法 |
|---|---|
| 手写 `config.toml` 和 `agents/*.toml`，容易出错 | 表单化管理，Apply 前有 Preview |
| Agent 和模型绑定过死，换模型要改多个文件 | Agent 与 Model 解耦，绑定关系在 GUI 中维护 |
| 整套 Agent Team 无法整体切换 | Profile / 运行模式一键切换（P1 完善） |
| 难以判断配置是否真的可用 | 内置 Diagnostics、Provider 连通性测试、模型能力校验 |
| 手工改配置可能破坏已有内容 | 全量备份 + Snapshot + Apply 后校验 + 失败自动回滚 |

目标用户：Codex Multi-Agent 用户、多模型（尤其是 DeepSeek 等第三方 Provider）用户、成本敏感型开发者。

---

## 核心功能

**已完成（V0.1 P0）：**

- **Provider 管理**：创建 / 编辑 / 禁用 / 删除 Provider；支持官方 Preset（如 DeepSeek）；API Key 交由 `cas-helper` 凭据链管理，**不落盘到 Codex 配置**
- **Model 管理**：随 Provider 自动发现，或手动添加；支持 `responses` / `chat` wire API；能力校验与启用/禁用
- **Agent 管理**：内置模板（Executor / Explorer / Reviewer / Tester）与自定义 Agent；Agent ↔ Model 绑定；启用/禁用；角色（Role）与阶段（Phase）编排配置
- **配置应用（Apply）**：Preview → Apply 两段式；Apply 后自动回读校验（hash）；失败自动回滚
- **备份与恢复**：Apply 前自动备份；Snapshot 列表 / 详情 / 恢复
- **冲突处理**：检测他人改动，视图 + 中止，或显式重新 Apply（带快照）
- **运行模式切换**：Default（Codex 全权负责）↔ Subagent（按 Role 启用多个 CAS 管理的子 Agent）；切换期间自动创建 Snapshot，失败自动回滚
- **项目排除（Project Exclusion）**：指定项目不做编排/模式切换管理
- **诊断（Diagnostics）**：只读检查 Codex 环境、配置可读可写性、Agent 就绪度等
- **设置**：自定义 `CODEX_HOME`（隔离测试环境）、自定义 `codex` 可执行路径、模型目录扫描等
- **Strict Stop 编排**：按 Role Phase（Discovery → Execution → Verification → Review）管理子 Agent 职责

**规划中（P1+）：** Profile 完整能力、Model Discovery 增强、Agent 编排自动配置的更深度集成、Token 使用监控、多 Agent 自动编排方案细化（见 `多Agent自动编排设计方案.md`、`Token 使用监控设计方案.md`）。

---

## 界面说明

导航共 6 个页面：

| 页面 | 内容 |
|---|---|
| **概览（Overview）** | 运行模式选择（Default / Subagent）、配置应用状态（Applied / PendingChanges / Drift / Conflict / RecoveryRequired / Unavailable）、Preview / Apply 入口、Snapshot 列表与恢复 |
| **Agents** | Agent 列表与详情、从模板创建/自定义创建、模型绑定、启用禁用、Role 与 Phase 设置 |
| **Providers** | Provider 列表与详情、Preset 填充、API Key 录入、连通性测试、启用/禁用 |
| **Models** | 模型列表、自动发现结果、手动添加、wire API 与能力配置、启用/禁用 |
| **诊断** | 运行只读 Diagnostics，展示环境检测与配置状态 |
| **设置** | 自定义 `CODEX_HOME`、codex 可执行路径、行为开关 |

---

## 工作原理

```
┌─────────────────────────────────────────────────────────┐
│  React + TypeScript 前端（Vite）                         │
│  └─ 通过 @tauri-apps/api 调用 IPC command                │
├─────────────────────────────────────────────────────────┤
│  Tauri 2 主进程（Rust，crate: codex-agent-switch）       │
│  ├─ Provider / Model / Agent / Configuration 服务层     │
│  ├─ SQLite 持久化（cas.db，rusqlite bundled）           │
│  └─ 写 Codex 配置时调用 cas-helper（同目录 exe）         │
├─────────────────────────────────────────────────────────┤
│  cas-helper（Rust，独立 crate）                          │
│  │  只做一件事：凭据存取（OS 凭据库 / 加密文件）          │
│  │  协议：命令 + JSON，退出码 0/2/3/4/5/6               │
│  └─ 被 Codex 以 auth.command 方式调用，返回 API Key     │
└─────────────────────────────────────────────────────────┘
```

核心链路：**CAS 维护 Agent ↔ Model 绑定 → Apply 时把 `cas_deepseek` 等 Provider（含 `auth.command` 指向绝对路径的 `cas-helper`）写入 Codex `config.toml`，把 Agent 写入 `agents/*.toml` → Codex 运行子 Agent 时通过 `cas-helper` 取凭据**。

ABI 约定：`cas-helper` 必须与主程序 `codex-agent-switch.exe` 同目录，路径在运行时以 `current_exe()` 推断，支持 debug/release/安装目录三种部署形态。

---

## 技术栈

| 层 | 技术 |
|---|---|
| 桌面框架 | Tauri 2（Rust 2024 edition） |
| 前端 | React 19 + TypeScript 7 + Vite 8 |
| 持久化 | SQLite（rusqlite 0.40，bundled 编译） |
| 网络 | reqwest 0.13（blocking / json / rustls），仅用于 Provider 连通性测试 |
| 凭据库 | `cas-secret-store`（工作区 crate）；Windows 上可选 OS 凭据库 |
| 配置解析 | toml_edit 0.23（保留注释与格式的 TOML 编辑） |
| 辅助工具 | tauri-plugin-opener、sha2（指纹）、uuid、windows-sys |

Tauri 依赖（除网络测试外）全部离线可构建：`CARGO_NET_OFFLINE=true` 时 `npm run build` + `cargo build --workspace --offline` 即可完成。

---

## 目录结构

```
.
├── src/                          # 前端（React + TS）
│   ├── App.tsx                   # 所有页面与 IPC 调用（单文件实现）
│   ├── api.ts                    # Tauri command 封装
│   ├── styles.css
│   └── main.tsx
├── src-tauri/
│   ├── src/                      # 后端（Rust）
│   │   ├── lib.rs                # IPC command 注册与 App 装配
│   │   ├── configuration.rs      # 配置状态机、Apply/Preview/回滚/Snapshot/诊断（核心）
│   │   ├── codex_config.rs       # Codex TOML 读写与编译
│   │   ├── codex_environment.rs  # Codex 环境检测
│   │   ├── agent.rs / model.rs / provider.rs / settings.rs / persistence.rs
│   │   ├── domain.rs             # 领域模型
│   │   └── migrations/           # SQLite 迁移（0010 张表演进）
│   ├── cas-helper/               # 凭据 helper（独立 crate）
│   ├── crates/cas-secret-store/  # 凭据存储抽象（工作区 crate）
│   ├── resources/model-definitions/  # 内置模型目录（include_str! 嵌入）
│   ├── icons/icon.ico
│   └── tauri.conf.json           # 构建/窗口/打包配置
├── dist/                         # Vite 构建产物（gitignore）
├── docs/
│   ├── README.md                 # 领域术语表入口
│   └── adr/0001~0013             # 架构决策记录（ADR）
├── CONTEXT.md                    # 领域建模词汇表
├── CONFIG_SYSTEM.md              # 配置系统设计笔记
├── d.md                          # 真实 E2E 验收步骤（随会话更新）
└── *.md                          # 10 份中文规格文档（source of truth）
```

> 注意：`.gitignore` 忽略所有 `*.md`（唯一例外是 `README.md`），因此规格文档、ADR、`d.md` 均**只在本地**，不进入 Git 仓库。新决策请同步更新本地文档。

---

## 环境要求

- **Windows 10/11**（当前开发与发布目标平台）
- **Rust**：2024 edition（开发机为 rustc 1.97+ / cargo 1.97+）
- **Node.js** ≥ 22（Vite 8 要求），使用 npm
- **Python**（仅 Tauri 首装时的 C 编译辅助链，构建 WebView2 无需）
- **Codex CLI**：运行目标 ≥ 0.144.0（multi-agent >= "v1"），无版本上限；建议通过独立安装获取可执行 `codex.exe`（Store 版 MSIX 的 WindowsApps 无 execution alias，需手动将 `C:\Users\<user>\.codex\.sandbox-bin\codex.exe` 复制到 `%USERPROFILE%\.local\bin\codex.exe` 并确保该目录在 PATH 中）
- **WebView2 Runtime**：Windows 11 自带；Windows 10 需安装

---

## 开发

```powershell
# 1. 安装前端依赖
npm install

# 2. 开发模式（热更新，Tauri 窗口连 Vite dev server）
npm run tauri dev

# 或分别启动：
npm run dev              # 仅 Vite（浏览器预览，IPC 不可用）
```

离线构建（不装任何新依赖）：

```powershell
$env:CARGO_NET_OFFLINE = "true"
npm run build                                  # tsc + vite build → dist/
cargo build --workspace --offline --manifest-path ".\src-tauri\Cargo.toml"
```

产物：

- `src-tauri\target\debug\codex-agent-switch.exe`（主程序）
- `src-tauri\target\debug\cas-helper.exe`（凭据 helper，须与主程序同目录）

> 注意：dev/debug 构建加载的是 `http://localhost:1420`（`devUrl`），**必须**先启动 Vite（`npm run dev` 或 `npm run tauri dev`），否则窗口内报「localhost 拒绝连接」属正常现象。release 构建则把前端嵌入 exe，无需 dev server。

---

## 构建与发布

> 当前 `src-tauri/tauri.conf.json` 中 `"bundle": { "active": false }`——**只产出裸 exe，不生成安装包**。安装包流程见下文第二节。

### 绿色版（两个 exe，免安装）

```powershell
$env:CARGO_NET_OFFLINE = "true"

# 1. 前端构建 + 主程序 release 构建（自动嵌入前端资源）
npx tauri build --no-bundle          # 或 cargo build --release --manifest-path .\src-tauri\Cargo.toml 后手动嵌入

# 2. 单独构建 cas-helper（tauri build 不会自动编译 workspace 其他 crate）
cargo build -p cas-helper --release --manifest-path ".\src-tauri\Cargo.toml"

# 3. 两个 exe 放同一目录即可分发：
#    src-tauri\target\release\codex-agent-switch.exe
#    src-tauri\target\release\cas-helper.exe
```

分发时用户目录下 `cas.db`、Snapshot、凭据存储会自动创建在 `%LOCALAPPDATA%\<identifier>`（当前 identifier `com.codexagentswitch.desktop`）。

### NSIS 安装器（单文件 setup.exe）

需要三步配置，然后把 `"bundle": { "active": false }` 改为：

```jsonc
"bundle": {
  "active": true,
  "targets": ["nsis"],
  "icon": ["icons/icon.ico", "icons/32x32.png", "icons/128x128.png", "icons/128x128@2x.png", "icons/icon.icns"],
  "externalBin": ["../target/release/cas-helper"]
  // 还需补充 publisher、copyright 等信息；Windows 上如需签名需额外处理
}
```

- 完整 PNG/ICNS 图标集：`npx tauri icon path/to/source.png` 自动生成
- `externalBin` 会把 `cas-helper.exe` 放进安装目录（与主 exe 同级），运行时路径推断依然有效
- 构建：`npx tauri build` → `src-tauri\target\release\bundle\nsis\*.exe`

### 发布前检查清单

- [ ] E2E 第 5 步（真实委派调用）通过
- [ ] `tauri.conf.json` 版本号与 `Cargo.toml` / `package.json` 一致（当前均为 0.2.0）
- [ ] 图标、identifier、publisher 完整
- [ ] 用干净的隔离 `CODEX_HOME` 跑一遍适用流程

---

## 安全模型

核心原则（详见《安全与凭据管理规范.md》）：

- **凭据不落盘到 Codex 配置**：`config.toml` 的 `auth.command` 指向 `cas-helper token <uuid>`，配置文件中只有 Credential UUID，永远没有 API Key 明文
- **严禁手动执行 `cas-helper token <uuid>`**：它会把 Key 输出到终端
- **forbid-list**：检测配置中已知的明文密钥模式；外部明文视为诊断提示，不阻塞不相关的 Apply
- **最小权限**：`cas-helper` 只做凭据存取，不做任何配置写入，无网络能力
- **互斥与事务**：OS 文件锁（Windows `LockFileEx`）是权威锁源；Apply 以事务方式执行，失败自动回滚
- **数据位置**：应用数据（`cas.db`）位于系统 `app_local_data_dir`，不写入 Codex 目录

---

## 配置应用机制

Apply 是一个「两段式 + 校验 + 可回滚」的事务：

1. **Preview**：编译期望状态 → 产出变更清单与 blocker（如 Agent 未就绪、冲突、helper 不可用）
2. **Apply**：
   - 按适用范围（共享文件碎片 / CAS 独占文件整体）计算语义指纹，检测外部改动
   - 写入前自动备份
   - 写入后回读 + hash 校验
   - 失败按 Journal 回滚（PREPARED → COMMITTED）
3. **冲突处理**只有两条路：查看 + 中止，或显式重新 Apply（携带新快照）
4. **Snapshot**：每次成功应用生成，可列出 / 查看 / 恢复

`ConfigurationStatus` 状态机：`Applied` → `PendingChanges` → `Drift` → `Conflict` → `RecoveryRequired`（启动时若检测到未完成事务，进入恢复引导）。

---

## 状态模型

- **Draft / Saved / Applied**：配置生命周期三态（对应 Codex 配置的"未改动 / 已保存到 CAS / 已写入磁盘"）
- **运行模式**：`Default`（Codex 全权）与 `Subagent`（多个 CAS 管理的子 Agent 按 Role 启用）
- **Agent 就绪度**：`READY` / 未就绪（缺 Provider、缺 Model），未就绪 Agent 不允许进入 Subagent 模式

---

## 端到端测试（E2E）

当前进展（见 `d.md`，随会话更新）：

| 步骤 | 内容 | 状态 |
|---|---|---|
| 1 | `codex --version` ≥ 0.144.0 | ✅ 通过 |
| 2 | 建立隔离 `CODEX_HOME`（`%LOCALAPPDATA%\CAS-E2E\codex-home`） | ✅ 通过 |
| 3 | 离线构建 + 启动应用 | ✅ 通过（dev 构建需 Vite dev server） |
| 4 | GUI 配置：DeepSeek Provider / Model / Agent 绑定 / Preview / Apply / 诊断 | ✅ 通过（`config.toml` 含 `cas_deepseek`、`wire_api = "responses"`、`auth.command` 绝对路径；`agents\cas-executor.toml` 正确；无明文 Key） |
| 5 | 真实委派：`CODEX_HOME` 指向隔离目录，`codex login` + 让 executor 真实调用 DeepSeek | ⏳ 未执行 |

E2E 操作要点：

- 全程用**普通 PowerShell**（Codex 内置沙箱终端无法执行 `codex --version`）
- 变量 `$CasE2eHome` 在**新的 PowerShell 会话中会丢失**，重新定义后再用
- Key 只粘贴进 GUI，不进入终端、不发送给任何人
- 成功标准：`/agent` 中出现 `executor`、返回 `CAS_DEEPSEEK_E2E_OK`、无 401/403 或协议错误、DeepSeek 控制台有请求

---

## 文档索引

仓库内的中文规格文档是**唯一事实来源（source of truth）**，改动前必读对应文档：

| 文档 | 覆盖内容 |
|---|---|
| `产品需求文档（PRD）.md` | 产品范围、用户故事 US-001~010、P0/P1 功能、V0.1 决策基线、成功路径 |
| `系统结构文档.md` | 架构分层、模块、运行时流程、计划中的 crate 布局 |
| `项目代码规范.md` | Rust/TS 编码规范、命名、错误策略、async、TOML 规则 |
| `数据模型文档.md` | 核心实体：Provider / Model / Agent / Profile、绑定与关系 |
| `持久化设计文档.md` | SQLite schema、迁移、仓储映射、事务 |
| `配置集成规范.md` | Codex 配置读写/编译/补丁/备份/恢复、所有权与冲突规则 |
| `安全与凭据管理规范.md` | 密钥、OS 凭据库、`cas-helper`、forbid-list（不可协商） |
| `Provider - Model 接入规范.md` | Provider / Preset / 模型目录接入、兼容性等级 |
| `Tauri Command - IPC 接口规范.md` | IPC command 契约、DTO、错误码、事件 |
| `UI - UX 设计文档.md` | 信息架构、页面、组件、状态 |
| `多Agent自动编排设计方案.md` | 编排（Strict Stop / Role / Phase）设计 |
| `Token 使用监控设计方案.md` | Token 用量监控方案 |

其他：

- `CONTEXT.md` / `docs/README.md`：领域建模词汇表（Ubiquitous Language）
- `docs/adr/0001 ~ 0013`：架构决策记录（如 Agent.key == Codex agent 名、cas-helper 绝对路径注入、fingerprint 校验、凭据每 Provider 一条等）
- `CONFIG_SYSTEM.md`：配置系统实现笔记

术语与标识符风格：中文叙述 + 英文标识符（如 `cas_deepseek`、`apply_configuration()`、`cas-helper`），新文本保持一致。

---

## Roadmap 与当前进度

- **V0.1（当前）**：P0 范围已实现并部分通过 E2E；发布产物流程待跑（见"构建与发布"）
- **V0.2 方向（P1）**：Profile 完整管理（激活/默认 Profile）、Model Discovery、编排深度集成、Token 监控
- 本仓库最初为**设计仓库**（AGENTS.md 仍以此定位），现已完成完整代码实现；文档优先于代码演进

如需了解某项具体设计，先读对应规格文档；欢迎通过 Issue/PR 贡献（贡献前请先阅读 `项目代码规范.md` 与对应领域文档）。