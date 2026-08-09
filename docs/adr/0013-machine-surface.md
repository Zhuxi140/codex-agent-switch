# ADR 0013: 机器接口表面——Bootstrap 最小化、--json、CLI 命名、取消语义

## 决策

- **Bootstrap 载荷最小化**：`app_get_bootstrap` 只含 `appVersion + CodexEnvironmentSummary + activeProfile（引用/ID）+ configurationStatus + running-operation 标记`。Providers/Models/Agents 明细一律走各自 list 命令，Bootstrap 不做"首页万能查询"，防止启动接口随领域变化持续膨胀。
- **CLI 输出双格式**：`cas status` / `cas doctor` 默认 human-readable 文本，提供 `--json` 稳定机器格式；JSON Schema 一旦发布即冻结，兼容演进。
- **CLI 命名收敛**：命令集 = `cas status | doctor | apply | profile activate <key> | profile list`；不设 `use` 别名；`config diff` 推迟 P1。
- **不做 operation_cancel**：取消语义复杂、收益低。短网络操作（test/discovery）直接等待超时；Apply/Restore 的文件 Mutation 阶段**不可取消**——写操作必须完成或走 Rollback，不存在"用户中途点掉"的中间态。

> ADR 0014 补充：Profiles 已移出 V0.1，因此 V0.1 Bootstrap 不含 `activeProfile`，CLI 也不实现 `profile activate/list`；这些契约在 Profiles 进入 P1 时再启用。其余本 ADR 决策不变。

## 背景

Bootstrap 膨胀会让每次启动接口背负全部领域数据；CLI 别名和模糊命名会增加脚本与文档歧义；Apply/Restore 若支持任意点取消，会产生 Journal 与文件不一致的中间态，违背"可验证、可恢复"的可靠性原则。

## 影响

- frontend 按 list 命令按需取数；bootstrap 只决定初始渲染骨架与状态栏。
- CLI 契约简单可脚本化，`--json` 供 Agent/CI 消费；doctor 退出码（0=Healthy/1=Warnings/2=Errors/3=Environment unavailable）与 helper 退出码空间分离（ADR 0008）。
- 取消需求若未来出现，扩展为明确的 `OPERATION_CANCELLED` 错误码 + 仅对"非写阶段"生效。
