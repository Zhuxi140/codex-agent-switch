# CAS 委派候选评分与成本控制

## 目标与边界

默认由 Primary 完成任务。只有任务看起来可能从并行或独立上下文中受益时，Primary 才在当前轮次构造少量事实并调用本地 `cas-helper job-assess`。这不是第二轮模型聊天，也不创建 Job、不启动 Child；`job-schedule` 仍负责实际 Runtime 准入。

小范围检索、单点修复、单命令测试直接跳过评估。用户或项目禁委派优先。评估输入由 Primary 从当前任务事实填写，不应为了填表再做大范围调查。

## 输入与评分 v1

命令：`cas-helper job-assess <database-path> <agent-key> <workspace-scope>`；stdin 输入一行 JSON。必填布尔字段为 `bounded`（允许范围明确）、`acceptance_defined`（验收明确）、`independent`（可独立交付）、`handoff_small`（准备与复核开销较小）。其余字段可省略，默认为 `false` 或 `0`。`delegation_forbidden=true` 直接留给 Primary；正常情况下遇到禁令甚至不应调用评估。

评分先检查硬门槛：四个必填字段都为 `true`，候选 Agent 为已启用且 Active，项目未被排除。然后计算 5 分制：角色证据 3 分、收益摊销 1 分、低交接成本 1 分；5 分才返回 `DELEGATE`。角色证据按 Agent 的 phase 计算：

| phase | 角色证据（任一） |
| --- | --- |
| DISCOVERY | `call_chains >= 2` 或 `modules >= 3` |
| EXECUTION | `work_units >= 2`，或 `modules >= 2` 且 `tests_defined=true` |
| REVIEW | `high_risk=true` |
| VERIFICATION | `environments >= 2` 或 `stages >= 2` |

`explicit_request=true` 可满足角色证据。收益摊销由 `estimated_minutes >= 15`、`work_units >= 2`、高风险 REVIEW 或明确要求中的任一项满足。时长与工作单元相关，最多只记 1 分；15 分钟不是通用硬门槛，高风险审查不要求长时间。任务优先级不进入评分。

返回 `CAS2|ASSESS|<PRIMARY/DELEGATE/UNAVAILABLE>|<score>|5|<reason>|<phase>`。`DELEGATE` 只允许进入 `job-schedule`，并不保证真正创建 Child；`PRIMARY` 由 Primary 完成；`UNAVAILABLE` 表示候选 Agent/phase 不可用，不应找其他角色顶替。

示例：`{"bounded":true,"acceptance_defined":true,"independent":true,"handoff_small":true,"work_units":2}`。若候选 Agent 是 EXECUTION，返回 `DELEGATE`；若只有 `work_units:1`，即使预估很久，也因未命中角色门槛而返回 `PRIMARY`。

## 评估本身的耗费

普通任务零评估工具调用，但全局短入口仍占上下文；当前测试限制它不超过 700 字符。候选任务还会读取按需规则（固定文本不超过 3100 字符）、组织字段并调用本地进程。这些仍消耗 Primary 的上下文和工具时间，只是不额外调用模型。不能把每个任务都送入评分，否则筛选成本会侵蚀收益。

评分是保守启发式，字段仍由 Primary 判断，不能证明委派一定省时或省额度。正式结论需对同类任务做 OFF/ON 配对，统计 Primary+Child 全部 token、墙钟时间、验收通过率与误分发率；若候选评估的额外开销或误分发抵消收益，就应收紧门槛。当前 v1 不把评分结果持久化，也不改变冻结的 TaskPacket 或 Job 契约。
