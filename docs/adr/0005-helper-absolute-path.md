# ADR 0005: cas-helper 绝对路径与禁止自定义 executable

## 决策

`auth.command` 一律为**绝对路径**，且该路径由 `cas-platform`（安装环境层）解析并注入 Configuration Compiler——**绝不允许写进 Provider Preset**（Preset 无权控制 executable）。V0.1 使用固定安装路径（稳定 launcher），避免版本升级导致路径漂移。所有文档中 `"cas-helper"` 相对路径示例全部修正为绝对路径。

## 背景

安全规范要求 `auth.command` 绝对路径，但示例与实现存在相对路径漏洞。Provider Preset 若携带 executable 路径，等于给"配置即代码"场景授予任意命令执行权，违反 forbid-list"禁止任意 auth.command"。

## 决策细则

- 唯一可信来源：`cas-platform` 按平台规则定位 helper，编译配置时注入绝对路径。
- Preset 中出现的 `auth.command` 一律视为 invalid configuration，拒绝编译（error 而非 warning）。
- 升级场景：稳定 launcher（如固定目录 + versioned symlink/别名）指向当前 helper 二进制。

## 影响

- 配置 schema 校验新增 `auth.command` 必须绝对路径约束。
- 安装/升级流程负责维护 launcher 稳定性；helper 本体可随版本替换。