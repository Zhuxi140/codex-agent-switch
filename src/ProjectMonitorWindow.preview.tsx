import { useEffect } from "react";

import { ProjectMonitorView } from "./ProjectMonitorWindow";
import type {
  AgentThreadInstanceResponse,
  AgentThreadProjectSummaryResponse,
  NativeSubagentSyncResponse,
} from "./api";

const project: AgentThreadProjectSummaryResponse = {
  workspaceScopeKey: "c:/workspace/codex-agent-switch",
  instanceCount: 4,
  agentCount: 2,
  runningCount: 1,
  recoveryRequiredCount: 0,
  totalTokens: 184_220,
  lastUsedAt: "2026-08-23T12:00:00Z",
};

const instance: AgentThreadInstanceResponse = {
  id: "preview-instance",
  agentId: "preview-agent",
  agentNameSnapshot: "Codex Native / Executor",
  codexThreadId: "019ffb28-0000-7000-8000-123456789abc",
  parentThreadId: "019ffb15-0000-7000-8000-123456789abc",
  workspaceScopeKey: project.workspaceScopeKey,
  status: "RUNNING",
  inputTokens: 0,
  cachedInputTokens: 0,
  outputTokens: 0,
  totalTokens: 42_180,
  currentContextTokens: 20_000,
  contextWindow: 256_000,
  runtimeFingerprint: "preview",
  createdAt: "2026-08-23T11:00:00Z",
  lastUsedAt: "2026-08-23T12:00:00Z",
  lastModelUsageAt: "2026-08-23T12:00:00Z",
  lastObservedAt: "2026-08-23T12:00:00Z",
  taskScopeKey: null,
  closedAt: null,
};

const sync: NativeSubagentSyncResponse = {
  capability: "SUPPORTED",
  sourcePath: "c:/users/demo/.codex/state_5.sqlite",
  discoveredCount: 4,
  syncedCount: 4,
  unmappedCount: 0,
  message: "原生同步正常。",
  attemptedAt: "2026-08-23T12:00:00Z",
  lastSuccessAt: "2026-08-23T12:00:00Z",
};

const noop = () => undefined;

export function ProjectMonitorWindowPreview() {
  useEffect(() => {
    document.documentElement.classList.add("project-monitor-document");
    return () => document.documentElement.classList.remove("project-monitor-document");
  }, []);

  return (
    <div className="project-monitor-preview">
      <ProjectMonitorView
        activeAgentCount={1}
        error={null}
        instances={[instance]}
        loading={false}
        observationDelta={8_410}
        onHide={noop}
        onOpenMain={noop}
        onRefresh={noop}
        onSelect={noop}
        onTogglePin={noop}
        orchestrationEnabled
        pinned
        projectExcluded={false}
        projects={[project]}
        selectedKey={project.workspaceScopeKey}
        sync={sync}
      />
      <ProjectMonitorView
        activeAgentCount={0}
        error="项目监控读取失败，请重试。"
        instances={[]}
        loading={false}
        observationDelta={0}
        onHide={noop}
        onOpenMain={noop}
        onRefresh={noop}
        onSelect={noop}
        onTogglePin={noop}
        orchestrationEnabled={false}
        pinned={false}
        projectExcluded={false}
        projects={[]}
        selectedKey={null}
        sync={null}
      />
    </div>
  );
}
