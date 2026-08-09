# ADR 0003: Capability 证据来源单一 Domain 枚举

## 决策

Capability 的证据来源（CapabilitySource）定义为一个唯一的 Domain 枚举：`OFFICIAL_PROVIDER / OFFICIAL_CODEX / CAS_BUILT_IN / RUNTIME_PROBE / USER_OVERRIDE / UNKNOWN`，所有文档（数据模型、持久化、IPC、Provider 接入）跟随该枚举。**Discovery/Model Catalog 的来源不算 Capability Evidence**——它们只描述模型的存在/获取方式。用户手工声明"Compatible"不写入 `metadata`，而是作为显式的 `USER_OVERRIDE` Evidence。

## 背景

数据模型文档与持久化规范此前对证据来源枚举不一致；且 Provider 层误用 Compatibility Level（PRESET/GENERIC/ADAPTER/GATEWAY）表达能力。这会把「接入方式」和「兼容结论」混成单一概念。

## 决策细则

- Provider 层不再定义 Compatibility Level。表现三要素：`source=PRESET|CUSTOM`、`adapter=RESPONSES|...`、`protocol=RESPONSES|...` 分离保存/展示；Provider 层至多推导 **Integration Readiness**（静态判断）。
- Codex Compatibility 主要属于 Model 层。
- 用户声明"Provider 已验证兼容" → `USER_OVERRIDE` Evidence，走 metadata resolution 合能进程，可被权威源推翻或兼容性检测覆盖。

## 影响

- `model_catalog` / `capabilities` 表结构统一以该枚举为 `evidence_source`；`provider` 表不再有 Level 列。
- UI 展示 Provider 集成就绪即可，不必展示伪兼容级别。