---
name: cas-delegate
description: 用户明确要求本次任务通过 CAS 委派子 Agent 时使用；不用于自动候选评估。
---

# CAS 用户委派

只处理用户本次明确提出的委派；`CAS:ON` 和评分建议不等于授权。`CAS:OFF` 或用户/项目禁委派时不分发。

1. 提取一个范围明确、可独立验收的子任务。缺少会改变范围的关键信息时先问用户。读取当前 CAS 编排规则，选匹配 phase 的 Active Agent；不猜模型。
2. 用 `job-plan` 提交 `explicit_request=true` 和完整 TaskPacket。返回 `PRIMARY`、`UNAVAILABLE`、`BLOCK` 或 `UNCERTAIN` 时停止分发并说明原因；不得改写评估事实或直调 `job-schedule` 绕过门槛。
3. 仅在准入后按规则使用原生子 Agent，完成 bind、结果观察与 Primary 审查。失败遵循当前 CAS 失败策略，不自行扩大任务或重试授权。
