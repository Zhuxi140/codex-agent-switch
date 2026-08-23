import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import {
  focusMainWindow,
  getProjectMonitorSnapshot,
  getSettings,
  hideProjectMonitor,
  setProjectMonitorAlwaysOnTop,
  type AgentThreadInstanceResponse,
  type AgentThreadProjectSummaryResponse,
  type Appearance,
  type NativeSubagentSyncResponse,
  type ProjectMonitorSnapshotResponse,
} from "./api";

const PROJECT_STORAGE_KEY = "cas.project-monitor.project";
const PIN_STORAGE_KEY = "cas.project-monitor.always-on-top";
const UNSCOPED_PROJECT_KEY = "__unscoped__";

type MonitorIconName = "close" | "external" | "pin" | "refresh";

function MonitorIcon({ name }: { name: MonitorIconName }) {
  let content: ReactNode;
  switch (name) {
    case "close":
      content = <path d="M6 6l12 12M18 6 6 18" />;
      break;
    case "external":
      content = <><path d="M14 4h6v6M20 4l-9 9" /><path d="M18 13v6a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h6" /></>;
      break;
    case "pin":
      content = <><path d="m8 3 8 8M14 3l7 7-4 1-4 4-1 4-7-7 4-1 4-4Z" /><path d="m9 15-6 6" /></>;
      break;
    case "refresh":
      content = <><path d="M20 7v5h-5M4 17v-5h5" /><path d="M6.1 9A7 7 0 0 1 18 6l2 2M18 15a7 7 0 0 1-11.9 3L4 16" /></>;
      break;
  }
  return (
    <svg aria-hidden="true" className="project-monitor-icon" fill="none" viewBox="0 0 24 24">
      <g stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" strokeWidth="1.8">
        {content}
      </g>
    </svg>
  );
}

function projectKey(project: AgentThreadProjectSummaryResponse): string {
  return project.workspaceScopeKey ?? UNSCOPED_PROJECT_KEY;
}

function workspaceName(value: string | null): string {
  if (!value) return "未归属项目";
  return value.split(/[\\/]/).filter(Boolean).at(-1) ?? value;
}

function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(value >= 10_000_000 ? 0 : 1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(value >= 100_000 ? 0 : 1)}K`;
  return value.toLocaleString("zh-CN");
}

function shortThreadId(value: string): string {
  return value.length > 16 ? `${value.slice(0, 8)}…${value.slice(-6)}` : value;
}

function formatAge(value: string | null): string {
  if (!value) return "尚未同步";
  const elapsed = Date.now() - new Date(value).getTime();
  if (!Number.isFinite(elapsed) || elapsed < 0) return "刚刚";
  if (elapsed < 60_000) return "刚刚";
  if (elapsed < 3_600_000) return `${Math.floor(elapsed / 60_000)} 分钟前`;
  if (elapsed < 86_400_000) return `${Math.floor(elapsed / 3_600_000)} 小时前`;
  return `${Math.floor(elapsed / 86_400_000)} 天前`;
}

function instanceStatusLabel(status: AgentThreadInstanceResponse["status"]): string {
  return {
    RUNNING: "运行中",
    IDLE: "可复用",
    RECOVERY_REQUIRED: "待恢复",
    CLOSED: "已关闭",
    UNKNOWN: "未知",
  }[status];
}

function errorMessage(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason && typeof reason === "object" && "message" in reason) {
    const message = (reason as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }
  return "项目监控读取失败，请重试。";
}

function applyAppearance(appearance: Appearance, customFontFamily: string | null): () => void {
  const colorScheme = window.matchMedia("(prefers-color-scheme: dark)");
  const applyTheme = () => {
    document.documentElement.dataset.theme = appearance === "SYSTEM"
      ? (colorScheme.matches ? "DARK" : "LIGHT")
      : appearance;
  };
  applyTheme();
  document.documentElement.style.setProperty(
    "--app-font-family",
    customFontFamily
      ? `"${customFontFamily}", var(--system-font-family)`
      : "var(--system-font-family)",
  );
  if (appearance === "SYSTEM") colorScheme.addEventListener("change", applyTheme);
  return () => colorScheme.removeEventListener("change", applyTheme);
}

interface MonitorViewProps {
  projects: AgentThreadProjectSummaryResponse[];
  selectedKey: string | null;
  instances: AgentThreadInstanceResponse[];
  sync: NativeSubagentSyncResponse | null;
  orchestrationEnabled: boolean;
  projectExcluded: boolean;
  activeAgentCount: number;
  observationDelta: number;
  loading: boolean;
  error: string | null;
  pinned: boolean;
  onSelect: (key: string) => void;
  onRefresh: () => void;
  onTogglePin: () => void;
  onOpenMain: () => void;
  onHide: () => void;
}

export function ProjectMonitorView({
  projects,
  selectedKey,
  instances,
  sync,
  orchestrationEnabled,
  projectExcluded,
  activeAgentCount,
  observationDelta,
  loading,
  error,
  pinned,
  onSelect,
  onRefresh,
  onTogglePin,
  onOpenMain,
  onHide,
}: MonitorViewProps) {
  const selectedProject = projects.find((project) => projectKey(project) === selectedKey) ?? null;
  const visibleInstances = useMemo(() => instances
    .filter((instance) => instance.status !== "CLOSED")
    .sort((left, right) => {
      const priority = { RUNNING: 0, RECOVERY_REQUIRED: 1, IDLE: 2, UNKNOWN: 3, CLOSED: 4 };
      return priority[left.status] - priority[right.status]
        || right.lastUsedAt.localeCompare(left.lastUsedAt);
    }), [instances]);
  const activeTokens = visibleInstances
    .filter((instance) => instance.status === "RUNNING")
    .reduce((total, instance) => total + instance.totalTokens, 0);
  const hasIdle = visibleInstances.some((instance) => instance.status === "IDLE");
  const status = !selectedProject
    ? { label: "请选择项目", tone: "muted" }
    : !selectedProject.workspaceScopeKey
      ? { label: "Scope 未识别", tone: "warning" }
    : !orchestrationEnabled
      ? { label: "Default 模式", tone: "muted" }
      : projectExcluded
        ? { label: "项目已排除", tone: "warning" }
        : selectedProject.recoveryRequiredCount > 0
          ? { label: "需要恢复", tone: "danger" }
          : selectedProject.runningCount > 0
            ? { label: "运行中", tone: "success" }
            : hasIdle
              ? { label: "可复用", tone: "info" }
              : { label: "允许，等待 Thread", tone: "info" };
  const allowed = Boolean(
    selectedProject?.workspaceScopeKey && orchestrationEnabled && !projectExcluded,
  );

  return (
    <section className="project-monitor-shell" aria-label="CAS 项目子 Agent 监控">
      <header className="project-monitor-titlebar" data-tauri-drag-region>
        <div data-tauri-drag-region>
          <span className={`project-monitor-live-dot ${status.tone}`} aria-hidden="true" />
          <strong data-tauri-drag-region>CAS 项目监控</strong>
        </div>
        <nav aria-label="浮窗操作">
          <button
            aria-label={pinned ? "取消始终置顶" : "始终置顶"}
            aria-pressed={pinned}
            className={pinned ? "active" : ""}
            onClick={onTogglePin}
            title={pinned ? "取消始终置顶" : "始终置顶"}
            type="button"
          >
            <MonitorIcon name="pin" />
          </button>
          <button aria-label="打开 CAS 主窗口" onClick={onOpenMain} title="打开 CAS 主窗口" type="button">
            <MonitorIcon name="external" />
          </button>
          <button aria-label="隐藏项目监控" onClick={onHide} title="隐藏" type="button">
            <MonitorIcon name="close" />
          </button>
        </nav>
      </header>

      <div className="project-monitor-body">
        <div className="project-monitor-project-row">
          <label>
            <span>监控项目</span>
            <select
              aria-label="选择监控项目"
              disabled={projects.length === 0}
              onChange={(event) => onSelect(event.target.value)}
              value={selectedKey ?? ""}
            >
              {projects.length === 0 && <option value="">暂无项目</option>}
              {projects.map((project) => (
                <option key={projectKey(project)} value={projectKey(project)}>
                  {workspaceName(project.workspaceScopeKey)}
                </option>
              ))}
            </select>
          </label>
          <button
            aria-busy={loading || undefined}
            aria-label="立即刷新"
            className="project-monitor-refresh"
            disabled={loading}
            onClick={onRefresh}
            title="立即刷新"
            type="button"
          >
            <MonitorIcon name="refresh" />
          </button>
        </div>

        {error && (
          <div className="project-monitor-error" role="alert">
            <span>{error}</span>
            <button onClick={onRefresh} type="button">重试</button>
          </div>
        )}

        {!error && (
          <>
            <div className="project-monitor-status" aria-live="polite">
              <div>
                <span>子 Agent</span>
                <strong>{allowed ? "允许" : "不允许"}</strong>
                <small>{activeAgentCount} 个已启用</small>
              </div>
              <span className={`project-monitor-status-pill ${status.tone}`}>{status.label}</span>
            </div>

            <dl className="project-monitor-metrics">
              <div>
                <dt>项目累计</dt>
                <dd>{formatTokens(selectedProject?.totalTokens ?? 0)}</dd>
              </div>
              <div>
                <dt>本次观察新增</dt>
                <dd>+{formatTokens(observationDelta)}</dd>
              </div>
              <div>
                <dt>运行中 Thread 累计</dt>
                <dd>{formatTokens(activeTokens)}</dd>
              </div>
            </dl>

            <div className="project-monitor-thread-list" aria-label="最近活跃子 Agent Thread">
              {loading && !selectedProject && (
                <div className="project-monitor-empty loading">正在读取原生 Thread…</div>
              )}
              {!loading && selectedProject && visibleInstances.length === 0 && (
                <div className="project-monitor-empty">该项目暂无可观察 Thread。</div>
              )}
              {visibleInstances.slice(0, 3).map((instance) => (
                <article key={instance.id}>
                  <span className={`project-monitor-thread-state ${instance.status.toLowerCase()}`} aria-hidden="true" />
                  <div>
                    <strong>{instance.agentNameSnapshot ?? "未映射 Agent"}</strong>
                    <small>{shortThreadId(instance.codexThreadId)} · {instanceStatusLabel(instance.status)}</small>
                  </div>
                  <span>{formatTokens(instance.totalTokens)}</span>
                </article>
              ))}
              {visibleInstances.length > 3 && (
                <small className="project-monitor-more">另有 {visibleInstances.length - 3} 个 Thread</small>
              )}
            </div>

            <footer>
              <span>{sync?.capability === "SUPPORTED" ? "原生同步正常" : "原生同步受限"}</span>
              <span>最近成功 {formatAge(sync?.lastSuccessAt ?? null)}</span>
            </footer>
          </>
        )}
      </div>
    </section>
  );
}

export function ProjectMonitorWindow() {
  const [snapshot, setSnapshot] = useState<ProjectMonitorSnapshotResponse | null>(null);
  const [selectedKey, setSelectedKey] = useState<string | null>(() => localStorage.getItem(PROJECT_STORAGE_KEY));
  const [pinned, setPinned] = useState(() => localStorage.getItem(PIN_STORAGE_KEY) !== "false");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const baseline = useRef<{ key: string; tokens: number } | null>(null);

  const load = useCallback(async (silent = false) => {
    if (!silent) setLoading(true);
    try {
      const loaded = await getProjectMonitorSnapshot(
        selectedKey === UNSCOPED_PROJECT_KEY ? null : selectedKey,
        selectedKey !== null,
      );
      setSnapshot(loaded);
      setError(null);
      const selectedExists = loaded.projects.some((project) => projectKey(project) === selectedKey);
      if (!selectedExists && loaded.projects.length > 0) {
        const nextKey = projectKey(loaded.projects[0]);
        setSelectedKey(nextKey);
        localStorage.setItem(PROJECT_STORAGE_KEY, nextKey);
      }
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      if (!silent) setLoading(false);
    }
  }, [selectedKey]);

  useEffect(() => {
    document.documentElement.classList.add("project-monitor-document");
    let removeThemeListener: () => void = () => undefined;
    getSettings()
      .then((settings) => {
        removeThemeListener = applyAppearance(settings.appearance, settings.customFontFamily);
      })
      .catch(() => undefined);
    return () => {
      removeThemeListener();
      document.documentElement.classList.remove("project-monitor-document");
    };
  }, []);

  useEffect(() => {
    void setProjectMonitorAlwaysOnTop(pinned).catch(() => undefined);
  }, [pinned]);

  useEffect(() => {
    void load();
    const timer = window.setInterval(() => void load(true), 3_000);
    return () => window.clearInterval(timer);
  }, [load]);

  const selectedProject = snapshot?.projects.find((project) => projectKey(project) === selectedKey) ?? null;
  if (selectedKey && selectedProject) {
    if (baseline.current?.key !== selectedKey) {
      baseline.current = { key: selectedKey, tokens: selectedProject.totalTokens };
    }
  } else {
    baseline.current = null;
  }
  const observationDelta = selectedProject && baseline.current
    ? Math.max(0, selectedProject.totalTokens - baseline.current.tokens)
    : 0;

  function selectProject(key: string) {
    baseline.current = null;
    setSelectedKey(key);
    localStorage.setItem(PROJECT_STORAGE_KEY, key);
  }

  function togglePin() {
    const next = !pinned;
    setPinned(next);
    localStorage.setItem(PIN_STORAGE_KEY, String(next));
  }

  return (
    <ProjectMonitorView
      activeAgentCount={snapshot?.activeAgentCount ?? 0}
      error={error}
      instances={snapshot?.instances ?? []}
      loading={loading}
      observationDelta={observationDelta}
      onHide={() => void hideProjectMonitor()}
      onOpenMain={() => void focusMainWindow()}
      onRefresh={() => void load()}
      onSelect={selectProject}
      onTogglePin={togglePin}
      orchestrationEnabled={snapshot?.orchestrationEnabled ?? false}
      pinned={pinned}
      projectExcluded={snapshot?.projectExcluded ?? false}
      projects={snapshot?.projects ?? []}
      selectedKey={selectedKey}
      sync={snapshot?.sync ?? null}
    />
  );
}
