# ADR 0009: forbid-list 执行范围（按 Ownership 分层）

## 决策

forbid-list（禁任意 `auth.command`、禁明文 token、禁明文 .env vault、fail closed）在两个层面强制：

1. **Compile/生成期（Hard Block）**：CAS 即将生成或管理的 CAS-owned 内容出现 forbidden 模式 → 拒绝生成/注入，阻断本次 Apply。
2. **Diagnostics 扫描**：CAS-owned 已存在资源出现 forbidden → Error 级诊断并阻断相关 Apply；**与本次 Apply 无关的 External 用户配置出现明文 token → Error 级安全诊断项（Preserve 不动），但不得阻断与它无关的 CAS Apply**。

## 背景

原方案无条件"发现任何外部明文 → 阻断整个 Apply"会误伤：用户旧的手写 Provider 配置可能带明文 token，若因此全局停摆，反而无法提供恢复路径。分层按 Ownership 决定阻断粒度，符合"配置安全优先于便利但不过度"的平衡。

## 影响

- Compiler 携带 forbidd清单作为硬约束；Diagnostics 单独扫描现有配置并分类（owned 与 external 分开呈现）。
- UI：外部配置的 forbidden 项显示为「安全注意：External 配置含明文凭据」，可继续其他操作。