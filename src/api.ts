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

export interface SettingsResponse {
  appearance: Appearance;
  autoBackupEnabled: boolean;
  updateChannel: string;
  customCodexHome: string | null;
}

export interface SettingsUpdateRequest {
  appearance?: Appearance;
  autoBackupEnabled?: boolean;
  updateChannel?: string;
  customCodexHome?: string | null;
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

export interface ConfigurationChange {
  operation: "CREATE" | "UPDATE" | "DELETE";
  resourceType: "CODEX_PROVIDER" | "CODEX_AGENT" | "MODEL_CATALOG";
  logicalKey: string;
  summary: string;
}

export interface ConfigurationApplyPreview {
  desiredStateHash: string;
  changes: ConfigurationChange[];
  blockers: DiagnosticIssue[];
  warnings: DiagnosticIssue[];
  hasChanges: boolean;
}

export interface ConfigurationApplyResponse {
  transactionId: string;
  status: "APPLIED" | "NO_CHANGES" | "FAILED_ROLLED_BACK" | "RECOVERY_REQUIRED";
  snapshotId: string | null;
  appliedAt: string | null;
  changedResourceCount: number;
  restartRecommended: boolean;
  warnings: DiagnosticIssue[];
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

export function previewConfigurationApply(): Promise<ConfigurationApplyPreview> {
  return invoke<ConfigurationApplyPreview>("configuration_preview_apply");
}

export function applyConfiguration(
  expectedDesiredStateHash: string | null,
): Promise<ConfigurationApplyResponse> {
  return invoke<ConfigurationApplyResponse>("configuration_apply", {
    request: { expectedDesiredStateHash },
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
export type CredentialStatus = "CONFIGURED" | "MISSING" | "STORE_UNAVAILABLE";

export interface ProviderSummary {
  id: string;
  providerKey: string;
  name: string;
  providerType: "PRESET" | "CUSTOM";
  protocol: ProviderProtocol;
  enabled: boolean;
  status: ProviderStatus;
  credentialStatus: CredentialStatus;
  modelCount: number;
}

export interface ProviderDetailResponse {
  id: string;
  providerKey: string;
  name: string;
  providerType: "PRESET" | "CUSTOM";
  baseUrl: string;
  protocol: ProviderProtocol;
  authStrategy: "OS_SECRET_HELPER";
  enabled: boolean;
  source: "BUILT_IN" | "USER";
  presetId: string | null;
  credentialStatus: CredentialStatus;
  modelCount: number;
  lastCheck: null;
  createdAt: string;
  updatedAt: string;
}

export interface ProviderCreateRequest {
  providerKey: string;
  name: string;
  presetId?: string | null;
  baseUrl: string;
  protocol: ProviderProtocol;
  auth: {
    strategy: "OS_SECRET_HELPER";
    secret: string;
  };
  enabled: boolean;
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
  modelId: string;
  displayName: string;
  enabled: boolean;
  lifecycle: ModelLifecycle;
  compatibility: CompatibilityLevel;
  contextWindow: number | null;
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
}

export interface ModelDetailResponse {
  id: string;
  provider: { id: string; name: string };
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

export function listModels(request: ModelListRequest = {}): Promise<ModelSummary[]> {
  return invoke<ModelSummary[]>("model_list", { request });
}

export function getModel(modelId: string): Promise<ModelDetailResponse> {
  return invoke<ModelDetailResponse>("model_get", { request: { modelId } });
}

export function addModel(request: ModelAddRequest): Promise<ModelDetailResponse> {
  return invoke<ModelDetailResponse>("model_add", { request });
}

export type SandboxPolicy =
  | "READ_ONLY"
  | "WORKSPACE_WRITE"
  | "DANGER_FULL_ACCESS"
  | "INHERIT";

export type ReasoningPolicy = "INHERIT" | "LOW" | "MEDIUM" | "HIGH" | "MODEL_DEFAULT";

export type AgentAvailability =
  | "READY"
  | "DISABLED"
  | "MODEL_MISSING"
  | "PROVIDER_UNAVAILABLE"
  | "INCOMPATIBLE_MODEL"
  | "INVALID_CONFIGURATION";

export interface AgentModelReference {
  id: string;
  providerId: string;
  providerName: string;
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
}

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
  modelId?: string | null;
}

export interface AgentUpdateRequest {
  agentId: string;
  name: string;
  description: string;
  instruction: string;
  sandboxPolicy: SandboxPolicy;
  reasoningPolicy: ReasoningPolicy;
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

export function setAgentEnabled(agentId: string, enabled: boolean): Promise<AgentDetailResponse> {
  return invoke<AgentDetailResponse>("agent_set_enabled", { request: { agentId, enabled } });
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
