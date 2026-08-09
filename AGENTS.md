# AGENTS.md

## What this repo is

Design-only repository for **Codex Agent Switch (CAS)** — a Tauri 2 desktop tool that manages
which model each Codex (multi-agent) subagent role uses, without hand-editing Codex config.

**There is no source code yet.** No `Cargo.toml`, `package.json`, CI, or tests exist. All
technical decisions live in the 10 Chinese-language spec docs in this directory. They are the
source of truth — read the relevant one before proposing any implementation:

| File | Covers |
|---|---|
| `产品需求文档（PRD）.md` | Product scope, user stories, P0/P1 for V0.1, success criteria |
| `系统结构文档.md` | Architecture: layers, modules, runtime flow, planned crate layout |
| `项目代码规范.md` | Rust/TS coding standards, naming, error strategy, async, TOML rules |
| `数据模型文档.md` | Core entities: Provider/Model/Agent/Profile, bindings, relations |
| `持久化设计文档.md` | SQLite schema, migrations, repository mapping, transactions |
| `配置集成规范.md` | Codex config read/compile/patch/backup/restore, ownership & conflict rules |
| `安全与凭据管理规范.md` | Secrets, OS credential store, `cas-helper`, forbid-list (non-negotiable) |
| `Provider - Model 接入规范.md` | Provider/preset/model-catalog integration, compatibility levels |
| `Tauri Command - IPC 接口规范.md` | IPC command contracts, DTOs, error codes, events |
| `UI - UX 设计文档.md` | Info architecture, pages, components, states |

Docs are written in Chinese; terminology mixes Chinese and English identifiers (e.g. `cas_deepseek`, `apply_configuration()`). Match that style in new text and code.

## 语言约束（全局）

- 中文为默认输入输出语言：与用户交流、代码注释、文档、Commit Message、Issue/PR 表述等默认使用中文（技术术语、代码标识符、命令名如 `cas-helper`、`apply_configuration()` 保持英文原样）。
- 除非用户明确要求英文，否则一切交流与产出使用中文。

## 行为约束（全局）— 减少常见 LLM 编码错误

> 取舍：以下准则偏重谨慎而非速度。对琐碎任务，可自行判断简化。

### 1. 先思考，再编码

- 不要假设；不要隐藏困惑；主动摆出取舍。
- 实现前：明确陈述你的假设。不确定就问。
- 存在多种解读时，全部列出，不要默默选一个。
- 存在更简单的方案时，直接说出来；必要时提出反对。
- 如果某事不清楚，停下来，指出让你困惑的地方并提问。

### 2. 简单优先

- 只写解决问题所需的最小代码，不做投机性设计。
- 不加未被要求的功能、不为单次使用代码做抽象、不加未被要求的「灵活性/可配置性」、不为不可能场景写错误处理。
- 如果写了 200 行而其实 50 行可以完成，重写。
- 自问：「资深工程师会认为这过度复杂吗？」如果是，简化。

### 3. 外科手术式修改

- 只动必须动的地方；只清理自己造成的乱子。
- 编辑既有代码时：不要「顺手改进」相邻代码、注释或格式；不要重构没坏的东西；即使你有别的写法，也遵循既有风格。
- 发现无关的遗留死代码，可以提及，但不要删除。
- 修改产生孤儿代码时：移除「你自己的修改」导致不再使用的 import/变量/函数；不要删除既有死代码（除非被要求）。
- 检验标准：每一处改动都能直接追溯到用户的请求。

### 4. 目标驱动的执行

- 把任务转化为可验证目标，循环直到验证通过：
  - 「加校验」→「为非法输入写测试，再让测试通过」
  - 「修 Bug」→「写能复现它的测试，再让它通过」
  - 「重构 X」→「保证重构前后测试都通过」
- 多步任务先给出简要计划：
  ```
  1. [步骤] → 验证：[检查方式]
  2. [步骤] → 验证：[检查方式]
  3. [步骤] → 验证：[检查方式]
  ```
- 成功标准越强，越能独立循环；弱的成功标准（「能跑起来就行」）会不断需要澄清。

### 5. 依赖安装及时止损

- 涉及依赖、组件、工具链或其他资源的下载与安装时，必须持续观察进度。
- 连续 60 秒无有效进度，或速度异常缓慢且预计无法在合理时间完成时，立即终止当前操作。
- 终止后显式告知用户卡住的命令、依赖或阶段，并请用户手动下载或安装。
- 未经用户明确同意，不得继续长时间等待、反复重试、擅自更换镜像或采用其他绕过方案。

> 这些准则生效的标志：diff 中无关改动变少、因过度设计而重写的次数变少、澄清问题发生在实现之前而非犯错之后。
