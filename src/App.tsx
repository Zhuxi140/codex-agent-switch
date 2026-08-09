import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { createPortal } from "react-dom";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  addModel,
  createAgent,
  createProvider,
  deleteAgent,
  deleteModel,
  deleteProvider,
  getAgent,
  getAppBootstrap,
  getCodexEnvironment,
  getConfigurationStatus,
  getRuntimeMode,
  getProvider,
  getSettings,
  listAgentPresets,
  listAgents,
  listModels,
  listProviders,
  listSnapshots,
  redetectCodex,
  removeAgentModelBinding,
  restoreSnapshot,
  runDiagnostics,
  setAgentModelBinding,
  setModelEnabled,
  testModelConnection,
  switchRuntimeMode,
  updateSettings,
  updateAgent,
  updateModel,
  updateProvider,
  type Appearance,
  type AgentDetailResponse,
  type AgentPresetResponse,
  type AgentSummary,
  type AppBootstrapResponse,
  type CodexEnvironmentResponse,
  type ConfigurationStatusResponse,
  type DiagnosticsResponse,
  type ModelSummary,
  type ProviderCreateRequest,
  type ProviderDetailResponse,
  type ProviderSummary,
  type ReasoningPolicy,
  type RuntimeModeResponse,
  type SandboxPolicy,
  type SnapshotSummary,
  type SettingsResponse,
} from "./api";

type Page = "overview" | "agents" | "providers" | "models" | "diagnostics" | "settings";

const navigation: Array<{ label: string; page?: Page }> = [
  { label: "概览", page: "overview" },
  { label: "Agents", page: "agents" },
  { label: "Providers", page: "providers" },
  { label: "Models", page: "models" },
  { label: "诊断", page: "diagnostics" },
  { label: "设置", page: "settings" },
];

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
        <div className="topbar-status" title={bootstrap?.codex.version ?? undefined}>
          <span className={bootstrap?.codex.detected ? "online" : ""} aria-hidden="true" />
          <span>
            {!bootstrap ? "正在检测" : bootstrap.codex.detected ? "Codex Ready" : "Codex 未检测"}
          </span>
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
        ) : page === "agents" ? (
          <AgentsPage />
        ) : page === "providers" ? (
          <ProvidersPage />
        ) : page === "diagnostics" ? (
          <DiagnosticsPage />
        ) : page === "settings" ? (
          <SettingsPage
            onAppearanceChange={setAppearance}
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
  const [selectedAgentId, setSelectedAgentId] = useState("");
  const [snapshots, setSnapshots] = useState<SnapshotSummary[]>([]);
  const [operation, setOperation] = useState<"switch" | "restore" | null>(null);
  const [configurationError, setConfigurationError] = useState<string | null>(null);
  const [configurationSuccess, setConfigurationSuccess] = useState<string | null>(null);

  const reloadConfiguration = useCallback(async () => {
    const [status, mode, agentList, history] = await Promise.all([
      getConfigurationStatus(),
      getRuntimeMode(),
      listAgents(),
      listSnapshots(6),
    ]);
    setConfiguration(status);
    setRuntimeMode(mode);
    setAgents(agentList);
    setSnapshots(history.items);
    setSelectedMode(mode.activeAgentId ? "SUBAGENT" : "DEFAULT");
    setSelectedAgentId(
      mode.activeAgentId
      ?? agentList.find((agent) => agent.availability === "READY")?.id
      ?? agentList[0]?.id
      ?? "",
    );
  }, []);

  useEffect(() => {
    reloadConfiguration().catch((reason: unknown) => {
      setConfigurationError(errorMessage(reason));
    });
  }, [reloadConfiguration]);

  async function handleModeSwitch() {
    const activeAgentId = selectedMode === "SUBAGENT" ? selectedAgentId : null;
    setOperation("switch");
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      const result = await switchRuntimeMode(activeAgentId);
      if (result.status === "FAILED_ROLLED_BACK" || result.status === "RECOVERY_REQUIRED") {
        throw new Error(
          result.status === "FAILED_ROLLED_BACK"
            ? "模式切换失败，磁盘配置已自动回滚。"
            : "模式切换未完成，需要先恢复配置事务。",
        );
      }
      const activeAgent = agents.find((agent) => agent.id === activeAgentId);
      setConfigurationSuccess(
        activeAgent
          ? `已切换到子 Agent：${activeAgent.name}。`
          : "已切换到 Default；Codex 将全权负责当前任务。",
      );
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConfigurationError(errorMessage(reason));
      await reloadConfiguration();
    } finally {
      setOperation(null);
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

  const selectedAgent = agents.find((agent) => agent.id === selectedAgentId);
  const targetAgentId = selectedMode === "SUBAGENT" ? selectedAgentId || null : null;
  const sameMode = targetAgentId === runtimeMode?.activeAgentId;
  const modeReady = selectedMode === "DEFAULT" || selectedAgent?.availability === "READY";
  const alreadySynchronized = sameMode && configuration?.status === "APPLIED";
  const activeAgent = agents.find((agent) => agent.id === runtimeMode?.activeAgentId);
  return (
    <>
      <header>
        <div>
          <span className="eyebrow">Runtime Mode</span>
          <h1>选择 Codex 的工作方式</h1>
          <p>使用 Default 让 Codex 全权负责，或只启用一个由 CAS 管理的子 Agent。</p>
        </div>
        <button className="secondary-button" disabled={detecting} onClick={onRedetect}>
          {detecting ? "检测中…" : "重新检测 Codex"}
        </button>
      </header>

      {error && (
        <section className="notice error">
          <strong>后端连接失败</strong>
          <p>{error}</p>
        </section>
      )}

      <section className="configuration-card runtime-mode-card">
        <div className="configuration-heading">
          <div>
            <span className="eyebrow">Mode Switch</span>
            <h2>运行模式</h2>
            <p>切换时自动创建 Snapshot，并在失败时回滚 CAS-owned 配置。</p>
          </div>
          <span className={`mode-status ${configuration?.status.toLowerCase() ?? "unavailable"}`}>
            {configuration?.status ?? "LOADING"}
          </span>
        </div>

        {configurationSuccess && <div className="success-banner">{configurationSuccess}</div>}
        {configurationError && <div className="inline-error">{configurationError}</div>}

        <div className="runtime-mode-options" role="radiogroup" aria-label="Codex 运行模式">
          <label className={`runtime-mode-option ${selectedMode === "DEFAULT" ? "selected" : ""}`}>
            <input
              checked={selectedMode === "DEFAULT"}
              name="runtime-mode"
              onChange={() => setSelectedMode("DEFAULT")}
              type="radio"
            />
            <span className="runtime-mode-copy">
              <strong>Default</strong>
              <small>不启用 CAS 子 Agent，不改动 Codex 的主模型、MCP 或其他外部配置。</small>
            </span>
            {runtimeMode?.activeAgentId === null && <span className="current-mode-tag">当前</span>}
          </label>

          <label className={`runtime-mode-option ${selectedMode === "SUBAGENT" ? "selected" : ""}`}>
            <input
              checked={selectedMode === "SUBAGENT"}
              disabled={agents.length === 0}
              name="runtime-mode"
              onChange={() => setSelectedMode("SUBAGENT")}
              type="radio"
            />
            <span className="runtime-mode-copy">
              <strong>使用子 Agent</strong>
              <small>从 Agents 中单选一个；Codex 只会看到该 Agent 的 CAS 投影。</small>
              <select
                aria-label="选择要启用的 Agent"
                disabled={selectedMode !== "SUBAGENT" || agents.length === 0}
                onChange={(event) => setSelectedAgentId(event.target.value)}
                onClick={(event) => event.stopPropagation()}
                value={selectedAgentId}
              >
                {agents.length === 0 && <option value="">请先创建 Agent</option>}
                {agents.map((agent) => (
                  <option key={agent.id} value={agent.id}>
                    {agent.name} · {availabilityLabel(agent.availability)}
                  </option>
                ))}
              </select>
              {selectedMode === "SUBAGENT" && selectedAgent && selectedAgent.availability !== "READY" && (
                <em>该 Agent 尚未就绪，请先在 Agents 页面完善 Model 与 Provider。</em>
              )}
            </span>
            {runtimeMode?.activeAgentId && (
              <span className="current-mode-tag">当前：{activeAgent?.name ?? "子 Agent"}</span>
            )}
          </label>
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
            {alreadySynchronized
              ? "当前模式已同步到 Codex。"
              : sameMode
                ? "当前定义或磁盘配置有变化，可重新同步。"
                : "确认后将立即切换，并只处理 CAS 拥有的配置。"}
          </span>
          <button
            className="primary-button"
            disabled={operation !== null || !runtimeMode || !modeReady || alreadySynchronized}
            onClick={handleModeSwitch}
            type="button"
          >
            {operation === "switch"
              ? "切换中…"
              : alreadySynchronized
                ? "当前已启用"
                : sameMode
                  ? "同步当前模式"
                  : "切换模式"}
          </button>
        </div>
      </section>

      {environment && <EnvironmentDetails environment={environment} />}

      <section className="configuration-card snapshot-card">
        <div className="configuration-heading">
          <div>
            <span className="eyebrow">Snapshots</span>
            <h2>最近备份</h2>
          </div>
        </div>
        {snapshots.length === 0 ? (
          <p className="empty-copy">首次切换模式前会自动生成 Snapshot。</p>
        ) : (
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
        )}
      </section>
    </>
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
            <span className={`result ${result.overall.toLowerCase()}`}>{result.overall}</span>
            <small>{result.checkedAt}</small>
          </div>
          {result.sections.map((section) => (
            <article className="diagnostic-section" key={section.key}>
              <h2>{section.title}</h2>
              <ul>
                {section.issues.map((issue) => (
                  <li key={`${issue.code}-${issue.message}`}>
                    <span aria-hidden="true" className={`diagnostic-icon ${issue.severity.toLowerCase()}`}>
                      {issue.severity === "ERROR" ? "✕" : issue.severity === "WARNING" ? "⚠" : "ⓘ"}
                    </span>
                    <div>
                      <strong>{issue.message}</strong>
                      <small>{issue.code}</small>
                    </div>
                  </li>
                ))}
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
  onFontFamilyChange,
}: {
  onAppearanceChange: (appearance: Appearance) => void;
  onFontFamilyChange: (fontFamily: string | null) => void;
}) {
  const [settings, setSettings] = useState<SettingsResponse | null>(null);
  const [environment, setEnvironment] = useState<CodexEnvironmentResponse | null>(null);
  const [customCodexHome, setCustomCodexHome] = useState("");
  const [customFontFamily, setCustomFontFamily] = useState("");
  const [saving, setSaving] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([getSettings(), getCodexEnvironment()])
      .then(([loadedSettings, loadedEnvironment]) => {
        setSettings(loadedSettings);
        setCustomCodexHome(loadedSettings.customCodexHome ?? "");
        setCustomFontFamily(loadedSettings.customFontFamily ?? "");
        setEnvironment(loadedEnvironment);
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
      });
      setSettings(updated);
      setCustomCodexHome(updated.customCodexHome ?? "");
      setCustomFontFamily(updated.customFontFamily ?? "");
      setEnvironment(await getCodexEnvironment());
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
      setEnvironment(await redetectCodex());
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setDetecting(false);
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
                <div><dt>Executable</dt><dd>{environment.executablePath ?? "未检测到"}</dd></div>
                <div><dt>CODEX_HOME</dt><dd>{environment.codexHome ?? "未定位"}</dd></div>
                <div><dt>Version</dt><dd>{environment.version ?? "未知"}</dd></div>
              </dl>
            )}
            <label className="field">
              <span>自定义 CODEX_HOME</span>
              <input
                onChange={(event) => setCustomCodexHome(event.target.value)}
                placeholder="留空以自动检测"
                value={customCodexHome}
              />
              <small>必须填写已经存在的绝对目录；留空恢复自动检测。</small>
            </label>
          </section>

          <section className="settings-card settings-grid">
            <label className="field">
              <span>Appearance</span>
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
              <span>Update Channel</span>
              <select
                onChange={(event) => setSettings({ ...settings, updateChannel: event.target.value })}
                value={settings.updateChannel}
              >
                <option value="STABLE">Stable</option>
                <option value="BETA">Beta</option>
              </select>
            </label>
            <label className="field full-width">
              <span>界面字体</span>
              <input
                maxLength={160}
                onChange={(event) => setCustomFontFamily(event.target.value)}
                placeholder="留空使用系统默认字体"
                value={customFontFamily}
              />
              <small>输入系统中已安装字体的准确名称；若系统找不到该字体，将自动回退。</small>
            </label>
            <label className="enabled-field full-width">
              <input checked disabled type="checkbox" />
              <span>
                模式切换前自动备份
                <small>V0.1 安全约束，保持启用，确保失败时可回滚。</small>
              </span>
            </label>
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
  const [activeAgentId, setActiveAgentId] = useState<string | null>(null);
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
      setActiveAgentId(mode.activeAgentId);
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
          isActive={selectedId === activeAgentId}
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
              isActive={agent.id === activeAgentId}
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
  return (
    <article className="agent-row">
      <div className="agent-main">
        <div className="provider-name-line">
          <h2>{agent.name}</h2>
          <span className={`agent-state ${agent.availability.toLowerCase()}`}>
            {availabilityLabel(agent.availability)}
          </span>
          {isActive && <span className="agent-state current">当前使用</span>}
        </div>
        <p>{agent.description}</p>
        <code>{agent.agentKey}</code>
      </div>
      <div className="agent-binding">
        <strong>{agent.model?.displayName ?? "No model assigned"}</strong>
        <span>{agent.model?.providerName ?? "Needs model"} · {agent.reasoningPolicy}</span>
      </div>
      <button className="secondary-button" onClick={() => onOpen(agent.id)}>管理</button>
    </article>
  );
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
  const [name, setName] = useState(initial?.name ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [instruction, setInstruction] = useState("");
  const [sandboxPolicy, setSandboxPolicy] = useState<SandboxPolicy>(
    initial?.defaultSandboxPolicy ?? "INHERIT",
  );
  const [reasoningPolicy, setReasoningPolicy] = useState<ReasoningPolicy>(
    initial?.defaultReasoningPolicy ?? "MODEL_DEFAULT",
  );
  const [modelId, setModelId] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [invalidField, setInvalidField] = useState<AgentFormField | null>(null);

  function selectTemplate(key: string) {
    setTemplateKey(key);
    const preset = presets.find((value) => value.key === key);
    setAgentKey(preset?.key ?? "");
    setName(preset?.name ?? "");
    setDescription(preset?.description ?? "");
    setInstruction("");
    setSandboxPolicy(preset?.defaultSandboxPolicy ?? "INHERIT");
    setReasoningPolicy(preset?.defaultReasoningPolicy ?? "MODEL_DEFAULT");
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
        modelId: modelId || null,
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
          <span>Key</span>
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

        <label className="field full-width">
          <span>Description</span>
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
          <span>Instructions {templateKey && <em>Optional override</em>}</span>
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
          reasoningPolicy={reasoningPolicy}
          sandboxPolicy={sandboxPolicy}
          setReasoningPolicy={setReasoningPolicy}
          setSandboxPolicy={setSandboxPolicy}
        />

        <label className="field full-width">
          <span>Model <em>Optional</em></span>
          <select
            aria-invalid={invalidField === "modelId"}
            onChange={(event) => {
              setModelId(event.target.value);
              if (invalidField === "modelId") setInvalidField(null);
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
                {model.providerName} / {model.displayName} — {compatibilityLabel(model.compatibility)}
              </option>
            ))}
          </select>
          {selectedModel?.compatibility === "UNKNOWN" && (
            <em>该 Model 兼容性未知；后端允许保存并保留明确警告。</em>
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
        setAgent(value);
        setModelId(value.modelBinding?.id ?? "");
      })
      .catch((reason: unknown) => setError(errorMessage(reason)));
  }, [agentId]);

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
    });
    if (localInvalidField) {
      setInvalidField(localInvalidField);
      setError("请修正红色标记的字段。");
      return;
    }
    setSaving(true);
    try {
      await updateAgent({
        agentId: agent.id,
        name: agent.name,
        description: agent.description,
        instruction: agent.instruction,
        sandboxPolicy: agent.sandboxPolicy,
        reasoningPolicy: agent.reasoningPolicy,
      });
      if (modelId !== (agent.modelBinding?.id ?? "")) {
        if (modelId) await setAgentModelBinding(agent.id, modelId);
        else await removeAgentModelBinding(agent.id);
      }
      const refreshed = await getAgent(agent.id);
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

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Agent Detail</span>
          <h2>{agent.name}</h2>
          <p><code>{agent.agentKey}</code> · {agent.agentType}{isActive ? " · 当前使用" : ""}</p>
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
          <span>Key</span>
          <div className="static-value">{agent.agentKey}</div>
          <small>身份字段不可在普通编辑中修改。</small>
        </label>

        <label className="field full-width">
          <span>Description</span>
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
          <span>Instructions</span>
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
          reasoningPolicy={agent.reasoningPolicy}
          sandboxPolicy={agent.sandboxPolicy}
          setReasoningPolicy={(value) => setAgent({ ...agent, reasoningPolicy: value })}
          setSandboxPolicy={(value) => setAgent({ ...agent, sandboxPolicy: value })}
        />

        <label className="field full-width">
          <span>Model</span>
          <select
            aria-invalid={invalidField === "modelId"}
            onChange={(event) => {
              setModelId(event.target.value);
              if (invalidField === "modelId") setInvalidField(null);
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
                {model.providerName} / {model.displayName} — {compatibilityLabel(model.compatibility)}
              </option>
            ))}
          </select>
          {invalidField === "modelId" && <small className="field-error">所选 Model 不存在或与该 Agent 不兼容。</small>}
        </label>

        <CompatibilityPanel compatibility={agent.compatibility} />

        {error && <div className="inline-error full-width" role="alert"><strong>Agent 保存失败</strong><span>{error}</span></div>}

        <div className="form-actions agent-actions full-width">
          <button
            className="danger-button"
            disabled={saving || isActive}
            onClick={() => void remove()}
            title={isActive ? "请先在概览切换到 Default 或其他 Agent" : undefined}
            type="button"
          >
            删除 Agent
          </button>
          <button className="primary-button" disabled={saving} type="submit">{saving ? "保存中…" : "保存更改"}</button>
        </div>
      </form>
    </section>
  );
}

function PolicyFields({
  reasoningPolicy,
  sandboxPolicy,
  setReasoningPolicy,
  setSandboxPolicy,
}: {
  reasoningPolicy: ReasoningPolicy;
  sandboxPolicy: SandboxPolicy;
  setReasoningPolicy: (value: ReasoningPolicy) => void;
  setSandboxPolicy: (value: SandboxPolicy) => void;
}) {
  return (
    <>
      <label className="field">
        <span>Sandbox</span>
        <select onChange={(event) => setSandboxPolicy(event.target.value as SandboxPolicy)} value={sandboxPolicy}>
          <option value="READ_ONLY">Read only</option>
          <option value="WORKSPACE_WRITE">Workspace write</option>
          <option value="DANGER_FULL_ACCESS">Danger full access</option>
          <option value="INHERIT">Inherit</option>
        </select>
      </label>
      <label className="field">
        <span>Reasoning</span>
        <select onChange={(event) => setReasoningPolicy(event.target.value as ReasoningPolicy)} value={reasoningPolicy}>
          <option value="MODEL_DEFAULT">Model default</option>
          <option value="LOW">Low</option>
          <option value="MEDIUM">Medium</option>
          <option value="HIGH">High</option>
          <option value="INHERIT">Inherit</option>
        </select>
      </label>
    </>
  );
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
    INVALID_CONFIGURATION: "Invalid",
  } as const;
  return labels[value];
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
    setSuccess("Provider 已安全保存。Credential 不会在界面中回显。");
    void load();
  }

  function handleUpdated() {
    setEditing(null);
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
    try {
      await deleteProvider(provider.id);
      setSuccess("Provider 及其 Credential 已删除。");
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
          <p>管理 Codex Agent 使用的 Responses API Provider。</p>
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
  const ready = provider.status === "READY" && provider.credentialStatus === "CONFIGURED";
  const label =
    provider.credentialStatus !== "CONFIGURED"
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
          <h2>{provider.name}</h2>
          <span className={`result ${ready ? "ready" : "blocked"}`}>{label}</span>
        </div>
        <p>
          <code>{provider.providerKey}</code>
          <span>Responses API</span>
          <span>{provider.providerType === "PRESET" ? "Preset" : "Custom"}</span>
        </p>
      </div>
      <div className="model-count">
        <strong>{provider.modelCount}</strong>
        <span>Models</span>
      </div>
      <div className="row-actions">
        <button className="ghost-button" disabled={deleting} onClick={onEdit}>编辑</button>
        <button className="danger-button" disabled={deleting} onClick={onDelete}>
          {deleting ? "删除中…" : "删除"}
        </button>
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
      {presetId === "deepseek" ? <DeepSeekLogo /> : name.trim().slice(0, 2).toUpperCase() || "?"}
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
        setActionError(`${model.displayName} · ${modelTestStatusLabel(result.status)}${latency}：${result.message}`);
      }
      await load();
    } catch (reason: unknown) {
      setActionError(errorMessage(reason));
    } finally {
      setTestingId(null);
    }
  }

  const enabledProviders = providers.filter((provider) => provider.enabled);
  const panelOpen = adding || editing !== null;

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Models</span>
          <h1>可绑定模型</h1>
          <p>查看模型来自哪里，以及是否已确认兼容 Codex Multi-Agent。</p>
        </div>
        {!panelOpen && enabledProviders.length > 0 && (
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
          providers={enabledProviders}
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
          {enabledProviders.length > 0 ? (
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
                <th>Compatibility</th>
                <th>Context</th>
                <th>Lifecycle</th>
                <th>Verification</th>
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
    ? "Untested"
    : model.lastTestStatus === "SUCCESS"
      ? "Passed"
      : modelTestStatusLabel(model.lastTestStatus);
  const verificationClass = model.lastTestStatus === null
    ? "untested"
    : model.lastTestStatus === "SUCCESS"
      ? "passed"
      : "failed";
  const lifecycleTitle = lifecycleDescription(model);
  const verificationTitle = [
    verificationDescription(model.lastTestStatus),
    model.lastTestedAt === null
      ? null
      : `最近测试：${new Date(model.lastTestedAt).toLocaleString()}${
        model.lastTestLatencyMs === null ? "" : ` · ${model.lastTestLatencyMs} ms`
      }`,
  ].filter(Boolean).join("\n");
  return (
    <tr>
      <td>{model.providerName}</td>
      <td>
        <strong>{model.displayName}</strong>
        <code>{model.modelId}</code>
      </td>
      <td>
        <ModelStatusBadge
          className={`compatibility ${model.compatibility.toLowerCase()}`}
          description={compatibilityDescription(model.compatibility)}
          label={compatibilityLabel(model.compatibility)}
        />
      </td>
      <td>{model.contextWindow ? formatTokenCount(model.contextWindow) : "Unknown"}</td>
      <td>
        <ModelStatusBadge
          className={lifecycle === "Active" ? "status-text ready" : "status-text"}
          description={lifecycleTitle}
          label={lifecycle}
        />
      </td>
      <td>
        <ModelStatusBadge
          className={`status-text ${verificationClass}`}
          description={verificationTitle}
          label={verification}
        />
      </td>
      <td>
        <div className="row-actions">
          <button className="secondary-button" disabled={deleting || testing} onClick={onTest}>
            {testing ? "测试中…" : "测试"}
          </button>
          <button className="ghost-button" disabled={deleting || testing} onClick={onEdit}>编辑</button>
          <button className="danger-button" disabled={deleting || testing} onClick={onDelete}>
            {deleting ? "删除中…" : "删除"}
          </button>
        </div>
      </td>
    </tr>
  );
}

function ModelStatusBadge({
  className,
  description,
  label,
}: {
  className: string;
  description: string;
  label: string;
}) {
  const [position, setPosition] = useState<{ above: boolean; left: number; top: number } | null>(null);

  function show(target: HTMLElement) {
    const bounds = target.getBoundingClientRect();
    const above = bounds.bottom + 110 > window.innerHeight;
    setPosition({
      above,
      left: Math.min(window.innerWidth - 170, Math.max(170, bounds.left + bounds.width / 2)),
      top: above ? bounds.top - 8 : bounds.bottom + 8,
    });
  }

  return (
    <>
      <span
        aria-label={`${label}：${description}`}
        className={className}
        onBlur={() => setPosition(null)}
        onFocus={(event) => show(event.currentTarget)}
        onMouseEnter={(event) => show(event.currentTarget)}
        onMouseLeave={() => setPosition(null)}
        tabIndex={0}
      >
        {label}
      </span>
      {position && createPortal(
        <span
          className={`model-status-tooltip ${position.above ? "above" : ""}`}
          role="tooltip"
          style={{ left: position.left, top: position.top }}
        >
          {description}
        </span>,
        document.body,
      )}
    </>
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
  const [contextError, setContextError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const parsedContextWindow = contextWindow.trim() === "" ? null : Number(contextWindow);
    if (parsedContextWindow !== null && (!Number.isSafeInteger(parsedContextWindow) || parsedContextWindow <= 0)) {
      setContextError("Context Window 必须是正整数。");
      return;
    }
    setSaving(true);
    setError(null);
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
              <option key={provider.id} value={provider.id}>{provider.name}</option>
            ))}
          </select>
          <small>Model 与 Provider 的组合在 CAS 内唯一。</small>
        </label>

        <label className="field">
          <span>Model ID</span>
          <input
            autoFocus
            maxLength={200}
            onChange={(event) => setModelId(event.target.value)}
            placeholder="provider/model-name"
            required
            value={modelId}
          />
          <small>必须与 Provider API 接受的标识完全一致。</small>
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
    COMPATIBLE: "并非专为 Codex 适配，但 CAS 已验证所需核心能力可以工作。",
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
  if (value === null) return "尚未使用当前 Provider Credential 发起基础 Responses API 测试。";
  const descriptions = {
    SUCCESS: "最近一次基础 Responses API 测试通过；这不代表完整 Codex 兼容性已验证。",
    CREDENTIAL_MISSING: "Provider Credential 不存在或已从系统凭据库移除。",
    AUTH_FAILED: "Provider 拒绝了当前 Credential。",
    MODEL_NOT_FOUND: "Provider 不识别当前 Model ID。",
    RATE_LIMITED: "Provider 在最近一次测试时触发了限流。",
    PROTOCOL_ERROR: "Endpoint 未返回有效的 Responses API 响应。",
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

type ProviderKind = "deepseek" | "custom";

const responsesApiReferences = [
  { name: "DeepSeek", support: "Responses API", url: "https://api-docs.deepseek.com/zh-cn/guides/responses_api" },
  { name: "阿里云百炼", support: "OpenAI 兼容", url: "https://help.aliyun.com/zh/model-studio/compatibility-with-openai-responses-api?mode=pure" },
  { name: "腾讯云 TokenHub", support: "兼容转换", url: "https://cloud.tencent.com.cn/document/product/1823/133813" },
  { name: "Xiaomi MiMo", support: "Responses API", url: "https://mimo.mi.com/docs/zh-CN/api/chat/responses" },
  { name: "火山引擎 · 火山方舟", support: "Responses API", url: "https://docs.volcengine.com/docs/6492/2241837?lang=zh" },
  { name: "Infercom", support: "Responses API", url: "https://docs.infercom.ai/en/features/responses-api" },
  { name: "MiniMax", support: "Responses API", url: "https://platform.minimax.io/docs/api-reference/responses-create" },
] as const;

interface ProviderFormState {
  providerKey: string;
  name: string;
  baseUrl: string;
  secret: string;
  enabled: boolean;
}

const formDefaults: Record<ProviderKind, ProviderFormState> = {
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
          <p>Credential 保持原值，不会读取或回显。</p>
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
    setSaving(true);
    setError(null);
    setInvalidField(null);
    setInvalidFieldMessage(null);
    const request: ProviderCreateRequest = {
      providerKey: form.providerKey,
      name: form.name,
      presetId: kind === "deepseek" ? "deepseek" : null,
      baseUrl: form.baseUrl,
      protocol: "RESPONSES",
      auth: { strategy: "OS_SECRET_HELPER", secret: form.secret },
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
      request.auth.secret = "";
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
          <h2>{kind === "deepseek" ? "添加 DeepSeek" : "添加 Custom Responses Provider"}</h2>
          <p>Credential 将保存到 Windows Credential Manager。</p>
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
            <small>自定义 · 官方预设；仅展示当前可直接保存的 Responses Provider。</small>
            <button className="reference-link" onClick={() => setReferenceOpen(true)} type="button">
              供应商 Responses API 支持参考
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
              setForm({ ...form, providerKey: event.target.value });
              if (invalidField === "providerKey") {
                setInvalidField(null);
                setInvalidFieldMessage(null);
              }
            }}
            pattern="[a-z0-9][a-z0-9_-]*"
            required
            value={form.providerKey}
          />
          <small className={invalidField === "providerKey" ? "field-error" : undefined}>
            {invalidField === "providerKey"
              ? invalidFieldMessage ?? "必须以小写字母或数字开头，且只能包含小写字母、数字、-、_。"
              : "稳定标识；仅允许小写字母、数字、`-` 和 `_`。"}
          </small>
        </label>

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
            <small>API Key 已从表单清空，请检查后重新输入。</small>
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
      aria-labelledby="responses-reference-title"
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
            <span className="eyebrow">Provider Reference</span>
            <h2 id="responses-reference-title">Responses API 支持参考</h2>
            <p>以下厂商已提供相关文档；实际 Codex 兼容性仍以 Model 连接测试为准。</p>
          </div>
          <button aria-label="关闭" autoFocus className="ghost-button" onClick={onClose} type="button">
            关闭
          </button>
        </header>

        <ul className="responses-reference-list">
          {responsesApiReferences.map((reference) => (
            <li key={reference.name}>
              <span>
                <strong>{reference.name}</strong>
                <small>{reference.support}</small>
              </span>
              <button
                className="reference-doc-link"
                onClick={() => void openReference(reference.url)}
                type="button"
              >
                查看文档 ↗
              </button>
            </li>
          ))}
        </ul>

        {openError && <small className="responses-reference-error">{openError}</small>}
        <small className="responses-reference-note">
          外部文档可能调整 Endpoint、支持模型或参数范围，请以厂商最新内容为准。
        </small>
      </section>
    </dialog>
  );
}

function EnvironmentDetails({ environment }: { environment: CodexEnvironmentResponse }) {
  return (
    <section className="environment-card">
      <div className="environment-heading">
        <div>
          <span className="eyebrow">Codex Environment</span>
          <h2>{environment.supported ? "客户端满足当前基线" : "客户端需要处理"}</h2>
        </div>
        <span className={environment.supported ? "result ready" : "result blocked"}>
          {environment.supported ? "READY" : "BLOCKED"}
        </span>
      </div>
      <dl>
        <EnvironmentField label="Executable" value={environment.executablePath} />
        <EnvironmentField label="CODEX_HOME" value={environment.codexHome} />
        <EnvironmentField
          label="Config access"
          value={`${environment.configurationReadable ? "可读" : "不可读"} / ${environment.configurationWritable ? "可写" : "不可写"}`}
        />
      </dl>
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
      <dd title={value ?? undefined}>{value ?? "未知"}</dd>
    </div>
  );
}

type AgentFormField = "agentKey" | "name" | "description" | "instruction" | "modelId";

function invalidAgentField(values: {
  agentKey?: string;
  name: string;
  description: string;
  instruction: string;
  instructionRequired: boolean;
}): AgentFormField | null {
  if (values.agentKey !== undefined && !/^[a-z][a-z0-9_-]{0,63}$/.test(values.agentKey)) {
    return "agentKey";
  }
  if (!values.name.trim() || values.name.length > 160) return "name";
  if (!values.description.trim() || values.description.length > 2000) return "description";
  if (values.instructionRequired && !values.instruction.trim()) return "instruction";
  if (values.instruction.length > 100000) return "instruction";
  return null;
}

function agentErrorField(reason: unknown): AgentFormField | null {
  const field = errorField(reason);
  if (["agentKey", "name", "description", "instruction", "modelId"].includes(field ?? "")) {
    return field as AgentFormField;
  }
  const code = errorCode(reason);
  if (code === "AGENT_NAME_CONFLICT") return "agentKey";
  if (code === "MODEL_NOT_FOUND" || code === "MODEL_INCOMPATIBLE") return "modelId";
  return null;
}

function errorMessage(reason: unknown): string {
  if (reason instanceof Error) return reason.message;
  if (
    typeof reason === "object" &&
    reason !== null &&
    "message" in reason &&
    typeof reason.message === "string"
  ) {
    return reason.message;
  }
  return "操作失败，请重试。";
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
