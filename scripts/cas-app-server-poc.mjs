import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { resolve } from "node:path";

const DEFAULT_TIMEOUT_MS = 120_000;

function parseArgs(argv) {
  const options = {
    agent: "executor",
    codex: "codex",
    codexHome: process.env.CODEX_HOME
      ?? (process.env.USERPROFILE ? `${process.env.USERPROFILE}\\.codex` : undefined)
      ?? (process.env.HOME ? `${process.env.HOME}/.codex` : undefined),
    cwd: process.cwd(),
    selfTest: false,
    timeoutMs: DEFAULT_TIMEOUT_MS,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--self-test") {
      options.selfTest = true;
      continue;
    }
    if (argument === "--help") {
      printHelp();
      process.exit(0);
    }
    const key = {
      "--agent": "agent",
      "--codex": "codex",
      "--codex-home": "codexHome",
      "--cwd": "cwd",
      "--timeout-ms": "timeoutMs",
    }[argument];
    if (!key || index + 1 >= argv.length) {
      throw new Error(`未知或缺少值的参数：${argument}`);
    }
    options[key] = key === "timeoutMs" ? Number(argv[index + 1]) : argv[index + 1];
    index += 1;
  }

  if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 1_000) {
    throw new Error("--timeout-ms 必须是大于等于 1000 的整数。");
  }
  options.cwd = resolve(options.cwd);
  return options;
}

function printHelp() {
  console.log(`CAS App Server Runtime Bridge POC

用法：
  npm.cmd run poc:app-server -- [选项]

选项：
  --agent <key>          要验证的自定义子 Agent，默认 executor
  --codex <path>         Codex 可执行文件，默认 codex
  --codex-home <path>    传给 App Server 的 CODEX_HOME
  --cwd <path>           POC 工作目录，默认当前目录
  --timeout-ms <number>  每个协议步骤超时，默认 ${DEFAULT_TIMEOUT_MS}
  --self-test            仅运行安全事件解析测试，不调用模型
`);
}

class Observation {
  constructor() {
    this.completedTurns = new Set();
    this.currentPhase = null;
    this.edges = new Map();
    this.modelByThread = new Map();
    this.stderrLineCount = 0;
    this.toolsByPhase = { first: new Set(), second: new Set() };
    this.receiversByPhase = { first: new Set(), second: new Set() };
    this.usageByThread = new Map();
    this.usageEventCountByThread = new Map();
  }

  observe(message) {
    if (message?.method === "turn/completed") {
      const turnId = message.params?.turn?.id;
      if (typeof turnId === "string") this.completedTurns.add(turnId);
      return;
    }

    if (message?.method === "thread/tokenUsage/updated") {
      const { threadId, tokenUsage } = message.params ?? {};
      const usage = sanitizeTokenUsage(tokenUsage);
      if (typeof threadId === "string" && usage) {
        this.usageByThread.set(threadId, usage);
        this.usageEventCountByThread.set(
          threadId,
          (this.usageEventCountByThread.get(threadId) ?? 0) + 1,
        );
      }
      return;
    }

    if (message?.method !== "item/started" && message?.method !== "item/completed") return;
    const item = message.params?.item;
    if (item?.type !== "collabAgentToolCall" && item?.type !== "collabToolCall") return;

    const sender = item.senderThreadId;
    const receivers = Array.isArray(item.receiverThreadIds)
      ? item.receiverThreadIds.filter((value) => typeof value === "string")
      : typeof item.receiverThreadId === "string"
        ? [item.receiverThreadId]
        : typeof item.newThreadId === "string"
          ? [item.newThreadId]
          : [];

    if (typeof sender === "string") {
      const known = this.edges.get(sender) ?? new Set();
      receivers.forEach((receiver) => known.add(receiver));
      this.edges.set(sender, known);
    }
    if (typeof item.model === "string") {
      receivers.forEach((receiver) => this.modelByThread.set(receiver, item.model));
    }
    if (this.currentPhase) {
      if (typeof item.tool === "string") this.toolsByPhase[this.currentPhase].add(item.tool);
      receivers.forEach((receiver) => this.receiversByPhase[this.currentPhase].add(receiver));
    }
  }

  summary(rootThreadId, resumeAccepted, turnIds) {
    const firstReceivers = this.receiversByPhase.first;
    const secondReceivers = this.receiversByPhase.second;
    const childThreadIds = [...new Set(
      [...this.edges.values()].flatMap((receivers) => [...receivers]),
    )].sort();
    const reusedChildIds = [...secondReceivers].filter((threadId) => firstReceivers.has(threadId));
    const newSecondTurnChildIds = [...secondReceivers].filter(
      (threadId) => !firstReceivers.has(threadId),
    );
    const childUsageThreadIds = childThreadIds.filter((threadId) =>
      this.usageByThread.has(threadId)
    );

    return {
      childReuseObserved: reusedChildIds.length > 0 && newSecondTurnChildIds.length === 0,
      childThreadIds,
      childUsageObserved: childUsageThreadIds.length > 0,
      childUsageThreadIds,
      firstTurnId: turnIds.first,
      firstTurnTools: [...this.toolsByPhase.first].sort(),
      modelByThread: Object.fromEntries(
        [...this.modelByThread.entries()].sort(([left], [right]) => left.localeCompare(right)),
      ),
      newSecondTurnChildIds,
      parentChildEdges: [...this.edges.entries()].map(([senderThreadId, receivers]) => ({
        receiverThreadIds: [...receivers].sort(),
        senderThreadId,
      })),
      protocolInitialized: true,
      resumeAccepted,
      rootUsageObserved: this.usageByThread.has(rootThreadId),
      reusedChildIds,
      rootThreadId,
      secondTurnId: turnIds.second,
      secondTurnTools: [...this.toolsByPhase.second].sort(),
      stderrLineCount: this.stderrLineCount,
      usageByThread: Object.fromEntries(
        [...this.usageByThread.entries()].sort(([left], [right]) => left.localeCompare(right)),
      ),
      usageEventCountByThread: Object.fromEntries(
        [...this.usageEventCountByThread.entries()]
          .sort(([left], [right]) => left.localeCompare(right)),
      ),
    };
  }
}

function nonNegativeInteger(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value : 0;
}

function sanitizeBreakdown(value) {
  if (!value || typeof value !== "object") return null;
  return {
    cacheWriteInputTokens: nonNegativeInteger(value.cacheWriteInputTokens),
    cachedInputTokens: nonNegativeInteger(value.cachedInputTokens),
    inputTokens: nonNegativeInteger(value.inputTokens),
    outputTokens: nonNegativeInteger(value.outputTokens),
    reasoningOutputTokens: nonNegativeInteger(value.reasoningOutputTokens),
    totalTokens: nonNegativeInteger(value.totalTokens),
  };
}

function sanitizeTokenUsage(value) {
  const last = sanitizeBreakdown(value?.last);
  const total = sanitizeBreakdown(value?.total);
  if (!last || !total) return null;
  return {
    last,
    modelContextWindow: Number.isSafeInteger(value.modelContextWindow)
      ? value.modelContextWindow
      : null,
    total,
  };
}

class AppServerClient {
  constructor(options, observation) {
    const env = { ...process.env };
    if (options.codexHome) env.CODEX_HOME = resolve(options.codexHome);
    this.observation = observation;
    this.nextId = 1;
    this.pending = new Map();
    this.process = spawn(options.codex, ["app-server", "--listen", "stdio://"], {
      cwd: options.cwd,
      env,
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });

    createInterface({ input: this.process.stdout }).on("line", (line) => {
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        this.failAll("App Server stdout 返回了非 JSONL 内容。");
        return;
      }
      this.observation.observe(message);
      if (message.id === undefined) return;
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      clearTimeout(pending.timer);
      if (message.error) {
        pending.reject(new Error(
          `${pending.method} 失败：${message.error.message ?? "未知 App Server 错误"}`,
        ));
      } else {
        pending.resolve(message.result);
      }
    });

    createInterface({ input: this.process.stderr }).on("line", () => {
      this.observation.stderrLineCount += 1;
    });

    this.process.on("error", (error) => this.failAll(`无法启动 App Server：${error.message}`));
    this.process.on("exit", (code, signal) => {
      this.failAll(`App Server 已退出（code=${code ?? "null"}, signal=${signal ?? "null"}）。`);
    });
  }

  failAll(message) {
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new Error(message));
    }
    this.pending.clear();
  }

  notify(method, params) {
    this.process.stdin.write(`${JSON.stringify({ method, params })}\n`);
  }

  request(method, params, timeoutMs) {
    const id = this.nextId;
    this.nextId += 1;
    return new Promise((resolveRequest, rejectRequest) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        rejectRequest(new Error(`${method} 等待超过 ${timeoutMs}ms。`));
      }, timeoutMs);
      this.pending.set(id, {
        method,
        reject: rejectRequest,
        resolve: resolveRequest,
        timer,
      });
      this.process.stdin.write(`${JSON.stringify({ id, method, params })}\n`);
    });
  }

  async waitForTurn(turnId, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (!this.observation.completedTurns.has(turnId)) {
      if (Date.now() >= deadline) throw new Error(`Turn ${turnId} 等待完成超时。`);
      await new Promise((resolveWait) => setTimeout(resolveWait, 100));
    }
  }

  async close() {
    this.failAll("POC 正在关闭 App Server。");
    if (this.process.exitCode !== null || this.process.signalCode !== null) return;
    this.process.kill();
    await Promise.race([
      new Promise((resolveExit) => this.process.once("exit", resolveExit)),
      new Promise((resolveWait) => setTimeout(resolveWait, 2_000)),
    ]);
  }
}

async function runPoc(options) {
  const observation = new Observation();
  const client = new AppServerClient(options, observation);
  let resumeAccepted = false;

  try {
    await client.request("initialize", {
      clientInfo: {
        name: "codex_agent_switch_poc",
        title: "Codex Agent Switch Runtime Bridge POC",
        version: "0.1.0",
      },
    }, options.timeoutMs);
    client.notify("initialized", {});

    const started = await client.request("thread/start", {
      approvalPolicy: "never",
      cwd: options.cwd,
      sandbox: "workspace-write",
      serviceName: "codex_agent_switch_poc",
    }, options.timeoutMs);
    const rootThreadId = started?.thread?.id;
    if (typeof rootThreadId !== "string") throw new Error("thread/start 未返回 Thread ID。");

    observation.currentPhase = "first";
    const first = await client.request("turn/start", {
      input: [{
        type: "text",
        text: `这是 CAS Runtime Bridge POC。必须只创建一次名为 ${options.agent} 的自定义子 Agent，让它只读取当前工作目录 package.json 的 name 字段，不得修改任何文件。等待它完成，但不要关闭子 Agent；只回复 POC_FIRST:<name>。`,
      }],
      threadId: rootThreadId,
    }, options.timeoutMs);
    const firstTurnId = first?.turn?.id;
    if (typeof firstTurnId !== "string") throw new Error("第一个 turn/start 未返回 Turn ID。");
    await client.waitForTurn(firstTurnId, options.timeoutMs);

    try {
      await client.request("thread/resume", { threadId: rootThreadId }, options.timeoutMs);
      resumeAccepted = true;
    } catch {
      resumeAccepted = false;
    }

    observation.currentPhase = "second";
    const second = await client.request("turn/start", {
      input: [{
        type: "text",
        text: `继续 CAS Runtime Bridge POC。必须复用上一轮创建的同一个 ${options.agent} 子 Agent Thread，不得创建新的子 Agent。向它发送 follow-up，只读取 package.json 的 version 字段，不得修改文件。等待完成后，只回复 POC_SECOND:<version>。`,
      }],
      threadId: rootThreadId,
    }, options.timeoutMs);
    const secondTurnId = second?.turn?.id;
    if (typeof secondTurnId !== "string") throw new Error("第二个 turn/start 未返回 Turn ID。");
    await client.waitForTurn(secondTurnId, options.timeoutMs);
    observation.currentPhase = null;

    const summary = observation.summary(rootThreadId, resumeAccepted, {
      first: firstTurnId,
      second: secondTurnId,
    });
    const passed = resumeAccepted
      && summary.rootUsageObserved
      && summary.childThreadIds.length > 0
      && summary.childReuseObserved
      && summary.childUsageObserved;
    console.log(JSON.stringify({ agentKey: options.agent, passed, ...summary }, null, 2));
    if (!passed) process.exitCode = 2;
  } finally {
    await client.close();
  }
}

function runSelfTest() {
  const observation = new Observation();
  observation.currentPhase = "first";
  observation.observe({
    method: "item/completed",
    params: {
      item: {
        prompt: "THIS_MUST_NOT_LEAK",
        receiverThreadIds: ["child-1"],
        senderThreadId: "root-1",
        status: "completed",
        tool: "spawn_agent",
        type: "collabAgentToolCall",
      },
    },
  });
  observation.observe({
    method: "thread/tokenUsage/updated",
    params: {
      threadId: "child-1",
      tokenUsage: {
        last: {
          cachedInputTokens: 2,
          inputTokens: 3,
          outputTokens: 4,
          reasoningOutputTokens: 1,
          totalTokens: 8,
        },
        modelContextWindow: 100_000,
        total: {
          cachedInputTokens: 2,
          inputTokens: 3,
          outputTokens: 4,
          reasoningOutputTokens: 1,
          totalTokens: 8,
        },
      },
      turnId: "turn-child-1",
    },
  });
  observation.currentPhase = "second";
  observation.observe({
    method: "item/completed",
    params: {
      item: {
        receiverThreadIds: ["child-1"],
        senderThreadId: "root-1",
        status: "completed",
        tool: "followup_task",
        type: "collabAgentToolCall",
      },
    },
  });
  const summary = observation.summary("root-1", true, {
    first: "turn-1",
    second: "turn-2",
  });
  assert.equal(summary.childReuseObserved, true);
  assert.equal(summary.childUsageObserved, true);
  assert.equal(JSON.stringify(summary).includes("THIS_MUST_NOT_LEAK"), false);
  assert.deepEqual(summary.reusedChildIds, ["child-1"]);
  console.log("CAS_APP_SERVER_POC_SELF_TEST_OK");
}

try {
  const options = parseArgs(process.argv.slice(2));
  if (options.selfTest) {
    runSelfTest();
  } else {
    await runPoc(options);
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
