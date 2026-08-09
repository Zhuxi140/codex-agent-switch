# ADR 0008: cas-helper 运行时协议

## 决策

`cas-helper` 唯一接口：`cas-helper token <uuid>`。一次性进程：启动 → 读 OS Secret Store → stdout 输出 token → 退出。**V0.1 不做 daemon、不做内存缓存、不做本地 Secret Server**——OS Keychain/Credential Manager 的交互交给平台自身处理；出现性能问题再优化。

## 退出码与输出约定

- `0`：成功，stdout 输出 token 本体（唯一允许出现在 stdout 的内容）
- `2`：参数错误（非 UUID 或用法错误）
- `3`：凭据不存在
- `4`：Secret Store 不可用
- `5`：访问拒绝（Access Denied）
- `6`：读取/内部错误
- stderr 一律结构化 JSON：`{"error": "<code>", "message": "..."}`，不输出 Secret。

## 背景

退出码与安全规范此前只有方向没有具体值；且需区分「Store 不可用」与「访问拒绝」以便诊断。固定协议保证 Codex 侧 `auth.command` 行为可预期、可插桩。

## 影响

- `cas-cli`/`cas-doctor` 通过 helper 退出码映射 Credential 状态（MISSING/STORE_UNAVAILABLE/DENIED）。
- 升级 helper 时若变更退出码，需保持以上映射兼容（扩展值前向兼容）。