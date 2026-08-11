# 贡献指南（Contributing Guide）

欢迎参与 Codex Agent Switch（CAS）的贡献！本项目是单人起步的开源项目，任何形式的帮助（Issue、PR、文档、反馈）都很有价值。

## 开发环境

- **Windows 10/11**（当前目标平台）
- **Rust**（2024 edition）+ **Node.js ≥ 22**
- **Codex CLI ≥ 0.144.0**（运行时需要，构建不需要）

## 常用命令

```bash
npm.cmd run build          # 前端构建验证（tsc + vite build）
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 单元测试
npm.cmd run bundle:windows # 生成 Windows 安装包（NSIS）
```

## 提 Issue

- 先搜索是否已有相同 Issue
- 描述：环境（Windows 版本、Codex 版本、CAS 版本）、复现步骤、期望行为、实际行为、截图或日志
- 涉及配置写入/凭据的问题请走 [SECURITY.md](SECURITY.md) 的私密渠道

## 提 PR

1. 从 `main` 分支创建特性分支，命名如 `fix/xxx`、`feat/xxx`
2. 遵循既有代码风格（外科手术式修改：只改必要部分）
3. 所有改动必须通过验证：
   - `npm.cmd run build` 通过
   - `cargo test --manifest-path src-tauri/Cargo.toml` 通过（涉及 Rust 改动时）
4. 每个 PR 保持单一职责，方便审查与回滚

## 代码与文档约定

- 交流语言默认中文；代码标识符保持英文
- 数据库变更必须新增迁移文件（`src-tauri/migrations/`），禁止修改已发布的迁移
- 设计文档与 ADR 属内部文档，不入版本控制

## 行为准则

- 尊重他人，就事论事
- 讨论围绕代码与事实，不做人身评价
- 维护者保留最终合入决定权
