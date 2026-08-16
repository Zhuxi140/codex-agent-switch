import { invoke } from "@tauri-apps/api/core";

export interface CodexEnvironmentSummary {
  detected: boolean;
  version: string | null;
  multiAgentAvailable: boolean;
}

export type DiagnosticSeverity = "INFO" | "WARNING" | "ERROR";

export interface DiagnosticIssue {
  code: string;
  severity: DiagnosticSeverity;
  message: string;
}

export interface DiagnosticSection {
  key: string;
  title: string;
  issues: DiagnosticIssue[];
}

export interface DiagnosticsResponse {
  overall: "HEALTHY" | "WARNING" | "ERROR";
  sections: DiagnosticSection[];
  checkedAt: string;
}

export interface CodexEnvironmentResponse extends CodexEnvironmentSummary {
  executablePath: string | null;
  codexHome: string | null;
  supported: boolean;
  configurationReadable: boolean;
  configurationWritable: boolean;
  issues: DiagnosticIssue[];
}

export type Appearance = "SYSTEM" | "LIGHT" | "DARK";
export type OrchestrationFailurePolicy = "STRICT_STOP" | "PRIMARY_FALLBACK";

export interface SettingsResponse {
  appearance: Appearance;
  autoBackupEnabled: boolean;
  updateChannel: string;
  customCodexHome: string | null;
  customFontFamily: string | null;
  orchestrationFailurePolicy: OrchestrationFailurePolicy;
}

export interface SettingsUpdateRequest {
  appearance?: Appearance;
  autoBackupEnabled?: boolean;
  updateChannel?: string;
  customCodexHome?: string | null;
  customFontFamily?: string | null;
  orchestrationFailurePolicy?: OrchestrationFailurePolicy;
}

export type ConfigurationStatus =
  | "APPLIED"
  | "PENDING_CHANGES"
  | "DRIFT"
  | "CONFLICT"
  | "RECOVERY_REQUIRED"
  | "UNAVAILABLE";

export interface AppBootstrapResponse {
  appVersion: string;
  ipcSchemaVersion: number;
  codex: CodexEnvironmentSummary;
  configurationStatus: ConfigurationStatus;
  runningOperationId: string | null;
  recoveryRequired: boolean;
}

export function getAppBootstrap(): Promise<AppBootstrapResponse> {
  return invoke<AppBootstrapResponse>("app_get_bootstrap");
}

export function getCodexEnvironment(): Promise<CodexEnvironmentResponse> {
  return invoke<CodexEnvironmentResponse>("codex_get_environment");
}

export function redetectCodex(): Promise<CodexEnvironmentResponse> {
  return invoke<CodexEnvironmentResponse>("codex_redetect");
}

export function getSettings(): Promise<SettingsResponse> {
  return invoke<SettingsResponse>("settings_get");
}

export function updateSettings(request: SettingsUpdateRequest): Promise<SettingsResponse> {
  return invoke<SettingsResponse>("settings_update", { request });
}

export function runDiagnostics(includeNetworkChecks = false): Promise<DiagnosticsResponse> {
  return invoke<DiagnosticsResponse>("diagnostics_run", {
    request: { includeNetworkChecks },
  });
}

export interface ConfigurationStatusResponse {
  status: ConfigurationStatus;
  desiredStateHash: string | null;
  lastAppliedAt: string | null;
  driftCount: number;
  conflictCount: number;
  restartRecommended: boolean;
  issues: DiagnosticIssue[];
}

export interface ConfigurationApplyResponse {
  transactionId: string;
  status: "APPLIED" | "NO_CHANGES" | "CONFLICT" | "FAILED_ROLLED_BACK" | "RECOVERY_REQUIRED";
  snapshotId: string | null;
  appliedAt: string | null;
  changedResourceCount: number;
  restartRecommended: boolean;
  warnings: DiagnosticIssue[];
  conflict: ConfigurationConflictResponse | null;
}

export interface ConfigurationConflictResponse {
  codexHome: string;
  desiredStateHash: string;
  conflictToken: string;
  canAdopt: boolean;
  resources: ConfigurationConflictResource[];
}

export interface ConfigurationConflictResource {
  code: "RESOURCE_OWNERSHIP_CONFLICT" | "MANAGED_RESOURCE_CONFLICT";
  resourceType: string;
  logicalKey: string;
  path: string;
  matchesDesired: boolean;
  replaceable: boolean;
}

export interface RuntimeModeResponse {
  activeBindings: ActiveAgentBinding[];
  legacyActiveAgentId: string | null;
}

export interface ProjectExclusion {
  id: string;
  projectPath: string;
  createdAt: string;
}

export interface ActiveAgentBinding {
  roleKey: string;
  phase: OrchestrationPhase;
  agentId: string;
}

export interface SnapshotSummary {
  id: string;
  reason: string;
  codexVersion: string | null;
  status: string;
  createdAt: string;
  resourceCount: number;
}

export interface SnapshotListResponse {
  items: SnapshotSummary[];
  nextCursor: string | null;
}

export interface SnapshotRestoreResponse {
  transactionId: string;
  restoredSnapshotId: string;
  restoredAt: string;
  configurationStatus: ConfigurationStatus;
  warnings: DiagnosticIssue[];
}

export function getConfigurationStatus(): Promise<ConfigurationStatusResponse> {
  return invoke<ConfigurationStatusResponse>("configuration_get_status");
}

export function getRuntimeMode(): Promise<RuntimeModeResponse> {
  return invoke<RuntimeModeResponse>("runtime_mode_get");
}

export function switchRuntimeMode(
  activeAgentIds: string[],
): Promise<ConfigurationApplyResponse> {
  return invoke<ConfigurationApplyResponse>("runtime_mode_switch", {
    request: { activeAgentIds },
  });
}

export function resolveRuntimeModeConflict(
  activeAgentIds: string[],
  strategy: "ADOPT" | "REPLACE",
  conflict: ConfigurationConflictResponse,
): Promise<ConfigurationApplyResponse> {
  return invoke<ConfigurationApplyResponse>("runtime_mode_resolve_conflict", {
    request: {
      activeAgentIds,
      strategy,
      expectedDesiredStateHash: conflict.desiredStateHash,
      expectedConflictToken: conflict.conflictToken,
    },
  });
}

export function listProjectExclusions(): Promise<ProjectExclusion[]> {
  return invoke<ProjectExclusion[]>("project_exclusion_list");
}

export function addProjectExclusion(projectPath: string): Promise<ProjectExclusion> {
  return invoke<ProjectExclusion>("project_exclusion_add", {
    request: { projectPath },
  });
}

export function deleteProjectExclusion(exclusionId: string): Promise<void> {
  return invoke<void>("project_exclusion_delete", {
    request: { exclusionId },
  });
}

export function listSnapshots(limit = 10): Promise<SnapshotListResponse> {
  return invoke<SnapshotListResponse>("snapshot_list", { request: { limit } });
}

export function restoreSnapshot(snapshotId: string): Promise<SnapshotRestoreResponse> {
  return invoke<SnapshotRestoreResponse>("snapshot_restore", { request: { snapshotId } });
}

export type ProviderProtocol = "RESPONSES";
export type ProviderStatus = "READY" | "DISABLED";
export type CredentialStatus =
  | "CONFIGURED"
  | "MISSING"
  | "STORE_UNAVAILABLE"
  | "CODEX_SESSION";
export type ProviderCacheSupport = "UNKNOWN" | "SUPPORTED" | "UNSUPPORTED";
export type ProviderCacheRetentionType = "UNKNOWN" | "APPROXIMATE" | "GUARANTEED";

export interface ProviderCacheProfile {
  cacheSupport: ProviderCacheSupport;
  retentionType: ProviderCacheRetentionType;
  retentionHintSeconds: number | null;
  source: string | null;
  lastVerifiedAt: string | null;
}

export interface ProviderSummary {
  id: string;
  providerKey: string;
  name: string;
  providerType: "PRESET" | "CUSTOM";
  presetId: string | null;
  protocol: ProviderProtocol;
  enabled: boolean;
  status: ProviderStatus;
  credentialStatus: CredentialStatus;
  modelCount: number;
  cacheSupport: ProviderCacheSupport;
}

export interface ProviderDetailResponse {
  id: string;
  providerKey: string;
  name: string;
  providerType: "PRESET" | "CUSTOM";
  baseUrl: string;
  protocol: ProviderProtocol;
  authStrategy: "OS_SECRET_HELPER" | "CODEX_SESSION";
  enabled: boolean;
  source: "BUILT_IN" | "USER";
  presetId: string | null;
  credentialStatus: CredentialStatus;
  modelCount: number;
  lastCheck: null;
  cacheProfile: ProviderCacheProfile;
  createdAt: string;
  updatedAt: string;
}

export interface ProviderCreateRequest {
  providerKey: string;
  name: string;
  presetId?: string | null;
  baseUrl: string;
  protocol: ProviderProtocol;
  auth:
    | {
      strategy: "OS_SECRET_HELPER";
      secret: string;
    }
    | {
      strategy: "NONE";
    };
  enabled: boolean;
}

export interface ProviderUpdateRequest {
  providerId: string;
  name: string;
  baseUrl: string;
  enabled: boolean;
  confirmOriginChange?: boolean;
  cacheProfile?: ProviderCacheProfile;
}

export interface DeleteResult {
  deleted: boolean;
}

export interface ProviderListRequest {
  search?: string | null;
  enabled?: boolean | null;
}

export function createProvider(
  request: ProviderCreateRequest,
): Promise<ProviderDetailResponse> {
  return invoke<ProviderDetailResponse>("provider_create", { request });
}

export function listProviders(request: ProviderListRequest = {}): Promise<ProviderSummary[]> {
  return invoke<ProviderSummary[]>("provider_list", { request });
}

export function getProvider(providerId: string): Promise<ProviderDetailResponse> {
  return invoke<ProviderDetailResponse>("provider_get", { request: { providerId } });
}

export function updateProvider(
  request: ProviderUpdateRequest,
): Promise<ProviderDetailResponse> {
  return invoke<ProviderDetailResponse>("provider_update", { request });
}

export function deleteProvider(providerId: string): Promise<DeleteResult> {
  return invoke<DeleteResult>("provider_delete", { request: { providerId } });
}

export type CompatibilityLevel =
  | "NATIVE"
  | "COMPATIBLE"
  | "GATEWAY_REQUIRED"
  | "UNSUPPORTED"
  | "UNKNOWN";

export type ModelLifecycle = "ACTIVE" | "DEPRECATED" | "PREVIEW" | "UNKNOWN";

export interface ModelSummary {
  id: string;
  providerId: string;
  providerName: string;
  providerKey: string;
  providerPresetId: string | null;
  modelId: string;
  displayName: string;
  enabled: boolean;
  lifecycle: ModelLifecycle;
  compatibility: CompatibilityLevel;
  contextWindow: number | null;
  source: "PRESET" | "USER" | "IMPORTED";
  reasoningStatus: "SUPPORTED" | "UNSUPPORTED" | "UNKNOWN";
  supportedReasoningEfforts: string[];
  defaultReasoningEffort: string | null;
  lastTestStatus: ModelConnectionTestStatus | null;
  lastTestedAt: string | null;
  lastTestLatencyMs: number | null;
}

export interface ModelListRequest {
  search?: string | null;
  providerId?: string | null;
  enabled?: boolean | null;
  compatibility?: CompatibilityLevel | null;
}

export interface ModelAddRequest {
  providerId: string;
  modelId: string;
  displayName?: string | null;
  contextWindow?: number | null;
}

export interface ModelDetailResponse {
  id: string;
  provider: { id: string; name: string; providerKey: string; presetId: string | null };
  modelId: string;
  displayName: string;
  enabled: boolean;
  lifecycle: ModelLifecycle;
  contextWindow: number | null;
  maxOutputTokens: number | null;
  reasoning: {
    status: "SUPPORTED" | "UNSUPPORTED" | "UNKNOWN";
    supportedEfforts: string[];
    defaultEffort: string | null;
  };
  capabilities: Array<{
    capability: string;
    status: "SUPPORTED" | "UNSUPPORTED" | "UNKNOWN";
    source: string;
    confidence: string;
    verifiedAt: string | null;
  }>;
  compatibility: {
    level: CompatibilityLevel;
    source: string;
    minimumCodexVersion: string | null;
    verifiedAt: string | null;
  };
  createdAt: string;
  updatedAt: string;
}

export type ModelConnectionTestStatus =
  | "SUCCESS"
  | "CREDENTIAL_MISSING"
  | "AUTH_FAILED"
  | "MODEL_NOT_FOUND"
  | "RATE_LIMITED"
  | "PROTOCOL_ERROR"
  | "UNREACHABLE"
  | "SERVER_ERROR";

export interface ModelConnectionTestResponse {
  status: ModelConnectionTestStatus;
  latencyMs: number | null;
  providerRequestId: string | null;
  message: string;
}

export function listModels(request: ModelListRequest = {}): Promise<ModelSummary[]> {
  return invoke<ModelSummary[]>("model_list", { request });
}

export function getModel(modelId: string): Promise<ModelDetailResponse> {
  return invoke<ModelDetailResponse>("model_get", { request: { modelId } });
}

export function addModel(request: ModelAddRequest): Promise<ModelDetailResponse> {
  return invoke<ModelDetailResponse>("model_add", { request });
}

export function updateModel(modelId: string, displayName: string): Promise<ModelDetailResponse> {
  return invoke<ModelDetailResponse>("model_update", { request: { modelId, displayName } });
}

export function setModelEnabled(modelId: string, enabled: boolean): Promise<ModelDetailResponse> {
  return invoke<ModelDetailResponse>("model_set_enabled", { request: { modelId, enabled } });
}

export function deleteModel(modelId: string): Promise<void> {
  return invoke<void>("model_delete", { request: { modelId } });
}

export function testModelConnection(modelId: string): Promise<ModelConnectionTestResponse> {
  return invoke<ModelConnectionTestResponse>("model_test_connection", { request: { modelId } });
}

export type SandboxPolicy =
  | "READ_ONLY"
  | "WORKSPACE_WRITE"
  | "DANGER_FULL_ACCESS"
  | "INHERIT";

export type ReasoningPolicy = "INHERIT" | "LOW" | "MEDIUM" | "HIGH" | "MODEL_DEFAULT";

export type AgentAvailability =
  | "READY"
  | "MODEL_MISSING"
  | "PROVIDER_UNAVAILABLE"
  | "INCOMPATIBLE_MODEL"
  | "UNVERIFIED_MODEL"
  | "INVALID_CONFIGURATION";

export interface AgentModelReference {
  id: string;
  providerId: string;
  providerName: string;
  providerKey: string;
  modelId: string;
  displayName: string;
}

export interface AgentBindingCompatibility {
  status: "COMPATIBLE" | "WARNING" | "INCOMPATIBLE" | "UNKNOWN";
  issues: Array<{
    code: string;
    severity: "INFO" | "WARNING" | "ERROR";
    message: string;
    source: string;
  }>;
}

export interface AgentSummary {
  id: string;
  agentKey: string;
  name: string;
  description: string;
  enabled: boolean;
  model: AgentModelReference | null;
  availability: AgentAvailability;
  reasoningPolicy: ReasoningPolicy;
  reuseStrategy: AgentReuseStrategy;
  cacheRetentionOverrideSeconds: number | null;
  roleKey: string | null;
  orchestrationPhase: OrchestrationPhase | null;
}

export type OrchestrationPhase = "DISCOVERY" | "EXECUTION" | "VERIFICATION" | "REVIEW";
export type AgentReuseStrategy = "AUTO" | "HOT" | "COLD";

export interface AgentDetailResponse {
  id: string;
  agentKey: string;
  name: string;
  description: string;
  instruction: string;
  agentType: "PRESET" | "CUSTOM" | "IMPORTED";
  enabled: boolean;
  sandboxPolicy: SandboxPolicy;
  reasoningPolicy: ReasoningPolicy;
  reuseStrategy: AgentReuseStrategy;
  cacheRetentionOverrideSeconds: number | null;
  roleKey: string | null;
  orchestrationPhase: OrchestrationPhase | null;
  requiredCapabilities: string[];
  preferredCapabilities: string[];
  modelBinding: AgentModelReference | null;
  compatibility: AgentBindingCompatibility;
  source: "CAS" | "USER" | "IMPORTED";
  managed: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface AgentPresetResponse {
  key: string;
  name: string;
  description: string;
  defaultSandboxPolicy: SandboxPolicy;
  defaultReasoningPolicy: ReasoningPolicy;
  requiredCapabilities: string[];
  roleKey: string;
  orchestrationPhase: OrchestrationPhase;
}

export interface AgentCreateRequest {
  agentKey: string;
  name: string;
  description: string;
  instruction: string;
  templateKey?: string | null;
  enabled: boolean;
  sandboxPolicy: SandboxPolicy;
  reasoningPolicy: ReasoningPolicy;
  reuseStrategy: AgentReuseStrategy;
  cacheRetentionOverrideSeconds: number | null;
  modelId?: string | null;
  roleKey: string;
  orchestrationPhase?: OrchestrationPhase | null;
}

export interface AgentUpdateRequest {
  agentId: string;
  name: string;
  description: string;
  instruction: string;
  sandboxPolicy: SandboxPolicy;
  reasoningPolicy: ReasoningPolicy;
  reuseStrategy: AgentReuseStrategy;
  cacheRetentionOverrideSeconds: number | null;
  modelId?: string | null;
  roleKey: string;
  orchestrationPhase: OrchestrationPhase;
}

export function listAgentPresets(): Promise<AgentPresetResponse[]> {
  return invoke<AgentPresetResponse[]>("agent_preset_list");
}

export function listAgents(): Promise<AgentSummary[]> {
  return invoke<AgentSummary[]>("agent_list", { request: {} });
}

export function getAgent(agentId: string): Promise<AgentDetailResponse> {
  return invoke<AgentDetailResponse>("agent_get", { request: { agentId } });
}

export function createAgent(request: AgentCreateRequest): Promise<AgentDetailResponse> {
  return invoke<AgentDetailResponse>("agent_create", { request });
}

export function updateAgent(request: AgentUpdateRequest): Promise<AgentDetailResponse> {
  return invoke<AgentDetailResponse>("agent_update", { request });
}

export function setAgentModelBinding(agentId: string, modelId: string): Promise<void> {
  return invoke("agent_set_model_binding", { request: { agentId, modelId } });
}

export function removeAgentModelBinding(agentId: string): Promise<AgentDetailResponse> {
  return invoke<AgentDetailResponse>("agent_remove_model_binding", { request: { agentId } });
}

export function deleteAgent(agentId: string): Promise<void> {
  return invoke("agent_delete", { request: { agentId } });
}

export type UsageStatus = "LIVE" | "FINAL" | "PARTIAL" | "UNKNOWN";
export type UsageSource =
  | "CODEX_APP_SERVER"
  | "CODEX_EXEC_JSONL"
  | "RESPONSES_PROXY";

export interface UsageQueryRequest {
  agentId?: string | null;
  providerId?: string | null;
  modelId?: string | null;
  codexSessionId?: string | null;
  from?: string | null;
  to?: string | null;
}

export interface UsageListRequest extends UsageQueryRequest {
  limit?: number;
}

export interface UsageSummaryResponse {
  recordCount: number;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  totalTokens: number;
}

export interface UsageRecordResponse {
  id: string;
  codexSessionId: string;
  codexThreadId: string;
  parentThreadId: string | null;
  agentId: string | null;
  agentNameSnapshot: string | null;
  providerId: string | null;
  providerNameSnapshot: string | null;
  modelId: string | null;
  modelNameSnapshot: string | null;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteInputTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  totalTokens: number;
  modelContextWindow: number | null;
  usageStatus: UsageStatus;
  source: UsageSource;
  startedAt: string;
  completedAt: string | null;
  updatedAt: string;
}

export type AgentThreadInstanceStatus =
  | "RUNNING"
  | "IDLE"
  | "RECOVERY_REQUIRED"
  | "CLOSED"
  | "UNKNOWN";

export interface AgentThreadInstanceResponse {
  id: string;
  agentId: string | null;
  agentNameSnapshot: string | null;
  codexThreadId: string;
  parentThreadId: string | null;
  workspaceScopeKey: string | null;
  status: AgentThreadInstanceStatus;
  inputTokens: number;
  cachedInputTokens: number;
  outputTokens: number;
  totalTokens: number;
  currentContextTokens: number | null;
  contextWindow: number | null;
  runtimeFingerprint: string | null;
  createdAt: string;
  lastUsedAt: string;
  lastModelUsageAt: string | null;
  lastObservedAt: string | null;
  taskScopeKey: string | null;
  closedAt: string | null;
}

export interface NativeSubagentSyncResponse {
  capability: "SUPPORTED" | "UNAVAILABLE" | "INCOMPATIBLE";
  sourcePath: string | null;
  discoveredCount: number;
  syncedCount: number;
  unmappedCount: number;
  message: string;
}

export interface AgentThreadInstanceListResponse {
  items: AgentThreadInstanceResponse[];
  sync: NativeSubagentSyncResponse;
}

export interface AgentThreadInstanceRecommendation {
  decision: "REUSE" | "SPAWN";
  reasonCode:
    | "EXACT_WORKSPACE_SCOPE_IDLE"
    | "CONTEXT_PRESSURE"
    | "CONTEXT_UNKNOWN"
    | "RUNTIME_FINGERPRINT_MISMATCH"
    | "RUNTIME_FINGERPRINT_UNKNOWN"
    | "CACHE_HINT_PRESSURE"
    | "NO_HEALTHY_IDLE_THREAD"
    | "NO_WORKSPACE_SCOPE_MATCH";
  message: string;
  workspaceScopeKey: string;
  candidateInstanceId: string | null;
  candidateThreadId: string | null;
  contextPressurePercent: number | null;
  contextPressureLimitPercent: number;
  reuseStrategy: AgentReuseStrategy;
  cacheSupport: ProviderCacheSupport;
  cacheRetentionType: ProviderCacheRetentionType;
  cacheRetentionHintSeconds: number | null;
  cacheRetentionSource: "NONE" | "PROVIDER" | "AGENT_OVERRIDE";
  cacheHint:
    | "UNKNOWN"
    | "UNSUPPORTED"
    | "SUPPORTED_NO_RETENTION_HINT"
    | "WITHIN_RETENTION_HINT"
    | "OUTSIDE_RETENTION_HINT";
  candidateAgeSeconds: number | null;
}

export interface AgentThreadExecutionResponse {
  action: "REUSED" | "SPAWNED";
  decision: "REUSE" | "SPAWN";
  reasonCode: AgentThreadInstanceRecommendation["reasonCode"];
  agentId: string;
  agentName: string;
  workspaceScopeKey: string;
  threadId: string;
  turnId: string;
  status: ManagedSessionStatus;
}

export function getUsageSummary(
  request: UsageQueryRequest = {},
): Promise<UsageSummaryResponse> {
  return invoke<UsageSummaryResponse>("usage_get_summary", { request });
}

export function listUsageRecords(
  request: UsageListRequest = {},
): Promise<UsageRecordResponse[]> {
  return invoke<UsageRecordResponse[]>("usage_list_records", { request });
}

export function listAgentThreadInstances(
  request: { agentId?: string | null; limit?: number } = {},
): Promise<AgentThreadInstanceListResponse> {
  return invoke<AgentThreadInstanceListResponse>("agent_thread_instance_list", { request });
}

export interface ScheduleDecisionResponse {
  id: string;
  createdAt: string;
  source: "HELPER" | "DESKTOP_PREVIEW" | "DESKTOP_EXECUTE" | string;
  agentId: string | null;
  agentNameSnapshot: string | null;
  workspaceScopeKey: string;
  parentThreadId: string | null;
  candidateThreadId: string | null;
  decision: string;
  reasonCode: string;
  runtimeFingerprint: string | null;
  contextPressurePercent: number | null;
  contextPressureLimitPercent: number;
  cacheHint: string;
  candidateAgeSeconds: number | null;
  claimed: boolean;
  taskScopeKey: string | null;
}

export function listAgentScheduleDecisions(
  request: { limit?: number } = {},
): Promise<ScheduleDecisionResponse[]> {
  return invoke<ScheduleDecisionResponse[]>("agent_schedule_decision_list", { request });
}

export function setAgentThreadInstanceWorkspaceScope(
  threadId: string,
  workspaceScopeKey: string | null,
): Promise<AgentThreadInstanceResponse> {
  return invoke<AgentThreadInstanceResponse>("agent_thread_instance_set_workspace_scope", {
    request: { threadId, workspaceScopeKey },
  });
}

export function recommendAgentThreadInstance(
  agentId: string,
  workspaceScopeKey: string,
  parentThreadId: string | null = null,
  taskScopeKey: string | null = null,
): Promise<AgentThreadInstanceRecommendation> {
  return invoke<AgentThreadInstanceRecommendation>("agent_thread_instance_recommend", {
    request: { agentId, workspaceScopeKey, parentThreadId, taskScopeKey },
  });
}

export function executeAgentThread(request: {
  agentId: string;
  workspaceScopeKey: string;
  cwd: string;
  input: string;
  expectedDecision: "REUSE" | "SPAWN";
  expectedCandidateThreadId: string | null;
  taskScopeKey?: string | null;
}): Promise<AgentThreadExecutionResponse> {
  return invoke<AgentThreadExecutionResponse>("agent_thread_instance_execute", { request });
}

export type RuntimeBridgeStatus =
  | "STOPPED"
  | "STARTING"
  | "RUNNING"
  | "DEGRADED"
  | "FAILED";

export type ProtocolCompatibility =
  | "UNVERIFIED"
  | "COMPATIBLE"
  | "LEGACY_COMPATIBLE"
  | "DEGRADED";

export type SchemaCapability =
  | "SUPPORTED"
  | "NOT_DECLARED"
  | "INCOMPATIBLE"
  | "UNAVAILABLE";

export type ManagedSessionStatus =
  | "IDLE"
  | "RUNNING"
  | "DETACHED"
  | "RECOVERY_REQUIRED"
  | "FAILED";

export interface ManagedSessionResponse {
  threadId: string;
  sessionId: string | null;
  origin: "STARTED" | "RESUMED";
  status: ManagedSessionStatus;
  cwd: string | null;
  activeTurnId: string | null;
  attachedAt: string;
}

export interface RuntimeBridgeStatusResponse {
  status: RuntimeBridgeStatus;
  protocolCompatibility: ProtocolCompatibility;
  schemaCapability: SchemaCapability;
  managedSessionCapability: SchemaCapability;
  agentExecutionCapability: SchemaCapability;
  codexVersion: string | null;
  serverUserAgent: string | null;
  usageEventCount: number;
  malformedEventCount: number;
  startedAt: string | null;
  lastEventAt: string | null;
  lastError: string | null;
  managedSession: ManagedSessionResponse | null;
}

export function startUsageMonitor(): Promise<RuntimeBridgeStatusResponse> {
  return invoke<RuntimeBridgeStatusResponse>("usage_monitor_start");
}

export function stopUsageMonitor(): Promise<RuntimeBridgeStatusResponse> {
  return invoke<RuntimeBridgeStatusResponse>("usage_monitor_stop");
}

export function getUsageMonitorStatus(): Promise<RuntimeBridgeStatusResponse> {
  return invoke<RuntimeBridgeStatusResponse>("usage_monitor_status");
}

export function startManagedUsageSession(cwd: string): Promise<ManagedSessionResponse> {
  return invoke<ManagedSessionResponse>("usage_managed_session_start", {
    request: { cwd },
  });
}

export function resumeManagedUsageSession(
  threadId: string,
  cwd?: string,
): Promise<ManagedSessionResponse> {
  return invoke<ManagedSessionResponse>("usage_managed_session_resume", {
    request: { threadId, cwd },
  });
}

export interface ManagedTurnStartResponse {
  threadId: string;
  turnId: string;
  status: ManagedSessionStatus;
}

export function startManagedUsageTurn(
  threadId: string,
  input: string,
): Promise<ManagedTurnStartResponse> {
  return invoke<ManagedTurnStartResponse>("usage_managed_turn_start", {
    request: { threadId, input },
  });
}
