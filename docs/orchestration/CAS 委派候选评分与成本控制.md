# CAS 委派候选评分与成本控制

## 目标与边界

默认由 Primary 完成任务。只有任务看起来可能从独立上下文中受益时，Primary 才在当前轮次构造少量事实并调用 CAS 本地 MCP 的 `assess_delegation`。这不是第二轮模型聊天；MCP 只读评估，自动候选只返回 `SUGGEST`，不创建 Job 或 Child。用户本次明确要求委派时，使用 `$cas-delegate` Skill；它通过 `job-plan` 提交完整 TaskPacket，硬门槛通过后才进入 Runtime 准入。`job-assess`、`job-schedule` 保留为兼容命令，不再是 Primary 的常规调用路径。

没有明确委派请求时，小范围检索、单点修复、单命令测试直接跳过评估。用户或项目禁委派优先。评估输入由 Primary 从当前任务事实填写，不应为了填表再做大范围调查。

## 输入与评分 v1

自动候选入口：CAS 本地 STDIO MCP `assess_delegation`，输入 `agent_key`、`workspace_scope`、`assessment`，不接受 `explicit_request` 或 TaskPacket。明确委派入口：`cas-helper job-plan <database-path> <agent-key> <workspace-scope>`；stdin 输入一行含 `assessment` 和完整 `task_packet` 的 JSON。`assessment` 的必填布尔字段为 `bounded`（允许范围明确）、`acceptance_defined`（验收明确）、`independent`（可独立交付）、`handoff_small`（准备与复核开销较小）。其余评估字段可省略，默认为 `false` 或 `0`。`task_packet` 使用原 `job-schedule` 的冻结结构。`delegation_forbidden=true` 直接留给 Primary；正常情况下遇到禁令甚至不应调用评估。

评分先检查硬门槛：四个必填字段都为 `true`，候选 Agent 为已启用且 Active，项目未被排除。然后计算 5 分制：角色证据 3 分、收益摊销 1 分、低交接成本 1 分；5 分且没有明确请求时返回 `SUGGEST`，有明确请求时才返回 `DELEGATE`。角色证据按 Agent 的 phase 计算：

| phase | 角色证据（任一） |
| --- | --- |
| DISCOVERY | `call_chains >= 2` 或 `modules >= 3` |
| EXECUTION | `work_units >= 2`，或 `modules >= 2` 且 `tests_defined=true` |
| REVIEW | `high_risk=true` |
| VERIFICATION | `environments >= 2` 或 `stages >= 2` |

`work_units` 计可独立交付、独立验收的目标，不是函数或需求条目的数量。多个改动若共享同一实现前置条件和验收范围，应作为一个工作单元；不确定时保守计数，不为取得 `SUGGEST` 拆分任务。

`explicit_request=true` 只能表示用户对本次任务明确要求委派，不能从 `CAS:ON`、评分结果或历史偏好推定；它可满足角色证据，但不能绕过硬门槛。收益摊销由 `estimated_minutes >= 15`、`work_units >= 2`、高风险 REVIEW 或明确要求中的任一项满足。时长与工作单元相关，最多只记 1 分；15 分钟不是通用硬门槛，高风险审查不要求长时间。任务优先级不进入评分。Primary 和 Child 的具体模型均随当前配置变化，不参与硬编码评分。

未达门槛时返回 `CAS2|ASSESS|<PRIMARY/UNAVAILABLE>|<score>|5|<reason>|<phase>`。达到门槛但未获明确请求时返回 `CAS2|ASSESS|SUGGEST|5|5|CANDIDATE_RECOMMENDED|<phase>`；Primary 只给用户简短建议，然后继续完成当前任务，不为建议暂停，不自行重试，不创建 Job/租约。明确请求且通过后，返回原调度协议 `CAS2|<REUSE/SPAWN/WAIT/BLOCK/EXISTING/UNCERTAIN>|<thread-or->|<reason>|<job>|<attempt-or->`；通过评估并不保证创建 Child。`UNAVAILABLE` 表示候选 Agent/phase 不可用，不应找其他角色顶替。

候选示例：`{"assessment":{"bounded":true,"acceptance_defined":true,"independent":true,"handoff_small":true,"work_units":2}}`。若候选 Agent 是 EXECUTION，返回 `SUGGEST`；只有用户明确要求时才设置 `explicit_request:true` 并提交完整 `task_packet`。若只有 `work_units:1`，即使预估很久，也因未命中角色门槛而返回 `PRIMARY`。

## 评估本身的耗费

普通任务零评估工具调用，但全局短入口仍占上下文；当前测试限制它不超过 700 字符。候选任务会读取按需规则、组织少量评估事实，再调用一次本地 MCP；不额外调用模型，也不预先构造 TaskPacket。只有用户明确要求本次委派时才支付 TaskPacket 准备成本。不能把每个任务都送入评分，否则筛选成本仍会侵蚀收益。

评分是保守启发式，字段仍由 Primary 判断，不能证明委派一定省时或省额度。正式结论需对同类任务做 OFF/ON 配对，统计 Primary+Child 全部 token、墙钟时间、验收通过率与误分发率；若候选评估的额外开销或误分发抵消收益，就应收紧门槛。当前 v1 不把评分结果持久化，也不改变冻结的 TaskPacket 或 Job 契约。

## 2026-09-25 离线回放

用当前 debug `cas-helper mcp-assess` 和临时 SQLite 回放 13 次 `assess_delegation`；未连接现用数据库，未创建 Job 或子 Agent。固定输入与预期已写入 `src-tauri/cas-helper/src/mcp_assessment.rs` 的 `assessment_replay_covers_small_tasks_and_active_role_bindings`，可用 `cargo test --manifest-path src-tauri/Cargo.toml -p cas-helper --bin cas-helper assessment_replay_covers_small_tasks_and_active_role_bindings` 复跑。结果为 5 次 `SUGGEST`、6 次 `PRIMARY`、2 次 `UNAVAILABLE`；禁用及未 Active 绑定的 Agent 均不可用，角色与高交接成本门槛按预期生效。

现有 `benchmarks/efficiency-fixture/prompts/` 的 `task-1a`、`task-1b` 按单个交付单元填写时均返回 `PRIMARY`。同目录的 `task-qualified` 是同文件双函数任务：假设 10 分钟，若把两个函数填为 `work_units=2`，返回 `SUGGEST`；按共享前置实现与最终验收填为 `work_units=1`，返回 `PRIMARY`。这些分钟数与工作单元是本次回放的人工假设，不是历史实测输入；v1 不持久化评估，故没有历史评分日志可读取。

本机测得全局 `AGENTS.md` 的 CAS 短入口 287 字符、第二回合一次性精简提示 68 字符、现用按需规则文件 3575 字符、显式委派 Skill 470 字符。字符数不能换算成真实 token；按需规则与 Skill 不应在普通小任务中加载。[2026-09-24 单文件 OFF/ON 样本](../../benchmarks/efficiency-fixture/results/2026-09-24-sol-luna-ab.json)中，OFF 68,938 原始 token，ON（Primary+Child）284,072，差 215,134、约 4.12 倍，验收均通过。但样本各只有一次成功运行，CLI/会话设置与重试存在差异，不能把差额归因于 CAS 提示词或一般委派策略。

本轮只澄清 `work_units` 口径，不根据单个有混杂因素的样本修改数值阈值。源码的按需规则已更新；现用全局规则尚未重新同步，未改动全局配置或 Hook 信任。真实节省仍需经单独授权的同条件、大任务配对实验验证。
