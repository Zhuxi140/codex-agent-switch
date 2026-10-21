# CAS 委派候选评分与成本控制

## 目标与边界

默认由 Primary 完成任务。只有任务看起来可能从独立上下文中受益时，Primary 才在当前轮次构造少量事实并调用 CAS 本地 MCP 的 `assess_delegation`。这不是第二轮模型聊天；MCP 只读评估，不创建 Job 或 Child。它返回 `PRIMARY`、`SUGGEST` 或 `DELEGATE`：只有 `DELEGATE` 允许 Primary 自主进入 Runtime 调度。用户本次明确要求委派时，仍可使用 `$cas-delegate` Skill；它是手动入口，不是自主委派的唯一入口。`job-assess`、`job-schedule` 保留为兼容命令，不再是 Primary 的常规调用路径。

没有明确委派请求时，小范围检索、单点修复、单命令测试直接跳过评估。用户或项目禁委派优先。评估输入由 Primary 从当前任务事实填写，不应为了填表再做大范围调查。

## 输入与评分 v1

自动候选入口：CAS 本地 STDIO MCP `assess_delegation`，输入 `agent_key`、`workspace_scope`、`assessment`，不接受 `explicit_request` 或 TaskPacket。Primary 自主判断固定使用 `profile=CONSERVATIVE`；`BALANCED`、`AGGRESSIVE` 保留给界面中的显式策略实验。Runtime 入口：`cas-helper job-plan <database-path> <agent-key> <workspace-scope>`；stdin 输入一行含同一份 `assessment` 和完整 `task_packet` 的 JSON。`assessment` 的必填布尔字段为 `bounded`（允许范围明确）、`acceptance_defined`（验收明确）、`independent`（可独立交付）、`handoff_small`（准备与复核开销较小）。其余评估字段可省略，默认为 `false` 或 `0`。`task_packet` 使用原 `job-schedule` 的冻结结构。`delegation_forbidden=true` 直接留给 Primary；正常情况下遇到禁令甚至不应调用评估。

评分先检查硬门槛：四个必填字段都为 `true`，候选 Agent 为已启用且 Active，项目未被排除。然后计算 5 分制：角色证据 3 分、收益摊销 1 分、低交接成本 1 分。达到当前档位阈值返回 `DELEGATE`；有角色证据且只差 1 分时返回 `SUGGEST`；其余返回 `PRIMARY`。

| phase | 保守 | 均衡 | 积极 |
| --- | --- | --- | --- |
| DISCOVERY | `call_chains >= 2` 或 `modules >= 3` | `call_chains >= 1` 或 `modules >= 2` | `call_chains >= 1` 或 `modules >= 1` |
| EXECUTION | `work_units >= 2`，或 `modules >= 2` 且已有测试 | `work_units >= 2` 或 `modules >= 2` | `work_units >= 1` 或 `modules >= 1` |
| REVIEW | `high_risk=true` | 同保守 | 同保守 |
| VERIFICATION | `environments >= 2` 或 `stages >= 2` | 同保守 | `environments >= 1` 或 `stages >= 1` |
| 收益时长 | 15 分钟 | 10 分钟 | 5 分钟 |
| 委派阈值 | 5 分 | 5 分 | 4 分 |

`work_units` 计可独立交付、独立验收的目标，不是函数或需求条目的数量。多个改动若共享同一实现前置条件和验收范围，应作为一个工作单元；不确定时保守计数，不为取得 `DELEGATE` 拆分任务。

`explicit_request=true` 只能表示用户对本次任务明确要求委派，不能从 `CAS:ON`、评分结果或历史偏好推定；它可满足角色证据，但不能绕过硬门槛。自主入口不提交该字段。收益摊销由档位最低时长、`work_units >= 2`、高风险 REVIEW 或明确要求中的任一项满足。时长与工作单元相关，最多只记 1 分；15 分钟不是通用硬门槛，高风险审查不要求长时间。任务优先级不进入评分。Primary 和 Child 的具体模型均随当前配置变化，不参与硬编码评分。

未达门槛时返回 `PRIMARY/UNAVAILABLE`。只差 1 分时返回 `SUGGEST/CANDIDATE_RECOMMENDED`；Primary 只向用户简短建议，不创建 Job。达到门槛时返回 `DELEGATE/AUTO_DELEGATION_APPROVED`；Primary 读取详细 Runtime 协议，以相同事实和完整 TaskPacket 调用 `job-plan`。用户明确委派且通过时返回 `DELEGATE/USER_REQUESTED`。`job-plan` 仍可能返回 `PRIMARY`、`UNAVAILABLE`、`BLOCK` 或 `UNCERTAIN`，因此 MCP 的 `DELEGATE` 是进入调度的许可，不是 Child 已创建的证明。

候选示例：`{"assessment":{"bounded":true,"acceptance_defined":true,"independent":true,"handoff_small":true,"profile":"CONSERVATIVE","work_units":2}}`。若候选 Agent 是 EXECUTION，返回 `DELEGATE`，Primary 随后提交完整 `task_packet`；只有用户通过手动入口明确要求时才设置 `explicit_request:true`。若只有 `work_units:1`，即使预估很久，也因未命中角色门槛而返回 `PRIMARY`。

## 评估本身的耗费

普通任务零评估工具调用，但全局短入口仍占上下文。候选任务只读取短门禁、组织少量评估事实并调用一次本地 MCP；不额外调用模型，也不预先构造 TaskPacket。只有返回 `DELEGATE` 或用户明确委派时，才读取包含路径、命令和 JSON 的详细执行协议并支付 TaskPacket 准备成本。不能把每个任务都送入评分，否则筛选成本仍会侵蚀收益。

评分是保守启发式，字段仍由 Primary 判断，不能证明委派一定省时或省额度。正式结论需对同类任务做 OFF/ON 配对，统计 Primary+Child 全部 token、墙钟时间、验收通过率与误分发率；若候选评估的额外开销或误分发抵消收益，就应收紧门槛。当前 v1 不把评分结果持久化，也不改变冻结的 TaskPacket 或 Job 契约。

## 历史回放说明

2026-09-25 的旧回放验证了“仅建议”版本，现已被自主委派语义取代。当前固定输入与预期位于 `src-tauri/cas-helper/src/mcp_assessment.rs` 的 `assessment_replay_covers_small_tasks_and_active_role_bindings`：合格候选返回 `DELEGATE`，临界候选返回 `SUGGEST`，硬门禁失败返回 `PRIMARY/UNAVAILABLE`，且只读 MCP 不修改数据库。

现有 `benchmarks/efficiency-fixture/prompts/` 的 `task-1a`、`task-1b` 按单个交付单元填写时均返回 `PRIMARY`。同目录的 `task-qualified` 是同文件双函数任务：若确有两个独立验收单元并填为 `work_units=2`，返回 `DELEGATE`；按共享前置实现与最终验收填为 `work_units=1`，返回 `PRIMARY`。这些分钟数与工作单元是人工假设，不是历史实测输入；当前版本不持久化评估，故没有历史评分日志可读取。

提示投影分为两层：`CAS_ORCHESTRATION.md` 只保留角色门槛、三种结果和详细协议入口，不包含 helper、数据库路径、命令或 TaskPacket；`CAS_DELEGATION_RUNTIME.md` 只在 `DELEGATE` 或用户明确委派后读取。字符数不能直接换算成真实 token。[2026-09-24 单文件 OFF/ON 样本](../../benchmarks/efficiency-fixture/results/2026-09-24-sol-luna-ab.json)中，OFF 68,938 原始 token，ON（Primary+Child）284,072，差 215,134、约 4.12 倍，验收均通过。但样本各只有一次成功运行，CLI/会话设置与重试存在差异，不能把差额归因于 CAS 提示词或一般委派策略。

本轮只澄清 `work_units` 口径，不根据单个有混杂因素的样本修改数值阈值。源码的按需规则已更新；现用全局规则尚未重新同步，未改动全局配置或 Hook 信任。真实节省仍需经单独授权的同条件、大任务配对实验验证。
