# ADR 0004: Applied 判定与事务边界

## 决策

V0.1 不启动 Codex 试运行。Applied 判定 = 文件写入成功 → 重新读盘/解析 → CAS-owned 文件或 Fragment 与 Expected Projection/Hash 一致 → `desiredHash` 一致 → Apply Journal 成功 `COMMITTED`。Shared `config.toml` 用 CAS Fragment/语义投影校验，Exclusive File 可用完整字节 Hash。无法确认 → `UNKNOWN / NEEDS_ATTENTION`；已陷入事务不确定状态 → `RECOVERY_REQUIRED`。

## 背景

PRD 可靠性要求"无法确认成功不得显示 Applied"。此前文档仅表述"post-write 重新读盘"，未定义 shared 与 exclusive 文件的校验差异，也未区分 UNKNOWN 与 RECOVERY_REQUIRED 两种失败语义。

## 决策细则（事务模型）

- 文件写入不持有长 SQLite 事务。短事务一：写 Journal `PREPARED` + 提交。文件事务：Snapshot → temp 写入 → atomic rename。短事务二：更新 `COMMITTED` + fingerprints。
- Apply 全程不做任何网络验证——Post-write Validation 是本地文件校验；Provider 网络测试属于独立操作（diagnostics/test_connection）。
- Journal 状态：`PREPARED → WRITING → VALIDATING → COMMITTED | ROLLING_BACK → ROLLED_BACK / FAILED / RECOVERY_REQUIRED`。启动时发现 PREPARED/中介态 → 恢复进程接管。

## 影响

- `configuration_state` 记录 `desiredHash`（CAS 侧生成的目标状态指纹）与 Applied 确认标志。
- UI 状态文案：成功=Applied；校验不确定=Needs attention；Journal 中断=Recovery required。
- 排除了"为确认而启动 Codex"的开销与不确定性。