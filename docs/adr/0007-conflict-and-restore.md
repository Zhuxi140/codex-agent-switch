# ADR 0007: 冲突处置与 Restore 策略

## 决策

检测到 External/Conflict 时 V0.1 只提供两种处置：① 查看差异并中止；② 重新应用 CAS Desired State（显式覆盖 + 自动 Snapshot）。**不提供「采用磁盘版本 / 更新 baseline」**——CAS Domain State 没有吸收磁盘修改的能力，仅改 baseline 会造成"CAS 以为这是自己的、却不知内容从何而来"。等到 Import/Adopt（P1 以后）能力成熟才允许采用磁盘版本。

## Restore 规则

- Snapshot 资源集 = 本次 Apply 涉及的文件级资源，绝不目录级复制。
- Shared 文件（`config.toml`）：Snapshot 保存完整 pre-image 用于灾难恢复；**普通 Restore 不整文件覆盖**——重新读取当前文件，只恢复 CAS-owned fragment（避免抹掉 Snapshot 之后用户改的 MCP/Projects）。
- Exclusive 文件（`agents/cas-*.*toml`、catalog）允许整文件恢复。
- Restore 前自动备份当前状态；与 Domain 业务状态解耦（ADR 0004 上下文一致）。

## 影响

- 冲突处置可选值约束到两个动作；UI 冲突弹窗只呈现这两项。
- Restore 实现分为 fragment 级与整文件级两条路径。