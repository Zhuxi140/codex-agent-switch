# ADR 0010: 凭据生命周期与 Apply 互斥

## 决策

- **V0.1 一个 Provider 最多个 CAS-managed Credential**，逻辑 key 固定 `default`；`credential_key` 保留为未来扩展（轮换/备用）。`NONE` / `EXTERNAL_ENV` 认证策略不创建 Secret Credential。
- **凭据删除由 Application Service 协调**：`remove_credential` = 删 DB 引用 + 删 OS Secret，两者必须成功；Secret 删除失败 → 明确返回失败/补偿，不假装已删除。Provider 删除同样需应用层同步删 OS Secret，**不能依赖 SQL CASCADE 处理 OS Secret**。
- `remove_credential` 后 Provider → `MISSING_CREDENTIAL`；引用它的有效 Agent 不可 Apply（阻断 + 提示补凭据）。
- **跨进程互斥锁**：Apply/Restore 使用真正的 OS 级锁文件（flock/LockFileEx）作为唯一互斥权威；`.lock` 文件存在与否不作为锁判断（防 stale file 误伤）。第二个 Apply/Restore 立即返回 `APPLY_ALREADY_RUNNING`。`configuration_state` / `apply_transactions` 只是状态 + Journal + Crash Recovery 证据，**不是互斥锁**。
- `status/list` 可随时读取；`doctor` 在 Apply 进行中报告 `APPLY_IN_PROGRESS`，避免把中间状态误判为 Drift。

## 影响

- Repository 层与 Secret 层分离：DB 事务成功不代表 OS Secret 已删，由 UseCase 编排两段式提交与补偿。
- CLI 与 GUI 共用同一把 OS 锁，天然防并发。
- doctor/status 对 operation 中状态有明确区分。