# ADR 0006: 语义指纹与资源归主（Ownership）基线

## 决策

冲突判定一律使用**语义指纹**，字节 hash 仅用于诊断。Cas 管理内容分两类：
- Shared 文件（`config.toml`）：只对 CAS-owned AST fragment 做语义指纹。空白/顺序无关/注释变化不构成冲突。
- Exclusive 文件（`agents/cas-*.toml`、CAS catalog）：整文档语义指纹。

`contentHash`（字节级）只用于诊断展示；`semanticHash / fragmentHash` 才是 Ownership 与冲突判定的依据。

## 决策细则

- `# Managed by Codex Agent Switch` 注释可以写入，但只是人类提示，**不参与 hash**——用户删除注释不构成冲突。
- Ownership 永远来自 `ManagedResource + logicalKey + fingerprint`，不来自文件命名或注释。
- ManagedResource 记录只在 **Apply 成功提交后**创建（首次 Apply：read-before-write 确认无冲突 → Journal 记录 → 写入 → post-write 验证成功 → 最后提交 Ownership 记录）。启动扫描只读展示，绝不认领。Apply 失败不留下假 Ownership。

## 影响

- 恢复/冲突场景以语义指纹判断"哪些 CAS 片段被外部改过"。
- Apply 的 post-write 验证 = 按语义指纹核对，而非字节相等（容忍格式噪声）。