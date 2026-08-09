可以真实 E2E，但先用隔离的 `CODEX_HOME`。暂时不要 Apply 到你的主 `~/.codex`：当前是开发构建，`cas-helper.exe` 会写成 `target\debug` 绝对路径，而且 [Tauri bundling 仍关闭](<C:\Users\zhuxi\Desktop\Codex Agent Switch\src-tauri\tauri.conf.json>)。

## 1. 正常 PowerShell 中检查 Codex

不要在 Codex 内置终端执行；我这里检测到了 Codex，但沙箱执行 `codex --version` 被 WindowsApps 拒绝。

```powershell
Set-Location "C:\Users\zhuxi\Desktop\Codex Agent Switch"

codex --version
```

要求：

- 命令能正常运行。
- 版本不低于 `0.144.0`。
- 如果出现 `Access is denied`、找不到命令或版本过低，先停止，把完整输出发我。

## 2. 建立隔离测试目录

```powershell
$CasE2eHome = Join-Path $env:LOCALAPPDATA "CAS-E2E\codex-home"

New-Item -ItemType Directory -Force -Path $CasE2eHome

if (-not (Test-Path (Join-Path $CasE2eHome "config.toml"))) {
    New-Item -ItemType File -Path (Join-Path $CasE2eHome "config.toml")
}

$CasE2eHome
```

官方要求自定义 `CODEX_HOME` 必须已经存在；它会承载配置、认证和 Agent 等状态。[OpenAI Docs：环境变量](https://learn.chatgpt.com/docs/config-file/environment-variables)

## 3. 构建并运行 CAS

全程离线，不安装依赖：

```powershell
$env:CARGO_NET_OFFLINE = "true"

npm.cmd run build

cargo build --workspace --offline --manifest-path ".\src-tauri\Cargo.toml"

Test-Path ".\src-tauri\target\debug\cas-helper.exe"

$CasExe = (Resolve-Path ".\src-tauri\target\debug\codex-agent-switch.exe").Path
Start-Process -FilePath $CasExe
```

`Test-Path` 必须返回 `True`。任何命令连续 60 秒没有有效进度，立即 `Ctrl+C`，把卡住的命令发我。

## 4. 在 CAS 中完成配置

按顺序操作：

1. 打开“设置”。
2. 将“自定义 CODEX_HOME”设为上面的 `$CasE2eHome`。
3. 保存，确认页面显示正确目录和 Codex 版本。
4. 打开 Providers，选择 DeepSeek Preset。
5. API Key 只粘贴进 CAS，不要发给我，也不要放进终端。
6. 确认 Models 自动出现 `DeepSeek V4 Flash`。
7. 打开 Agents，从 `Executor` 模板创建 Agent。
8. 将 Executor 绑定到 `DeepSeek V4 Flash`。
9. 回到概览，先点击 Preview。
10. 没有 blocker 后再 Apply。
11. 运行 Diagnostics。

Apply 后应该生成：

```powershell
Get-ChildItem -Recurse $CasE2eHome

Get-Content -Raw (Join-Path $CasE2eHome "config.toml")

Get-Content -Raw (Join-Path $CasE2eHome "agents\cas-executor.toml")
```

预期：

- `config.toml` 包含 `model_providers.cas_deepseek`、`wire_api = "responses"` 和 `auth.command`。
- Agent 文件包含 `name = "executor"`、`model = "deepseek-v4-flash"`。
- 文件中只有 Credential UUID，没有 API Key。
- 不要手动执行 `cas-helper token <uuid>`，它会把 Key 输出到终端。

## 5. 真实调用 DeepSeek 子 Agent

仍在普通 PowerShell 中：

```powershell
$env:CODEX_HOME = $CasE2eHome

codex login
```

然后进入一个已有 Git 项目并启动：

```powershell
Set-Location "你的某个 Git 项目"
codex
```

发送：

```text
必须把下面任务委派给名为 executor 的自定义 subagent，不要由主 Agent 自己完成。

让 executor 只回答 CAS_DEEPSEEK_E2E_OK，不得修改任何文件。
```

运行过程中输入 `/agent`，确认确实出现 `executor` 线程。Codex 当前会加载 `$CODEX_HOME/agents/*.toml`，直接要求委派后可通过 `/agent` 检查子线程。[OpenAI Docs：Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)

成功标准：

- `/agent` 中出现 `executor`。
- 返回 `CAS_DEEPSEEK_E2E_OK`。
- 没有 `401/403`、模型不存在、Responses 协议或 `auth.command` 错误。
- DeepSeek 控制台能看到对应请求。

先完成第 1 步，把 `codex --version` 输出发我；然后我继续陪你逐项验收。