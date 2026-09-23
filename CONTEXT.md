# Codex Agent Switch

CAS 管理 Primary 与原生 Child Agent 的委派、调度和运行状态。

## Language

**委派候选评估（Delegation Assessment）**：
Primary 针对一个可能适合委派的任务提供结构化事实，CAS 只读计算其是否值得进入调度。它不是创建 Job 或授予执行权限的决定。
_Avoid_: 调度准入、自动派发

**调度准入（Schedule Admission）**：
CAS Runtime 根据当前 Agent、权限、排除、租约和并发等事实，决定已提交的委派 Job 能否进入执行路径。
_Avoid_: 委派候选评估、评分
