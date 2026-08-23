import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  addProjectExclusion,
  addModel,
  createAgent,
  createProvider,
  deleteAgent,
  deleteModel,
  deleteProvider,
  deleteProjectExclusion,
  getAgent,
  getAppBootstrap,
  getCodexEnvironment,
  getConfigurationStatus,
  getRuntimeMode,
  getProvider,
  getSettings,
  getUsageSummary,
  getUsageMonitorStatus,
  listAgentPresets,
  listAgentScheduleDecisions,
  listAgentThreadInstances,
  listAgentThreadProjects,
  listAgents,
  listModels,
  listProviders,
  listProjectExclusions,
  listSnapshots,
  listUsageRecords,
  recommendAgentThreadInstance,
  redetectCodex,
  restoreSnapshot,
  resolveRuntimeModeConflict,
  runDiagnostics,
  setModelEnabled,
  startUsageMonitor,
  stopUsageMonitor,
  setAgentThreadInstanceWorkspaceScope,
  testModelConnection,
  switchRuntimeMode,
  updateSettings,
  updateAgent,
  updateModel,
  updateProvider,
  type Appearance,
  type AgentDetailResponse,
  type AgentPresetResponse,
  type AgentReuseStrategy,
  type AgentSummary,
  type AgentThreadInstanceResponse,
  type AgentThreadInstanceRecommendation,
  type AgentThreadInstanceStatus,
  type AgentThreadProjectSummaryResponse,
  type AppBootstrapResponse,
  type CodexEnvironmentResponse,
  type ConfigurationStatusResponse,
  type ConfigurationConflictResponse,
  type DiagnosticsResponse,
  type ModelSummary,
  type NativeSubagentSyncResponse,
  type OrchestrationPhase,
  type ProviderCreateRequest,
  type ProviderCacheRetentionType,
  type ProviderCacheSupport,
  type ProviderDetailResponse,
  type ProviderSummary,
  type ProjectExclusion,
  type ReasoningPolicy,
  type RuntimeModeResponse,
  type RuntimeBridgeStatusResponse,
  type SandboxPolicy,
  type ScheduleDecisionResponse,
  type SnapshotSummary,
  type SettingsResponse,
  type UsageQueryRequest,
  type UsageRecordResponse,
  type UsageStatus,
  type UsageSummaryResponse,
} from "./api";

type Page =
  | "overview"
  | "usage"
  | "agents"
  | "providers"
  | "models"
  | "diagnostics"
  | "settings";

const navigation: Array<{ label: string; page?: Page }> = [
  { label: "概览", page: "overview" },
  { label: "用量监控", page: "usage" },
  { label: "Agents", page: "agents" },
  { label: "Providers", page: "providers" },
  { label: "Models", page: "models" },
  { label: "诊断", page: "diagnostics" },
  { label: "设置", page: "settings" },
];

type IconName =
  | "alert"
  | "book"
  | "check"
  | "chevron-down"
  | "chevron-right"
  | "chevron-up"
  | "close"
  | "clock"
  | "copy"
  | "edit"
  | "external-link"
  | "info"
  | "refresh"
  | "threads"
  | "tokens"
  | "trash"
  | "users"
  | "x-circle";

function UiIcon({ name }: { name: IconName }) {
  let content: ReactNode;
  switch (name) {
    case "alert":
      content = <><path d="M10.3 4.2 2.7 17.4A2 2 0 0 0 4.4 20h15.2a2 2 0 0 0 1.7-2.6L13.7 4.2a2 2 0 0 0-3.4 0Z" /><path d="M12 9v4M12 17h.01" /></>;
      break;
    case "book":
      content = <><path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20" /><path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2Z" /></>;
      break;
    case "check":
      content = <path d="m5 12 4 4L19 6" />;
      break;
    case "chevron-down":
      content = <path d="m6 9 6 6 6-6" />;
      break;
    case "chevron-right":
      content = <path d="m9 18 6-6-6-6" />;
      break;
    case "chevron-up":
      content = <path d="m18 15-6-6-6 6" />;
      break;
    case "close":
      content = <path d="M6 6l12 12M18 6 6 18" />;
      break;
    case "clock":
      content = <><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></>;
      break;
    case "copy":
      content = <><rect x="8" y="8" width="12" height="12" rx="2" /><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2" /></>;
      break;
    case "edit":
      content = <><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L8 18l-4 1 1-4Z" /></>;
      break;
    case "external-link":
      content = <><path d="M14 4h6v6M20 4l-9 9" /><path d="M18 13v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h6" /></>;
      break;
    case "info":
      content = <><circle cx="12" cy="12" r="9" /><path d="M12 11v5" /><path d="M12 8h.01" /></>;
      break;
    case "refresh":
      content = <><path d="M20 7v5h-5" /><path d="M4 17v-5h5" /><path d="M6.1 9A7 7 0 0 1 18 6l2 2M18 15a7 7 0 0 1-11.9 3L4 16" /></>;
      break;
    case "threads":
      content = <><circle cx="6" cy="5" r="2" /><circle cx="18" cy="7" r="2" /><circle cx="6" cy="19" r="2" /><path d="M8 5h3a3 3 0 0 1 3 3v7a4 4 0 0 1-4 4H8M14 10h2a2 2 0 0 0 2-2" /></>;
      break;
    case "tokens":
      content = <><ellipse cx="12" cy="5" rx="8" ry="3" /><path d="M4 5v6c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 11v6c0 1.7 3.6 3 8 3s8-1.3 8-3v-6" /></>;
      break;
    case "trash":
      content = <><path d="M3 6h18M8 6V4h8v2M19 6l-1 15H6L5 6M10 11v6M14 11v6" /></>;
      break;
    case "users":
      content = <><circle cx="9" cy="8" r="3" /><path d="M3 20a6 6 0 0 1 12 0M16 4a3 3 0 0 1 0 6M17 14a5 5 0 0 1 4 5" /></>;
      break;
    case "x-circle":
      content = <><circle cx="12" cy="12" r="9" /><path d="m9 9 6 6M15 9l-6 6" /></>;
      break;
  }
  return (
    <svg aria-hidden="true" className="ui-icon" fill="none" viewBox="0 0 24 24">
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8">
        {content}
      </g>
    </svg>
  );
}

function Tooltip({
  children,
  content,
  focusable = false,
  label,
}: {
  children: ReactNode;
  content: string;
  focusable?: boolean;
  label?: string;
}) {
  const tooltipId = useId();
  const anchorRef = useRef<HTMLSpanElement>(null);
  const openTimer = useRef<number | null>(null);
  const closeTimer = useRef<number | null>(null);
  const [position, setPosition] = useState<{ above: boolean; left: number; top: number } | null>(null);

  function clearTimers() {
    if (openTimer.current !== null) window.clearTimeout(openTimer.current);
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    openTimer.current = null;
    closeTimer.current = null;
  }

  function show(delay: number) {
    clearTimers();
    const open = () => {
      const bounds = anchorRef.current?.getBoundingClientRect();
      if (!bounds) return;
      const above = bounds.bottom + 130 > window.innerHeight;
      setPosition({
        above,
        left: Math.min(window.innerWidth - 156, Math.max(156, bounds.left + bounds.width / 2)),
        top: above ? bounds.top - 8 : bounds.bottom + 8,
      });
    };
    if (delay === 0) open();
    else openTimer.current = window.setTimeout(open, delay);
  }

  function hide(delay = 0) {
    if (openTimer.current !== null) window.clearTimeout(openTimer.current);
    openTimer.current = null;
    if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
    closeTimer.current = window.setTimeout(() => setPosition(null), delay);
  }

  useEffect(() => () => clearTimers(), []);

  return (
    <>
      <span
        aria-describedby={position ? tooltipId : undefined}
        aria-label={focusable ? label ?? content : undefined}
        className="tooltip-anchor"
        onBlur={() => hide(80)}
        onFocus={() => show(0)}
        onKeyDown={(event) => {
          if (event.key === "Escape") hide();
        }}
        onMouseEnter={() => show(800)}
        onMouseLeave={() => hide(120)}
        ref={anchorRef}
        tabIndex={focusable ? 0 : undefined}
      >
        {children}
      </span>
      {position && createPortal(
        <span
          className={`ui-tooltip ${position.above ? "above" : ""}`}
          id={tooltipId}
          onMouseEnter={() => {
            if (closeTimer.current !== null) window.clearTimeout(closeTimer.current);
            closeTimer.current = null;
          }}
          onMouseLeave={() => hide()}
          role="tooltip"
          style={{ left: position.left, top: position.top }}
        >
          {content}
        </span>,
        document.body,
      )}
    </>
  );
}

function IconButton({
  disabled = false,
  icon,
  label,
  loading = false,
  onClick,
  state = "idle",
  tone = "neutral",
}: {
  disabled?: boolean;
  icon: IconName;
  label: string;
  loading?: boolean;
  onClick: () => void;
  state?: "idle" | "error" | "success";
  tone?: "neutral" | "danger";
}) {
  return (
    <Tooltip content={label}>
      <button
        aria-busy={loading || undefined}
        aria-label={label}
        className={`icon-button ${tone}`}
        data-state={loading ? "loading" : state}
        disabled={disabled || loading}
        onClick={onClick}
        type="button"
      >
        <UiIcon name={loading ? "refresh" : icon} />
      </button>
    </Tooltip>
  );
}

function CopyIconButton({ label, value }: { label: string; value: string }) {
  const [state, setState] = useState<"idle" | "error" | "success">("idle");
  const resetTimer = useRef<number | null>(null);

  useEffect(() => () => {
    if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
  }, []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(value);
      setState("success");
    } catch {
      setState("error");
    }
    if (resetTimer.current !== null) window.clearTimeout(resetTimer.current);
    resetTimer.current = window.setTimeout(() => setState("idle"), 2500);
  }

  const statusLabel = state === "success" ? "已复制" : state === "error" ? "复制失败" : label;
  return (
    <>
      <IconButton
        icon={state === "success" ? "check" : "copy"}
        label={statusLabel}
        onClick={() => void copy()}
        state={state}
      />
      <span aria-live="polite" className="sr-only">{state === "idle" ? "" : statusLabel}</span>
    </>
  );
}

function InfoTip({ label }: { label: string }) {
  return (
    <Tooltip content={label} focusable label={label}>
      <span className="info-tip"><UiIcon name="info" /></span>
    </Tooltip>
  );
}

export function App() {
  const [page, setPage] = useState<Page>("overview");
  const [bootstrap, setBootstrap] = useState<AppBootstrapResponse | null>(null);
  const [environment, setEnvironment] = useState<CodexEnvironmentResponse | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [appearance, setAppearance] = useState<Appearance>("SYSTEM");
  const [customFontFamily, setCustomFontFamily] = useState<string | null>(null);

  useEffect(() => {
    getAppBootstrap().then(setBootstrap).catch((reason: unknown) => {
      setError(errorMessage(reason));
    });
    getCodexEnvironment().then(setEnvironment).catch((reason: unknown) => {
      setError(errorMessage(reason));
    });
    getSettings()
      .then((settings) => {
        setAppearance(settings.appearance);
        setCustomFontFamily(settings.customFontFamily);
      })
      .catch((reason: unknown) => setError(errorMessage(reason)));
  }, []);

  useEffect(() => {
    const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
    const apply = () => {
      document.documentElement.dataset.theme =
        appearance === "SYSTEM" ? (colorScheme.matches ? "DARK" : "LIGHT") : appearance;
    };
    apply();
    if (appearance === "SYSTEM") {
      colorScheme.addEventListener("change", apply);
      return () => colorScheme.removeEventListener("change", apply);
    }
  }, [appearance]);

  useEffect(() => {
    document.documentElement.style.setProperty(
      "--app-font-family",
      customFontFamily
        ? `"${customFontFamily}", var(--system-font-family)`
        : "var(--system-font-family)",
    );
  }, [customFontFamily]);

  function handleRedetect() {
    setDetecting(true);
    setError(null);
    redetectCodex()
      .then((result) => {
        setEnvironment(result);
        setBootstrap((current) =>
          current
            ? {
                ...current,
                codex: {
                  detected: result.detected,
                  version: result.version,
                  multiAgentAvailable: result.multiAgentAvailable,
                },
              }
            : current,
        );
      })
      .catch((reason: unknown) => setError(errorMessage(reason)))
      .finally(() => setDetecting(false));
  }

  return (
    <div className="app-shell">
      <header className="app-topbar" data-tauri-drag-region>
        <div className="app-brand" data-tauri-drag-region>
          <div className="brand-copy">
            <strong>Codex Agent Switch</strong>
            <p>Responses-first</p>
          </div>
        </div>
        <nav className="app-switcher" aria-label="主导航">
          {navigation.map((item) => (
            <button
              aria-current={item.page === page ? "page" : undefined}
              className={item.page === page ? "active" : ""}
              disabled={!item.page}
              key={item.label}
              onClick={() => item.page && setPage(item.page)}
              type="button"
            >
              {item.label}
            </button>
          ))}
        </nav>
        <div className="topbar-status">
          <span className={bootstrap?.codex.detected ? "online" : ""} aria-hidden="true" />
          <Tooltip
            content={bootstrap?.codex.version ? `Codex ${bootstrap.codex.version}` : "尚未读取到 Codex 版本"}
            focusable
            label={bootstrap?.codex.version ? `Codex 版本 ${bootstrap.codex.version}` : "尚未读取到 Codex 版本"}
          >
            <span>
              {!bootstrap ? "正在检测" : bootstrap.codex.detected ? "Codex Ready" : "Codex 未检测"}
            </span>
          </Tooltip>
        </div>
      </header>

      <main>
        {page === "overview" ? (
          <OverviewPage
            detecting={detecting}
            environment={environment}
            error={error}
            onRedetect={handleRedetect}
          />
        ) : page === "usage" ? (
          <UsagePage />
        ) : page === "agents" ? (
          <AgentsPage />
        ) : page === "providers" ? (
          <ProvidersPage />
        ) : page === "diagnostics" ? (
          <DiagnosticsPage />
        ) : page === "settings" ? (
          <SettingsPage
            onAppearanceChange={setAppearance}
            onEnvironmentChange={setEnvironment}
            onFontFamilyChange={setCustomFontFamily}
          />
        ) : (
          <ModelsPage onOpenProviders={() => setPage("providers")} />
        )}
      </main>
    </div>
  );
}

interface OverviewPageProps {
  detecting: boolean;
  environment: CodexEnvironmentResponse | null;
  error: string | null;
  onRedetect: () => void;
}

function OverviewPage({
  detecting,
  environment,
  error,
  onRedetect,
}: OverviewPageProps) {
  const [configuration, setConfiguration] = useState<ConfigurationStatusResponse | null>(null);
  const [runtimeMode, setRuntimeMode] = useState<RuntimeModeResponse | null>(null);
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [selectedMode, setSelectedMode] = useState<"DEFAULT" | "SUBAGENT">("DEFAULT");
  const [selectedAgentsByRole, setSelectedAgentsByRole] = useState<Record<string, string>>({});
  const [snapshots, setSnapshots] = useState<SnapshotSummary[]>([]);
  const [operation, setOperation] = useState<"switch" | "restore" | null>(null);
  const [refreshingMode, setRefreshingMode] = useState(false);
  const [agentConfigOpen, setAgentConfigOpen] = useState(false);
  const [policySaving, setPolicySaving] = useState(false);
  const [configurationError, setConfigurationError] = useState<string | null>(null);
  const [configurationSuccess, setConfigurationSuccess] = useState<string | null>(null);
  const [configurationConflict, setConfigurationConflict] =
    useState<ConfigurationConflictResponse | null>(null);
  const [conflictActiveAgentIds, setConflictActiveAgentIds] = useState<string[]>([]);
  const [conflictBusy, setConflictBusy] = useState(false);
  const [conflictError, setConflictError] = useState<string | null>(null);
  const [failurePolicy, setFailurePolicy] =
    useState<SettingsResponse["orchestrationFailurePolicy"]>("STRICT_STOP");

  const reloadConfiguration = useCallback(async () => {
    const [statusResult, modeResult, agentsResult, historyResult, settingsResult] = await Promise.allSettled([
      withTimeout(getConfigurationStatus(), "读取配置状态"),
      withTimeout(getRuntimeMode(), "读取运行模式"),
      withTimeout(listAgents(), "读取 Agent"),
      withTimeout(listSnapshots(6), "读取 Snapshot"),
      withTimeout(getSettings(), "读取编排设置"),
    ]);

    if (statusResult.status === "fulfilled") setConfiguration(statusResult.value);
    if (agentsResult.status === "fulfilled") setAgents(agentsResult.value);
    if (historyResult.status === "fulfilled") setSnapshots(historyResult.value.items);
    if (settingsResult.status === "fulfilled") {
      setFailurePolicy(settingsResult.value.orchestrationFailurePolicy);
    }

    if (modeResult.status === "fulfilled") {
      const mode = modeResult.value;
      setRuntimeMode(mode);
      const currentBindings = Object.fromEntries(
        mode.activeBindings.map((binding) => [binding.roleKey, binding.agentId]),
      );
      if (mode.legacyActiveAgentId && agentsResult.status === "fulfilled") {
        const legacy = agentsResult.value.find((agent) => agent.id === mode.legacyActiveAgentId);
        if (legacy?.roleKey) currentBindings[legacy.roleKey] = legacy.id;
      }
      setSelectedMode(
        mode.activeBindings.length > 0 || mode.legacyActiveAgentId ? "SUBAGENT" : "DEFAULT",
      );
      setSelectedAgentsByRole((current) => {
        if (Object.keys(currentBindings).length > 0) return currentBindings;
        if (agentsResult.status !== "fulfilled") return current;
        return Object.fromEntries(
          Object.entries(current).filter(([roleKey, agentId]) =>
            agentsResult.value.some((agent) => agent.id === agentId && agent.roleKey === roleKey),
          ),
        );
      });
    }

    const failure = [statusResult, modeResult, agentsResult, historyResult, settingsResult]
      .find((result) => result.status === "rejected");
    if (failure?.status === "rejected") throw failure.reason;
  }, []);

  useEffect(() => {
    reloadConfiguration().catch((reason: unknown) => {
      setConfigurationError(errorMessage(reason));
    });
  }, [reloadConfiguration]);

  async function handleModeSwitch() {
    const activeAgentIds = selectedMode === "SUBAGENT"
      ? Object.values(selectedAgentsByRole).filter(Boolean)
      : [];
    setOperation("switch");
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      const result = await withTimeout(switchRuntimeMode(activeAgentIds), "切换运行模式", 15_000);
      if (result.status === "CONFLICT") {
        if (!result.conflict) throw new Error("配置冲突详情缺失，请刷新状态后重试。");
        setConfigurationConflict(result.conflict);
        setConflictActiveAgentIds(activeAgentIds);
        setConflictError(null);
        return;
      }
      if (result.status === "FAILED_ROLLED_BACK" || result.status === "RECOVERY_REQUIRED") {
        throw new Error(
          result.status === "FAILED_ROLLED_BACK"
            ? "模式切换失败，磁盘配置已自动回滚。"
            : "模式切换未完成，需要先恢复配置事务。",
        );
      }
      const reasoningWarning = result.warnings.find((warning) =>
        warning.code === "AGENT_REASONING_DOWNGRADED"
        || warning.code === "AGENT_REASONING_INHERIT_RESOLVED"
      );
      setConfigurationSuccess(
        `${activeAgentIds.length > 0
          ? `已启用 ${activeAgentIds.length} 个子 Agent。请完全退出并重启 Codex，再新建任务；父任务权限保持 Auto 或 Workspace。`
          : "已切换到 Default。请在 Codex 中新建任务使模式变更生效。"}${
          reasoningWarning ? ` ${reasoningWarning.message}` : ""
        }`,
      );
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConfigurationError(errorMessage(reason));
      await reloadConfiguration();
    } finally {
      setOperation(null);
    }
  }

  async function handleConflictResolution(strategy: "ADOPT" | "REPLACE") {
    if (!configurationConflict || conflictBusy) return;
    if (
      strategy === "REPLACE"
      && !window.confirm(
        "备份并替换冲突的 CAS 配置？CAS 只处理列出的配置片段和 CAS 命名文件，失败时自动恢复。",
      )
    ) {
      return;
    }
    setConflictBusy(true);
    setConflictError(null);
    try {
      const result = await withTimeout(
        resolveRuntimeModeConflict(
          conflictActiveAgentIds,
          strategy,
          configurationConflict,
        ),
        strategy === "ADOPT" ? "接管现有配置" : "备份并替换配置",
        15_000,
      );
      if (result.status === "CONFLICT") {
        if (!result.conflict) throw new Error("配置冲突详情缺失，请取消后重新同步。");
        setConfigurationConflict(result.conflict);
        setConflictError("检测到磁盘配置在确认期间发生变化，已刷新冲突详情，请重新确认。");
        return;
      }
      if (result.status === "FAILED_ROLLED_BACK" || result.status === "RECOVERY_REQUIRED") {
        throw new Error(
          result.status === "FAILED_ROLLED_BACK"
            ? "配置替换失败，磁盘内容已自动回滚。"
            : "配置替换未完成，需要先恢复配置事务。",
        );
      }
      setConfigurationConflict(null);
      setConflictActiveAgentIds([]);
      setConfigurationSuccess(
        strategy === "ADOPT"
          ? "已接管语义一致的现有配置。"
          : "已有 CAS 配置已备份并替换。请完全退出并重启 Codex，再新建任务。",
      );
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConflictError(errorMessage(reason));
    } finally {
      setConflictBusy(false);
    }
  }

  async function handleModeRefresh() {
    setRefreshingMode(true);
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConfigurationError(errorMessage(reason));
    } finally {
      setRefreshingMode(false);
      setOperation((current) => current === "switch" ? null : current);
    }
  }

  async function handleFailurePolicyChange(
    nextPolicy: SettingsResponse["orchestrationFailurePolicy"],
  ) {
    if (nextPolicy === failurePolicy || policySaving) return;
    const previousPolicy = failurePolicy;
    setFailurePolicy(nextPolicy);
    setPolicySaving(true);
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      const updated = await updateSettings({ orchestrationFailurePolicy: nextPolicy });
      setFailurePolicy(updated.orchestrationFailurePolicy);
      setConfigurationSuccess("子 Agent 失败策略已保存；请同步当前模式并重启 Codex 后生效。");
      try {
        await reloadConfiguration();
      } catch (reason: unknown) {
        setConfigurationError(`策略已保存，但状态刷新失败：${errorMessage(reason)}`);
      }
    } catch (reason: unknown) {
      setFailurePolicy(previousPolicy);
      setConfigurationError(`失败策略保存失败：${errorMessage(reason)}`);
    } finally {
      setPolicySaving(false);
    }
  }

  async function handleRestore(snapshot: SnapshotSummary) {
    if (!window.confirm(`恢复 Snapshot ${snapshot.createdAt}？当前相关资源会先自动备份。`)) {
      return;
    }
    setOperation("restore");
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      await restoreSnapshot(snapshot.id);
      setConfigurationSuccess("Snapshot 已恢复；如需恢复当前模式，请重新同步一次。");
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConfigurationError(errorMessage(reason));
      await reloadConfiguration();
    } finally {
      setOperation(null);
    }
  }

  const orchestratableAgents = agents.filter((agent) => agent.roleKey && agent.orchestrationPhase);
  const roleGroups = Object.entries(
    orchestratableAgents.reduce<Record<string, AgentSummary[]>>((groups, agent) => {
      const roleKey = agent.roleKey as string;
      (groups[roleKey] ??= []).push(agent);
      return groups;
    }, {}),
  ).sort(([leftRole, leftAgents], [rightRole, rightAgents]) => {
    const phaseOrder: Record<OrchestrationPhase, number> = {
      DISCOVERY: 1,
      EXECUTION: 2,
      VERIFICATION: 3,
      REVIEW: 4,
    };
    const leftPhase = leftAgents[0]?.orchestrationPhase ?? "REVIEW";
    const rightPhase = rightAgents[0]?.orchestrationPhase ?? "REVIEW";
    return phaseOrder[leftPhase] - phaseOrder[rightPhase] || leftRole.localeCompare(rightRole);
  });
  const configuredAgentIds = Object.values(selectedAgentsByRole).filter(Boolean).sort();
  const targetAgentIds = selectedMode === "SUBAGENT" ? configuredAgentIds : [];
  const currentAgentIds = runtimeMode?.activeBindings.map((binding) => binding.agentId).sort() ?? [];
  const defaultIsCurrent = runtimeMode?.activeBindings.length === 0 && !runtimeMode.legacyActiveAgentId;
  const sameMode = selectedMode === "DEFAULT"
    ? defaultIsCurrent
    : targetAgentIds.length > 0
      && !runtimeMode?.legacyActiveAgentId
      && targetAgentIds.length === currentAgentIds.length
      && targetAgentIds.every((id, index) => id === currentAgentIds[index]);
  const selectedAgents = configuredAgentIds
    .map((id) => agents.find((agent) => agent.id === id))
    .filter((agent): agent is AgentSummary => Boolean(agent));
  const modeReady = selectedMode === "DEFAULT"
    || (selectedAgents.length > 0 && selectedAgents.every((agent) => agent.availability === "READY"));
  const alreadySynchronized = sameMode && configuration?.status === "APPLIED";
  const runtimeUsesSubagents = currentAgentIds.length > 0 || Boolean(runtimeMode?.legacyActiveAgentId);
  const restartPending = runtimeUsesSubagents && Boolean(configuration?.restartRecommended);
  const hasExecutionAgent = selectedAgents.some((agent) => agent.orchestrationPhase === "EXECUTION");
  const visibleSelectedAgents = selectedAgents.slice(0, 2);
  const hiddenSelectedAgents = selectedAgents.slice(2);
  const latestSnapshot = snapshots[0];
  return (
    <>
      {configurationConflict && (
        <ConfigurationConflictDialog
          busy={conflictBusy}
          conflict={configurationConflict}
          error={conflictError}
          onAdopt={() => void handleConflictResolution("ADOPT")}
          onClose={() => {
            if (conflictBusy) return;
            setConfigurationConflict(null);
            setConflictActiveAgentIds([]);
            setConflictError(null);
          }}
          onReplace={() => void handleConflictResolution("REPLACE")}
        />
      )}
      {environment && (
        <EnvironmentDetails
          detecting={detecting}
          environment={environment}
          onRedetect={onRedetect}
        />
      )}

      <header className="overview-heading">
        <div>
          <h1>选择 Codex 的工作方式</h1>
        </div>
      </header>

      {error && (
        <section className="notice error">
          <strong>后端连接失败</strong>
          <p>{error}</p>
        </section>
      )}

      <section className="configuration-card runtime-mode-card">
        <div className="configuration-heading">
          <div className="overview-mode-title">
            <h2>运行模式</h2>
            <InfoTip label="切换前自动创建 Snapshot；同步失败时回滚 CAS-owned 配置。" />
          </div>
          <span className={`mode-status ${
            restartPending
              ? "restart-required"
              : configuration?.status.toLowerCase() ?? "unavailable"
          }`}>
            {restartPending
              ? "RESTART REQUIRED"
              : configuration?.status ?? "LOADING"}
          </span>
        </div>

        {configurationSuccess && <div className="success-banner">{configurationSuccess}</div>}
        {configurationError && <div className="inline-error">{configurationError}</div>}
        {restartPending && (
          <div className="inline-error" role="alert">
            <strong>Codex 尚未加载最新配置</strong>
            <span>
              检测到运行中的 Codex 早于最近一次 CAS 配置同步。请完全退出 Codex，再重新启动并创建全新任务；继续使用当前任务仍会沿用旧的 Multi-Agent 运行时。
            </span>
          </div>
        )}
        <div className="runtime-mode-options" role="radiogroup" aria-label="Codex 运行模式">
          <div className={`runtime-mode-option ${selectedMode === "DEFAULT" ? "selected" : ""}`}>
            <input
              checked={selectedMode === "DEFAULT"}
              id="runtime-mode-default"
              name="runtime-mode"
              onChange={() => setSelectedMode("DEFAULT")}
              type="radio"
            />
            <div className="runtime-mode-copy">
              <div className="runtime-mode-title">
                <label htmlFor="runtime-mode-default"><strong>Default</strong></label>
                <InfoTip label="CAS 不写入子 Agent 编排配置；Codex 主模型、MCP 与其他外部配置保持不变。" />
              </div>
              <small>Codex 全权负责，不启用 CAS 子 Agent。</small>
            </div>
            {defaultIsCurrent && <span className="current-mode-tag">当前</span>}
          </div>

          <div className={`runtime-mode-option ${selectedMode === "SUBAGENT" ? "selected" : ""}`}>
            <input
              checked={selectedMode === "SUBAGENT"}
              id="runtime-mode-subagent"
              name="runtime-mode"
              onChange={() => setSelectedMode("SUBAGENT")}
              type="radio"
            />
            <div className="runtime-mode-copy">
              <div className="runtime-mode-title">
                <label htmlFor="runtime-mode-subagent"><strong>使用子 Agent</strong></label>
                <InfoTip
                  label={`不同 Role 可以同时运行，同一 Role 只能选择一个 Agent。Primary 负责规划、审查与收束；子 Agent 继承 Primary 的实时权限。切换后需完全退出并重启 Codex，再新建任务，并保持 Auto 或 Workspace。当前失败策略：${failurePolicy === "STRICT_STOP" ? "Strict Stop，委派失败后停止" : "Primary Fallback，警告后由 Primary 接管"}。`}
                />
              </div>
              <small>按 Role 配置 Agent；同一 Role 只启用一个。</small>
              <div className="selected-agent-summary" aria-label="已配置的子 Agent">
                {selectedAgents.length === 0 ? (
                  <small>尚未配置子 Agent。</small>
                ) : (
                  <>
                    <strong className="selected-agent-count">已配置 {selectedAgents.length} 个</strong>
                    {visibleSelectedAgents.map((agent) => (
                      <span key={agent.id}>
                        {agent.model?.providerKey ?? "未绑定供应商"} / {agent.name}
                      </span>
                    ))}
                    {hiddenSelectedAgents.length > 0 && (
                      <Tooltip
                        content={hiddenSelectedAgents.map((agent) =>
                          `${agent.model?.providerKey ?? "未绑定供应商"} / ${agent.name}`
                        ).join(" · ")}
                        focusable
                        label={`另外 ${hiddenSelectedAgents.length} 个 Agent`}
                      >
                        <span>+{hiddenSelectedAgents.length}</span>
                      </Tooltip>
                    )}
                  </>
                )}
              </div>
              <button
                className="secondary-button configure-agent-button"
                onClick={() => {
                  setSelectedMode("SUBAGENT");
                  setAgentConfigOpen(true);
                }}
                type="button"
              >
                配置子 Agent
              </button>
              {selectedMode === "SUBAGENT" && selectedAgents.some((agent) => agent.availability !== "READY") && (
                <em>存在尚未就绪的 Agent，请先在 Agents 页面完善 Model 与 Provider。</em>
              )}
              {selectedMode === "SUBAGENT" && selectedAgents.length > 0 && !hasExecutionAgent && (
                <em>
                  未启用 EXECUTION Agent：
                  {failurePolicy === "STRICT_STOP"
                    ? "Strict Stop 会阻止所有写入任务。"
                    : "Primary Fallback 会警告后由 Primary 接管。"}
                </em>
              )}
            </div>
            {runtimeMode && (runtimeMode.activeBindings.length > 0 || runtimeMode.legacyActiveAgentId) && (
              <span className="current-mode-tag">当前：{runtimeMode.activeBindings.length || 1} 个</span>
            )}
          </div>
        </div>

        <div className="failure-policy-panel">
          <label htmlFor="overview-failure-policy">
            <span className="failure-policy-label">
              <strong>失败策略</strong>
              <InfoTip label="Strict Stop 在委派失败后停止；Primary Fallback 会先显式警告，再由 Primary 接管。" />
            </span>
            <select
              disabled={policySaving}
              id="overview-failure-policy"
              onChange={(event) => void handleFailurePolicyChange(
                event.target.value as SettingsResponse["orchestrationFailurePolicy"],
              )}
              value={failurePolicy}
            >
              <option value="STRICT_STOP">Strict Stop（推荐）</option>
              <option value="PRIMARY_FALLBACK">Primary Fallback</option>
            </select>
          </label>
          {policySaving && <small role="status">正在保存失败策略…</small>}
          {failurePolicy === "PRIMARY_FALLBACK" && (
            <div className="orchestration-warning" role="alert">
              Primary 与子 Agent 都需要 Auto 或 Workspace 权限。该回退依赖编排指令约束，
              不是文件权限隔离。
            </div>
          )}
        </div>

        {configuration && configuration.issues.length > 0 && (
          <ul className="configuration-issues">
            {configuration.issues.map((issue) => (
              <li key={`${issue.code}-${issue.message}`}>
                <strong>{issue.code}</strong>
                <span>{issue.message}</span>
              </li>
            ))}
          </ul>
        )}

        <div className="mode-switch-actions">
          <span>
            {restartPending && alreadySynchronized
              ? "磁盘配置已同步，但运行中的 Codex 仍在使用旧配置。"
              : alreadySynchronized
                ? "当前模式已同步到 Codex。"
                : sameMode
                  ? "当前定义或磁盘配置有变化，可重新同步。"
                  : "确认后将立即切换，并只处理 CAS 拥有的配置。"}
          </span>
          <div className="mode-action-buttons">
            <button
              className="secondary-button"
              disabled={refreshingMode || operation === "restore"}
              onClick={handleModeRefresh}
              type="button"
            >
              {refreshingMode ? "刷新中…" : "刷新状态"}
            </button>
            <button
              aria-busy={operation === "switch"}
              className="primary-button"
              disabled={operation !== null || refreshingMode || !runtimeMode || !modeReady || alreadySynchronized}
              onClick={handleModeSwitch}
              type="button"
            >
              {operation === "switch"
                ? "切换中…"
                : restartPending && alreadySynchronized
                  ? "等待重启 Codex"
                  : alreadySynchronized
                    ? "当前已启用"
                    : sameMode
                      ? "同步当前模式"
                      : "切换模式"}
            </button>
          </div>
        </div>
      </section>

      {agentConfigOpen && (
        <AgentConfigurationDialog
          failurePolicy={failurePolicy}
          hasExecutionAgent={hasExecutionAgent}
          onClose={() => setAgentConfigOpen(false)}
          onSelect={(roleKey, agentId) => setSelectedAgentsByRole((current) => ({
            ...current,
            [roleKey]: agentId,
          }))}
          roleGroups={roleGroups}
          selectedAgents={selectedAgents}
          selectedAgentsByRole={selectedAgentsByRole}
        />
      )}

      {!latestSnapshot ? (
        <section className="snapshot-strip snapshot-empty">
          <strong>最近备份</strong>
          <small>首次切换模式前会自动生成 Snapshot。</small>
        </section>
      ) : (
        <details className="snapshot-strip snapshot-disclosure">
          <summary>
            <span className="snapshot-summary-copy">
              <strong>最近备份</strong>
              <small>{latestSnapshot.createdAt} · {latestSnapshot.resourceCount} files</small>
            </span>
            <span className="snapshot-summary-state">
              查看 {snapshots.length} 条
              <UiIcon name="chevron-down" />
            </span>
          </summary>
          <div className="snapshot-list">
            {snapshots.map((snapshot) => (
              <div className="snapshot-row" key={snapshot.id}>
                <div>
                  <strong>{snapshot.reason}</strong>
                  <small>{snapshot.createdAt} · {snapshot.resourceCount} files</small>
                </div>
                <button
                  className="secondary-button"
                  disabled={operation !== null || snapshot.status !== "READY"}
                  onClick={() => handleRestore(snapshot)}
                  type="button"
                >
                  恢复
                </button>
              </div>
            ))}
          </div>
        </details>
      )}
    </>
  );
}

function ConfigurationConflictDialog({
  busy,
  conflict,
  error,
  onAdopt,
  onClose,
  onReplace,
}: {
  busy: boolean;
  conflict: ConfigurationConflictResponse;
  error: string | null;
  onAdopt: () => void;
  onClose: () => void;
  onReplace: () => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const externallyModified = conflict.resources.some(
    (resource) => resource.code === "MANAGED_RESOURCE_CONFLICT",
  );

  useEffect(() => {
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  return (
    <dialog
      aria-labelledby="configuration-conflict-title"
      className="configuration-conflict-dialog"
      onCancel={(event) => {
        if (busy) event.preventDefault();
        else onClose();
      }}
      onClick={(event) => {
        if (!busy && event.target === event.currentTarget) onClose();
      }}
      ref={dialogRef}
    >
      <section className="configuration-conflict-card">
        <header>
          <div>
            <span className="eyebrow">Configuration Safety</span>
            <h2 id="configuration-conflict-title">
              {externallyModified ? "CAS 管理的配置已被外部修改" : "发现已有 Codex 配置"}
            </h2>
            <p>
              {externallyModified
                ? "CAS 检测到已管理资源在外部发生变化。同步已暂停，磁盘内容尚未改变。"
                : "当前 CODEX_HOME 已存在 CAS 准备管理的配置，但本机 CAS 尚未登记这些资源的所有权。同步已暂停，磁盘内容尚未改变。"}
            </p>
          </div>
          <IconButton
            disabled={busy}
            icon="close"
            label="取消同步并关闭"
            onClick={onClose}
          />
        </header>

        <div className="conflict-home">
          <span>CODEX_HOME</span>
          <div className="copyable-code">
            <Tooltip content={conflict.codexHome} focusable label={`CODEX_HOME：${conflict.codexHome}`}>
              <code>{conflict.codexHome}</code>
            </Tooltip>
            <CopyIconButton label="复制 CODEX_HOME" value={conflict.codexHome} />
          </div>
        </div>

        <div className="configuration-conflict-list">
          {conflict.resources.map((resource) => (
            <article key={`${resource.resourceType}-${resource.logicalKey}`}>
              <div>
                <strong>{resource.logicalKey}</strong>
                <span className="conflict-kind">
                  {resource.code === "RESOURCE_OWNERSHIP_CONFLICT"
                    ? "尚未登记所有权"
                    : "已被外部修改"}
                </span>
              </div>
              <small>{resource.resourceType}</small>
              <div className="copyable-code">
                <Tooltip content={resource.path} focusable label={`资源路径：${resource.path}`}>
                  <code>{resource.path}</code>
                </Tooltip>
                <CopyIconButton label={`复制 ${resource.logicalKey} 路径`} value={resource.path} />
              </div>
              <p>
                {resource.matchesDesired
                  ? "磁盘语义与 CAS 当前期望一致。"
                  : "磁盘语义与 CAS 当前期望不同，不能直接接管。"}
                {!resource.replaceable && " 此资源不能由 CAS 自动替换。"}
              </p>
            </article>
          ))}
        </div>

        {!conflict.canAdopt && (
          <div className="orchestration-warning" role="note">
            “接管一致配置”仅在所有资源均未登记、且与当前 CAS 期望完全一致时可用。
            这不会把任意磁盘配置反向导入 CAS。
          </div>
        )}
        {error && <div className="inline-error" role="alert">{error}</div>}

        <footer>
          <button className="secondary-button" disabled={busy} onClick={onClose} type="button">
            取消同步
          </button>
          <Tooltip
            content={conflict.canAdopt ? "接管与 CAS 期望一致的现有配置" : "现有配置与 CAS 期望不完全一致"}
            focusable={!conflict.canAdopt}
            label="接管一致配置说明"
          >
            <button
              className="secondary-button"
              disabled={busy || !conflict.canAdopt}
              onClick={onAdopt}
              type="button"
            >
              {busy ? "处理中…" : "接管一致配置"}
            </button>
          </Tooltip>
          <button
            className="danger-button"
            disabled={busy || conflict.resources.some((resource) => !resource.replaceable)}
            onClick={onReplace}
            type="button"
          >
            {busy ? "处理中…" : "备份并替换"}
          </button>
        </footer>
      </section>
    </dialog>
  );
}

function AgentConfigurationDialog({
  failurePolicy,
  hasExecutionAgent,
  onClose,
  onSelect,
  roleGroups,
  selectedAgents,
  selectedAgentsByRole,
}: {
  failurePolicy: SettingsResponse["orchestrationFailurePolicy"];
  hasExecutionAgent: boolean;
  onClose: () => void;
  onSelect: (roleKey: string, agentId: string) => void;
  roleGroups: Array<[string, AgentSummary[]]>;
  selectedAgents: AgentSummary[];
  selectedAgentsByRole: Record<string, string>;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  return (
    <dialog
      aria-labelledby="agent-config-title"
      className="agent-config-dialog"
      onCancel={onClose}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      ref={dialogRef}
    >
      <section className="agent-config-card">
        <header>
          <div>
            <span className="eyebrow">Subagents</span>
            <h2 id="agent-config-title">配置子 Agent</h2>
            <p>不同 Role 可以同时运行；同一 Role 只能启用一个 Agent。</p>
          </div>
          <IconButton icon="close" label="关闭子 Agent 配置" onClick={onClose} />
        </header>

        <div className="role-agent-selectors">
          {roleGroups.length === 0 && (
            <div className="empty-copy">请先在 Agents 页面创建带 Role 与 Phase 的 Agent。</div>
          )}
          {roleGroups.map(([roleKey, candidates]) => {
            const selected = candidates.find((agent) => agent.id === selectedAgentsByRole[roleKey]);
            return (
              <label className="role-agent-selector" key={roleKey}>
                <span>
                  <strong>{roleKey}</strong>
                  <small>{selected?.orchestrationPhase ?? candidates[0]?.orchestrationPhase}</small>
                </span>
                <select
                  aria-label={`选择 ${roleKey} Agent`}
                  onChange={(event) => onSelect(roleKey, event.target.value)}
                  value={selectedAgentsByRole[roleKey] ?? ""}
                >
                  <option value="">不启用</option>
                  {candidates.map((agent) => (
                    <option key={agent.id} value={agent.id}>
                      {agent.model?.providerKey ?? "未绑定供应商"} / {agent.name}（{agent.agentKey}）
                      {" · "}{availabilityLabel(agent.availability)}
                    </option>
                  ))}
                </select>
              </label>
            );
          })}
        </div>

        <div className="agent-config-feedback">
          {selectedAgents.some((agent) => agent.availability !== "READY") && (
            <em>存在尚未就绪的 Agent，请先在 Agents 页面完善 Model 与 Provider。</em>
          )}
          {selectedAgents.length > 0 && !hasExecutionAgent && (
            <em>
              未启用 EXECUTION Agent：
              {failurePolicy === "STRICT_STOP"
                ? "Strict Stop 会阻止所有写入任务。"
                : "Primary Fallback 会警告后由 Primary 接管。"}
            </em>
          )}
        </div>

        <footer>
          <small>关闭窗口后，点击“切换模式”或“同步当前模式”写入 Codex 配置。</small>
          <button className="primary-button" onClick={onClose} type="button">完成</button>
        </footer>
      </section>
    </dialog>
  );
}

function DiagnosticsPage() {
  const [result, setResult] = useState<DiagnosticsResponse | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run() {
    setRunning(true);
    setError(null);
    try {
      setResult(await runDiagnostics(false));
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setRunning(false);
    }
  }

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Diagnostics</span>
          <h1>检查 CAS 与 Codex 状态</h1>
          <p>只读检查环境、SQLite、配置投影、Credential 引用和 Agent 绑定。</p>
        </div>
        <button className="primary-button" disabled={running} onClick={run} type="button">
          {running ? "检查中…" : "运行诊断"}
        </button>
      </header>

      {error && <div className="inline-error">{error}</div>}

      {!result && !error && <section className="notice">诊断不会修改数据库或 Codex 文件。</section>}

      {result && (
        <section className="diagnostics-result">
          <div className="diagnostics-summary">
            <StatusBadge
              className={`result ${result.overall.toLowerCase()}`}
              description={
                result.overall === "HEALTHY"
                  ? "全部只读检查均未发现需要处理的问题。"
                  : result.overall === "WARNING"
                    ? "存在不会立即阻断 CAS、但建议检查的项目。"
                    : "存在会影响 CAS 或 Codex 配置正常工作的错误。"
              }
              label={result.overall === "HEALTHY" ? "正常" : result.overall === "WARNING" ? "需注意" : "错误"}
            />
            <small>{result.checkedAt}</small>
          </div>
          {result.sections.map((section) => (
            <article className="diagnostic-section" key={section.key}>
              <h2>{section.title}</h2>
              <ul>
                {section.issues.map((issue) => {
                  const firstLine = issue.message.split(/\r?\n/, 1)[0];
                  const hasDetails = issue.message.length > 140 || firstLine !== issue.message;
                  const preview = firstLine.length > 140 ? `${firstLine.slice(0, 137)}…` : firstLine;
                  const severityLabel = issue.severity === "ERROR" ? "错误" : issue.severity === "WARNING" ? "警告" : "信息";
                  const severityIcon: IconName = issue.severity === "ERROR" ? "x-circle" : issue.severity === "WARNING" ? "alert" : "info";
                  return (
                    <li key={`${issue.code}-${issue.message}`}>
                      <span className={`diagnostic-icon ${issue.severity.toLowerCase()}`}>
                        <UiIcon name={severityIcon} />
                        <span className="sr-only">{severityLabel}</span>
                      </span>
                      <div className="diagnostic-issue-copy">
                        <strong>{preview}</strong>
                        <span className="diagnostic-issue-meta">
                          <code>{issue.code}</code>
                          <CopyIconButton label={`复制诊断代码 ${issue.code}`} value={issue.code} />
                        </span>
                        {hasDetails && (
                          <details className="diagnostic-details">
                            <summary>查看完整说明</summary>
                            <p>{issue.message}</p>
                          </details>
                        )}
                      </div>
                    </li>
                  );
                })}
              </ul>
            </article>
          ))}
        </section>
      )}
    </>
  );
}

function SettingsPage({
  onAppearanceChange,
  onEnvironmentChange,
  onFontFamilyChange,
}: {
  onAppearanceChange: (appearance: Appearance) => void;
  onEnvironmentChange: (environment: CodexEnvironmentResponse) => void;
  onFontFamilyChange: (fontFamily: string | null) => void;
}) {
  const [settings, setSettings] = useState<SettingsResponse | null>(null);
  const [environment, setEnvironment] = useState<CodexEnvironmentResponse | null>(null);
  const [customCodexHome, setCustomCodexHome] = useState("");
  const [customFontFamily, setCustomFontFamily] = useState("");
  const [projectExclusions, setProjectExclusions] = useState<ProjectExclusion[]>([]);
  const [projectPath, setProjectPath] = useState("");
  const [projectPathError, setProjectPathError] = useState<string | null>(null);
  const [exclusionError, setExclusionError] = useState<string | null>(null);
  const [exclusionBusy, setExclusionBusy] = useState<string | null>(null);
  const [copyStatus, setCopyStatus] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([getSettings(), getCodexEnvironment(), listProjectExclusions()])
      .then(([loadedSettings, loadedEnvironment, loadedExclusions]) => {
        setSettings(loadedSettings);
        setCustomCodexHome(loadedSettings.customCodexHome ?? "");
        setCustomFontFamily(loadedSettings.customFontFamily ?? "");
        setEnvironment(loadedEnvironment);
        setProjectExclusions(loadedExclusions);
      })
      .catch((reason: unknown) => setError(errorMessage(reason)));
  }, []);

  async function save(event: FormEvent) {
    event.preventDefault();
    if (!settings) return;
    setSaving(true);
    setError(null);
    setSuccess(null);
    try {
      const updated = await updateSettings({
        appearance: settings.appearance,
        autoBackupEnabled: true,
        updateChannel: settings.updateChannel,
        customCodexHome: customCodexHome.trim() || null,
        customFontFamily: customFontFamily.trim() || null,
        orchestrationFailurePolicy: settings.orchestrationFailurePolicy,
      });
      setSettings(updated);
      setCustomCodexHome(updated.customCodexHome ?? "");
      setCustomFontFamily(updated.customFontFamily ?? "");
      const updatedEnvironment = await getCodexEnvironment();
      setEnvironment(updatedEnvironment);
      onEnvironmentChange(updatedEnvironment);
      onAppearanceChange(updated.appearance);
      onFontFamilyChange(updated.customFontFamily);
      setSuccess("设置已保存。");
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  async function redetect() {
    setDetecting(true);
    setError(null);
    try {
      const updatedEnvironment = await redetectCodex();
      setEnvironment(updatedEnvironment);
      onEnvironmentChange(updatedEnvironment);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setDetecting(false);
    }
  }

  async function addExclusion() {
    const value = projectPath.trim();
    setProjectPathError(null);
    setExclusionError(null);
    if (!value) {
      setProjectPathError("请输入需要排除的项目绝对路径。");
      return;
    }
    setExclusionBusy("add");
    try {
      const added = await addProjectExclusion(value);
      setProjectExclusions((current) =>
        [...current, added].sort((left, right) =>
          left.projectPath.localeCompare(right.projectPath),
        ),
      );
      setProjectPath("");
      setSuccess("项目排除已添加；请在该项目中新建 Codex 会话。");
    } catch (reason: unknown) {
      if (errorField(reason) === "projectPath") {
        setProjectPathError(errorMessage(reason));
      } else {
        setExclusionError(errorMessage(reason));
      }
    } finally {
      setExclusionBusy(null);
    }
  }

  async function removeExclusion(exclusion: ProjectExclusion) {
    setExclusionBusy(exclusion.id);
    setExclusionError(null);
    try {
      await deleteProjectExclusion(exclusion.id);
      setProjectExclusions((current) =>
        current.filter((item) => item.id !== exclusion.id),
      );
      setSuccess("项目排除已移除，CAS 接管的权限字段已安全恢复。");
    } catch (reason: unknown) {
      setExclusionError(errorMessage(reason));
    } finally {
      setExclusionBusy(null);
    }
  }

  async function copyConversationMarker(marker: "CAS:OFF" | "CAS:ON") {
    try {
      await navigator.clipboard.writeText(marker);
      setCopyStatus(`已复制 ${marker}`);
    } catch {
      setCopyStatus("复制失败，请手动选择文本。");
    }
  }

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Settings</span>
          <h1>CAS 设置</h1>
          <p>管理 CAS 自身行为；Provider、Model 和 Agent 仍在各自页面配置。</p>
        </div>
      </header>

      {error && <div className="inline-error settings-message">{error}</div>}
      {success && <div className="success-banner">{success}</div>}
      {!settings ? (
        !error && <section className="notice">正在读取设置…</section>
      ) : (
        <form className="settings-form" onSubmit={save}>
          <section className="settings-card">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">Codex</span>
                <h2>Codex Installation</h2>
                <p>默认使用环境变量或用户目录；高级用户可覆盖 CODEX_HOME。</p>
              </div>
              <button className="secondary-button" disabled={detecting} onClick={redetect} type="button">
                {detecting ? "检测中…" : "重新检测"}
              </button>
            </div>
            {environment && (
              <dl className="settings-environment">
                {[
                  { label: "Executable", value: environment.executablePath, fallback: "未检测到" },
                  { label: "CODEX_HOME", value: environment.codexHome, fallback: "未定位" },
                  { label: "Version", value: environment.version, fallback: "未知" },
                ].map(({ label, value, fallback }) => (
                  <div key={label}>
                    <dt>{label}</dt>
                    <dd className="settings-environment-value">
                      {value ? (
                        <>
                          <Tooltip content={value} focusable label={`${label}：${value}`}>
                            <code>{value}</code>
                          </Tooltip>
                          <CopyIconButton label={`复制 ${label}`} value={value} />
                        </>
                      ) : (
                        <span>{fallback}</span>
                      )}
                    </dd>
                  </div>
                ))}
              </dl>
            )}
            <label className="field">
              <span className="field-label-with-info">
                自定义 CODEX_HOME
                <InfoTip label="填写已经存在的绝对目录；留空会恢复为环境变量或用户目录自动检测。" />
              </span>
              <input
                onChange={(event) => setCustomCodexHome(event.target.value)}
                placeholder="留空以自动检测"
                value={customCodexHome}
              />
            </label>
          </section>

          <section className="settings-card settings-grid">
            <label className="field">
              <span className="field-label-with-info">
                Appearance
                <InfoTip label="控制 CAS 界面主题；跟随系统会响应 Windows 的浅色或深色设置。" />
              </span>
              <select
                onChange={(event) => {
                  const value = event.target.value as Appearance;
                  setSettings({ ...settings, appearance: value });
                  onAppearanceChange(value);
                }}
                value={settings.appearance}
              >
                <option value="SYSTEM">跟随系统</option>
                <option value="LIGHT">浅色</option>
                <option value="DARK">深色</option>
              </select>
            </label>
            <label className="field">
              <span className="field-label-with-info">
                Update Channel
                <InfoTip label="Stable 优先稳定版本；Beta 可接收测试版本。" />
              </span>
              <select
                onChange={(event) => setSettings({ ...settings, updateChannel: event.target.value })}
                value={settings.updateChannel}
              >
                <option value="STABLE">Stable</option>
                <option value="BETA">Beta</option>
              </select>
            </label>
            <label className="field full-width">
              <span className="field-label-with-info">
                界面字体
                <InfoTip label="输入系统中已安装字体的准确名称；找不到时自动回退到系统默认字体。" />
              </span>
              <input
                maxLength={160}
                onChange={(event) => setCustomFontFamily(event.target.value)}
                placeholder="留空使用系统默认字体"
                value={customFontFamily}
              />
            </label>
            <label className="enabled-field full-width">
              <input checked disabled type="checkbox" />
              <span className="field-label-with-info">
                模式切换前自动备份
                <InfoTip label="当前版本固定启用：切换失败时可恢复 CAS 接管的配置资源。" />
              </span>
            </label>
          </section>

          <section className="settings-card">
            <div className="panel-heading">
              <div>
                <span className="eyebrow">Orchestration</span>
                <h2>编排排除</h2>
                <p>让指定项目或当前对话暂时不使用 CAS 子 Agent。</p>
              </div>
            </div>

            <div className="project-exclusion-add">
              <label className="field">
                <span className="field-label-with-info">
                  项目绝对路径
                  <InfoTip label="CAS 会保留式修改该项目的 .codex/config.toml，并在移除排除项时恢复由 CAS 接管的字段。" />
                </span>
                <input
                  aria-invalid={Boolean(projectPathError)}
                  onChange={(event) => {
                    setProjectPath(event.target.value);
                    setProjectPathError(null);
                  }}
                  placeholder="例如 C:\Projects\standalone-app"
                  value={projectPath}
                />
                {projectPathError ? (
                  <small className="field-error">{projectPathError}</small>
                ) : (
                  <small>排除仅对该目录及其子目录中的新会话生效。</small>
                )}
              </label>
              <button
                className="secondary-button"
                disabled={exclusionBusy !== null}
                onClick={addExclusion}
                type="button"
              >
                {exclusionBusy === "add" ? "添加中…" : "添加项目"}
              </button>
            </div>

            <div className="orchestration-warning">
              项目排除仅对新 Codex 会话生效，且项目必须已被 Codex 标记为 trusted。
              若没有生效，请先检查项目信任状态。排除期间 Primary 使用
              Workspace 可写权限，并关闭 multi-agent。
            </div>

            <div className="project-exclusion-list" aria-label="已排除项目">
              {projectExclusions.length === 0 ? (
                <p className="empty-copy">当前没有项目排除项。</p>
              ) : (
                projectExclusions.map((exclusion) => (
                  <div className="project-exclusion-entry" key={exclusion.id}>
                    <span className="project-exclusion-copy">
                      <span className="project-exclusion-path">
                        <Tooltip content={exclusion.projectPath} focusable label={`排除路径：${exclusion.projectPath}`}>
                          <strong>{exclusion.projectPath}</strong>
                        </Tooltip>
                        <CopyIconButton label="复制排除路径" value={exclusion.projectPath} />
                      </span>
                      <small>仅匹配该目录及其子目录</small>
                    </span>
                    <IconButton
                      disabled={exclusionBusy !== null}
                      icon="trash"
                      label={exclusionBusy === exclusion.id ? "正在移除项目排除" : `移除项目排除 ${exclusion.projectPath}`}
                      loading={exclusionBusy === exclusion.id}
                      onClick={() => removeExclusion(exclusion)}
                      tone="danger"
                    />
                  </div>
                ))
              )}
            </div>
            {exclusionError && <div className="inline-error">{exclusionError}</div>}

            <div className="conversation-control">
              <div>
                <strong>当前对话临时排除</strong>
                <p>
                  CAS:OFF 与 CAS:ON 只切换编排规则，不改变当前权限。子 Agent 写入要求
                  <code>/permissions</code>保持 Auto 或 Workspace。
                </p>
              </div>
              <div className="conversation-command-grid">
                <article>
                  <code>CAS:OFF</code>
                  <span>发送后运行 /permissions，选择 Auto 或 Workspace。</span>
                  <IconButton
                    icon="copy"
                    label="复制 CAS:OFF"
                    onClick={() => void copyConversationMarker("CAS:OFF")}
                  />
                </article>
                <article>
                  <code>CAS:ON</code>
                  <span>恢复编排后继续保持 Auto 或 Workspace。</span>
                  <IconButton
                    icon="copy"
                    label="复制 CAS:ON"
                    onClick={() => void copyConversationMarker("CAS:ON")}
                  />
                </article>
              </div>
              {copyStatus && <small role="status">{copyStatus}</small>}
            </div>
          </section>

          <div className="form-actions">
            <button className="primary-button" disabled={saving} type="submit">
              {saving ? "保存中…" : "保存设置"}
            </button>
          </div>
        </form>
      )}
    </>
  );
}

function AgentsPage() {
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [activeAgentIds, setActiveAgentIds] = useState<string[]>([]);
  const [models, setModels] = useState<ModelSummary[]>([]);
  const [presets, setPresets] = useState<AgentPresetResponse[]>([]);
  const [loading, setLoading] = useState(true);
  const [creating, setCreating] = useState(false);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [agentList, modelList, presetList, mode] = await Promise.all([
        listAgents(),
        listModels({ enabled: true }),
        listAgentPresets(),
        getRuntimeMode(),
      ]);
      setAgents(agentList);
      setModels(modelList);
      setPresets(presetList);
      setActiveAgentIds([
        ...mode.activeBindings.map((binding) => binding.agentId),
        ...(mode.legacyActiveAgentId ? [mode.legacyActiveAgentId] : []),
      ]);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  function changed(message: string) {
    setSuccess(message);
    void load();
  }

  const panelOpen = creating || selectedId !== null;

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Agents</span>
          <h1>Codex 子 Agent</h1>
          <p>角色只描述职责；Provider 通过所绑定的 Model 间接确定。</p>
        </div>
        {!panelOpen && (
          <button
            className="primary-button"
            disabled={loading}
            onClick={() => {
              setCreating(true);
              setSuccess(null);
            }}
          >
            创建 Agent
          </button>
        )}
      </header>

      {success && <div className="success-banner" role="status">{success}</div>}

      {creating && (
        <CreateAgentPanel
          models={models}
          onCancel={() => setCreating(false)}
          onCreated={() => {
            setCreating(false);
            changed("Agent 已创建；绑定与 Capability 要求已原子保存。");
          }}
          presets={presets}
        />
      )}

      {selectedId && (
        <AgentDetailPanel
          agentId={selectedId}
          isActive={activeAgentIds.includes(selectedId)}
          models={models}
          onBack={() => setSelectedId(null)}
          onChanged={changed}
          onDeleted={() => {
            setSelectedId(null);
            changed("Agent 已删除。");
          }}
        />
      )}

      {!panelOpen && error && (
        <section className="notice error provider-notice">
          <strong>无法读取 Agent</strong>
          <p>{error}</p>
          <button className="secondary-button" onClick={() => void load()}>重试</button>
        </section>
      )}

      {!panelOpen && loading && <section className="notice provider-notice">正在读取 Agent…</section>}

      {!panelOpen && !loading && !error && agents.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">A</div>
          <h2>还没有 Agent</h2>
          <p>从 Executor、Explorer、Reviewer、Tester 模板开始，或创建自定义角色。</p>
          <button className="primary-button" onClick={() => setCreating(true)}>创建 Agent</button>
        </section>
      )}

      {!panelOpen && !loading && !error && agents.length > 0 && (
        <section className="agent-list" aria-label="Agent 列表">
          {agents.map((agent) => (
            <AgentRow
              agent={agent}
              isActive={activeAgentIds.includes(agent.id)}
              key={agent.id}
              onOpen={setSelectedId}
            />
          ))}
        </section>
      )}
    </>
  );
}

function AgentRow({
  agent,
  isActive,
  onOpen,
}: {
  agent: AgentSummary;
  isActive: boolean;
  onOpen: (id: string) => void;
}) {
  const bindingSummary = agent.model
    ? `${agent.model.providerKey} / ${agent.model.displayName}`
    : "未绑定供应商 / 模型";
  const orchestrationSummary = agent.roleKey && agent.orchestrationPhase
    ? `${agent.roleKey} / ${agent.orchestrationPhase}`
    : "未分类";
  return (
    <article className="agent-row">
      <div className="agent-main">
        <div className="provider-name-line">
          <h2>{agent.name}</h2>
          <StatusBadge
            className={`agent-state ${agent.availability.toLowerCase()}`}
            description={availabilityDescription(agent.availability)}
            icon={agent.availability === "READY" ? "check" : agent.availability === "INCOMPATIBLE_MODEL" ? "x-circle" : "alert"}
            label={availabilityLabel(agent.availability)}
          />
          {isActive && <span className="agent-state current"><UiIcon name="check" />当前使用</span>}
        </div>
        <Tooltip content={agent.description} focusable label={`Agent 描述：${agent.description}`}>
          <p>{agent.description}</p>
        </Tooltip>
        <span className="agent-technical-line">
          <Tooltip
            content={`${bindingSummary} · ${orchestrationSummary}`}
            focusable
            label={`Agent 绑定：${bindingSummary}；${orchestrationSummary}`}
          >
            <code>{bindingSummary} · {orchestrationSummary}</code>
          </Tooltip>
          <CopyIconButton
            label={`复制 ${agent.name} 的绑定信息`}
            value={`${bindingSummary} · ${orchestrationSummary}`}
          />
        </span>
      </div>
      <div className="agent-binding">
        <span className="agent-binding-heading">
          <strong>Agent Key</strong>
          <CopyIconButton label={`复制 Agent Key ${agent.agentKey}`} value={agent.agentKey} />
        </span>
        <span>{agent.agentKey} · {agent.reasoningPolicy}</span>
      </div>
      <IconButton icon="edit" label={`管理 Agent ${agent.name}`} onClick={() => onOpen(agent.id)} />
    </article>
  );
}

type UsageRange = "TODAY" | "7_DAYS" | "ALL" | "CUSTOM";

function UsagePage() {
  const [agents, setAgents] = useState<AgentSummary[]>([]);
  const [agentError, setAgentError] = useState<string | null>(null);

  useEffect(() => {
    listAgents()
      .then(setAgents)
      .catch((reason: unknown) => setAgentError(errorMessage(reason)));
  }, []);

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Usage</span>
          <h1>用量监控</h1>
          <p>查看 Primary 与子 Agent 的 Token 使用汇总和会话明细。</p>
        </div>
      </header>
      {agentError && (
        <section className="notice error">
          Agent 筛选器读取失败：{agentError}
        </section>
      )}
      <UsageMonitorCard />
      <AgentThreadInstancesPanel />
      <ScheduleDecisionsPanel />
      <AgentUsagePanel agents={agents} />
    </>
  );
}

function scheduleDecisionSourceLabel(source: string): string {
  return {
    HELPER: "helper 预检",
    DESKTOP_PREVIEW: "界面预览",
    DESKTOP_EXECUTE: "界面执行",
  }[source] ?? source;
}

function ExpandableMessage({ className, text }: { className?: string; text: string }) {
  const firstLine = text.split(/\r?\n/, 1)[0];
  const hasDetails = text.length > 160 || firstLine !== text;
  if (!hasDetails) return <small className={className}>{text}</small>;
  const preview = firstLine.length > 160 ? `${firstLine.slice(0, 157)}…` : firstLine;
  return (
    <div className={`expandable-message ${className ?? ""}`}>
      <small>{preview}</small>
      <details>
        <summary>查看完整信息</summary>
        <p>{text}</p>
      </details>
    </div>
  );
}

function ScheduleDecisionsPanel() {
  const [decisions, setDecisions] = useState<ScheduleDecisionResponse[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    setError(null);
    try {
      setDecisions(await listAgentScheduleDecisions({ limit: 30 }));
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(true), 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  return (
    <section className="agent-instance-card" aria-labelledby="schedule-decision-title">
      <header>
        <div>
          <span className="eyebrow">Schedule Decisions</span>
          <h2 id="schedule-decision-title">调度决策记录</h2>
          <p>只追加记录 helper 预检与界面预览/执行产生的每次 REUSE / SPAWN 决策及其原因。</p>
        </div>
        <div className="runtime-monitor-actions">
          <button className="secondary-button" disabled={loading} onClick={() => void load()} type="button">
            {loading ? "刷新中…" : "刷新"}
          </button>
        </div>
      </header>
      {error && <div className="inline-error" role="alert">{error}</div>}
      {!loading && !error && decisions.length === 0 && (
        <div className="usage-empty">尚无调度决策记录；helper 预检或界面评估后才会出现。</div>
      )}
      {!error && decisions.length > 0 && (
        <div className="usage-record-list">
          {decisions.map((decision) => (
            <article
              className={`reuse-recommendation ${decision.decision.toLowerCase()}`}
              key={decision.id}
            >
              <strong>{decision.decision}</strong>
              <span>{decision.reasonCode}</span>
              <span className="reuse-recommendation-meta">
                {formatUsageDate(decision.createdAt)}
                {" · "}{scheduleDecisionSourceLabel(decision.source)}
                {" · "}{decision.agentNameSnapshot ?? decision.agentId ?? "Unknown Agent"}
                {" · "}Scope {decision.workspaceScopeKey}
                {decision.candidateThreadId
                  ? ` · 候选 ${shortThreadId(decision.candidateThreadId)}`
                  : ""}
                {decision.parentThreadId
                  ? ` · Primary ${shortThreadId(decision.parentThreadId)}`
                  : ""}
                {decision.claimed ? " · 已锁定" : ""}
                {decision.taskScopeKey ? ` · Task ${decision.taskScopeKey}` : ""}
                {" · "}Cache {decision.cacheHint}
              </span>
              <span className="schedule-decision-actions">
                {decision.parentThreadId && (
                  <CopyIconButton label="复制 Primary Thread ID" value={decision.parentThreadId} />
                )}
                {decision.candidateThreadId && (
                  <CopyIconButton label="复制候选 Thread ID" value={decision.candidateThreadId} />
                )}
                <InfoTip
                  label={`Primary：${decision.parentThreadId ?? "未知"}\n候选：${decision.candidateThreadId ?? "—"}\n指纹：${decision.runtimeFingerprint ?? "未知"}`}
                />
              </span>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function UsageMonitorCard() {
  const [monitor, setMonitor] = useState<RuntimeBridgeStatusResponse | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setMonitor(await getUsageMonitorStatus());
      setError(null);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  async function setRunning(running: boolean) {
    setBusy(true);
    setError(null);
    try {
      setMonitor(running ? await startUsageMonitor() : await stopUsageMonitor());
    } catch (reason: unknown) {
      setError(errorMessage(reason));
      await refresh();
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="runtime-monitor-card">
      <header>
        <div>
          <span className="eyebrow">Runtime Bridge</span>
          <h2>Token Usage 监控</h2>
          <p>仅统计由当前 CAS Runtime Bridge 启动或恢复的 Codex 会话。</p>
        </div>
        <div className="runtime-monitor-actions">
          <button className="secondary-button" disabled={busy} onClick={() => void refresh()} type="button">
            刷新
          </button>
          <button
            className="primary-button"
            disabled={busy || monitor?.status === "RUNNING"}
            onClick={() => void setRunning(true)}
            type="button"
          >
            {busy ? "处理中…" : "启动监控"}
          </button>
          <button
            className="secondary-button"
            disabled={busy || !monitor || monitor.status === "STOPPED"}
            onClick={() => void setRunning(false)}
            type="button"
          >
            停止
          </button>
        </div>
      </header>
      {monitor && (
        <dl className="runtime-monitor-grid">
          <EnvironmentField label="Bridge" value={monitor.status} />
          <EnvironmentField label="Usage Schema" value={monitor.schemaCapability} />
          <EnvironmentField label="Session Schema" value={monitor.managedSessionCapability} />
          <EnvironmentField label="Agent Execution" value={monitor.agentExecutionCapability} />
          <EnvironmentField label="Protocol" value={monitor.protocolCompatibility} />
          <EnvironmentField label="Bound Thread" value={monitor.managedSession?.threadId ?? null} />
          <EnvironmentField label="Session State" value={monitor.managedSession?.status ?? null} />
          <EnvironmentField label="最近事件" value={monitor.lastEventAt ? formatDataAge(monitor.lastEventAt) : null} />
          <EnvironmentField label="启动于" value={monitor.startedAt ? formatUsageDate(monitor.startedAt) : null} />
        </dl>
      )}
      {(monitor?.status === "FAILED" || monitor?.status === "DEGRADED") && (
        <small className="runtime-monitor-warning">
          事件流已断开或降级；生命周期与 Token 数据停留在最近事件（
          {monitor.lastEventAt ? formatDataAge(monitor.lastEventAt) : "未知"}
          ），恢复前不得当作当前事实。
        </small>
      )}
      {monitor?.lastError && <ExpandableMessage className="runtime-monitor-warning" text={monitor.lastError} />}
      {error && <ExpandableMessage className="runtime-monitor-warning" text={error} />}
      <small className="runtime-monitor-note">
        独立启动的 Codex Desktop / CLI 不会被旁路监听；异常退出后需显式恢复原 Thread，
        CAS 不会自动重放上一个 Turn。
      </small>
    </section>
  );
}

function AgentThreadInstancesPanel() {
  const [selectedProject, setSelectedProject] =
    useState<AgentThreadProjectSummaryResponse | null>(null);

  return selectedProject
    ? <AgentThreadProjectDetail project={selectedProject} onBack={() => setSelectedProject(null)} />
    : <AgentThreadProjectOverview onOpen={setSelectedProject} />;
}

function AgentThreadProjectOverview({
  onOpen,
}: {
  onOpen: (project: AgentThreadProjectSummaryResponse) => void;
}) {
  const [projects, setProjects] = useState<AgentThreadProjectSummaryResponse[]>([]);
  const [nativeSync, setNativeSync] = useState<NativeSubagentSyncResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    setError(null);
    try {
      const loaded = await listAgentThreadProjects();
      setProjects(loaded.items);
      setNativeSync(loaded.sync);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      if (!silent) setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(true), 5000);
    return () => window.clearInterval(timer);
  }, [load]);

  return (
    <section className="agent-instance-card" aria-labelledby="agent-instance-title">
      <header>
        <div>
          <span className="eyebrow">Subagent Threads</span>
          <h2 id="agent-instance-title">子 Agent 项目</h2>
          <p>按 Workspace Scope 汇总子 Agent Thread；项目概览每 5 秒同步一次。</p>
        </div>
        <div className="runtime-monitor-actions">
          {nativeSync && (
            <StatusBadge
              className={`result ${nativeSync.capability === "SUPPORTED" ? "ready" : "blocked"}`}
              description={nativeSync.message}
              icon={nativeSync.capability === "SUPPORTED" ? "check" : "alert"}
              label={`Native ${nativeSync.capability}`}
            />
          )}
          <button className="secondary-button" disabled={loading} onClick={() => void load()} type="button">
            {loading ? "同步中…" : "同步原生状态"}
          </button>
        </div>
      </header>

      {error && <div className="inline-error" role="alert">{error}</div>}
      {nativeSync && (
        <span
          className={nativeSync.capability === "SUPPORTED"
            ? "native-sync-note runtime-monitor-note"
            : "native-sync-note runtime-monitor-warning"}
        >
          <ExpandableMessage
            text={`${nativeSync.message}${nativeSync.capability === "SUPPORTED"
              ? ` 已识别 ${nativeSync.discoveredCount} 个，映射 ${nativeSync.syncedCount} 个，未映射 ${nativeSync.unmappedCount} 个。`
              : ""}`}
          />
          {nativeSync.sourcePath && (
            <CopyIconButton label={`复制原生状态来源路径 ${nativeSync.sourcePath}`} value={nativeSync.sourcePath} />
          )}
        </span>
      )}
      {!loading && !error && projects.length === 0 && (
        <div className="usage-empty">尚未识别到可映射的 Primary 原生子 Agent Thread。</div>
      )}
      {!error && projects.length > 0 && (
        <div className="agent-project-list">
          {projects.map((project) => (
            <Tooltip
              content={project.workspaceScopeKey ?? "尚未识别 Workspace Scope"}
              key={project.workspaceScopeKey ?? "__unscoped__"}
            >
              <button
                aria-label={`${project.workspaceScopeKey ?? "未归属项目"}：${project.agentCount} 个 Agents，${project.instanceCount} 个 Threads，${formatTokenCount(project.totalTokens)} Tokens，最近活动 ${formatDataAge(project.lastUsedAt)}`}
                className="agent-project-row"
                onClick={() => onOpen(project)}
                type="button"
              >
                <span className="agent-project-main">
                  <strong>{workspaceProjectName(project.workspaceScopeKey)}</strong>
                  <code>
                    {project.workspaceScopeKey ?? "未归属项目，可进入后修正 Scope"}
                  </code>
                  <span className="agent-project-badges">
                    {project.runningCount > 0 && (
                      <span className="agent-instance-status running">{project.runningCount} 运行中</span>
                    )}
                    {project.recoveryRequiredCount > 0 && (
                      <span className="agent-instance-status recovery_required">
                        {project.recoveryRequiredCount} 待恢复
                      </span>
                    )}
                  </span>
                </span>
                <span className="agent-project-metrics">
                  <ProjectMetric icon="users" label="Agents" value={String(project.agentCount)} />
                  <ProjectMetric icon="threads" label="Threads" value={String(project.instanceCount)} />
                  <ProjectMetric icon="tokens" label="Tokens" value={formatTokenCount(project.totalTokens)} />
                  <ProjectMetric icon="clock" label="最近活动" value={formatDataAge(project.lastUsedAt)} />
                </span>
                <span className="agent-project-open" aria-hidden="true"><UiIcon name="chevron-right" /></span>
              </button>
            </Tooltip>
          ))}
        </div>
      )}
    </section>
  );
}

function ProjectMetric({ icon, label, value }: { icon: IconName; label: string; value: string }) {
  return (
    <span className="agent-project-metric">
      <Tooltip content={`${label}：${value}`}>
        <span className="agent-project-metric-icon"><UiIcon name={icon} /></span>
      </Tooltip>
      <strong>{value}</strong>
    </span>
  );
}

interface AgentThreadGroup {
  key: string;
  name: string;
  instances: AgentThreadInstanceResponse[];
  totalTokens: number;
  runningCount: number;
  recoveryRequiredCount: number;
  lastUsedAt: string;
}

function AgentThreadProjectDetail({
  project,
  onBack,
}: {
  project: AgentThreadProjectSummaryResponse;
  onBack: () => void;
}) {
  const [instances, setInstances] = useState<AgentThreadInstanceResponse[]>([]);
  const [nativeSync, setNativeSync] = useState<NativeSubagentSyncResponse | null>(null);
  const [scopeDrafts, setScopeDrafts] = useState<Record<string, string>>({});
  const [recommendations, setRecommendations] =
    useState<Record<string, AgentThreadInstanceRecommendation>>({});
  const [expandedAgent, setExpandedAgent] = useState<string | null>(null);
  const [expandedInstance, setExpandedInstance] = useState<string | null>(null);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [savingScope, setSavingScope] = useState<string | null>(null);
  const [checkingRecommendation, setCheckingRecommendation] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async (cursor: string | null = null, append = false) => {
    if (append) setLoadingMore(true);
    else setLoading(true);
    setError(null);
    try {
      const loaded = await listAgentThreadInstances({
        ...(project.workspaceScopeKey === null
          ? { unscoped: true }
          : { workspaceScopeKey: project.workspaceScopeKey }),
        limit: 50,
        cursor,
      });
      setInstances((current) => append
        ? [...current, ...loaded.items.filter((item) => !current.some((value) => value.id === item.id))]
        : loaded.items);
      setNextCursor(loaded.nextCursor);
      setNativeSync(loaded.sync);
      setScopeDrafts((current) => ({
        ...current,
        ...Object.fromEntries(loaded.items.map((instance) => [
          instance.id,
          current[instance.id] ?? instance.workspaceScopeKey ?? "",
        ])),
      }));
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
      setLoadingMore(false);
    }
  }, [project.workspaceScopeKey]);

  useEffect(() => {
    void load();
  }, [load]);

  async function saveWorkspaceScope(instance: AgentThreadInstanceResponse) {
    setSavingScope(instance.id);
    setError(null);
    try {
      const updated = await setAgentThreadInstanceWorkspaceScope(
        instance.codexThreadId,
        scopeDrafts[instance.id]?.trim() || null,
      );
      setInstances((current) => updated.workspaceScopeKey === project.workspaceScopeKey
        ? current.map((item) => item.id === updated.id ? updated : item)
        : current.filter((item) => item.id !== updated.id));
      setScopeDrafts((current) => ({
        ...current,
        [updated.id]: updated.workspaceScopeKey ?? "",
      }));
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setSavingScope(null);
    }
  }

  async function checkRecommendation(instance: AgentThreadInstanceResponse) {
    const scope = scopeDrafts[instance.id]?.trim();
    if (!instance.agentId || !scope) {
      setError("请先填写并保存 Workspace Scope，再评估 Primary 调度约束。");
      return;
    }
    setCheckingRecommendation(instance.id);
    setError(null);
    try {
      const recommendation = await recommendAgentThreadInstance(
        instance.agentId,
        scope,
        instance.parentThreadId,
      );
      setRecommendations((current) => ({ ...current, [instance.id]: recommendation }));
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setCheckingRecommendation(null);
    }
  }

  const groups = groupAgentThreadInstances(instances);

  return (
    <section className="agent-instance-card" aria-labelledby="agent-instance-title">
      <header>
        <div>
          <span className="eyebrow">Subagent Threads</span>
          <h2 id="agent-instance-title">{workspaceProjectName(project.workspaceScopeKey)}</h2>
          <p>
            <Tooltip
              content={project.workspaceScopeKey ?? "未归属项目"}
              focusable
              label={`Workspace Scope：${project.workspaceScopeKey ?? "未归属项目"}`}
            >
              <span>{project.workspaceScopeKey ?? "未归属项目"}</span>
            </Tooltip>
            {" · "}已加载 {instances.length}/{project.instanceCount} 个 Thread，详情按需展开。
          </p>
        </div>
        <div className="runtime-monitor-actions">
          {nativeSync && (
            <StatusBadge
              className={`result ${nativeSync.capability === "SUPPORTED" ? "ready" : "blocked"}`}
              description={nativeSync.message}
              icon={nativeSync.capability === "SUPPORTED" ? "check" : "alert"}
              label={`Native ${nativeSync.capability}`}
            />
          )}
          <button className="secondary-button" onClick={onBack} type="button">返回项目</button>
          <IconButton
            disabled={loading}
            icon="refresh"
            label={loading ? "正在刷新项目详情" : "刷新项目详情"}
            loading={loading}
            onClick={() => void load()}
          />
        </div>
      </header>

      {error && <div className="inline-error" role="alert">{error}</div>}
      {nativeSync && (
        <span
          className={nativeSync.capability === "SUPPORTED"
            ? "native-sync-note runtime-monitor-note"
            : "native-sync-note runtime-monitor-warning"}
        >
          <ExpandableMessage
            text={`${nativeSync.message}${nativeSync.capability === "SUPPORTED"
              ? ` 已识别 ${nativeSync.discoveredCount} 个，映射 ${nativeSync.syncedCount} 个，未映射 ${nativeSync.unmappedCount} 个。`
              : ""}`}
          />
          {nativeSync.sourcePath && (
            <CopyIconButton label={`复制原生状态来源路径 ${nativeSync.sourcePath}`} value={nativeSync.sourcePath} />
          )}
        </span>
      )}
      {!loading && !error && instances.length === 0 && (
        <div className="usage-empty">该项目下暂无子 Agent Thread。</div>
      )}
      {!error && instances.length > 0 && (
        <div className="agent-project-agent-list">
          {groups.map((group) => (
            <article className="agent-project-agent" key={group.key}>
              <button
                aria-expanded={expandedAgent === group.key}
                className="agent-project-agent-toggle"
                onClick={() => setExpandedAgent((current) => current === group.key ? null : group.key)}
                type="button"
              >
                <span>
                  <strong>{group.name}</strong>
                  <small>{group.instances.length} Threads · 最近 {formatDataAge(group.lastUsedAt)}</small>
                </span>
                <span className="agent-project-agent-summary">
                  {group.runningCount > 0 && <em>{group.runningCount} 运行中</em>}
                  {group.recoveryRequiredCount > 0 && <em>{group.recoveryRequiredCount} 待恢复</em>}
                  <strong>{formatTokenCount(group.totalTokens)} Tokens</strong>
                  <span aria-hidden="true">
                    <UiIcon name={expandedAgent === group.key ? "chevron-up" : "chevron-down"} />
                  </span>
                </span>
              </button>
              {expandedAgent === group.key && (
                <div className="agent-thread-compact-list">
                  {group.instances.map((instance) => (
                    <article className="agent-thread-compact-row" key={instance.id}>
                      <div className="agent-thread-compact-header">
                        <span>
                          <Tooltip content={instance.codexThreadId} focusable label={`Thread ID：${instance.codexThreadId}`}>
                            <code>Thread {shortThreadId(instance.codexThreadId)}</code>
                          </Tooltip>
                          <CopyIconButton label="复制 Thread ID" value={instance.codexThreadId} />
                          <StatusBadge
                            className={`agent-instance-status ${instance.status.toLowerCase()}`}
                            description={agentInstanceStatusDescription(instance.status)}
                            icon={instance.status === "IDLE" ? "check" : instance.status === "RUNNING" ? "clock" : instance.status === "CLOSED" ? "x-circle" : "alert"}
                            label={agentInstanceStatusLabel(instance.status)}
                          />
                        </span>
                        <span>
                          <strong>{formatTokenCount(instance.totalTokens)} Tokens</strong>
                          <small>{formatDataAge(instance.lastUsedAt)}</small>
                          <IconButton
                            icon={expandedInstance === instance.id ? "chevron-up" : "chevron-down"}
                            label={expandedInstance === instance.id ? "收起 Thread 详情" : "展开 Thread 详情"}
                            onClick={() => setExpandedInstance((current) => current === instance.id ? null : instance.id)}
                          />
                        </span>
                      </div>
                      {expandedInstance === instance.id && (
                        <div className="agent-thread-details">
                          <div className="agent-thread-identifiers">
                            <span>
                              <Tooltip content={instance.codexThreadId} focusable label={`Child Thread ID：${instance.codexThreadId}`}>
                                <code>Child {instance.codexThreadId}</code>
                              </Tooltip>
                              <CopyIconButton label="复制 Child Thread ID" value={instance.codexThreadId} />
                            </span>
                            {instance.parentThreadId && (
                              <span>
                                <Tooltip content={instance.parentThreadId} focusable label={`Primary Thread ID：${instance.parentThreadId}`}>
                                  <code>Primary {instance.parentThreadId}</code>
                                </Tooltip>
                                <CopyIconButton label="复制 Primary Thread ID" value={instance.parentThreadId} />
                              </span>
                            )}
                            {instance.taskScopeKey && (
                              <span>
                                <Tooltip content={instance.taskScopeKey} focusable label={`Task Scope：${instance.taskScopeKey}`}>
                                  <code>Task {instance.taskScopeKey}</code>
                                </Tooltip>
                                <CopyIconButton label="复制 Task Scope" value={instance.taskScopeKey} />
                              </span>
                            )}
                            <span className="thread-observation">
                              <small>数据 {formatDataAge(instance.lastObservedAt)} · 模型使用 {formatDataAge(instance.lastModelUsageAt)}</small>
                              <InfoTip label={`最近观察：${formatUsageDate(instance.lastObservedAt ?? instance.lastUsedAt)}\n最近模型使用：${instance.lastModelUsageAt ? formatUsageDate(instance.lastModelUsageAt) : "未知，缓存判定按未知处理"}`} />
                            </span>
                          </div>
                          <dl>
                            <UsageRecordMetric label="Total" value={instance.totalTokens} />
                            <UsageRecordMetric label="Context" value={instance.currentContextTokens} />
                            <UsageRecordMetric label="Cached" value={hasDetailedInstanceUsage(instance) ? instance.cachedInputTokens : null} />
                            <UsageRecordMetric label="Input" value={hasDetailedInstanceUsage(instance) ? instance.inputTokens : null} />
                            <UsageRecordMetric label="Output" value={hasDetailedInstanceUsage(instance) ? instance.outputTokens : null} />
                          </dl>
                          <div className="agent-instance-scope">
                            <input
                              aria-label={`${instance.agentNameSnapshot ?? "Agent"} Workspace Scope`}
                              onChange={(event) => {
                                setScopeDrafts((current) => ({ ...current, [instance.id]: event.target.value }));
                                setRecommendations((current) => {
                                  const next = { ...current };
                                  delete next[instance.id];
                                  return next;
                                });
                              }}
                              placeholder="Workspace Scope，例如 c:/workspace/project"
                              value={scopeDrafts[instance.id] ?? ""}
                            />
                            <button className="ghost-button" disabled={savingScope === instance.id} onClick={() => void saveWorkspaceScope(instance)} type="button">
                              {savingScope === instance.id ? "保存中…" : "保存 Scope"}
                            </button>
                            <button className="ghost-button" disabled={checkingRecommendation === instance.id} onClick={() => void checkRecommendation(instance)} type="button">
                              {checkingRecommendation === instance.id ? "评估中…" : "评估复用"}
                            </button>
                          </div>
                          {recommendations[instance.id] && (
                            <div
                              className={`reuse-recommendation ${recommendations[instance.id].decision.toLowerCase()}`}
                            >
                              <strong>{recommendations[instance.id].decision}</strong>
                              <span>{recommendations[instance.id].message}</span>
                              <span className="reuse-recommendation-meta">
                                {recommendations[instance.id].reasonCode}
                                {" · "}Context {recommendations[instance.id].contextPressurePercent ?? "—"}
                                /{recommendations[instance.id].contextPressureLimitPercent}%
                                {" · "}Cache {recommendations[instance.id].cacheHint}
                              </span>
                              <InfoTip label="该结果基于当前 CAS 数据的预览；实际委派前 cas-helper 会重新读取 Codex 原生状态并按租约重新决策，结果可能变化。" />
                            </div>
                          )}
                        </div>
                      )}
                    </article>
                  ))}
                </div>
              )}
            </article>
          ))}
        </div>
      )}
      {nextCursor && (
        <button
          className="secondary-button agent-thread-load-more"
          disabled={loadingMore}
          onClick={() => void load(nextCursor, true)}
          type="button"
        >
          {loadingMore ? "加载中…" : "加载更多 Thread"}
        </button>
      )}
    </section>
  );
}

function workspaceProjectName(scope: string | null): string {
  if (!scope) return "未归属项目";
  const normalized = scope.replace(/[\\/]+$/, "");
  return normalized.split(/[\\/]/).pop() || scope;
}

function groupAgentThreadInstances(instances: AgentThreadInstanceResponse[]): AgentThreadGroup[] {
  const groups = new Map<string, AgentThreadGroup>();
  for (const instance of instances) {
    const name = instance.agentNameSnapshot ?? "Unknown Agent";
    const key = instance.agentId ?? `snapshot:${name}`;
    const group = groups.get(key) ?? {
      key,
      name,
      instances: [],
      totalTokens: 0,
      runningCount: 0,
      recoveryRequiredCount: 0,
      lastUsedAt: instance.lastUsedAt,
    };
    group.instances.push(instance);
    group.totalTokens += instance.totalTokens;
    group.runningCount += instance.status === "RUNNING" ? 1 : 0;
    group.recoveryRequiredCount += instance.status === "RECOVERY_REQUIRED" ? 1 : 0;
    if (instance.lastUsedAt > group.lastUsedAt) group.lastUsedAt = instance.lastUsedAt;
    groups.set(key, group);
  }
  return [...groups.values()].sort((left, right) => right.lastUsedAt.localeCompare(left.lastUsedAt));
}

function AgentUsagePanel({ agents }: { agents: AgentSummary[] }) {
  const [range, setRange] = useState<UsageRange>("7_DAYS");
  const [customFrom, setCustomFrom] = useState(() => localDateInputValue(new Date()));
  const [customTo, setCustomTo] = useState(() => localDateInputValue(new Date()));
  const [agentId, setAgentId] = useState("");
  const [summary, setSummary] = useState<UsageSummaryResponse | null>(null);
  const [records, setRecords] = useState<UsageRecordResponse[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const loadUsage = useCallback(async () => {
    const query = usageQuery(range, agentId, customFrom, customTo);
    if (!query) {
      setError("自定义时间范围无效：结束日期不能早于开始日期。");
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const [nextSummary, nextRecords] = await Promise.all([
        getUsageSummary(query),
        listUsageRecords({ ...query, limit: 50 }),
      ]);
      setSummary(nextSummary);
      setRecords(nextRecords);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, [agentId, customFrom, customTo, range]);

  useEffect(() => {
    void loadUsage();
  }, [loadUsage]);

  return (
    <section className="agent-usage-card" aria-labelledby="agent-usage-title">
      <header className="agent-usage-heading">
        <div>
          <span className="eyebrow">Agent Usage</span>
          <h2 id="agent-usage-title">Token 使用</h2>
          <p>统计 CAS Runtime Bridge 已确认的累计 Usage，不包含费用估算。</p>
        </div>
        <IconButton
          disabled={loading}
          icon="refresh"
          label={loading ? "正在刷新 Token 使用" : "刷新 Token 使用"}
          loading={loading}
          onClick={() => void loadUsage()}
        />
      </header>

      <div className="agent-usage-filters">
        <label>
          <span>Agent</span>
          <select onChange={(event) => setAgentId(event.target.value)} value={agentId}>
            <option value="">全部记录（含 Primary）</option>
            {agents.map((agent) => (
              <option key={agent.id} value={agent.id}>
                {agent.model?.providerKey ?? "未绑定供应商"} / {agent.name}（{agent.agentKey}）
              </option>
            ))}
          </select>
        </label>
        <div className="usage-range-switch" aria-label="Usage 时间范围">
          {([
            ["TODAY", "今天"],
            ["7_DAYS", "7 天"],
            ["ALL", "全部"],
            ["CUSTOM", "自定义"],
          ] as const).map(([value, label]) => (
            <button
              aria-pressed={range === value}
              className={range === value ? "active" : ""}
              key={value}
              onClick={() => setRange(value)}
              type="button"
            >
              {label}
            </button>
          ))}
        </div>
        {range === "CUSTOM" && (
          <div className="usage-custom-range">
            <label>
              <span>开始</span>
              <input
                max={customTo}
                onChange={(event) => setCustomFrom(event.target.value)}
                type="date"
                value={customFrom}
              />
            </label>
            <label>
              <span>结束</span>
              <input
                min={customFrom}
                onChange={(event) => setCustomTo(event.target.value)}
                type="date"
                value={customTo}
              />
            </label>
          </div>
        )}
      </div>

      {error && <div className="inline-error" role="alert">{error}</div>}

      {summary && !error && (
        <div className="usage-summary-grid">
          <UsageMetric label="Total" value={summary.totalTokens} />
          <UsageMetric label="Threads" value={summary.recordCount} />
          <UsageMetric label="Input" value={summary.inputTokens} />
          <UsageMetric label="Cached input" value={summary.cachedInputTokens} />
          <UsageMetric label="Output" value={summary.outputTokens} />
          <UsageMetric label="Reasoning" value={summary.reasoningOutputTokens} />
        </div>
      )}

      <div className="usage-status-legend" aria-label="Usage 状态说明">
        {(["LIVE", "FINAL", "PARTIAL", "UNKNOWN"] as UsageStatus[]).map((status) => (
          <StatusBadge
            className={`usage-status ${status.toLowerCase()}`}
            description={`${usageStatusShortDescription(status)} ${usageStatusDescription(status)}`}
            icon={status === "FINAL" ? "check" : status === "LIVE" ? "clock" : status === "PARTIAL" ? "alert" : "info"}
            key={status}
            label={status}
          />
        ))}
      </div>

      {!loading && !error && records.length === 0 && (
        <div className="usage-empty">当前筛选范围还没有可靠的 Token Usage。</div>
      )}

      {!error && records.length > 0 && (
        <div className="usage-record-list" aria-label="Usage 会话明细">
          {records.map((record) => (
            <article className="usage-record" key={record.id}>
              <header>
                <div>
                  <strong>
                    {record.providerNameSnapshot ?? "Unknown provider"}
                    {" / "}
                    {record.modelNameSnapshot ?? "Unknown model"}
                  </strong>
                  <small>
                    {record.agentNameSnapshot ?? "Primary / 未归属 Agent"}
                    {" · "}
                    {formatUsageDate(record.updatedAt)}
                  </small>
                </div>
                <StatusBadge
                  className={`usage-status ${record.usageStatus.toLowerCase()}`}
                  description={usageStatusDescription(record.usageStatus)}
                  icon={record.usageStatus === "FINAL" ? "check" : record.usageStatus === "LIVE" ? "clock" : record.usageStatus === "PARTIAL" ? "alert" : "info"}
                  label={record.usageStatus}
                />
              </header>
              <dl>
                <UsageRecordMetric label="Total" value={record.totalTokens} />
                <UsageRecordMetric label="Input" value={record.inputTokens} />
                <UsageRecordMetric label="Cached" value={record.cachedInputTokens} />
                <UsageRecordMetric label="Output" value={record.outputTokens} />
                <UsageRecordMetric label="Reasoning" value={record.reasoningOutputTokens} />
              </dl>
              <footer>
                <span className="usage-thread-id">
                  <Tooltip content={record.codexThreadId} focusable label={`Thread ID：${record.codexThreadId}`}>
                    <code>Thread {shortThreadId(record.codexThreadId)}</code>
                  </Tooltip>
                  <CopyIconButton label="复制 Thread ID" value={record.codexThreadId} />
                </span>
                <span>{record.parentThreadId ? "Subagent" : "Primary"}</span>
                <span>{record.source.replaceAll("_", " ")}</span>
              </footer>
            </article>
          ))}
        </div>
      )}
    </section>
  );
}

function UsageMetric({ label, value }: { label: string; value: number }) {
  return (
    <article>
      <span>{label}</span>
      <Tooltip
        content={new Intl.NumberFormat("en-US").format(value)}
        focusable
        label={`${label}：${new Intl.NumberFormat("en-US").format(value)}`}
      >
        <strong>{formatTokenCount(value)}</strong>
      </Tooltip>
    </article>
  );
}

function UsageRecordMetric({ label, value }: { label: string; value: number | null }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>
        <Tooltip
          content={value === null ? "当前数据源不提供该 Token 明细" : new Intl.NumberFormat("en-US").format(value)}
          focusable
          label={`${label}：${value === null ? "当前数据源不提供该 Token 明细" : new Intl.NumberFormat("en-US").format(value)}`}
        >
          <span>{value === null ? "—" : formatTokenCount(value)}</span>
        </Tooltip>
      </dd>
    </div>
  );
}

function hasDetailedInstanceUsage(instance: AgentThreadInstanceResponse): boolean {
  return instance.totalTokens === 0
    || instance.inputTokens + instance.cachedInputTokens + instance.outputTokens > 0;
}

function usageQuery(
  range: UsageRange,
  agentId: string,
  customFrom: string,
  customTo: string,
): UsageQueryRequest | null {
  const query: UsageQueryRequest = agentId ? { agentId } : {};
  if (range === "ALL") return query;
  const from = new Date();
  from.setHours(0, 0, 0, 0);
  const to = new Date(from);
  to.setDate(to.getDate() + 1);
  if (range === "7_DAYS") from.setDate(from.getDate() - 6);
  if (range === "CUSTOM") {
    const customStart = new Date(`${customFrom}T00:00:00`);
    const customEnd = new Date(`${customTo}T00:00:00`);
    if (
      !customFrom
      || !customTo
      || Number.isNaN(customStart.getTime())
      || Number.isNaN(customEnd.getTime())
      || customEnd < customStart
    ) {
      return null;
    }
    customEnd.setDate(customEnd.getDate() + 1);
    return { ...query, from: customStart.toISOString(), to: customEnd.toISOString() };
  }
  return { ...query, from: from.toISOString(), to: to.toISOString() };
}

function usageStatusDescription(status: UsageStatus): string {
  return {
    LIVE: "会话仍在运行，Token 数字可能继续增长。",
    FINAL: "Thread 已完成，当前累计 Usage 已确认。",
    PARTIAL: "Provider、旧协议或异常断流只提供了部分可靠字段。",
    UNKNOWN: "没有可靠 Usage，CAS 不会使用本地分词器猜测精确值。",
  }[status];
}

function usageStatusShortDescription(status: UsageStatus): string {
  return {
    LIVE: "仍在更新",
    FINAL: "已确认",
    PARTIAL: "部分数据",
    UNKNOWN: "无法确认",
  }[status];
}

function agentInstanceStatusLabel(status: AgentThreadInstanceStatus): string {
  return {
    RUNNING: "运行中",
    IDLE: "空闲",
    RECOVERY_REQUIRED: "需要恢复",
    CLOSED: "已关闭",
    UNKNOWN: "未知",
  }[status];
}

function agentInstanceStatusDescription(status: AgentThreadInstanceStatus): string {
  return {
    RUNNING: "依据 rollout 尾部最近事件判定为运行中；数据新鲜度取决于最近一次同步。",
    IDLE: "依据 rollout 尾部事件判定最近一次 Turn 已完成，可作为复用候选。",
    RECOVERY_REQUIRED: "Thread 曾异常中断，复用前必须显式恢复。",
    CLOSED: "Thread 已关闭，不再参与复用。",
    UNKNOWN: "CAS 尚未获得足够事件来判断 Thread 状态。",
  }[status];
}

function formatUsageDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function formatDataAge(value: string | null): string {
  if (!value) {
    return "未知";
  }
  const time = new Date(value).getTime();
  if (Number.isNaN(time)) {
    return "未知";
  }
  const seconds = Math.max(0, Math.floor((Date.now() - time) / 1000));
  if (seconds < 60) {
    return `${seconds} 秒前`;
  }
  if (seconds < 3600) {
    return `${Math.floor(seconds / 60)} 分钟前`;
  }
  if (seconds < 86400) {
    return `${Math.floor(seconds / 3600)} 小时前`;
  }
  return `${Math.floor(seconds / 86400)} 天前`;
}

function localDateInputValue(date: Date): string {
  return new Date(date.getTime() - date.getTimezoneOffset() * 60_000)
    .toISOString()
    .slice(0, 10);
}

function shortThreadId(value: string): string {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function CreateAgentPanel({
  models,
  onCancel,
  onCreated,
  presets,
}: {
  models: ModelSummary[];
  onCancel: () => void;
  onCreated: () => void;
  presets: AgentPresetResponse[];
}) {
  const initial = presets[0];
  const [templateKey, setTemplateKey] = useState(initial?.key ?? "");
  const [agentKey, setAgentKey] = useState(initial?.key ?? "");
  const [roleKey, setRoleKey] = useState(initial?.roleKey ?? "");
  const [orchestrationPhase, setOrchestrationPhase] = useState<OrchestrationPhase>(
    initial?.orchestrationPhase ?? "EXECUTION",
  );
  const [name, setName] = useState(initial?.name ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [instruction, setInstruction] = useState("");
  const [sandboxPolicy, setSandboxPolicy] = useState<SandboxPolicy>(
    initial?.defaultSandboxPolicy ?? "INHERIT",
  );
  const [reasoningPolicy, setReasoningPolicy] = useState<ReasoningPolicy>(
    initial?.defaultReasoningPolicy ?? "MODEL_DEFAULT",
  );
  const [reuseStrategy, setReuseStrategy] = useState<AgentReuseStrategy>("AUTO");
  const [cacheRetentionOverrideSeconds, setCacheRetentionOverrideSeconds] =
    useState<number | null>(null);
  const [modelId, setModelId] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [invalidField, setInvalidField] = useState<AgentFormField | null>(null);

  function selectTemplate(key: string) {
    setTemplateKey(key);
    const preset = presets.find((value) => value.key === key);
    setAgentKey(preset?.key ?? "");
    setRoleKey(preset?.roleKey ?? "");
    setOrchestrationPhase(preset?.orchestrationPhase ?? "EXECUTION");
    setName(preset?.name ?? "");
    setDescription(preset?.description ?? "");
    setInstruction("");
    setSandboxPolicy(preset?.defaultSandboxPolicy ?? "INHERIT");
    setReasoningPolicy(preset?.defaultReasoningPolicy ?? "MODEL_DEFAULT");
    setReuseStrategy("AUTO");
    setCacheRetentionOverrideSeconds(null);
    setError(null);
    setInvalidField(null);
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(null);
    setInvalidField(null);
    const localInvalidField = invalidAgentField({
      agentKey,
      name,
      description,
      instruction,
      instructionRequired: !templateKey,
      roleKey,
      cacheRetentionOverrideSeconds,
    });
    if (localInvalidField) {
      setInvalidField(localInvalidField);
      setError("请修正红色标记的字段。");
      return;
    }
    setSaving(true);
    try {
      await createAgent({
        agentKey,
        name,
        description,
        instruction,
        templateKey: templateKey || null,
        enabled: true,
        sandboxPolicy,
        reasoningPolicy,
        reuseStrategy,
        cacheRetentionOverrideSeconds,
        modelId: modelId || null,
        roleKey,
        orchestrationPhase,
      });
      onCreated();
    } catch (reason: unknown) {
      setError(errorMessage(reason));
      setInvalidField(agentErrorField(reason));
    } finally {
      setSaving(false);
    }
  }

  const selectedModel = models.find((model) => model.id === modelId);

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Create Agent</span>
          <h2>创建职责与模型绑定</h2>
          <p>模板定义 Capability 要求，但不会固定 Provider。</p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={onCancel}>取消</button>
      </div>

      <form className="provider-form" noValidate onSubmit={submit}>
        <label className="field full-width">
          <span>Template</span>
          <select onChange={(event) => selectTemplate(event.target.value)} value={templateKey}>
            {presets.map((preset) => <option key={preset.key} value={preset.key}>{preset.name}</option>)}
            <option value="">Blank / Custom</option>
          </select>
        </label>

        <label className="field">
          <span>Name</span>
          <input
            aria-invalid={invalidField === "name"}
            maxLength={160}
            onChange={(event) => {
              setName(event.target.value);
              if (invalidField === "name") setInvalidField(null);
            }}
            required
            value={name}
          />
          {invalidField === "name" && <small className="field-error">Name 不能为空，且最多 160 个字符。</small>}
        </label>

        <label className="field">
          <span>Agent Key（内部标识）</span>
          <input
            aria-invalid={invalidField === "agentKey"}
            maxLength={64}
            onChange={(event) => {
              setAgentKey(event.target.value);
              if (invalidField === "agentKey") setInvalidField(null);
            }}
            pattern="[a-z][a-z0-9_-]*"
            required
            value={agentKey}
          />
          <small className={invalidField === "agentKey" ? "field-error" : undefined}>
            {invalidField === "agentKey"
              ? "Key 必须以小写字母开头，只能包含小写字母、数字、-、_，且不能重复。"
              : "创建后不可修改。"}
          </small>
        </label>

        <label className="field">
          <span className="field-label-with-info">
            Role
            <InfoTip label="Role 表示编排职责；运行时同一 Role 最多启用一个 Agent。" />
          </span>
          <input
            aria-invalid={invalidField === "roleKey"}
            maxLength={64}
            onChange={(event) => {
              setRoleKey(event.target.value);
              if (invalidField === "roleKey") setInvalidField(null);
            }}
            pattern="[a-z][a-z0-9_-]*"
            required
            value={roleKey}
          />
          {invalidField === "roleKey" && (
            <small className="field-error">Role 必须以小写字母开头，只能包含小写字母、数字、-、_。</small>
          )}
        </label>

        <label className="field">
          <span className="field-label-with-info">
            Phase
            <InfoTip label="决定 Primary 在 Discovery、Execution、Verification 或 Review 的哪个阶段委派该 Agent。" />
          </span>
          <select
            onChange={(event) => setOrchestrationPhase(event.target.value as OrchestrationPhase)}
            value={orchestrationPhase}
          >
            <option value="DISCOVERY">Discovery</option>
            <option value="EXECUTION">Execution</option>
            <option value="VERIFICATION">Verification</option>
            <option value="REVIEW">Review</option>
          </select>
        </label>

        <ReuseStrategyField value={reuseStrategy} onChange={setReuseStrategy} />
        <AgentCacheRetentionField
          invalid={invalidField === "cacheRetentionOverrideSeconds"}
          onChange={(value) => {
            setCacheRetentionOverrideSeconds(value);
            if (invalidField === "cacheRetentionOverrideSeconds") setInvalidField(null);
          }}
          value={cacheRetentionOverrideSeconds}
        />

        <label className="field full-width">
          <span className="field-label-with-info">
            Description
            <InfoTip label="用于 Agent 列表和选择界面中的简短职责说明，最多 2000 个字符。" />
          </span>
          <textarea
            aria-invalid={invalidField === "description"}
            maxLength={2000}
            onChange={(event) => {
              setDescription(event.target.value);
              if (invalidField === "description") setInvalidField(null);
            }}
            required
            rows={3}
            value={description}
          />
          {invalidField === "description" && <small className="field-error">Description 不能为空，且最多 2000 个字符。</small>}
        </label>

        <label className="field full-width">
          <span className="field-label-with-info">
            Instructions {templateKey && <em>Optional override</em>}
            <InfoTip label={templateKey ? "留空时使用 CAS 后端正式模板；填写后覆盖模板指令。" : "写明该 Agent 的职责、边界与必须遵守的行为约束。"} />
          </span>
          <textarea
            aria-invalid={invalidField === "instruction"}
            maxLength={100000}
            onChange={(event) => {
              setInstruction(event.target.value);
              if (invalidField === "instruction") setInvalidField(null);
            }}
            placeholder={templateKey ? "留空时使用后端正式模板" : "描述 Agent 的职责与行为约束"}
            required={!templateKey}
            rows={6}
            value={instruction}
          />
          {invalidField === "instruction" && <small className="field-error">Custom Agent 必须填写 Instructions。</small>}
        </label>

        <PolicyFields
          model={selectedModel}
          reasoningInvalid={invalidField === "reasoningPolicy"}
          reasoningPolicy={reasoningPolicy}
          sandboxPolicy={sandboxPolicy}
          setReasoningPolicy={setReasoningPolicy}
          setSandboxPolicy={setSandboxPolicy}
        />

        <label className="field full-width">
          <span>供应商 / 模型 <em>Optional</em></span>
          <select
            aria-invalid={invalidField === "modelId"}
            onChange={(event) => {
              const nextModelId = event.target.value;
              const nextModel = models.find((model) => model.id === nextModelId);
              setModelId(nextModelId);
              setReasoningPolicy(normalizeReasoningPolicy(reasoningPolicy, nextModel));
              if (invalidField === "modelId") setInvalidField(null);
              if (invalidField === "reasoningPolicy") setInvalidField(null);
            }}
            value={modelId}
          >
            <option value="">No model assigned</option>
            {models.map((model) => (
              <option
                disabled={model.compatibility === "UNSUPPORTED" || model.compatibility === "GATEWAY_REQUIRED"}
                key={model.id}
                value={model.id}
              >
                {model.providerKey} / {model.displayName} — {compatibilityLabel(model.compatibility)}
              </option>
            ))}
          </select>
          {selectedModel?.compatibility === "UNKNOWN" && (
            <em>该 Model 可保存，但启用 Agent 前必须在 Models 页面完成工具闭环测试。</em>
          )}
          {invalidField === "modelId" && <small className="field-error">所选 Model 不存在或与该 Agent 不兼容。</small>}
        </label>

        {error && <div className="inline-error full-width" role="alert"><strong>Agent 保存失败</strong><span>{error}</span></div>}

        <div className="form-actions full-width">
          <button className="secondary-button" disabled={saving} onClick={onCancel} type="button">取消</button>
          <button className="primary-button" disabled={saving} type="submit">{saving ? "保存中…" : "创建 Agent"}</button>
        </div>
      </form>
    </section>
  );
}

function AgentDetailPanel({
  agentId,
  isActive,
  models,
  onBack,
  onChanged,
  onDeleted,
}: {
  agentId: string;
  isActive: boolean;
  models: ModelSummary[];
  onBack: () => void;
  onChanged: (message: string) => void;
  onDeleted: () => void;
}) {
  const [agent, setAgent] = useState<AgentDetailResponse | null>(null);
  const [modelId, setModelId] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [invalidField, setInvalidField] = useState<AgentFormField | null>(null);

  useEffect(() => {
    setAgent(null);
    setError(null);
    setInvalidField(null);
    getAgent(agentId)
      .then((value) => {
        const nextModelId = value.modelBinding?.id ?? "";
        const selectedModel = models.find((model) => model.id === nextModelId);
        setAgent({
          ...value,
          reasoningPolicy: normalizeReasoningPolicy(value.reasoningPolicy, selectedModel),
        });
        setModelId(nextModelId);
      })
      .catch((reason: unknown) => setError(errorMessage(reason)));
  }, [agentId, models]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!agent) return;
    setError(null);
    setInvalidField(null);
    const localInvalidField = invalidAgentField({
      name: agent.name,
      description: agent.description,
      instruction: agent.instruction,
      instructionRequired: true,
      roleKey: agent.roleKey ?? "",
      cacheRetentionOverrideSeconds: agent.cacheRetentionOverrideSeconds,
    });
    if (localInvalidField) {
      setInvalidField(localInvalidField);
      setError("请修正红色标记的字段。");
      return;
    }
    setSaving(true);
    try {
      const refreshed = await updateAgent({
        agentId: agent.id,
        name: agent.name,
        description: agent.description,
        instruction: agent.instruction,
        sandboxPolicy: agent.sandboxPolicy,
        reasoningPolicy: agent.reasoningPolicy,
        reuseStrategy: agent.reuseStrategy,
        cacheRetentionOverrideSeconds: agent.cacheRetentionOverrideSeconds,
        modelId: modelId || null,
        roleKey: agent.roleKey ?? "",
        orchestrationPhase: agent.orchestrationPhase ?? "EXECUTION",
      });
      setAgent(refreshed);
      setModelId(refreshed.modelBinding?.id ?? "");
      onChanged("Agent 配置已保存。");
    } catch (reason: unknown) {
      setError(errorMessage(reason));
      setInvalidField(agentErrorField(reason));
    } finally {
      setSaving(false);
    }
  }

  async function remove() {
    if (!agent || !window.confirm(`删除 Agent “${agent.name}”？`)) return;
    setSaving(true);
    setError(null);
    try {
      await deleteAgent(agent.id);
      onDeleted();
    } catch (reason: unknown) {
      setError(errorMessage(reason));
      setSaving(false);
    }
  }

  if (error && !agent) {
    return <section className="notice error"><strong>无法读取 Agent</strong><p>{error}</p><button className="secondary-button" onClick={onBack}>返回</button></section>;
  }
  if (!agent) return <section className="notice">正在读取 Agent…</section>;
  const selectedModel = models.find((model) => model.id === modelId);

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Agent Detail</span>
          <h2>{agent.name}</h2>
          <p>
            <code>
              {agent.modelBinding
                ? `${agent.modelBinding.providerKey} / ${agent.modelBinding.displayName}`
                : "未绑定供应商 / 模型"}
            </code>
            {" · "}{agent.agentType}{isActive ? " · 当前使用" : ""}
          </p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={onBack}>返回列表</button>
      </div>

      <form className="provider-form" noValidate onSubmit={submit}>
        <label className="field">
          <span>Name</span>
          <input
            aria-invalid={invalidField === "name"}
            maxLength={160}
            onChange={(event) => {
              setAgent({ ...agent, name: event.target.value });
              if (invalidField === "name") setInvalidField(null);
            }}
            required
            value={agent.name}
          />
          {invalidField === "name" && <small className="field-error">Name 不能为空，且最多 160 个字符。</small>}
        </label>

        <label className="field">
          <span className="field-label-with-info">
            Agent Key（内部标识）
            <InfoTip label="Agent Key 是不可变身份字段，不能在普通编辑中修改。" />
          </span>
          <div className="static-value static-value-with-action">
            <code>{agent.agentKey}</code>
            <CopyIconButton label={`复制 Agent Key ${agent.agentKey}`} value={agent.agentKey} />
          </div>
        </label>

        <label className="field">
          <span className="field-label-with-info">
            Role
            <InfoTip label="Role 表示编排职责；运行时同一 Role 最多启用一个 Agent。" />
          </span>
          <input
            aria-invalid={invalidField === "roleKey"}
            disabled={isActive}
            maxLength={64}
            onChange={(event) => {
              setAgent({ ...agent, roleKey: event.target.value });
              if (invalidField === "roleKey") setInvalidField(null);
            }}
            required
            value={agent.roleKey ?? ""}
          />
          {(invalidField === "roleKey" || isActive) && (
            <small className={invalidField === "roleKey" ? "field-error" : undefined}>
              {invalidField === "roleKey" ? "Role 格式无效。" : "当前启用时不可修改 Role。"}
            </small>
          )}
        </label>

        <label className="field">
          <span className="field-label-with-info">
            Phase
            <InfoTip label="决定 Primary 在 Discovery、Execution、Verification 或 Review 的哪个阶段委派该 Agent。" />
          </span>
          <select
            disabled={isActive}
            onChange={(event) => setAgent({
              ...agent,
              orchestrationPhase: event.target.value as OrchestrationPhase,
            })}
            value={agent.orchestrationPhase ?? "EXECUTION"}
          >
            <option value="DISCOVERY">Discovery</option>
            <option value="EXECUTION">Execution</option>
            <option value="VERIFICATION">Verification</option>
            <option value="REVIEW">Review</option>
          </select>
          {isActive && <small>当前启用时不可修改 Phase。</small>}
        </label>

        <ReuseStrategyField
          value={agent.reuseStrategy}
          onChange={(value) => setAgent({ ...agent, reuseStrategy: value })}
        />
        <AgentCacheRetentionField
          invalid={invalidField === "cacheRetentionOverrideSeconds"}
          onChange={(value) => {
            setAgent({ ...agent, cacheRetentionOverrideSeconds: value });
            if (invalidField === "cacheRetentionOverrideSeconds") setInvalidField(null);
          }}
          value={agent.cacheRetentionOverrideSeconds}
        />

        <label className="field full-width">
          <span className="field-label-with-info">
            Description
            <InfoTip label="用于 Agent 列表和选择界面中的简短职责说明，最多 2000 个字符。" />
          </span>
          <textarea
            aria-invalid={invalidField === "description"}
            maxLength={2000}
            onChange={(event) => {
              setAgent({ ...agent, description: event.target.value });
              if (invalidField === "description") setInvalidField(null);
            }}
            required
            rows={3}
            value={agent.description}
          />
          {invalidField === "description" && <small className="field-error">Description 不能为空，且最多 2000 个字符。</small>}
        </label>

        <label className="field full-width">
          <span className="field-label-with-info">
            Instructions
            <InfoTip label="写明该 Agent 的职责、边界与必须遵守的行为约束。" />
          </span>
          <textarea
            aria-invalid={invalidField === "instruction"}
            maxLength={100000}
            onChange={(event) => {
              setAgent({ ...agent, instruction: event.target.value });
              if (invalidField === "instruction") setInvalidField(null);
            }}
            required
            rows={8}
            value={agent.instruction}
          />
          {invalidField === "instruction" && <small className="field-error">Instructions 不能为空。</small>}
        </label>

        <PolicyFields
          model={selectedModel}
          reasoningInvalid={invalidField === "reasoningPolicy"}
          reasoningPolicy={agent.reasoningPolicy}
          sandboxPolicy={agent.sandboxPolicy}
          setReasoningPolicy={(value) => setAgent({ ...agent, reasoningPolicy: value })}
          setSandboxPolicy={(value) => setAgent({ ...agent, sandboxPolicy: value })}
        />

        <label className="field full-width">
          <span>供应商 / 模型</span>
          <select
            aria-invalid={invalidField === "modelId"}
            onChange={(event) => {
              const nextModelId = event.target.value;
              const nextModel = models.find((model) => model.id === nextModelId);
              setModelId(nextModelId);
              setAgent({
                ...agent,
                reasoningPolicy: normalizeReasoningPolicy(agent.reasoningPolicy, nextModel),
              });
              if (invalidField === "modelId") setInvalidField(null);
              if (invalidField === "reasoningPolicy") setInvalidField(null);
            }}
            value={modelId}
          >
            <option value="">No model assigned</option>
            {models.map((model) => (
              <option
                disabled={model.compatibility === "UNSUPPORTED" || model.compatibility === "GATEWAY_REQUIRED"}
                key={model.id}
                value={model.id}
              >
                {model.providerKey} / {model.displayName} — {compatibilityLabel(model.compatibility)}
              </option>
            ))}
          </select>
          {invalidField === "modelId" && <small className="field-error">所选 Model 不存在或与该 Agent 不兼容。</small>}
        </label>

        <CompatibilityPanel compatibility={agent.compatibility} />

        {error && <div className="inline-error full-width" role="alert"><strong>Agent 保存失败</strong><span>{error}</span></div>}

        <div className="form-actions agent-actions full-width">
          <Tooltip
            content={isActive ? "请先在概览切换到 Default 或其他 Agent" : "删除该 Agent"}
            focusable={isActive}
            label="删除 Agent 可用性说明"
          >
            <button
              className="danger-button"
              disabled={saving || isActive}
              onClick={() => void remove()}
              type="button"
            >
              删除 Agent
            </button>
          </Tooltip>
          <button className="primary-button" disabled={saving} type="submit">{saving ? "保存中…" : "保存更改"}</button>
        </div>
      </form>
    </section>
  );
}

function PolicyFields({
  model,
  reasoningInvalid,
  reasoningPolicy,
  sandboxPolicy,
  setReasoningPolicy,
  setSandboxPolicy,
}: {
  model: ModelSummary | undefined;
  reasoningInvalid: boolean;
  reasoningPolicy: ReasoningPolicy;
  sandboxPolicy: SandboxPolicy;
  setReasoningPolicy: (value: ReasoningPolicy) => void;
  setSandboxPolicy: (value: SandboxPolicy) => void;
}) {
  const reasoningOptions = availableReasoningPolicies(model);
  return (
    <>
      <label className="field">
        <span className="field-label-with-info">
          Sandbox
          <InfoTip label="限制子 Agent 可读取或修改的文件范围；Inherit 会沿用 Primary 当前策略。" />
        </span>
        <select onChange={(event) => setSandboxPolicy(event.target.value as SandboxPolicy)} value={sandboxPolicy}>
          <option value="READ_ONLY">Read only</option>
          <option value="WORKSPACE_WRITE">Workspace write</option>
          <option value="DANGER_FULL_ACCESS">Danger full access</option>
          <option value="INHERIT">Inherit</option>
        </select>
      </label>
      <label className="field">
        <span className="field-label-with-info">
          Reasoning
          <InfoTip label="控制子 Agent 的推理强度；CAS 会按所选 Model 的已知能力限制可选等级。" />
        </span>
        <select
          aria-invalid={reasoningInvalid}
          onChange={(event) => setReasoningPolicy(event.target.value as ReasoningPolicy)}
          value={reasoningPolicy}
        >
          {reasoningOptions.map((option) => (
            <option key={option} value={option}>
              {reasoningPolicyLabel(option, model)}
            </option>
          ))}
        </select>
        <small className={reasoningInvalid ? "field-error" : undefined}>
          {reasoningInvalid
            ? "所选 Model 不支持该 Reasoning。"
            : model?.source === "PRESET"
              ? "已按 CAS 内置 Model 的官方能力限制可选级别。"
              : "自定义 Model 能力未知时，可选择 Inherit。"}
        </small>
      </label>
    </>
  );
}

function ReuseStrategyField({
  onChange,
  value,
}: {
  onChange: (value: AgentReuseStrategy) => void;
  value: AgentReuseStrategy;
}) {
  return (
    <label className="field">
      <span className="field-label-with-info">
        Thread 复用策略
        <InfoTip label="这是缓存感知调度偏好；Workspace Scope、运行状态与 Context 健康始终优先。" />
      </span>
      <select
        onChange={(event) => onChange(event.target.value as AgentReuseStrategy)}
        value={value}
      >
        <option value="AUTO">自动（推荐）</option>
        <option value="HOT">偏热</option>
        <option value="COLD">偏冷</option>
      </select>
    </label>
  );
}

function AgentCacheRetentionField({
  invalid,
  onChange,
  value,
}: {
  invalid: boolean;
  onChange: (value: number | null) => void;
  value: number | null;
}) {
  const inputId = useId();
  const [referenceOpen, setReferenceOpen] = useState(false);
  return (
    <>
      {referenceOpen && (
        <DocumentationReferenceDialog
          description="缓存时长可能随模式、模型和系统负载变化；这里只提供填写 Agent 软调度窗口时的资料入口。"
          eyebrow="Cache Reference"
          note="CAS 不把缓存时长当作 SLA。请按实际接口模式选取保守值，并以 Usage 中的 Cached Input 验证。"
          onClose={() => setReferenceOpen(false)}
          references={cacheRetentionReferences}
          title="各厂商缓存时长参考"
          titleId="cache-retention-reference-title"
        />
      )}
      <div className="field">
        <div className="cache-retention-heading">
          <button className="reference-link" onClick={() => setReferenceOpen(true)} type="button">
            <UiIcon name="book" />
            缓存参考
          </button>
          <label htmlFor={inputId}>缓存复用窗口（分钟）<em>Optional</em></label>
        </div>
        <input
          aria-invalid={invalid}
          id={inputId}
          max={525600}
          min={0.1}
          onChange={(event) => {
            const minutes = event.currentTarget.valueAsNumber;
            onChange(Number.isFinite(minutes) ? Math.round(minutes * 60) : null);
          }}
          placeholder="留空继承 Provider"
          step={0.1}
          type="number"
          value={value === null ? "" : value / 60}
        />
        <small className={invalid ? "field-error" : undefined}>
          {invalid
            ? "缓存复用窗口必须大于 0，且不能超过 525600 分钟。"
            : "仅作为软调度提示；与 Provider 配置同时存在时采用更短值。"}
        </small>
      </div>
    </>
  );
}

const ALL_REASONING_POLICIES: ReasoningPolicy[] = [
  "MODEL_DEFAULT",
  "LOW",
  "MEDIUM",
  "HIGH",
  "INHERIT",
];

function availableReasoningPolicies(model: ModelSummary | undefined): ReasoningPolicy[] {
  if (!model || model.source !== "PRESET") return ALL_REASONING_POLICIES;
  const efforts = new Set(model.supportedReasoningEfforts.map((effort) => effort.toUpperCase()));
  return ALL_REASONING_POLICIES.filter((policy) => (
    policy === "MODEL_DEFAULT" || (policy !== "INHERIT" && efforts.has(policy))
  ));
}

function normalizeReasoningPolicy(
  policy: ReasoningPolicy,
  model: ModelSummary | undefined,
): ReasoningPolicy {
  return availableReasoningPolicies(model).includes(policy) ? policy : "MODEL_DEFAULT";
}

function reasoningPolicyLabel(
  policy: ReasoningPolicy,
  model: ModelSummary | undefined,
): string {
  if (policy === "MODEL_DEFAULT") {
    const defaultEffort = model?.defaultReasoningEffort;
    return defaultEffort ? `Model default (${defaultEffort})` : "Model default";
  }
  return policy === "INHERIT"
    ? "Inherit"
    : `${policy[0]}${policy.slice(1).toLowerCase()}`;
}

function CompatibilityPanel({ compatibility }: { compatibility: AgentDetailResponse["compatibility"] }) {
  return (
    <div className={`compatibility-panel ${compatibility.status.toLowerCase()} full-width`}>
      <strong>Binding: {compatibility.status}</strong>
      {compatibility.issues.length > 0 && (
        <ul>{compatibility.issues.map((issue) => <li key={`${issue.code}-${issue.message}`}>{issue.message}</li>)}</ul>
      )}
    </div>
  );
}

function availabilityLabel(value: AgentSummary["availability"]): string {
  const labels = {
    READY: "Ready",
    MODEL_MISSING: "Needs model",
    PROVIDER_UNAVAILABLE: "Provider unavailable",
    INCOMPATIBLE_MODEL: "Incompatible",
    UNVERIFIED_MODEL: "Needs test",
    INVALID_CONFIGURATION: "Invalid",
  } as const;
  return labels[value];
}

function availabilityDescription(value: AgentSummary["availability"]): string {
  const descriptions = {
    READY: "Agent 已启用，Provider、Model 与兼容性检查均已就绪。",
    MODEL_MISSING: "Agent 尚未绑定 Model。",
    PROVIDER_UNAVAILABLE: "绑定 Model 所属的 Provider 已停用或不可用。",
    INCOMPATIBLE_MODEL: "绑定 Model 未通过 Codex Agent 兼容性要求。",
    UNVERIFIED_MODEL: "绑定 Model 尚未通过 Responses API 与工具闭环测试。",
    INVALID_CONFIGURATION: "绑定 Model 已停用；请在 Models 页面重新启用或更换绑定。",
  } as const;
  return descriptions[value];
}

function ProvidersPage() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<ProviderDetailResponse | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [credentialWarning, setCredentialWarning] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setProviders(await listProviders());
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  function handleCreated() {
    setAdding(false);
    setCredentialWarning(null);
    setSuccess("Provider 已保存；Codex Native 复用当前登录，其余 Credential 不会在界面中回显。");
    void load();
  }

  function handleUpdated() {
    setEditing(null);
    setCredentialWarning(null);
    setSuccess("Provider 已更新；Credential 保持不变。");
    void load();
  }

  async function handleEdit(providerId: string) {
    setActionError(null);
    setSuccess(null);
    try {
      setEditing(await getProvider(providerId));
    } catch (reason: unknown) {
      setActionError(errorMessage(reason));
    }
  }

  async function handleDelete(provider: ProviderSummary) {
    if (!window.confirm(`确定删除 Provider “${provider.name}”吗？此操作不会删除已生成的 Codex 快照。`)) {
      return;
    }
    setDeletingId(provider.id);
    setActionError(null);
    setSuccess(null);
    setCredentialWarning(null);
    try {
      const result = await deleteProvider(provider.id);
      if (result.credentialCleanupPending) {
        setCredentialWarning(
          "Provider 已删除，但 Windows Credential 清理暂未完成。CAS 已记录待办，将在启动或刷新 Provider 时自动重试。",
        );
      } else {
        setSuccess("Provider 及其关联 Credential（如有）已删除。");
      }
      await load();
    } catch (reason: unknown) {
      setActionError(errorMessage(reason));
    } finally {
      setDeletingId(null);
    }
  }

  const panelOpen = adding || editing !== null;

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Providers</span>
          <h1>模型服务来源</h1>
          <p>管理第三方 Responses API，以及复用当前 ChatGPT 登录的 Codex 原生模型。</p>
        </div>
        {!panelOpen && (
          <button
            className="primary-button"
            onClick={() => {
              setAdding(true);
              setSuccess(null);
            }}
          >
            添加 Provider
          </button>
        )}
      </header>

      {success && <div className="success-banner" role="status">{success}</div>}
      {credentialWarning && <div className="warning-banner" role="status">{credentialWarning}</div>}
      {actionError && (
        <div className="inline-error provider-notice" role="alert">
          <strong>Provider 操作失败</strong>
          <span>{actionError}</span>
        </div>
      )}

      {adding && (
        <AddProviderPanel onCancel={() => setAdding(false)} onCreated={handleCreated} />
      )}

      {editing && (
        <EditProviderPanel
          onCancel={() => setEditing(null)}
          onUpdated={handleUpdated}
          provider={editing}
        />
      )}

      {!panelOpen && error && (
        <section className="notice error provider-notice">
          <strong>无法读取 Provider</strong>
          <p>{error}</p>
          <button className="secondary-button" onClick={() => void load()}>
            重试
          </button>
        </section>
      )}

      {!panelOpen && loading && <section className="notice provider-notice">正在读取 Provider…</section>}

      {!panelOpen && !loading && !error && providers.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">P</div>
          <h2>还没有 Provider</h2>
          <p>添加模型 Provider 后，即可为 Codex 子 Agent 分配外部模型。</p>
          <button className="primary-button" onClick={() => setAdding(true)}>
            添加 Provider
          </button>
        </section>
      )}

      {!panelOpen && !loading && !error && providers.length > 0 && (
        <section className="provider-list" aria-label="Provider 列表">
          {providers.map((provider) => (
            <ProviderRow
              deleting={deletingId === provider.id}
              key={provider.id}
              onDelete={() => void handleDelete(provider)}
              onEdit={() => void handleEdit(provider.id)}
              provider={provider}
            />
          ))}
        </section>
      )}
    </>
  );
}

function ProviderRow({
  deleting,
  onDelete,
  onEdit,
  provider,
}: {
  deleting: boolean;
  onDelete: () => void;
  onEdit: () => void;
  provider: ProviderSummary;
}) {
  const ready = provider.status === "READY"
    && (provider.credentialStatus === "CONFIGURED" || provider.credentialStatus === "CODEX_SESSION");
  const label =
    provider.credentialStatus === "CODEX_SESSION"
      ? "Codex session"
      : provider.credentialStatus !== "CONFIGURED"
      ? "Credential missing"
      : provider.status === "DISABLED"
        ? "Disabled"
        : "Ready";

  return (
    <article className="provider-row">
      <ProviderIcon
        className="provider-avatar"
        name={provider.name}
        presetId={provider.presetId}
      />
      <div className="provider-main">
        <div className="provider-name-line">
          <h2>{provider.providerKey}</h2>
          <span className={`result ${ready ? "ready" : "blocked"}`}>{label}</span>
        </div>
        <p>
          <span>{provider.name}</span>
          <span>Responses API</span>
          <span>{provider.providerType === "PRESET" ? "Preset" : "Custom"}</span>
          <span>Cache {provider.cacheSupport}</span>
        </p>
      </div>
      <div className="model-count">
        <strong>{provider.modelCount}</strong>
        <span>Models</span>
      </div>
      <div className="row-actions">
        <IconButton disabled={deleting} icon="edit" label={`编辑 Provider ${provider.name}`} onClick={onEdit} />
        <IconButton
          disabled={deleting}
          icon="trash"
          label={deleting ? `正在删除 Provider ${provider.name}` : `删除 Provider ${provider.name}`}
          loading={deleting}
          onClick={onDelete}
          tone="danger"
        />
      </div>
    </article>
  );
}

function ProviderIcon({
  className,
  name,
  presetId,
}: {
  className: string;
  name: string;
  presetId: string | null;
}) {
  return (
    <span className={className} aria-hidden="true">
      {presetId === "deepseek"
        ? <DeepSeekLogo />
        : presetId === "codex-native"
          ? ">_"
          : name.trim().slice(0, 2).toUpperCase() || "?"}
    </span>
  );
}

function DeepSeekLogo() {
  return (
    <svg className="provider-icon-logo" viewBox="0 0 1391 1024" xmlns="http://www.w3.org/2000/svg">
      <path
        d="M1299.71873948 109.08164852c-12.94676356-6.47485468-18.53640721 5.86654827-26.09973268 12.13814317-2.57756953 2.02376031-4.77807747 4.65435413-6.9785854 7.08168819-18.91788751 20.63528526-41.02017805 34.19182812-69.92725219 32.57311445-42.23384508-2.42733405-78.29772513 11.12773591-110.16384908 44.10589701-6.77679853-40.66668282-29.28560862-64.94444203-63.55255448-80.5232723-17.95608583-8.09209545-36.06388005-16.1856638-48.63358202-33.78825438-8.74900746-12.5431898-11.12773591-26.50330641-15.52727889-40.26016326-2.7823022-8.29535522-5.56460442-16.79249731-14.92044537-18.20942412-10.19244639-1.61871367-14.16337637 7.08168818-18.15934561 14.36516325-15.93232553 29.74073376-22.12880269 62.51710799-21.49692993 95.69705596 1.36684831 74.65672404 32.27117059 134.13819156 93.62469003 176.42358804 6.9800583 4.8546681 8.77551959 9.71080912 6.57501167 16.7910244-4.17271686 14.56695011-9.18056624 28.7303265-13.55506996 43.29727663-2.78377509 9.30723537-6.95501906 11.32952278-16.74241882 7.28347506-33.66158524-14.36516325-62.745407-35.60875491-88.46513227-61.30196805-43.62425973-43.09548975-83.0772755-90.64060097-132.26613965-127.86659668a581.67054343 581.67054343 0 0 0-35.07703915-24.481019c-50.20074429-49.77065841 6.57501167-90.63912807 19.72650787-95.49526908 13.73181759-5.05792788 4.77955037-22.4572587-39.65480261-22.25547183s-85.07599655 15.3770434-136.87041529 35.60875492c-7.58541892 3.03416756-15.55379103 5.25971475-23.69596497 7.08168819-47.03990759-9.10544849-95.84876434-11.12773591-146.85960188-5.26118764-96.02551195 10.92594905-172.73103555 57.25739323-229.12678414 136.36373875-67.72674425 95.09022244-83.68558192 203.13015422-64.13582166 315.82149433 20.48504978 118.76262107 79.88992666 217.09027084 171.13736116 293.9710692 94.6350973 79.71317902 203.61031862 118.76262107 327.93607115 111.27735912 75.51542292-4.45256725 159.57953934-14.77020989 254.41642352-96.71040901 23.92426399 12.13961607 49.03715574 16.99575708 90.66564022 20.63675815 32.09295007 3.03416756 62.97223311-1.61871367 86.87145787-6.67664152 37.45429471-8.09209545 34.84874013-43.49906349 21.3187094-49.97244528-109.7838417-52.19946535-85.68283009-30.95587367-107.58333375-48.15194474 55.78891504-67.37324899 139.85303146-137.3770918 172.73103558-364.17669887 2.60408167-18.00616433 0.40357375-29.33568711 0-43.90263723-0.20325977-8.90218873 1.76894914-12.34287584 11.75960868-13.3532831 27.49014733-3.23742733 54.19671351-10.92594905 78.70277175-24.68280587 71.11440706-39.65480265 99.81822142-104.80250446 106.59649285-182.89844271 1.01188015-11.9363563-0.20178686-24.27775924-12.56970195-30.54935415M679.88691083 811.94067418c-106.36966673-85.37941333-157.98733782-113.5014334-179.30604723-112.28776638-19.92829476 1.21366703-16.33737217 24.48101902-11.96286845 39.65480263 4.5777635 14.97199677 10.57098089 25.2896394 18.94145388 38.44113563 5.76786417 8.70040186 9.76383341 21.64863831-5.78995764 31.35944742-34.26841876 21.64863831-93.82647692-7.28347506-96.60730623-8.69892897-69.3454579-41.67856295-127.33635378-96.710409-168.17978423-171.97249367-39.45154288-72.43117687-62.33888747-150.1220685-66.13306982-233.07267484-1.01188015-20.02992464 4.77955037-27.11161282 24.27923215-30.75408683a235.19659216 235.19659216 0 0 1 77.91771776-2.0222874c108.59668681 16.18713669 201.05631542 65.75453531 278.54394725 144.25552022 44.23256614 44.71125762 77.69089163 98.12439001 112.18613649 150.32385536 36.64567431 55.43394691 76.0986901 108.24024576 126.2994344 151.53604948 17.72778682 15.17378365 31.86465106 26.7065662 45.41972101 35.20370828-40.84343042 4.65435413-108.99878765 5.66476139-155.60860934-31.96628093m51.00936468-334.83953884c0-8.90218873 6.9815312-15.98387693 15.75557788-15.98387693q2.98408908 0.05155138 5.36134465 1.01188016a15.85720779 15.85720779 0 0 1 10.16740716 14.97199677 15.78061715 15.78061715 0 0 1-15.73053865 15.98387692 15.60386953 15.60386953 0 0 1-15.55379104-15.98387692m158.39238445 82.95060636c-10.1423679 4.24930749-20.30830215 7.89178146-30.09570189 8.29535522-15.12370514 0.81009328-31.66286419-5.46150162-40.61513143-13.15002333-13.96011661-11.93782919-23.92573689-18.61447073-28.09845373-39.45301577-1.79546129-8.90218873-0.81009328-22.65904557 0.78505404-30.54935414 3.59092258-16.99575708-0.40504665-27.92023321-12.13961607-37.8343021-9.55910075-8.09356835-21.72522894-10.31911553-35.07703915-10.31911553-4.98281013 0-9.55910075-2.22407429-12.94823646-4.04604772a13.27669246 13.27669246 0 0 1-5.76639126-18.61299785c1.39041466-2.8323807 8.16868608-9.71228202 9.76236049-10.92594903 18.13136058-10.52090241 39.04649623-7.08021529 58.36795747 0.81009328 17.93104659 7.48526194 31.46107731 21.24359167 51.01083759 40.66668278 19.92829476 23.46913885 23.51921733 29.94399353 34.84874012 47.54511124 8.97877936 13.75685685 17.14746545 27.92023321 22.71206985 44.1044241 3.41270206 10.11732865-0.98684091 18.41121097-12.74644957 23.46913885"
        fill="currentColor"
      />
    </svg>
  );
}

function ModelsPage({ onOpenProviders }: { onOpenProviders: () => void }) {
  const [models, setModels] = useState<ModelSummary[]>([]);
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [editing, setEditing] = useState<ModelSummary | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [testingId, setTestingId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [modelList, providerList] = await Promise.all([
        listModels(),
        listProviders(),
      ]);
      setModels(modelList);
      setProviders(providerList);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  function handleCreated() {
    setAdding(false);
    setSuccess("Model 已添加；未知能力保持 UNKNOWN，不会被自动升级。");
    void load();
  }

  function handleUpdated() {
    setEditing(null);
    setSuccess("Model 已更新。");
    void load();
  }

  async function handleDelete(model: ModelSummary) {
    if (!window.confirm(`确定删除 Model “${model.displayName}”吗？`)) return;
    setDeletingId(model.id);
    setActionError(null);
    setSuccess(null);
    try {
      await deleteModel(model.id);
      setSuccess("Model 已删除。");
      await load();
    } catch (reason: unknown) {
      setActionError(errorMessage(reason));
    } finally {
      setDeletingId(null);
    }
  }

  async function handleTest(model: ModelSummary) {
    setTestingId(model.id);
    setActionError(null);
    setSuccess(null);
    try {
      const result = await testModelConnection(model.id);
      const latency = result.latencyMs === null ? "" : ` · ${result.latencyMs} ms`;
      if (result.status === "SUCCESS") {
        setSuccess(`${model.displayName} 测试成功${latency}：${result.message}`);
      } else {
        const attribution = result.status === "PROTOCOL_ERROR"
          ? "可能原因：Model ID 不存在或拼写错误、Provider Base URL 错误、API Key 无效。"
          : "";
        setActionError(
          `${model.displayName} · ${modelTestStatusLabel(result.status)}${latency}：${result.message}${attribution ? ` ${attribution}` : ""}`,
        );
      }
      await load();
    } catch (reason: unknown) {
      setActionError(errorMessage(reason));
    } finally {
      setTestingId(null);
    }
  }

  const enabledProviders = providers.filter((provider) => provider.enabled);
  const addableProviders = enabledProviders.filter(
    (provider) => provider.presetId !== "codex-native",
  );
  const panelOpen = adding || editing !== null;

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Models</span>
          <h1>可绑定模型</h1>
          <p>验证 Responses API、Function Calling 工具闭环与 Codex Multi-Agent 兼容性。</p>
        </div>
        {!panelOpen && addableProviders.length > 0 && (
          <button
            className="primary-button"
            onClick={() => {
              setAdding(true);
              setSuccess(null);
            }}
          >
            添加 Model
          </button>
        )}
      </header>

      {success && <div className="success-banner" role="status">{success}</div>}
      {actionError && (
        <div className="inline-error provider-notice" role="alert">
          <strong>Model 操作失败</strong>
          <span>{actionError}</span>
        </div>
      )}

      {adding && (
        <AddModelPanel
          onCancel={() => setAdding(false)}
          onCreated={handleCreated}
          providers={addableProviders}
        />
      )}

      {editing && (
        <EditModelPanel
          model={editing}
          onCancel={() => setEditing(null)}
          onUpdated={handleUpdated}
        />
      )}

      {!panelOpen && error && (
        <section className="notice error provider-notice">
          <strong>无法读取 Model</strong>
          <p>{error}</p>
          <button className="secondary-button" onClick={() => void load()}>重试</button>
        </section>
      )}

      {!panelOpen && loading && <section className="notice provider-notice">正在读取 Model…</section>}

      {!panelOpen && !loading && !error && providers.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">M</div>
          <h2>请先添加 Provider</h2>
          <p>Model 必须属于一个已保存的 Responses Provider。</p>
          <button className="primary-button" onClick={onOpenProviders}>前往 Providers</button>
        </section>
      )}

      {!panelOpen && !loading && !error && providers.length > 0 && models.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">M</div>
          <h2>还没有 Model</h2>
          <p>手动添加 Provider 实际接受的 Model ID；能力信息默认保持 Unknown。</p>
          {addableProviders.length > 0 ? (
            <button className="primary-button" onClick={() => setAdding(true)}>添加 Model</button>
          ) : (
            <button className="secondary-button" onClick={onOpenProviders}>启用 Provider</button>
          )}
        </section>
      )}

      {!panelOpen && !loading && !error && models.length > 0 && (
        <section className="model-table-wrap" aria-label="Model 列表">
          <table className="model-table">
            <thead>
              <tr>
                <th>Provider</th>
                <th>Model</th>
                <th>
                  <span className="table-heading-with-info">
                    Compatibility
                    <InfoTip label="CAS 对模型接入方式与 Codex 工具调用兼容性的判断；聚焦具体状态可查看原因。" />
                  </span>
                </th>
                <th>
                  <span className="table-heading-with-info">
                    Context
                    <InfoTip label="模型可供 Codex 使用的上下文窗口；Unknown 表示尚未配置。" />
                  </span>
                </th>
                <th>
                  <span className="table-heading-with-info">
                    Lifecycle
                    <InfoTip label="模型当前是否允许用于新 Agent 配置；Disabled 不影响已有历史记录。" />
                  </span>
                </th>
                <th>
                  <span className="table-heading-with-info">
                    Verification
                    <InfoTip label="最近一次 Responses API 与 Function Calling 工具闭环测试结果。" />
                  </span>
                </th>
                <th aria-label="操作" />
              </tr>
            </thead>
            <tbody>
              {models.map((model) => (
                <ModelRow
                  deleting={deletingId === model.id}
                  key={model.id}
                  model={model}
                  onDelete={() => void handleDelete(model)}
                  onEdit={() => {
                    setActionError(null);
                    setSuccess(null);
                    setEditing(model);
                  }}
                  onTest={() => void handleTest(model)}
                  testing={testingId === model.id}
                />
              ))}
            </tbody>
          </table>
        </section>
      )}
    </>
  );
}

function ModelRow({
  deleting,
  model,
  onDelete,
  onEdit,
  onTest,
  testing,
}: {
  deleting: boolean;
  model: ModelSummary;
  onDelete: () => void;
  onEdit: () => void;
  onTest: () => void;
  testing: boolean;
}) {
  const lifecycle = !model.enabled
    ? "Disabled"
    : model.lifecycle === "ACTIVE"
      ? "Active"
      : model.lifecycle === "UNKNOWN"
        ? "Unknown"
        : model.lifecycle;
  const verification = model.lastTestStatus === null
    ? model.providerPresetId === "codex-native" ? "Codex Native" : "Untested"
    : model.lastTestStatus === "SUCCESS"
      ? "Passed"
      : modelTestStatusLabel(model.lastTestStatus);
  const verificationClass = model.lastTestStatus === null
    ? model.providerPresetId === "codex-native" ? "passed" : "untested"
    : model.lastTestStatus === "SUCCESS"
      ? "passed"
      : "failed";
  const lifecycleTitle = lifecycleDescription(model);
  const verificationTitle = [
    model.providerPresetId === "codex-native"
      ? "原生模型复用当前 Codex 的 ChatGPT 登录会话；实际可用性由当前账号与 Codex 客户端决定。"
      : verificationDescription(model.lastTestStatus),
    model.lastTestedAt === null
      ? null
      : `最近测试：${new Date(model.lastTestedAt).toLocaleString()}${
        model.lastTestLatencyMs === null ? "" : ` · ${model.lastTestLatencyMs} ms`
      }`,
  ].filter(Boolean).join("\n");
  return (
    <tr className={!model.enabled ? "model-disabled" : undefined}>
      <td>
        <code>{model.providerKey}</code>
        <small>{model.providerName}</small>
      </td>
      <td>
        <strong>{model.displayName}</strong>
        <span className="model-id-line">
          <Tooltip content={model.modelId} focusable label={`Model ID：${model.modelId}`}>
            <code>{model.modelId}</code>
          </Tooltip>
          <CopyIconButton label={`复制 Model ID ${model.modelId}`} value={model.modelId} />
        </span>
      </td>
      <td>
        <StatusBadge
          className={`compatibility ${model.compatibility.toLowerCase()}`}
          description={compatibilityDescription(model.compatibility)}
          icon={model.compatibility === "UNKNOWN" ? "alert" : model.compatibility === "UNSUPPORTED" || model.compatibility === "GATEWAY_REQUIRED" ? "x-circle" : "check"}
          label={compatibilityLabel(model.compatibility)}
        />
      </td>
      <td>{model.contextWindow ? formatTokenCount(model.contextWindow) : "Unknown"}</td>
      <td>
        <StatusBadge
          className={lifecycle === "Active" ? "status-text ready" : "status-text"}
          description={lifecycleTitle}
          icon={lifecycle === "Active" ? "check" : "alert"}
          label={lifecycle}
        />
      </td>
      <td>
        <StatusBadge
          className={`status-text ${verificationClass}`}
          description={verificationTitle}
          icon={verificationClass === "passed" ? "check" : verificationClass === "failed" ? "x-circle" : "alert"}
          label={verification}
        />
      </td>
      <td>
        <div className="row-actions">
          <Tooltip
            content={model.providerPresetId === "codex-native"
              ? "Codex 原生模型复用当前登录会话，无需第三方 Responses API 测试。"
              : "验证 Responses API 与 Function Calling 工具闭环。"}
            focusable={model.providerPresetId === "codex-native"}
            label="Model 测试说明"
          >
            <button
              className="secondary-button"
              disabled={deleting || testing || model.providerPresetId === "codex-native"}
              onClick={onTest}
            >
              {model.providerPresetId === "codex-native" ? "原生" : testing ? "测试中…" : "测试"}
            </button>
          </Tooltip>
          <IconButton
            disabled={deleting || testing}
            icon="edit"
            label={`编辑 Model ${model.displayName}`}
            onClick={onEdit}
          />
          <IconButton
            disabled={deleting || testing}
            icon="trash"
            label={deleting ? `正在删除 Model ${model.displayName}` : `删除 Model ${model.displayName}`}
            loading={deleting}
            onClick={onDelete}
            tone="danger"
          />
        </div>
      </td>
    </tr>
  );
}

function StatusBadge({
  className,
  description,
  icon,
  label,
}: {
  className: string;
  description: string;
  icon?: IconName;
  label: string;
}) {
  return (
    <Tooltip content={description} focusable label={`${label}：${description}`}>
      <span className={className}>
        {icon && <UiIcon name={icon} />}
        <span>{label}</span>
      </span>
    </Tooltip>
  );
}

function AddModelPanel({
  providers,
  onCancel,
  onCreated,
}: {
  providers: ProviderSummary[];
  onCancel: () => void;
  onCreated: () => void;
}) {
  const [providerId, setProviderId] = useState(providers[0]?.id ?? "");
  const [modelId, setModelId] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [contextWindow, setContextWindow] = useState("");
  const [modelIdError, setModelIdError] = useState<string | null>(null);
  const [contextError, setContextError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const parsedContextWindow = contextWindow.trim() === "" ? null : Number(contextWindow);
    if (modelId.length === 0 || modelId.trim() !== modelId || /[\r\n]/.test(modelId)) {
      setModelIdError("Model ID 不能为空，首尾不能有空格，且不能包含换行。");
      return;
    }
    if (parsedContextWindow !== null && (!Number.isSafeInteger(parsedContextWindow) || parsedContextWindow <= 0)) {
      setContextError("Context Window 必须是正整数。");
      return;
    }
    setSaving(true);
    setError(null);
    setModelIdError(null);
    setContextError(null);
    try {
      await addModel({
        providerId,
        modelId,
        displayName: displayName.trim() || null,
        contextWindow: parsedContextWindow,
      });
      onCreated();
    } catch (reason: unknown) {
      setError(errorMessage(reason));
      if (errorCode(reason) === "MODEL_ID_CONFLICT" || errorField(reason) === "modelId") {
        setModelIdError(errorMessage(reason));
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Add Model</span>
          <h2>添加 Custom Model</h2>
          <p>Model ID 会原样发送给所选 Provider。</p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={onCancel}>取消</button>
      </div>

      <form className="provider-form" onSubmit={submit}>
        <label className="field full-width">
          <span>Provider</span>
          <select
            onChange={(event) => setProviderId(event.target.value)}
            required
            value={providerId}
          >
            {providers.map((provider) => (
              <option key={provider.id} value={provider.id}>
                {provider.providerKey} — {provider.name}
              </option>
            ))}
          </select>
          <small>Model 与 Provider 的组合在 CAS 内唯一。</small>
        </label>

        <label className="field">
          <span>Model ID</span>
          <input
            aria-invalid={modelIdError ? true : undefined}
            autoFocus
            maxLength={200}
            onChange={(event) => {
              setModelId(event.target.value);
              setModelIdError(null);
            }}
            placeholder="provider/model-name"
            required
            value={modelId}
          />
          <small className={modelIdError ? "field-error" : undefined}>
            {modelIdError ?? "必须与 Provider API 接受的标识完全一致。"}
          </small>
        </label>

        <label className="field">
          <span>Display Name <em>Optional</em></span>
          <input
            maxLength={160}
            onChange={(event) => setDisplayName(event.target.value)}
            placeholder={modelId || "Model name"}
            value={displayName}
          />
          <small>留空时使用 Model ID。</small>
        </label>

        <label className="field">
          <span>Context Window <em>Optional</em></span>
          <input
            aria-invalid={contextError ? true : undefined}
            inputMode="numeric"
            min={1}
            onChange={(event) => {
              setContextWindow(event.target.value);
              setContextError(null);
            }}
            placeholder="128000"
            step={1}
            type="number"
            value={contextWindow}
          />
          {contextError
            ? <small className="field-error">{contextError}</small>
            : <small>留空表示 Unknown；该值会标记为用户声明，不代表 CAS 已验证。</small>}
        </label>

        <div className="unknown-note full-width">
          <strong>Compatibility: Unknown</strong>
          <span>手工添加不会声明未经验证的 Capability 或 Codex 兼容性。</span>
        </div>

        {error && (
          <div className="inline-error full-width" role="alert">
            <strong>Model 保存失败</strong>
            <span>{error}</span>
          </div>
        )}

        <div className="form-actions full-width">
          <button className="secondary-button" disabled={saving} onClick={onCancel} type="button">
            取消
          </button>
          <button className="primary-button" disabled={saving} type="submit">
            {saving ? "保存中…" : "保存 Model"}
          </button>
        </div>
      </form>
    </section>
  );
}

function EditModelPanel({
  model,
  onCancel,
  onUpdated,
}: {
  model: ModelSummary;
  onCancel: () => void;
  onUpdated: () => void;
}) {
  const [displayName, setDisplayName] = useState(model.displayName);
  const [enabled, setEnabled] = useState(model.enabled);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await updateModel(model.id, displayName);
      if (enabled !== model.enabled) await setModelEnabled(model.id, enabled);
      onUpdated();
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Edit Model</span>
          <h2>编辑 {model.displayName}</h2>
          <p><code>{model.modelId}</code> 是稳定身份，不能在编辑时更换。</p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={onCancel}>取消</button>
      </div>

      <form className="provider-form" onSubmit={submit}>
        <div className="field full-width">
          <span>Provider</span>
          <div className="static-value">
            <code>{model.providerKey}</code> — {model.providerName}
          </div>
          <small>Model 绑定的 Provider 不可在编辑时更换。</small>
        </div>

        <label className="field full-width">
          <span>Display Name</span>
          <input
            autoFocus
            maxLength={160}
            onChange={(event) => setDisplayName(event.target.value)}
            required
            value={displayName}
          />
        </label>

        <label className="enabled-field full-width">
          <input
            checked={enabled}
            onChange={(event) => setEnabled(event.target.checked)}
            type="checkbox"
          />
          <span>
            <strong>Enabled</strong>
            <small>停用后不会进入新的 Agent 绑定和配置生成。</small>
          </span>
        </label>

        {error && (
          <div className="inline-error full-width" role="alert">
            <strong>Model 更新失败</strong>
            <span>{error}</span>
          </div>
        )}

        <div className="form-actions full-width">
          <button className="secondary-button" disabled={saving} onClick={onCancel} type="button">取消</button>
          <button className="primary-button" disabled={saving} type="submit">
            {saving ? "保存中…" : "保存修改"}
          </button>
        </div>
      </form>
    </section>
  );
}

function compatibilityLabel(value: ModelSummary["compatibility"]): string {
  const labels = {
    NATIVE: "Native",
    COMPATIBLE: "Compatible",
    GATEWAY_REQUIRED: "Gateway required",
    UNSUPPORTED: "Unsupported",
    UNKNOWN: "Unknown",
  } as const;
  return labels[value];
}

function compatibilityDescription(value: ModelSummary["compatibility"]): string {
  const descriptions = {
    NATIVE: "有较强证据确认该模型针对 Codex 或其所需行为完成原生适配。",
    COMPATIBLE: "CAS已验证Responses API 和 Function Calling 工具闭环，可用于Codex Agent。（但请根据Codex中实际提示为准）",
    GATEWAY_REQUIRED: "当前不能直接用于 Codex，需要协议转换网关。",
    UNSUPPORTED: "存在明确的不兼容问题，不能用于 Codex Agent。",
    UNKNOWN: "CAS 暂无足够证据判断该模型是否兼容 Codex Agent。",
  } as const;
  return descriptions[value];
}

function lifecycleDescription(model: ModelSummary): string {
  if (!model.enabled) return "该模型已停用，不会进入新的 Agent 绑定或配置生成。";
  const descriptions = {
    ACTIVE: "当前正式可用的模型。",
    PREVIEW: "模型可以使用，但接口、行为或模型身份仍可能变化。",
    DEPRECATED: "模型仍可调用，但 Provider 已宣布未来移除，不建议用于新绑定。",
    UNKNOWN: "CAS 尚未获得该模型生命周期的可靠信息。",
  } as const;
  return descriptions[model.lifecycle];
}

function verificationDescription(value: ModelSummary["lastTestStatus"]): string {
  if (value === null) return "尚未使用当前 Provider Credential 发起 Responses Function Calling 工具闭环测试。";
  const descriptions = {
    SUCCESS: "最近一次 Responses API 与 Function Calling 工具闭环测试通过。",
    CREDENTIAL_MISSING: "Provider Credential 不存在或已从系统凭据库移除。",
    AUTH_FAILED: "Provider 拒绝了当前 Credential。",
    MODEL_NOT_FOUND: "Provider 不识别当前 Model ID。",
    RATE_LIMITED: "Provider 在最近一次测试时触发了限流。",
    PROTOCOL_ERROR: "Endpoint 返回异常（非 2xx 或响应无法解析），请检查 Model ID、Provider Base URL 与 API Key。",
    UNREACHABLE: "最近一次测试无法连接 Provider，或请求超时。",
    SERVER_ERROR: "Provider 在最近一次测试时返回服务端错误。",
  } as const;
  return descriptions[value];
}

function modelTestStatusLabel(value: Exclude<Awaited<ReturnType<typeof testModelConnection>>["status"], "SUCCESS">): string {
  const labels = {
    CREDENTIAL_MISSING: "Credential missing",
    AUTH_FAILED: "Auth failed",
    MODEL_NOT_FOUND: "Model not found",
    RATE_LIMITED: "Rate limited",
    PROTOCOL_ERROR: "Responses protocol error",
    UNREACHABLE: "Unreachable",
    SERVER_ERROR: "Server error",
  } as const;
  return labels[value];
}

function formatTokenCount(value: number): string {
  return new Intl.NumberFormat("en-US", { notation: "compact" }).format(value);
}

type ProviderKind = "codex-native" | "deepseek" | "custom";

interface DocumentationReference {
  readonly name: string;
  readonly support: string;
  readonly links: readonly { readonly label: string; readonly url: string }[];
  readonly description?: string;
}

const responsesApiReferences: DocumentationReference[] = [
  { name: "DeepSeek", support: "Responses API", links: [{ label: "Responses API 文档", url: "https://api-docs.deepseek.com/zh-cn/guides/responses_api" }] },
  { name: "阿里云百炼", support: "OpenAI 兼容", links: [
    { label: "Responses API 文档", url: "https://help.aliyun.com/zh/model-studio/compatibility-with-openai-responses-api?mode=pure" },
    { label: "模型与工具能力", url: "https://help.aliyun.com/en/model-studio/text-generation-model/" },
  ], description: "用于 CAS/Codex Agent 时，应选择模型能力表中标明支持内置工具的模型；不支持内置工具的模型无法保证完整的工具调用与 Agent 工具闭环，最终以 CAS Model 工具闭环测试结果为准。" },
  { name: "腾讯云 TokenHub", support: "兼容转换", links: [{ label: "Responses API 文档", url: "https://cloud.tencent.com.cn/document/product/1823/133813" }] },
  { name: "Xiaomi MiMo", support: "Responses API", links: [{ label: "Responses API 文档", url: "https://mimo.mi.com/docs/zh-CN/api/chat/responses" }] },
  { name: "火山引擎 · 火山方舟", support: "Responses API", links: [{ label: "Responses API 文档", url: "https://docs.volcengine.com/docs/6492/2241837?lang=zh" }] },
  { name: "Infercom", support: "Responses API", links: [{ label: "Responses API 文档", url: "https://docs.infercom.ai/en/features/responses-api" }] },
  { name: "MiniMax", support: "Responses API", links: [{ label: "Responses API 文档", url: "https://platform.minimax.io/docs/api-reference/responses-create" }] },
];

const cacheRetentionReferences: DocumentationReference[] = [
  {
    name: "DeepSeek",
    support: "自动缓存 · 通常数小时至数天",
    links: [{ label: "上下文硬盘缓存", url: "https://api-docs.deepseek.com/zh-cn/guides/kv_cache/" }],
    description: "官方说明为尽力而为，缓存不再使用后通常会在数小时至数天后清理。",
  },
  {
    name: "阿里百炼（阿里云）",
    support: "显式 5 分钟 · 隐式无固定时长",
    links: [{
      label: "Context Cache",
      url: "https://www.alibabacloud.com/help/zh/model-studio/context-cache?spm=a2c63.p38356.help-menu-search-2400256.d_1",
    }],
    description: "显式缓存命中后会重置 5 分钟；隐式缓存由系统定期清理。",
  },
  {
    name: "火山方舟",
    support: "以官方文档当前说明为准",
    links: [{
      label: "上下文缓存",
      url: "https://ark.volcengine.com/region:cn-beijing/docs/82379/1602228?lang=zh",
    }],
  },
  {
    name: "MiniMax",
    support: "被动缓存动态 · 主动缓存 5 分钟",
    links: [
      {
        label: "Prompt 缓存",
        url: "https://platform.minimaxi.com/docs/api-reference/text-prompt-caching",
      },
      {
        label: "Anthropic 主动缓存",
        url: "https://platform.minimaxi.com/docs/api-reference/anthropic-api-compatible-cache",
      },
    ],
    description: "被动缓存会按系统负载调整；Anthropic 兼容主动缓存命中后刷新 5 分钟生命周期。",
  },
  {
    name: "ChatGPT（OpenAI）",
    support: "无官方明示 · 社区讨论约 30 分钟",
    links: [{
      label: "OpenAI 社区讨论",
      url: "https://community.openai.com/t/gpt-5-6-prompt-caching-fails-on-partial-prefixes/1386887/6?utm_source=chatgpt.com",
    }],
    description: "该时长来自社区讨论，不是 OpenAI 官方承诺，不应作为保证值。",
  },
];

interface ProviderFormState {
  providerKey: string;
  name: string;
  baseUrl: string;
  secret: string;
  enabled: boolean;
}

const formDefaults: Record<ProviderKind, ProviderFormState> = {
  "codex-native": {
    providerKey: "codex-native",
    name: "Codex Native (ChatGPT)",
    baseUrl: "https://api.openai.com/v1/",
    secret: "",
    enabled: true,
  },
  deepseek: {
    providerKey: "deepseek",
    name: "DeepSeek",
    baseUrl: "https://api.deepseek.com/",
    secret: "",
    enabled: true,
  },
  custom: {
    providerKey: "",
    name: "",
    baseUrl: "https://",
    secret: "",
    enabled: true,
  },
};

function EditProviderPanel({
  onCancel,
  onUpdated,
  provider,
}: {
  onCancel: () => void;
  onUpdated: () => void;
  provider: ProviderDetailResponse;
}) {
  const [name, setName] = useState(provider.name);
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl);
  const [enabled, setEnabled] = useState(provider.enabled);
  const [cacheSupport, setCacheSupport] = useState<ProviderCacheSupport>(
    provider.cacheProfile.cacheSupport,
  );
  const [retentionType, setRetentionType] = useState<ProviderCacheRetentionType>(
    provider.cacheProfile.retentionType,
  );
  const [retentionMinutes, setRetentionMinutes] = useState(
    provider.cacheProfile.retentionHintSeconds === null
      ? ""
      : String(provider.cacheProfile.retentionHintSeconds / 60),
  );
  const [cacheSource, setCacheSource] = useState(provider.cacheProfile.source ?? "");
  const [cacheVerifiedAt, setCacheVerifiedAt] = useState(
    provider.cacheProfile.lastVerifiedAt ?? "",
  );
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    let confirmOriginChange = false;
    try {
      confirmOriginChange = new URL(provider.baseUrl).origin !== new URL(baseUrl).origin;
    } catch {
      // 具体 URL 校验由后端统一执行。
    }
    if (
      confirmOriginChange &&
      !window.confirm("Endpoint Origin 已变化。现有 Credential 将发送到新服务，确定继续吗？")
    ) {
      return;
    }

    setSaving(true);
    setError(null);
    try {
      await updateProvider({
        providerId: provider.id,
        name,
        baseUrl,
        enabled,
        confirmOriginChange,
        cacheProfile: {
          cacheSupport,
          retentionType,
          retentionHintSeconds: retentionType === "UNKNOWN"
            ? null
            : Math.round(Number(retentionMinutes) * 60),
          source: cacheSource.trim() || null,
          lastVerifiedAt: cacheVerifiedAt || null,
        },
      });
      onUpdated();
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Edit Provider</span>
          <h2>编辑 {provider.name}</h2>
          <p>
            {provider.presetId === "codex-native"
              ? "该预设复用当前 Codex 的 ChatGPT 登录会话，不保存 API Key。"
              : "Credential 保持原值，不会读取或回显。"}
          </p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={onCancel}>取消</button>
      </div>

      <form className="provider-form" onSubmit={submit}>
        <label className="field">
          <span>Name</span>
          <input
            autoFocus
            maxLength={120}
            onChange={(event) => setName(event.target.value)}
            required
            value={name}
          />
        </label>

        <div className="field">
          <span>Provider Key</span>
          <div className="static-value"><code>{provider.providerKey}</code></div>
          <small>稳定身份不可修改。</small>
        </div>

        {provider.presetId === "codex-native" ? (
          <div className="field full-width">
            <span>Authentication</span>
            <div className="static-value">Current Codex / ChatGPT session</div>
            <small>可用模型与配额由当前登录账号和 Codex 客户端决定。</small>
          </div>
        ) : (
          <label className="field full-width">
            <span>Base URL</span>
            <input
              inputMode="url"
              maxLength={2048}
              onChange={(event) => setBaseUrl(event.target.value)}
              required
              type="url"
              value={baseUrl}
            />
            <small>跨 Origin 修改会要求额外确认；远程地址必须使用 HTTPS。</small>
          </label>
        )}

        <label className="enabled-field full-width">
          <input
            checked={enabled}
            onChange={(event) => setEnabled(event.target.checked)}
            type="checkbox"
          />
          <span>
            <strong>Enabled</strong>
            <small>停用后，该 Provider 及其 Model 不进入配置生成。</small>
          </span>
        </label>

        <section className="cache-profile-editor full-width" aria-labelledby="cache-profile-title">
          <header>
            <div>
              <strong id="cache-profile-title">Provider Cache Profile</strong>
              <small>仅作为 Thread 复用软提示，不作为请求正确性的依赖。</small>
            </div>
          </header>
          <div className="cache-profile-grid">
            <label className="field">
              <span>Cache Support</span>
              <select
                onChange={(event) => {
                  const value = event.target.value as ProviderCacheSupport;
                  setCacheSupport(value);
                  if (value !== "SUPPORTED") {
                    setRetentionType("UNKNOWN");
                    setRetentionMinutes("");
                  }
                }}
                value={cacheSupport}
              >
                <option value="UNKNOWN">Unknown</option>
                <option value="SUPPORTED">Supported</option>
                <option value="UNSUPPORTED">Unsupported</option>
              </select>
            </label>
            <label className="field">
              <span>Retention Type</span>
              <select
                disabled={cacheSupport !== "SUPPORTED"}
                onChange={(event) => {
                  const value = event.target.value as ProviderCacheRetentionType;
                  setRetentionType(value);
                  if (value === "UNKNOWN") setRetentionMinutes("");
                }}
                value={retentionType}
              >
                <option value="UNKNOWN">Unknown</option>
                <option value="APPROXIMATE">Approximate</option>
                <option value="GUARANTEED">Guaranteed</option>
              </select>
            </label>
            <label className="field">
              <span>Retention Hint（分钟）</span>
              <input
                disabled={retentionType === "UNKNOWN"}
                min={1}
                onChange={(event) => setRetentionMinutes(event.target.value)}
                required={retentionType !== "UNKNOWN"}
                step={1}
                type="number"
                value={retentionMinutes}
              />
            </label>
            <label className="field">
              <span>Last Verified</span>
              <input
                onChange={(event) => setCacheVerifiedAt(event.target.value)}
                type="date"
                value={cacheVerifiedAt}
              />
            </label>
            <label className="field full-width">
              <span>Source</span>
              <input
                maxLength={2048}
                onChange={(event) => setCacheSource(event.target.value)}
                placeholder="官方文档 URL 或内部验证说明"
                value={cacheSource}
              />
            </label>
          </div>
        </section>

        {error && (
          <div className="inline-error full-width" role="alert">
            <strong>Provider 更新失败</strong>
            <span>{error}</span>
          </div>
        )}

        <div className="form-actions full-width">
          <button className="secondary-button" disabled={saving} onClick={onCancel} type="button">取消</button>
          <button className="primary-button" disabled={saving} type="submit">
            {saving ? "保存中…" : "保存修改"}
          </button>
        </div>
      </form>
    </section>
  );
}

function AddProviderPanel({
  onCancel,
  onCreated,
}: {
  onCancel: () => void;
  onCreated: () => void;
}) {
  const [kind, setKind] = useState<ProviderKind>("custom");
  const [form, setForm] = useState<ProviderFormState>(formDefaults.custom);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [invalidField, setInvalidField] = useState<string | null>(null);
  const [invalidFieldMessage, setInvalidFieldMessage] = useState<string | null>(null);
  const [referenceOpen, setReferenceOpen] = useState(false);

  function selectKind(selected: ProviderKind) {
    if (selected === kind) return;
    setKind(selected);
    setForm({ ...formDefaults[selected] });
    setError(null);
    setInvalidField(null);
    setInvalidFieldMessage(null);
  }

  function cancel() {
    setForm((current) => ({ ...current, secret: "" }));
    onCancel();
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!/^[a-z0-9][a-z0-9_-]*$/.test(form.providerKey)) {
      setInvalidField("providerKey");
      setInvalidFieldMessage(
        "Provider Key 不能包含空格；必须以小写字母或数字开头，且只能包含小写字母、数字、-、_。",
      );
      return;
    }
    setSaving(true);
    setError(null);
    setInvalidField(null);
    setInvalidFieldMessage(null);
    const request: ProviderCreateRequest = {
      providerKey: form.providerKey,
      name: form.name,
      presetId: kind === "custom" ? null : kind,
      baseUrl: form.baseUrl,
      protocol: "RESPONSES",
      auth: kind === "codex-native"
        ? { strategy: "NONE" }
        : { strategy: "OS_SECRET_HELPER", secret: form.secret },
      enabled: form.enabled,
    };

    let created = false;
    try {
      await createProvider(request);
      created = true;
    } catch (reason: unknown) {
      setError(errorMessage(reason));
      setInvalidField(errorField(reason));
      setInvalidFieldMessage(
        errorCode(reason) === "PROVIDER_KEY_CONFLICT" ? errorMessage(reason) : null,
      );
    } finally {
      if (request.auth.strategy === "OS_SECRET_HELPER") request.auth.secret = "";
      setForm((current) => ({ ...current, secret: "" }));
      setSaving(false);
    }
    if (created) onCreated();
  }

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Add Provider</span>
          <h2>
            {kind === "codex-native"
              ? "添加 Codex Native"
              : kind === "deepseek"
                ? "添加 DeepSeek"
                : "添加 Custom Responses Provider"}
          </h2>
          <p>
            {kind === "codex-native"
              ? "复用当前 Codex 的 ChatGPT 登录，无需 API Key。"
              : "Credential 将保存到 Windows Credential Manager。"}
          </p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={cancel}>取消</button>
      </div>

      {referenceOpen && (
        <ProviderResponsesReferenceDialog onClose={() => setReferenceOpen(false)} />
      )}

      <form className="provider-form" onSubmit={submit}>
        <fieldset className="provider-preset-selector">
          <legend>Provider Preset</legend>
          <div className="preset-grid">
            <button
              aria-pressed={kind === "codex-native"}
              className={`preset-option ${kind === "codex-native" ? "selected" : ""}`}
              onClick={() => selectKind("codex-native")}
              type="button"
            >
              <ProviderIcon className="preset-icon" name="Codex Native" presetId="codex-native" />
              <span className="preset-name">Codex Native</span>
            </button>
            <button
              aria-pressed={kind === "custom"}
              className={`preset-option ${kind === "custom" ? "selected" : ""}`}
              onClick={() => selectKind("custom")}
              type="button"
            >
              <ProviderIcon className="preset-icon placeholder" name="自定义" presetId={null} />
              <span className="preset-name">自定义</span>
            </button>
            <button
              aria-pressed={kind === "deepseek"}
              className={`preset-option ${kind === "deepseek" ? "selected" : ""}`}
              onClick={() => selectKind("deepseek")}
              type="button"
            >
              <ProviderIcon className="preset-icon" name="DeepSeek" presetId="deepseek" />
              <span className="preset-name">DeepSeek</span>
            </button>
          </div>
          <div className="preset-helper-row">
            <small>Codex 原生登录 · 自定义 · 官方预设。</small>
            <button className="reference-link" onClick={() => setReferenceOpen(true)} type="button">
              <UiIcon name="book" />
              API 支持参考
            </button>
          </div>
        </fieldset>

        <label className="field">
          <span>Name</span>
          <input
            aria-invalid={invalidField === "name"}
            autoFocus
            maxLength={120}
            onChange={(event) => {
              setForm({ ...form, name: event.target.value });
              if (invalidField === "name") setInvalidField(null);
            }}
            required
            value={form.name}
          />
          <small className={invalidField === "name" ? "field-error" : undefined}>
            {invalidField === "name" ? "Name 不能为空，且最多 120 个字符。" : "Provider 在 CAS 中显示的名称。"}
          </small>
        </label>

        <label className="field">
          <span>Provider Key</span>
          <input
            aria-invalid={invalidField === "providerKey"}
            maxLength={64}
            onChange={(event) => {
              const providerKey = event.target.value;
              setForm({ ...form, providerKey });
              if (providerKey.length > 0 && !/^[a-z0-9][a-z0-9_-]*$/.test(providerKey)) {
                setInvalidField("providerKey");
                setInvalidFieldMessage(
                  "Provider Key 不能包含空格；必须以小写字母或数字开头，且只能包含小写字母、数字、-、_。",
                );
              } else if (invalidField === "providerKey") {
                setInvalidField(null);
                setInvalidFieldMessage(null);
              }
            }}
            required
            value={form.providerKey}
          />
          <small className={invalidField === "providerKey" ? "field-error" : undefined}>
            {invalidField === "providerKey"
              ? invalidFieldMessage ?? "必须以小写字母或数字开头，且只能包含小写字母、数字、-、_。"
              : "稳定标识；仅允许小写字母、数字、`-` 和 `_`。"}
          </small>
        </label>

        {kind === "codex-native" ? (
          <div className="field full-width">
            <span>Authentication</span>
            <div className="static-value">Current Codex / ChatGPT session</div>
            <small>将自动添加 GPT-5.6 Sol、Terra 与 Luna；具体可用性受当前账号限制。</small>
          </div>
        ) : (
          <>
            <label className="field full-width">
              <span>Base URL</span>
              <input
                aria-invalid={invalidField === "baseUrl"}
                inputMode="url"
                maxLength={2048}
                onChange={(event) => {
                  setForm({ ...form, baseUrl: event.target.value });
                  if (invalidField === "baseUrl") setInvalidField(null);
                }}
                required
                type="url"
                value={form.baseUrl}
              />
              <small className={invalidField === "baseUrl" ? "field-error" : undefined}>
                {invalidField === "baseUrl" ? "请输入含域名的 HTTPS URL；HTTP 仅允许 localhost 或回环地址。" : "远程地址必须使用 HTTPS；HTTP 仅允许本机回环地址。"}
              </small>
            </label>

            <label className="field full-width">
              <span>API Key / Bearer Token</span>
              <input
                aria-invalid={invalidField === "auth.secret"}
                autoComplete="new-password"
                maxLength={2560}
                onChange={(event) => {
                  setForm({ ...form, secret: event.target.value });
                  if (invalidField === "auth.secret") setInvalidField(null);
                }}
                required
                spellCheck={false}
                type="password"
                value={form.secret}
              />
              <small className={invalidField === "auth.secret" ? "field-error" : undefined}>
                {invalidField === "auth.secret" ? "API Key 不能为空，也不能包含换行。" : "只在本次提交期间存在于表单内存，提交完成后立即清空。"}
              </small>
            </label>
          </>
        )}

        <div className="field">
          <span>Protocol</span>
          <div className="static-value">Responses API</div>
        </div>

        <label className="enabled-field">
          <input
            checked={form.enabled}
            onChange={(event) => setForm({ ...form, enabled: event.target.checked })}
            type="checkbox"
          />
          <span>
            <strong>Enabled</strong>
            <small>保存后允许该 Provider 进入后续绑定流程。</small>
          </span>
        </label>

        {error && (
          <div className="inline-error full-width" role="alert">
            <strong>Provider 保存失败</strong>
            <span>{error}</span>
            {kind !== "codex-native" && <small>API Key 已从表单清空，请检查后重新输入。</small>}
          </div>
        )}

        <div className="form-actions full-width">
          <button className="secondary-button" disabled={saving} onClick={cancel} type="button">
            取消
          </button>
          <button className="primary-button" disabled={saving} type="submit">
            {saving ? "保存中…" : "保存 Provider"}
          </button>
        </div>
      </form>
    </section>
  );
}

function ProviderResponsesReferenceDialog({ onClose }: { onClose: () => void }) {
  return (
    <DocumentationReferenceDialog
      description="以下厂商已提供相关文档；实际 Codex 兼容性仍以 Model 连接测试为准。"
      eyebrow="Provider Reference"
      note="外部文档可能调整 Endpoint、支持模型或参数范围，请以厂商最新内容为准。"
      onClose={onClose}
      references={responsesApiReferences}
      title="Responses API 支持参考"
      titleId="responses-reference-title"
    />
  );
}

function DocumentationReferenceDialog({
  description,
  eyebrow,
  note,
  onClose,
  references,
  title,
  titleId,
}: {
  description: string;
  eyebrow: string;
  note: string;
  onClose: () => void;
  references: readonly DocumentationReference[];
  title: string;
  titleId: string;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const [openError, setOpenError] = useState<string | null>(null);

  useEffect(() => {
    const dialog = dialogRef.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  async function openReference(url: string) {
    setOpenError(null);
    try {
      await openUrl(url);
    } catch {
      setOpenError("文档打开失败，请检查系统默认浏览器设置后重试。");
    }
  }

  return (
    <dialog
      aria-labelledby={titleId}
      className="responses-reference-dialog"
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
      ref={dialogRef}
    >
      <section className="responses-reference-card">
        <header>
          <div>
            <span className="eyebrow">{eyebrow}</span>
            <h2 id={titleId}>{title}</h2>
            <p>{description}</p>
          </div>
          <IconButton icon="close" label={`关闭${title}`} onClick={onClose} />
        </header>

        <ul className="responses-reference-list">
          {references.map((reference) => (
            <li key={reference.name}>
              <div className="reference-info">
                <span>
                  <strong>{reference.name}</strong>
                  <small>{reference.support}</small>
                </span>
                {reference.description && (
                  <small className="reference-description">{reference.description}</small>
                )}
              </div>
              <div className="reference-links">
                {reference.links.map((link, i) => (
                  <button
                    key={i}
                    className="reference-doc-link"
                    onClick={() => void openReference(link.url)}
                    type="button"
                  >
                    <span>{link.label}</span>
                    <UiIcon name="external-link" />
                  </button>
                ))}
              </div>
            </li>
          ))}
        </ul>

        {openError && <small className="responses-reference-error">{openError}</small>}
        <small className="responses-reference-note">{note}</small>
      </section>
    </dialog>
  );
}

function EnvironmentDetails({
  detecting,
  environment,
  onRedetect,
}: {
  detecting: boolean;
  environment: CodexEnvironmentResponse;
  onRedetect: () => void;
}) {
  const configAccess =
    environment.configurationReadable && environment.configurationWritable
      ? "配置可读写"
      : `${environment.configurationReadable ? "可读" : "不可读"} / ${environment.configurationWritable ? "可写" : "不可写"}`;

  return (
    <section className="environment-card overview-environment-card">
      <div className="baseline-status">
        <span className={`baseline-indicator ${environment.supported ? "ready" : "blocked"}`}>
          <UiIcon name={environment.supported ? "check" : "x-circle"} />
          <span className="sr-only">{environment.supported ? "正常" : "需要处理"}</span>
        </span>
        <strong>{environment.supported ? "客户端满足当前基线" : "客户端需要处理"}</strong>
        <InfoTip
          label={environment.supported
            ? "Codex 版本、Multi-Agent 能力与配置目录访问均满足 CAS 当前基线。"
            : "至少一项 Codex 版本、Multi-Agent 能力或配置目录访问检查未通过；请查看下方问题。"}
        />
        <small>{configAccess}</small>
      </div>
      <div className="baseline-home">
        <span>CODEX_HOME</span>
        {environment.codexHome ? (
          <>
            <Tooltip content={environment.codexHome} focusable label={`CODEX_HOME：${environment.codexHome}`}>
              <code>{environment.codexHome}</code>
            </Tooltip>
            <CopyIconButton label="复制 CODEX_HOME" value={environment.codexHome} />
          </>
        ) : (
          <code>未知</code>
        )}
      </div>
      <div className="baseline-actions">
        <small>目录或显示异常时重新检测</small>
        <button className="ghost-button" disabled={detecting} onClick={onRedetect} type="button">
          {detecting ? "检测中…" : "重新检测"}
        </button>
      </div>
      {environment.issues.length > 0 && (
        <ul className="issue-list">
          {environment.issues.map((issue) => (
            <li key={issue.code}>
              <strong>{issue.severity}</strong>
              <span>{issue.message}</span>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function EnvironmentField({ label, value }: { label: string; value: string | null }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd className="environment-field-value">
        <Tooltip
          content={value ?? "当前数据源未提供该字段"}
          focusable
          label={`${label}：${value ?? "未知"}`}
        >
          <span>{value ?? "未知"}</span>
        </Tooltip>
        {label === "Bound Thread" && value && (
          <CopyIconButton label="复制 Bound Thread ID" value={value} />
        )}
      </dd>
    </div>
  );
}

type AgentFormField =
  | "agentKey"
  | "roleKey"
  | "name"
  | "description"
  | "instruction"
  | "modelId"
  | "reasoningPolicy"
  | "cacheRetentionOverrideSeconds";

function invalidAgentField(values: {
  agentKey?: string;
  name: string;
  description: string;
  instruction: string;
  instructionRequired: boolean;
  roleKey: string;
  cacheRetentionOverrideSeconds: number | null;
}): AgentFormField | null {
  if (values.agentKey !== undefined && !/^[a-z][a-z0-9_-]{0,63}$/.test(values.agentKey)) {
    return "agentKey";
  }
  if (!/^[a-z][a-z0-9_-]{0,63}$/.test(values.roleKey)) return "roleKey";
  if (!values.name.trim() || values.name.length > 160) return "name";
  if (!values.description.trim() || values.description.length > 2000) return "description";
  if (values.instructionRequired && !values.instruction.trim()) return "instruction";
  if (values.instruction.length > 100000) return "instruction";
  if (
    values.cacheRetentionOverrideSeconds !== null
    && (
      values.cacheRetentionOverrideSeconds <= 0
      || values.cacheRetentionOverrideSeconds > 31_536_000
    )
  ) {
    return "cacheRetentionOverrideSeconds";
  }
  return null;
}

function agentErrorField(reason: unknown): AgentFormField | null {
  const field = errorField(reason);
  if (
    [
      "agentKey",
      "roleKey",
      "name",
      "description",
      "instruction",
      "modelId",
      "reasoningPolicy",
      "cacheRetentionOverrideSeconds",
    ].includes(field ?? "")
  ) {
    return field as AgentFormField;
  }
  const code = errorCode(reason);
  if (code === "AGENT_NAME_CONFLICT") return "agentKey";
  if (code === "MODEL_NOT_FOUND" || code === "MODEL_INCOMPATIBLE") return "modelId";
  return null;
}

function errorMessage(reason: unknown): string {
  let message = "操作失败，请重试。";
  if (reason instanceof Error) {
    message = reason.message;
  } else if (
    typeof reason === "object" &&
    reason !== null &&
    "message" in reason &&
    typeof reason.message === "string"
  ) {
    message = reason.message;
  }
  const blockerCode =
    typeof reason === "object" &&
    reason !== null &&
    "details" in reason &&
    typeof reason.details === "object" &&
    reason.details !== null &&
    "blockerCode" in reason.details &&
    typeof reason.details.blockerCode === "string"
      ? reason.details.blockerCode
      : null;
  return blockerCode ? `${message}（${blockerCode}）` : message;
}

function withTimeout<T>(promise: Promise<T>, label: string, timeoutMs = 10_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = window.setTimeout(
      () => reject(new Error(`${label}超时。请确认 Codex 未占用配置后刷新状态。`)),
      timeoutMs,
    );
    promise.then(
      (value) => {
        window.clearTimeout(timeout);
        resolve(value);
      },
      (reason: unknown) => {
        window.clearTimeout(timeout);
        reject(reason);
      },
    );
  });
}

function errorField(reason: unknown): string | null {
  if (typeof reason !== "object" || reason === null || !("details" in reason)) return null;
  const details = reason.details;
  if (typeof details !== "object" || details === null || !("field" in details)) return null;
  return typeof details.field === "string" ? details.field : null;
}

function errorCode(reason: unknown): string | null {
  if (typeof reason !== "object" || reason === null || !("code" in reason)) return null;
  return typeof reason.code === "string" ? reason.code : null;
}
