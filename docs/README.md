# CAS 决策档案（docs/adr/）

本目录记录 Codex Agent Switch（CAS）领域建模与设计盘问（grilling session）产生的架构决策。术语统一见根目录 `CONTEXT.md`。决策编号按时间顺序递增。

## ADR 清单

| # | 决策 | 一句话 |
|---|------|--------|
| 0001 | [Binding 分层语义](./adr/0001-binding-layers.md) | Base Binding（Agent 默认绑定）与 Profile Binding（激活覆盖）分离；激活 Profile 绝不改写 Base。 |
| 0002 | [Agent 身份键与 Codex name 恒等](./adr/0002-agent-key-name-identity.md) | `Agent.key == Codex agent.name`；UI 名改称 `displayName`；外部改 name = Externally Modified，不是 External。 |
| 0003 | [Capability 证据来源枚举](./adr/0003-capability-source-enum.md) | 统一 Domain 枚举 `OFFICIAL_PROVIDER/OFFICIAL_CODEX/CAS_BUILT_IN/RUNTIME_PROBE/USER_OVERRIDE/UNKNOWN`；Provider 只表达接入方式+Integration Readiness，不定义 Compatibility。 |
| 0004 | [Applied 判定与事务边界](./adr/0004-applied-verification.md) | 不试跑 Codex；写后重读+期望投影/指纹；Journal PREPARED→COMMITTED；短 DB 事务，文件事务独立；无法确认 → UNKNOWN / RECOVERY_REQUIRED。 |
| 0005 | [cas-helper 绝对路径](./adr/0005-helper-absolute-path.md) | `auth.command` 一律绝对路径；路径由 cas-platform 注入，Preset 不许携带 executable。 |
| 0006 | [语义指纹与 Ownership 基线](./adr/0006-semantic-fingerprint.md) | Shared 文件按 fragment 语义指纹、独占文件整文档语义指纹；contentHash 仅诊断；ManagedResource 在 Apply 成功后创建。 |
| 0007 | [冲突处置与 Restore](./adr/0007-conflict-and-restore.md) | 冲突只给「查看中止 / 显式覆盖」二选；Restore 对 Shared 文件只恢复 CAS fragment，Exclusive 文件整体恢复。 |
| 0008 | [cas-helper 运行时协议](./adr/0008-helper-protocol.md) | `cas-helper token <uuid>`；退出码 0/2/3/4/5/6；stderr 结构化 JSON；无缓存、无 daemon。 |
| 0009 | [forbid-list 分层执行](./adr/0009-forbid-scope.md) | 生成期 Hard Block；External 配置明文 = 安全诊断，不阻断无关 Apply（按 Ownership 分层）。 |
| 0010 | [凭据生命周期与 Apply 锁](./adr/0010-credential-lifecycle-and-lock.md) | 单 Provider 单凭据 key=default；Secret 删除由应用层协调（非 CASCADE）；跨进程 OS 锁为互斥权威，DB 状态不是锁。 |
| 0011 | [首启产物与 Profile 删除](./adr/0011-first-run-and-profile-deletion.md) | 模板≠实体；首启不创建 Agent，Default Profile Lazy Create；Active Profile 不可直接删。 |
| 0012 | [Codex 版本能力探测](./adr/0012-codex-version-capability.md) | Feature Probe 为主，Version Capability Registry 为辅；未知字段 Preserve；只阻断受影响功能。 |
| 0013 | [机器接口表面](./adr/0013-machine-surface.md) | Bootstrap 最小载荷；`--json` 稳定格式；CLI 命名收敛（无 use 别名）；不做 operation_cancel。 |
| 0014 | [V0.1 Responses-first 与 PoC 门禁](./adr/0014-v0-1-responses-first.md) | DeepSeek V4 Flash 为首个 Direct Preset，Custom Responses 为通用入口；Provider-neutral、Windows-first、Profiles 等延后；先过四项 PoC。 |

## 跨文档修正汇总

盘问中发现并拍板的文档矛盾/缺口。主文档以 ADR 0001—0014 为约束；若旧章节仍保留未来方案，应以各主文档顶部的「V0.1 决策基线」和最终范围章节为准：

1. **数据模型文档**：`CapabilitySource` 枚举换为 ADR 0003 版本；去掉 Provider `Compatibility Level`，改 source/adapter/protocol + Integration Readiness；`Agent.name` 改 `displayName`；确认无 formal Draft/Applied 状态机之争（用 UI 局部 Draft + CAS 级 Saved+Pending，见 ADR 0001/0004）。
2. **持久化文档**：`CapabilitySource` 冲突以 ADR 0003 为准；`models.lifecycle` 确认纳入；`provider_test` 保留最近 20 条；ManagedResource 创建时机 = Apply 成功后（ADR 0006）；自动 Snapshot 保留最近 20，Pin 受磁盘保护（ADR 0011 相关，Retention 采纳 Q5）。
3. **配置集成文档**：`auth.command` 绝对路径（ADR 0005）；Shared 文件 fragment 语义指纹 + 独享文件整文档（ADR 0006）；冲突处置只有两选项（ADR 0007）。
4. **安全与凭据文档**：cas-helper 退出码 0/2/3/4/5/6 与 stderr JSON（ADR 0008）；forbid 分层执行（ADR 0009）；凭据删除两段式（ADR 0010）。
5. **IPC 文档**：Bootstrap payload 最小化（ADR 0013）；`operationType` 补 MODEL_VERIFY（及诊断类多阶段操作，仅多为阶段才走事件）；avoids `operation_cancel`。
6. **UI 文档**：Discard 仅限未保存表单（ADR 语义，非全局丢弃）；`provider_update` 高级项只读（Q4-1）；首启不创建 Agent 实体（ADR 0011）。
7. **CLI 文档**：V0.1 命令集 `status|doctor|apply`；Profile CLI 随 P1 引入且无 `use` 别名；`--json` 稳定 schema；doctor 退出码 0/1/2/3（ADR 0013、0014）。

## 未决点（留给后续实现/版本轮盘）

- JSON Schema 稳定性承诺后，前端 `api/types` 的代码生成方式。
- Import/Adopt 能力（P1）——届时 "采用磁盘版本" 才允许开放（ADR 0007 预留）。
- 多凭证/凭据轮换（schema 已预留 `credential_key`）。
- Model Capability Runtime Probe 的具体协议。（P1）
- CI/发布管道与 helper launcher 安装路径最终实现选择（跟随发布规范）。

## V0.1 当前路线

V0.1 采用 ADR 0014：Responses-first、Provider-neutral、Windows-first、单层 Base Binding。DeepSeek V4 Flash Direct Preset 与 Custom Responses Provider 同属 P0；Profiles、Discovery、Runtime Probe、Import / Adopt 不属于 P0；完整功能开发必须先通过四个 PoC Gate。
