# ADR 0001: Binding 分层语义

## 决策

Agent 与 Model 的关联分两层：**Base Binding**（Agent 自带、独立于 Profile 的默认绑定）与 **Profile Binding**（Profile 内对 Agent 默认绑定的覆盖）。激活 Profile 只选择「当前生效的覆盖层」，**绝不改写**任何 Agent 的 Base Binding。

## 背景

领域建模时曾有两种方案：把「生效绑定」单一化（激活 Profile 时把 Profile 绑定写入 Agent 绑定），或叠加覆盖。单一化的风险：切换 Profile 变成破坏性操作——Agent 从 Profile A 切到 B 时，原本的 Base 绑定被 A 覆盖后丢失，无法判断「没有 Profile 覆盖时该用什么」。叠加方案让 Agent 本身保持稳定身份（满足 PRD 原则「Role 与 Model 分离」），Profile 只是选择层。

## 决策

- `AgentModelBinding`（一对多、`agent_id` 唯一）是 **Base Binding**，永不因激活 Profile 而改写。
- `ProfileAgentBinding`（`profile_id + agent_id` 唯一）是 Profile 激活期的**覆盖**，只在 Profile 生效期间参与。
- 生效语义 = `Base Binding + 激活 Profile 的覆盖`（无覆盖则回到 Base）。
- 删除 Profile、切回 Default、或用户直接改 Base Binding 时，均不会互相污染。

## 影响

- UI「Agent 绑定」编辑的是 Base Binding；Profile 详情页编辑的是覆盖层，两者 UI 明确分开。
- Apply 编译时合并两层，Codex 侧只见最终结果。
- 覆盖层不写回 Agent 的 Base，避免「改一个 Profile 就把 Agent 默认配置改了」。

## 备选方案（否决）

- **单一生效绑定（拷贝式）**（否决）：激活 Profile 时把覆盖写入 Agent 绑定。简单，但切换会丢失原 Base，无法回退，且与「Profile 只是配置组合」的产品定位冲突。恢复。