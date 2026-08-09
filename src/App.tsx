import { useCallback, useEffect, useState, type FormEvent } from "react";

import {
  addModel,
  applyConfiguration,
  createAgent,
  createProvider,
  deleteAgent,
  getAgent,
  getAppBootstrap,
  getCodexEnvironment,
  getConfigurationStatus,
  getSettings,
  listAgentPresets,
  listAgents,
  listModels,
  listProviders,
  listSnapshots,
  previewConfigurationApply,
  redetectCodex,
  removeAgentModelBinding,
  restoreSnapshot,
  runDiagnostics,
  setAgentEnabled,
  setAgentModelBinding,
  updateSettings,
  updateAgent,
  type Appearance,
  type AgentDetailResponse,
  type AgentPresetResponse,
  type AgentSummary,
  type AppBootstrapResponse,
  type CodexEnvironmentResponse,
  type ConfigurationApplyPreview,
  type ConfigurationStatusResponse,
  type DiagnosticsResponse,
  type ModelSummary,
  type ProviderCreateRequest,
  type ProviderSummary,
  type ReasoningPolicy,
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

  useEffect(() => {
    getAppBootstrap().then(setBootstrap).catch((reason: unknown) => {
      setError(errorMessage(reason));
    });
    getSettings()
      .then((settings) => setAppearance(settings.appearance))
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
            bootstrap={bootstrap}
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
          <SettingsPage onAppearanceChange={setAppearance} />
        ) : (
          <ModelsPage onOpenProviders={() => setPage("providers")} />
        )}
      </main>
    </div>
  );
}

interface OverviewPageProps {
  bootstrap: AppBootstrapResponse | null;
  detecting: boolean;
  environment: CodexEnvironmentResponse | null;
  error: string | null;
  onRedetect: () => void;
}

function OverviewPage({
  bootstrap,
  detecting,
  environment,
  error,
  onRedetect,
}: OverviewPageProps) {
  const [configuration, setConfiguration] = useState<ConfigurationStatusResponse | null>(null);
  const [preview, setPreview] = useState<ConfigurationApplyPreview | null>(null);
  const [snapshots, setSnapshots] = useState<SnapshotSummary[]>([]);
  const [operation, setOperation] = useState<"preview" | "apply" | "restore" | null>(null);
  const [configurationError, setConfigurationError] = useState<string | null>(null);
  const [configurationSuccess, setConfigurationSuccess] = useState<string | null>(null);

  const reloadConfiguration = useCallback(async () => {
    const [status, history] = await Promise.all([
      getConfigurationStatus(),
      listSnapshots(6),
    ]);
    setConfiguration(status);
    setSnapshots(history.items);
  }, []);

  useEffect(() => {
    reloadConfiguration().catch((reason: unknown) => {
      setConfigurationError(errorMessage(reason));
    });
  }, [reloadConfiguration]);

  async function handlePreview() {
    setOperation("preview");
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      setPreview(await previewConfigurationApply());
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConfigurationError(errorMessage(reason));
    } finally {
      setOperation(null);
    }
  }

  async function handleApply() {
    if (!preview) return;
    setOperation("apply");
    setConfigurationError(null);
    setConfigurationSuccess(null);
    try {
      const result = await applyConfiguration(preview.desiredStateHash);
      setConfigurationSuccess(
        result.status === "APPLIED"
          ? `已应用 ${result.changedResourceCount} 个资源；已创建回滚 Snapshot。`
          : result.status === "NO_CHANGES"
            ? "磁盘配置已与 Desired State 一致。"
            : "Apply 未完成，配置已自动回滚。",
      );
      setPreview(null);
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
      setConfigurationSuccess("Snapshot 已恢复；操作前状态也已另存为 Snapshot。");
      setPreview(null);
      await reloadConfiguration();
    } catch (reason: unknown) {
      setConfigurationError(errorMessage(reason));
      await reloadConfiguration();
    } finally {
      setOperation(null);
    }
  }

  return (
    <>
      <header>
        <div>
          <span className="eyebrow">V0.1 · Configuration Projection</span>
          <h1>管理 Codex 子 Agent 模型绑定</h1>
          <p>先预览 CAS 管理资源，再以快照、冲突检测与自动回滚安全写入 Codex。</p>
        </div>
        <button className="secondary-button" disabled={detecting} onClick={onRedetect}>
          {detecting ? "检测中…" : "重新检测 Codex"}
        </button>
      </header>

      {error ? (
        <section className="notice error">
          <strong>后端连接失败</strong>
          <p>{error}</p>
        </section>
      ) : !bootstrap ? (
        <section className="notice">正在读取应用状态…</section>
      ) : (
        <section className="status-grid" aria-label="应用状态">
          <StatusCard label="应用版本" value={bootstrap.appVersion} />
          <StatusCard
            label="Codex"
            value={bootstrap.codex.detected ? bootstrap.codex.version ?? "已检测" : "未检测"}
          />
          <StatusCard
            label="配置状态"
            value={configuration?.status ?? bootstrap.configurationStatus}
          />
        </section>
      )}

      {environment && <EnvironmentDetails environment={environment} />}

      <section className="configuration-card">
        <div className="configuration-heading">
          <div>
            <span className="eyebrow">Apply</span>
            <h2>配置预览与安全应用</h2>
            <p>只修改 CAS-owned Provider fragment 与 `agents/cas-*.toml`。</p>
          </div>
          <button
            className="secondary-button"
            disabled={operation !== null}
            onClick={handlePreview}
            type="button"
          >
            {operation === "preview" ? "编译中…" : "预览 Apply"}
          </button>
        </div>

        {configurationSuccess && <div className="success-banner">{configurationSuccess}</div>}
        {configurationError && <div className="inline-error">{configurationError}</div>}

        {configuration && configuration.issues.length > 0 && !preview && (
          <ul className="configuration-issues">
            {configuration.issues.map((issue) => (
              <li key={`${issue.code}-${issue.message}`}>
                <strong>{issue.code}</strong>
                <span>{issue.message}</span>
              </li>
            ))}
          </ul>
        )}

        {preview && (
          <div className="apply-preview">
            <div className="preview-summary">
              <strong>{preview.changes.length} 个逻辑变化</strong>
              <span>{preview.blockers.length} blockers · {preview.warnings.length} warnings</span>
            </div>
            {preview.changes.length > 0 ? (
              <ul className="change-list">
                {preview.changes.map((change) => (
                  <li key={`${change.resourceType}-${change.logicalKey}`}>
                    <span className={`change-operation ${change.operation.toLowerCase()}`}>
                      {change.operation}
                    </span>
                    <div>
                      <strong>{change.logicalKey}</strong>
                      <small>{change.summary}</small>
                    </div>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="empty-copy">当前磁盘配置已与 Desired State 一致。</p>
            )}
            {[...preview.blockers, ...preview.warnings].length > 0 && (
              <ul className="configuration-issues">
                {[...preview.blockers, ...preview.warnings].map((issue) => (
                  <li key={`${issue.code}-${issue.message}`}>
                    <strong>{issue.severity}</strong>
                    <span>{issue.message}</span>
                  </li>
                ))}
              </ul>
            )}
            <div className="form-actions">
              <button className="secondary-button" onClick={() => setPreview(null)} type="button">
                关闭预览
              </button>
              <button
                className="primary-button"
                disabled={!preview.hasChanges || preview.blockers.length > 0 || operation !== null}
                onClick={handleApply}
                type="button"
              >
                {operation === "apply" ? "应用中…" : "Apply"}
              </button>
            </div>
          </div>
        )}
      </section>

      <section className="configuration-card snapshot-card">
        <div className="configuration-heading">
          <div>
            <span className="eyebrow">Snapshots</span>
            <h2>最近备份</h2>
          </div>
        </div>
        {snapshots.length === 0 ? (
          <p className="empty-copy">首次 Apply 前会自动生成 Snapshot。</p>
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
}: {
  onAppearanceChange: (appearance: Appearance) => void;
}) {
  const [settings, setSettings] = useState<SettingsResponse | null>(null);
  const [environment, setEnvironment] = useState<CodexEnvironmentResponse | null>(null);
  const [customCodexHome, setCustomCodexHome] = useState("");
  const [saving, setSaving] = useState(false);
  const [detecting, setDetecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([getSettings(), getCodexEnvironment()])
      .then(([loadedSettings, loadedEnvironment]) => {
        setSettings(loadedSettings);
        setCustomCodexHome(loadedSettings.customCodexHome ?? "");
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
      });
      setSettings(updated);
      setCustomCodexHome(updated.customCodexHome ?? "");
      setEnvironment(await getCodexEnvironment());
      onAppearanceChange(updated.appearance);
      setSuccess("设置已保存，Codex 环境已按新路径重新检测。");
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
            <label className="enabled-field full-width">
              <input checked disabled type="checkbox" />
              <span>
                Apply 前自动备份
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
      const [agentList, modelList, presetList] = await Promise.all([
        listAgents(),
        listModels({ enabled: true }),
        listAgentPresets(),
      ]);
      setAgents(agentList);
      setModels(modelList);
      setPresets(presetList);
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
          {agents.map((agent) => <AgentRow agent={agent} key={agent.id} onOpen={setSelectedId} />)}
        </section>
      )}
    </>
  );
}

function AgentRow({ agent, onOpen }: { agent: AgentSummary; onOpen: (id: string) => void }) {
  return (
    <article className="agent-row">
      <div className="agent-main">
        <div className="provider-name-line">
          <h2>{agent.name}</h2>
          <span className={`agent-state ${agent.availability.toLowerCase()}`}>
            {availabilityLabel(agent.availability)}
          </span>
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
  const [enabled, setEnabled] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function selectTemplate(key: string) {
    setTemplateKey(key);
    const preset = presets.find((value) => value.key === key);
    setAgentKey(preset?.key ?? "");
    setName(preset?.name ?? "");
    setDescription(preset?.description ?? "");
    setInstruction("");
    setSandboxPolicy(preset?.defaultSandboxPolicy ?? "INHERIT");
    setReasoningPolicy(preset?.defaultReasoningPolicy ?? "MODEL_DEFAULT");
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await createAgent({
        agentKey,
        name,
        description,
        instruction,
        templateKey: templateKey || null,
        enabled,
        sandboxPolicy,
        reasoningPolicy,
        modelId: modelId || null,
      });
      onCreated();
    } catch (reason: unknown) {
      setError(errorMessage(reason));
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

      <form className="provider-form" onSubmit={submit}>
        <label className="field full-width">
          <span>Template</span>
          <select onChange={(event) => selectTemplate(event.target.value)} value={templateKey}>
            {presets.map((preset) => <option key={preset.key} value={preset.key}>{preset.name}</option>)}
            <option value="">Blank / Custom</option>
          </select>
        </label>

        <label className="field">
          <span>Name</span>
          <input maxLength={160} onChange={(event) => setName(event.target.value)} required value={name} />
        </label>

        <label className="field">
          <span>Key</span>
          <input
            maxLength={64}
            onChange={(event) => setAgentKey(event.target.value)}
            pattern="[a-z][a-z0-9_-]*"
            required
            value={agentKey}
          />
          <small>创建后不可修改。</small>
        </label>

        <label className="field full-width">
          <span>Description</span>
          <textarea maxLength={2000} onChange={(event) => setDescription(event.target.value)} required rows={3} value={description} />
        </label>

        <label className="field full-width">
          <span>Instructions {templateKey && <em>Optional override</em>}</span>
          <textarea
            maxLength={100000}
            onChange={(event) => setInstruction(event.target.value)}
            placeholder={templateKey ? "留空时使用后端正式模板" : "描述 Agent 的职责与行为约束"}
            required={!templateKey}
            rows={6}
            value={instruction}
          />
        </label>

        <PolicyFields
          reasoningPolicy={reasoningPolicy}
          sandboxPolicy={sandboxPolicy}
          setReasoningPolicy={setReasoningPolicy}
          setSandboxPolicy={setSandboxPolicy}
        />

        <label className="field full-width">
          <span>Model <em>Optional</em></span>
          <select onChange={(event) => setModelId(event.target.value)} value={modelId}>
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
        </label>

        <label className="enabled-field full-width">
          <input checked={enabled} onChange={(event) => setEnabled(event.target.checked)} type="checkbox" />
          <span>Enabled<small>禁用后仍保留角色与绑定。</small></span>
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
  models,
  onBack,
  onChanged,
  onDeleted,
}: {
  agentId: string;
  models: ModelSummary[];
  onBack: () => void;
  onChanged: (message: string) => void;
  onDeleted: () => void;
}) {
  const [agent, setAgent] = useState<AgentDetailResponse | null>(null);
  const [modelId, setModelId] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setAgent(null);
    setError(null);
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
    setSaving(true);
    setError(null);
    try {
      await updateAgent({
        agentId: agent.id,
        name: agent.name,
        description: agent.description,
        instruction: agent.instruction,
        sandboxPolicy: agent.sandboxPolicy,
        reasoningPolicy: agent.reasoningPolicy,
      });
      await setAgentEnabled(agent.id, agent.enabled);
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
          <p><code>{agent.agentKey}</code> · {agent.agentType}</p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={onBack}>返回列表</button>
      </div>

      <form className="provider-form" onSubmit={submit}>
        <label className="field">
          <span>Name</span>
          <input maxLength={160} onChange={(event) => setAgent({ ...agent, name: event.target.value })} required value={agent.name} />
        </label>

        <label className="field">
          <span>Key</span>
          <div className="static-value">{agent.agentKey}</div>
          <small>身份字段不可在普通编辑中修改。</small>
        </label>

        <label className="field full-width">
          <span>Description</span>
          <textarea maxLength={2000} onChange={(event) => setAgent({ ...agent, description: event.target.value })} required rows={3} value={agent.description} />
        </label>

        <label className="field full-width">
          <span>Instructions</span>
          <textarea maxLength={100000} onChange={(event) => setAgent({ ...agent, instruction: event.target.value })} required rows={8} value={agent.instruction} />
        </label>

        <PolicyFields
          reasoningPolicy={agent.reasoningPolicy}
          sandboxPolicy={agent.sandboxPolicy}
          setReasoningPolicy={(value) => setAgent({ ...agent, reasoningPolicy: value })}
          setSandboxPolicy={(value) => setAgent({ ...agent, sandboxPolicy: value })}
        />

        <label className="field full-width">
          <span>Model</span>
          <select onChange={(event) => setModelId(event.target.value)} value={modelId}>
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
        </label>

        <label className="enabled-field full-width">
          <input checked={agent.enabled} onChange={(event) => setAgent({ ...agent, enabled: event.target.checked })} type="checkbox" />
          <span>Enabled<small>停用不会删除 Instructions 或 Model Binding。</small></span>
        </label>

        <CompatibilityPanel compatibility={agent.compatibility} />

        {error && <div className="inline-error full-width" role="alert"><strong>Agent 保存失败</strong><span>{error}</span></div>}

        <div className="form-actions agent-actions full-width">
          <button className="danger-button" disabled={saving} onClick={() => void remove()} type="button">删除 Agent</button>
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
    READY: "Active",
    DISABLED: "Disabled",
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
  const [error, setError] = useState<string | null>(null);
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

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Providers</span>
          <h1>模型服务来源</h1>
          <p>管理 Codex Agent 使用的 Responses API Provider。</p>
        </div>
        {!adding && (
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

      {adding && (
        <AddProviderPanel onCancel={() => setAdding(false)} onCreated={handleCreated} />
      )}

      {!adding && error && (
        <section className="notice error provider-notice">
          <strong>无法读取 Provider</strong>
          <p>{error}</p>
          <button className="secondary-button" onClick={() => void load()}>
            重试
          </button>
        </section>
      )}

      {!adding && loading && <section className="notice provider-notice">正在读取 Provider…</section>}

      {!adding && !loading && !error && providers.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">P</div>
          <h2>还没有 Provider</h2>
          <p>添加模型 Provider 后，即可为 Codex 子 Agent 分配外部模型。</p>
          <button className="primary-button" onClick={() => setAdding(true)}>
            添加 Provider
          </button>
        </section>
      )}

      {!adding && !loading && !error && providers.length > 0 && (
        <section className="provider-list" aria-label="Provider 列表">
          {providers.map((provider) => (
            <ProviderRow key={provider.id} provider={provider} />
          ))}
        </section>
      )}
    </>
  );
}

function ProviderRow({ provider }: { provider: ProviderSummary }) {
  const ready = provider.status === "READY" && provider.credentialStatus === "CONFIGURED";
  const label =
    provider.credentialStatus !== "CONFIGURED"
      ? "Credential missing"
      : provider.status === "DISABLED"
        ? "Disabled"
        : "Ready";

  return (
    <article className="provider-row">
      <div className="provider-avatar" aria-hidden="true">
        {provider.name.slice(0, 1).toUpperCase()}
      </div>
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
    </article>
  );
}

function ModelsPage({ onOpenProviders }: { onOpenProviders: () => void }) {
  const [models, setModels] = useState<ModelSummary[]>([]);
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [modelList, providerList] = await Promise.all([
        listModels(),
        listProviders({ enabled: true }),
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

  return (
    <>
      <header className="page-header">
        <div>
          <span className="eyebrow">Models</span>
          <h1>可绑定模型</h1>
          <p>查看模型来自哪里，以及是否已确认兼容 Codex Multi-Agent。</p>
        </div>
        {!adding && providers.length > 0 && (
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

      {adding && (
        <AddModelPanel
          onCancel={() => setAdding(false)}
          onCreated={handleCreated}
          providers={providers}
        />
      )}

      {!adding && error && (
        <section className="notice error provider-notice">
          <strong>无法读取 Model</strong>
          <p>{error}</p>
          <button className="secondary-button" onClick={() => void load()}>重试</button>
        </section>
      )}

      {!adding && loading && <section className="notice provider-notice">正在读取 Model…</section>}

      {!adding && !loading && !error && providers.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">M</div>
          <h2>请先添加 Provider</h2>
          <p>Model 必须属于一个已保存的 Responses Provider。</p>
          <button className="primary-button" onClick={onOpenProviders}>前往 Providers</button>
        </section>
      )}

      {!adding && !loading && !error && providers.length > 0 && models.length === 0 && (
        <section className="empty-state">
          <div className="empty-icon">M</div>
          <h2>还没有 Model</h2>
          <p>手动添加 Provider 实际接受的 Model ID；能力信息默认保持 Unknown。</p>
          <button className="primary-button" onClick={() => setAdding(true)}>添加 Model</button>
        </section>
      )}

      {!adding && !loading && !error && models.length > 0 && (
        <section className="model-table-wrap" aria-label="Model 列表">
          <table className="model-table">
            <thead>
              <tr>
                <th>Provider</th>
                <th>Model</th>
                <th>Compatibility</th>
                <th>Context</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {models.map((model) => (
                <ModelRow key={model.id} model={model} />
              ))}
            </tbody>
          </table>
        </section>
      )}
    </>
  );
}

function ModelRow({ model }: { model: ModelSummary }) {
  const status = !model.enabled
    ? "Disabled"
    : model.lifecycle === "ACTIVE"
      ? "Ready"
      : model.lifecycle === "UNKNOWN"
        ? "Unverified"
        : model.lifecycle;
  return (
    <tr>
      <td>{model.providerName}</td>
      <td>
        <strong>{model.displayName}</strong>
        <code>{model.modelId}</code>
      </td>
      <td>
        <span className={`compatibility ${model.compatibility.toLowerCase()}`}>
          {compatibilityLabel(model.compatibility)}
        </span>
      </td>
      <td>{model.contextWindow ? formatTokenCount(model.contextWindow) : "Unknown"}</td>
      <td><span className={status === "Ready" ? "status-text ready" : "status-text"}>{status}</span></td>
    </tr>
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
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    try {
      await addModel({
        providerId,
        modelId,
        displayName: displayName.trim() || null,
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

function formatTokenCount(value: number): string {
  return new Intl.NumberFormat("en-US", { notation: "compact" }).format(value);
}

type ProviderKind = "deepseek" | "custom";

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

function AddProviderPanel({
  onCancel,
  onCreated,
}: {
  onCancel: () => void;
  onCreated: () => void;
}) {
  const [kind, setKind] = useState<ProviderKind | null>(null);
  const [form, setForm] = useState<ProviderFormState>(formDefaults.custom);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function selectKind(selected: ProviderKind) {
    setKind(selected);
    setForm({ ...formDefaults[selected] });
    setError(null);
  }

  function cancel() {
    setForm((current) => ({ ...current, secret: "" }));
    onCancel();
  }

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!kind) return;

    setSaving(true);
    setError(null);
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
    } finally {
      request.auth.secret = "";
      setForm((current) => ({ ...current, secret: "" }));
      setSaving(false);
    }
    if (created) onCreated();
  }

  if (!kind) {
    return (
      <section className="provider-panel">
        <div className="panel-heading">
          <div>
            <span className="eyebrow">Add Provider</span>
            <h2>选择接入方式</h2>
            <p>这里只显示当前版本能够直接保存的 Responses Provider。</p>
          </div>
          <button className="ghost-button" onClick={cancel}>取消</button>
        </div>
        <div className="preset-grid">
          <button className="preset-option" onClick={() => selectKind("deepseek")}>
            <strong>DeepSeek</strong>
            <span>V4 Flash 官方 Responses Preset</span>
            <small>预填官方 Base URL</small>
          </button>
          <button className="preset-option" onClick={() => selectKind("custom")}>
            <strong>Custom Responses</strong>
            <span>任意已验证的 Responses API Provider</span>
            <small>手动填写连接信息</small>
          </button>
        </div>
      </section>
    );
  }

  return (
    <section className="provider-panel">
      <div className="panel-heading">
        <div>
          <span className="eyebrow">Add Provider</span>
          <h2>{kind === "deepseek" ? "添加 DeepSeek" : "添加 Custom Responses Provider"}</h2>
          <p>Credential 将保存到 Windows Credential Manager。</p>
        </div>
        <button className="ghost-button" disabled={saving} onClick={() => setKind(null)}>
          更换类型
        </button>
      </div>

      <form className="provider-form" onSubmit={submit}>
        <label className="field">
          <span>Name</span>
          <input
            autoFocus
            maxLength={120}
            onChange={(event) => setForm({ ...form, name: event.target.value })}
            required
            value={form.name}
          />
          <small>Provider 在 CAS 中显示的名称。</small>
        </label>

        <label className="field">
          <span>Provider Key</span>
          <input
            maxLength={64}
            onChange={(event) => setForm({ ...form, providerKey: event.target.value })}
            pattern="[a-z0-9][a-z0-9_-]*"
            required
            value={form.providerKey}
          />
          <small>稳定标识；仅允许小写字母、数字、`-` 和 `_`。</small>
        </label>

        <label className="field full-width">
          <span>Base URL</span>
          <input
            inputMode="url"
            maxLength={2048}
            onChange={(event) => setForm({ ...form, baseUrl: event.target.value })}
            required
            type="url"
            value={form.baseUrl}
          />
          <small>远程地址必须使用 HTTPS；HTTP 仅允许本机回环地址。</small>
        </label>

        <label className="field full-width">
          <span>API Key / Bearer Token</span>
          <input
            autoComplete="new-password"
            maxLength={2560}
            onChange={(event) => setForm({ ...form, secret: event.target.value })}
            required
            spellCheck={false}
            type="password"
            value={form.secret}
          />
          <small>只在本次提交期间存在于表单内存，提交完成后立即清空。</small>
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

function StatusCard({ label, value }: { label: string; value: string }) {
  return (
    <article className="status-card">
      <span>{label}</span>
      <strong>{value}</strong>
    </article>
  );
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
