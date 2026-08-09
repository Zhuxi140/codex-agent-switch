# ADR 0002: Agent 身份键与 Codex name 恒等

## 决策

`Agent.key` 与 Codex agent 的 `name` 恒等：生成 `cas-<key>.toml` 时强制 `name="<key>"`。CAS 侧面向 UI 的名字改称 `displayName`，只作展示，不写入 Codex。已登记的 CAS-owned 文件被外部改 `name` 时归类为 **Externally Modified**（`CAS_RESOURCE_EXTERNALLY_MODIFIED`），不重新归类为 External；从未登记的才是 External。

## 背景

Codex 以 agent 文件内 `name` 为身份（文件名只是约定）。若 CAS 内部 key 与 Codex name 脱钩，用户改一次 Codex 侧 name 就导致 CAS-ownered 资源"失联"，要么静默接管（危险）、要么全部丢给 External（错伤 CAS 自己的资源）。恒等 + 强制 `name="<key>"` 让身份映射零歧义。

## 决策细则

- `Agent.key` 不可变（`[a-z][a-z0-9_-]*`），= Codex `name`。
- 文件名为 `cas-<key>.toml`（文件名是约定，不是身份；身份始终是 file 内 `name`）。
- 用户编辑的是 `displayName` + Description + Instruction，不影响 Codex 侧身份。
- Externally Modified ≠ External：前者是 CAS 已登记资源被改（冲突、拦截、需用户裁定）；后者是遗漏登记的资源（仅展示 + Import P1 再认领）。

## 影响

- 指纹校验同时覆盖文件哈希与关键字段（name）；`name` 被外部改 → `CAS_RESOURCE_EXTERNALLY_MODIFIED`，Apply 中止进入冲突流程。
- UI Agent 列表展示 `displayName`，Codex 侧显示 `key`。
- 引用（logicalKey / origin）始终以 key 为锚，改名不丢注册。