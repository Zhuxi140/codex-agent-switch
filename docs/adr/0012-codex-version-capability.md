# ADR 0012: Codex 版本能力探测——Feature Probe 为主，版本登记为辅

## 决策

不硬编码"Codex >= X 才能运行"版本门禁。能力判定以 **Feature Probe（特性探测）** 为主：启动时解析 `config.toml`/`agents/*.toml` 结构，解析失败 → 相关功能标记 `Unsupported` 并禁用（提示配置损坏或格式变化）；未知字段一律 Preserve（Unknown Means Preserve）。`codex version` 不作为主门禁，但通过 **Version Capability Registry（登记表，随版发布）** 影响能力判断：某能力已知需要 ≥ 某版本时，注册表给出该能力的已知下限。**解析成功 ≠ 当前 Codex 一定支持 CAS 所需 multi-agent/provider 行为**——某个特性未知时只阻断受影响功能，不让整个应用进入 Unsupported。

## 背景

版本无法单一决定能力（厂商行为随分发渠道差异），纯版本门禁会误伤可用的旧版/非标准构建；纯探测又无法覆盖"格式能解析但运行时行为缺能力"的盲区。两者配合：探测为默认判定，注册表补充已知边界，未来出现矛盾的证据时以探测为准并回注注册表。

## 影响

- `CodexEnvironment` 状态细化：Detected / Unsupported（探测失败）/ Ready/功能受限（分能力）。
- Diagnostics 增加"版本×能力"注册表判定项，作为 Warning 级提示而非全局 Error。
- 文档约定：注册表随包维护，格式变化只改 `cas-codex` 适配层。