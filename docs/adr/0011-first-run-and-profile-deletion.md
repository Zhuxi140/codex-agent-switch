# ADR 0011: 首启产物与 Profile 删除规则

## 决策

**模板不是 Agent 实体。** 首次启动**不自动创建**任何 Agent 实体；Executor/Explorer/Reviewer/Tester 始终留在 `resources/agent-presets`，用户点击创建时才实例化为 Agent。`Default Profile` 也只做 **Lazy Create**——首次真正需要时（首次激活/Apply 前）才落库，首启导航不做成"必须产生实体"的步骤。

## 背景

PRD 建议首启"创建 Default Profile + 推荐 Executor"，但如果首启就写入 4 个 `enabled=false` 的 Agent，会污染 Agents 列表与状态语义——用户一打开应用就看到一堆"幽灵 Agent"，且 `enabled=false` 与"External/模板"的概念边界被模糊。模板作为只读资源保留，实例化是显式动作。

## Profile 删除规则

- **Active Profile 不允许直接删除**：必须先显式激活另一个 Profile 再删除旧 Profile；不搞"删除时偷偷切回 Default"（隐藏副作用）。
- 删除非 active Profile：bindings 级联删除，不触碰 Agent 的 Base Binding（ADR 0001 语义不变）。

## 影响

- first-run 导航：Add Provider / 创建 Agent 都是可选步骤，跳过即进入空应用（空态引导，非强制产物）。
- DB 里不会因首启出现无引用的空 Profile/Agent。