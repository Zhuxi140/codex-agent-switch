mod agent;
mod codex_config;
mod codex_environment;
mod codex_schema_probe;
mod configuration;
mod domain;
mod model;
mod persistence;
mod provider;
mod runtime_bridge;
mod settings;
mod usage;

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

use agent::{
    AgentBindingResponse, AgentCreateRequest, AgentDeleteRequest, AgentDetailResponse,
    AgentGetRequest, AgentListRequest, AgentPresetResponse, AgentRemoveModelBindingRequest,
    AgentService, AgentSetEnabledRequest, AgentSetModelBindingRequest, AgentSummary,
    AgentUpdateRequest,
};
use codex_environment::CodexEnvironment;
use configuration::{
    CodexMcpServerResponse, ConfigurationApplyPreview, ConfigurationApplyRequest,
    ConfigurationApplyResponse, ConfigurationService, ConfigurationStatus,
    ConfigurationStatusResponse, DiagnosticsResponse, DiagnosticsRunRequest,
    ProjectExclusionAddRequest, ProjectExclusionDeleteRequest, ProjectExclusionResponse,
    RuntimeModeConflictResolveRequest, RuntimeModeResponse, RuntimeModeSwitchRequest,
    SnapshotDetailResponse, SnapshotGetRequest, SnapshotListRequest, SnapshotListResponse,
    SnapshotRestoreRequest, SnapshotRestoreResponse,
};
use model::{
    ModelAddRequest, ModelConnectionTestResponse, ModelDeleteRequest, ModelDetailResponse,
    ModelGetRequest, ModelListRequest, ModelService, ModelSetEnabledRequest, ModelSummary,
    ModelTestConnectionRequest, ModelUpdateRequest,
};
use provider::{
    ApiError, DeleteResult, ProviderCreateRequest, ProviderDeleteRequest, ProviderDetailResponse,
    ProviderGetRequest, ProviderListRequest, ProviderService, ProviderSummary,
    ProviderUpdateRequest,
};
use runtime_bridge::{
    AgentThreadExecutionResponse, ManagedSessionRecoveryRequest, ManagedSessionResponse,
    ManagedSessionResumeRequest, ManagedSessionStartRequest, ManagedTurnStartRequest,
    ManagedTurnStartResponse, RuntimeBridgeService, RuntimeBridgeStatusResponse,
};
use settings::{SettingsResponse, SettingsUpdateRequest};
use usage::{
    AgentScheduleDecisionListRequest, AgentThreadCleanupRequest, AgentThreadCleanupResponse,
    AgentThreadExecutionRequest, AgentThreadInstanceListRequest, AgentThreadInstanceListResponse,
    AgentThreadInstanceRecommendRequest, AgentThreadInstanceRecommendation,
    AgentThreadInstanceResponse, AgentThreadInstanceReuseStateRequest,
    AgentThreadInstanceWorkspaceScopeRequest, AgentThreadProjectListResponse,
    AgentThreadProjectSummaryResponse, NativeSubagentSyncResponse,
    RuntimeEnforcementEventListRequest, RuntimeEnforcementEventResponse, ScheduleDecisionResponse,
    UsageListRequest, UsageQueryRequest, UsageRecordResponse, UsageService, UsageSummaryResponse,
};

const PROJECT_MONITOR_WINDOW_LABEL: &str = "project-monitor";

#[derive(Default)]
struct NativeObserverService {
    state: Mutex<NativeObserverState>,
}

#[derive(Default)]
struct NativeObserverState {
    source_path: Option<String>,
    last_success_at: Option<String>,
}

impl NativeObserverService {
    fn sync(
        &self,
        usage: &UsageService,
        configuration: &ConfigurationService,
    ) -> Result<NativeSubagentSyncResponse, ApiError> {
        let attempted_at = usage.current_timestamp().map_err(ApiError::from)?;
        let response = sync_native_subagents_once(usage, configuration)?;
        let mut observer = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let source_path = response.source_path().map(str::to_owned);
        if observer.source_path != source_path {
            observer.source_path = source_path;
            observer.last_success_at = None;
        }
        if response.is_supported() {
            observer.last_success_at = Some(attempted_at.clone());
        }
        Ok(response.with_observer_timestamps(attempted_at, observer.last_success_at.clone()))
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMonitorSnapshotRequest {
    workspace_scope_key: Option<String>,
    #[serde(default)]
    include_instances: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProjectMonitorSnapshotResponse {
    projects: Vec<AgentThreadProjectSummaryResponse>,
    instances: Vec<AgentThreadInstanceResponse>,
    sync: NativeSubagentSyncResponse,
    orchestration_enabled: bool,
    active_agent_count: usize,
    project_excluded: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppBootstrapResponse {
    app_version: &'static str,
    ipc_schema_version: u32,
    codex: CodexEnvironmentSummary,
    configuration_status: ConfigurationStatus,
    running_operation_id: Option<String>,
    recovery_required: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexEnvironmentSummary {
    detected: bool,
    version: Option<String>,
    multi_agent_available: bool,
    runtime_hooks_available: bool,
}

#[tauri::command]
fn app_get_bootstrap(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<AppBootstrapResponse, ApiError> {
    let environment = state.environment().map_err(ApiError::from)?;
    let configuration = state.get_status();
    let recovery_required = matches!(configuration.status, ConfigurationStatus::RecoveryRequired);

    Ok(AppBootstrapResponse {
        app_version: env!("CARGO_PKG_VERSION"),
        ipc_schema_version: 5,
        codex: CodexEnvironmentSummary {
            detected: environment.detected,
            version: environment.version,
            multi_agent_available: environment.multi_agent_available,
            runtime_hooks_available: environment.runtime_hooks_available,
        },
        configuration_status: configuration.status,
        running_operation_id: configuration.active_operation_id.clone(),
        recovery_required,
    })
}

#[tauri::command]
fn codex_get_environment(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<CodexEnvironment, ApiError> {
    state.environment().map_err(ApiError::from)
}

#[tauri::command]
fn codex_redetect(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<CodexEnvironment, ApiError> {
    codex_environment::clear_capability_cache();
    state.environment().map_err(ApiError::from)
}

#[tauri::command]
fn codex_mcp_server_list(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<Vec<CodexMcpServerResponse>, ApiError> {
    state.list_mcp_servers().map_err(ApiError::from)
}

#[tauri::command]
fn settings_get(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<SettingsResponse, ApiError> {
    state.get_settings().map_err(ApiError::from)
}

#[tauri::command]
fn settings_update(
    state: tauri::State<'_, ConfigurationService>,
    request: SettingsUpdateRequest,
) -> Result<SettingsResponse, ApiError> {
    state.update_settings(request).map_err(ApiError::from)
}

#[tauri::command]
fn provider_create(
    state: tauri::State<'_, ProviderService>,
    request: ProviderCreateRequest,
) -> Result<ProviderDetailResponse, ApiError> {
    state.create(request)
}

#[tauri::command]
fn provider_list(
    state: tauri::State<'_, ProviderService>,
    request: ProviderListRequest,
) -> Result<Vec<ProviderSummary>, ApiError> {
    state.list(request)
}

#[tauri::command]
fn provider_get(
    state: tauri::State<'_, ProviderService>,
    request: ProviderGetRequest,
) -> Result<ProviderDetailResponse, ApiError> {
    state.get(request)
}

#[tauri::command]
fn provider_update(
    state: tauri::State<'_, ProviderService>,
    request: ProviderUpdateRequest,
) -> Result<ProviderDetailResponse, ApiError> {
    state.update(request)
}

#[tauri::command]
fn provider_delete(
    state: tauri::State<'_, ProviderService>,
    request: ProviderDeleteRequest,
) -> Result<DeleteResult, ApiError> {
    state.delete(request)
}

#[tauri::command]
fn model_list(
    state: tauri::State<'_, ModelService>,
    request: ModelListRequest,
) -> Result<Vec<ModelSummary>, ApiError> {
    state.list(request)
}

#[tauri::command]
fn model_get(
    state: tauri::State<'_, ModelService>,
    request: ModelGetRequest,
) -> Result<ModelDetailResponse, ApiError> {
    state.get(request)
}

#[tauri::command]
fn model_add(
    state: tauri::State<'_, ModelService>,
    request: ModelAddRequest,
) -> Result<ModelDetailResponse, ApiError> {
    state.add(request)
}

#[tauri::command]
fn model_update(
    state: tauri::State<'_, ModelService>,
    request: ModelUpdateRequest,
) -> Result<ModelDetailResponse, ApiError> {
    state.update(request)
}

#[tauri::command]
fn model_set_enabled(
    state: tauri::State<'_, ModelService>,
    request: ModelSetEnabledRequest,
) -> Result<ModelDetailResponse, ApiError> {
    state.set_enabled(request)
}

#[tauri::command]
fn model_delete(
    state: tauri::State<'_, ModelService>,
    request: ModelDeleteRequest,
) -> Result<(), ApiError> {
    state.delete(request)
}

#[tauri::command]
async fn model_test_connection(
    state: tauri::State<'_, ModelService>,
    request: ModelTestConnectionRequest,
) -> Result<ModelConnectionTestResponse, ApiError> {
    state.test_connection(request).await
}

#[tauri::command]
fn agent_preset_list(state: tauri::State<'_, AgentService>) -> Vec<AgentPresetResponse> {
    state.presets()
}

#[tauri::command]
fn agent_list(
    state: tauri::State<'_, AgentService>,
    request: AgentListRequest,
) -> Result<Vec<AgentSummary>, ApiError> {
    state.list(request)
}

#[tauri::command]
fn agent_get(
    state: tauri::State<'_, AgentService>,
    request: AgentGetRequest,
) -> Result<AgentDetailResponse, ApiError> {
    state.get(request)
}

#[tauri::command]
fn agent_create(
    state: tauri::State<'_, AgentService>,
    request: AgentCreateRequest,
) -> Result<AgentDetailResponse, ApiError> {
    state.create(request)
}

#[tauri::command]
fn agent_update(
    state: tauri::State<'_, AgentService>,
    request: AgentUpdateRequest,
) -> Result<AgentDetailResponse, ApiError> {
    state.update(request)
}

#[tauri::command]
fn agent_set_enabled(
    state: tauri::State<'_, AgentService>,
    request: AgentSetEnabledRequest,
) -> Result<AgentDetailResponse, ApiError> {
    state.set_enabled(request)
}

#[tauri::command]
fn agent_set_model_binding(
    state: tauri::State<'_, AgentService>,
    request: AgentSetModelBindingRequest,
) -> Result<AgentBindingResponse, ApiError> {
    state.set_model_binding(request)
}

#[tauri::command]
fn agent_remove_model_binding(
    state: tauri::State<'_, AgentService>,
    request: AgentRemoveModelBindingRequest,
) -> Result<AgentDetailResponse, ApiError> {
    state.remove_model_binding(request)
}

#[tauri::command]
fn agent_delete(
    state: tauri::State<'_, AgentService>,
    request: AgentDeleteRequest,
) -> Result<(), ApiError> {
    state.delete(request)
}

#[tauri::command]
fn configuration_get_status(
    state: tauri::State<'_, ConfigurationService>,
) -> ConfigurationStatusResponse {
    state.get_status()
}

#[tauri::command]
fn configuration_preview_apply(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<ConfigurationApplyPreview, ApiError> {
    state.preview_apply().map_err(ApiError::from)
}

#[tauri::command]
fn configuration_apply(
    state: tauri::State<'_, ConfigurationService>,
    request: ConfigurationApplyRequest,
) -> Result<ConfigurationApplyResponse, ApiError> {
    state.apply(request).map_err(ApiError::from)
}

#[tauri::command]
fn runtime_mode_get(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<RuntimeModeResponse, ApiError> {
    state.runtime_mode().map_err(ApiError::from)
}

#[tauri::command]
fn runtime_mode_switch(
    state: tauri::State<'_, ConfigurationService>,
    request: RuntimeModeSwitchRequest,
) -> Result<ConfigurationApplyResponse, ApiError> {
    state.switch_runtime_mode(request).map_err(ApiError::from)
}

#[tauri::command]
fn runtime_mode_resolve_conflict(
    state: tauri::State<'_, ConfigurationService>,
    request: RuntimeModeConflictResolveRequest,
) -> Result<ConfigurationApplyResponse, ApiError> {
    state
        .resolve_runtime_mode_conflict(request)
        .map_err(ApiError::from)
}

#[tauri::command]
fn project_exclusion_list(
    state: tauri::State<'_, ConfigurationService>,
) -> Result<Vec<ProjectExclusionResponse>, ApiError> {
    state.list_project_exclusions().map_err(ApiError::from)
}

#[tauri::command]
fn project_exclusion_add(
    state: tauri::State<'_, ConfigurationService>,
    request: ProjectExclusionAddRequest,
) -> Result<ProjectExclusionResponse, ApiError> {
    state.add_project_exclusion(request).map_err(ApiError::from)
}

#[tauri::command]
fn project_exclusion_delete(
    state: tauri::State<'_, ConfigurationService>,
    request: ProjectExclusionDeleteRequest,
) -> Result<(), ApiError> {
    state
        .delete_project_exclusion(request)
        .map_err(ApiError::from)
}

#[tauri::command]
fn snapshot_list(
    state: tauri::State<'_, ConfigurationService>,
    request: SnapshotListRequest,
) -> Result<SnapshotListResponse, ApiError> {
    state.snapshot_list(request).map_err(ApiError::from)
}

#[tauri::command]
fn snapshot_get(
    state: tauri::State<'_, ConfigurationService>,
    request: SnapshotGetRequest,
) -> Result<SnapshotDetailResponse, ApiError> {
    state.snapshot_get(request).map_err(ApiError::from)
}

#[tauri::command]
fn snapshot_restore(
    state: tauri::State<'_, ConfigurationService>,
    request: SnapshotRestoreRequest,
) -> Result<SnapshotRestoreResponse, ApiError> {
    state.snapshot_restore(request).map_err(ApiError::from)
}

#[tauri::command]
fn diagnostics_run(
    state: tauri::State<'_, ConfigurationService>,
    request: DiagnosticsRunRequest,
) -> Result<DiagnosticsResponse, ApiError> {
    state.run_diagnostics(request).map_err(ApiError::from)
}

#[tauri::command]
fn usage_get_summary(
    state: tauri::State<'_, UsageService>,
    request: UsageQueryRequest,
) -> Result<UsageSummaryResponse, ApiError> {
    state.summary(request)
}

#[tauri::command]
fn usage_list_records(
    state: tauri::State<'_, UsageService>,
    request: UsageListRequest,
) -> Result<Vec<UsageRecordResponse>, ApiError> {
    state.list(request)
}

#[tauri::command]
fn agent_thread_instance_list(
    state: tauri::State<'_, UsageService>,
    configuration: tauri::State<'_, ConfigurationService>,
    observer: tauri::State<'_, NativeObserverService>,
    request: AgentThreadInstanceListRequest,
) -> Result<AgentThreadInstanceListResponse, ApiError> {
    let sync = observer.sync(&state, &configuration)?;
    let page = state.list_agent_instances(request)?;
    Ok(AgentThreadInstanceListResponse::new(page, sync))
}

#[tauri::command]
fn agent_thread_project_list(
    state: tauri::State<'_, UsageService>,
    configuration: tauri::State<'_, ConfigurationService>,
    observer: tauri::State<'_, NativeObserverService>,
) -> Result<AgentThreadProjectListResponse, ApiError> {
    let sync = observer.sync(&state, &configuration)?;
    let items = state.list_agent_thread_projects()?;
    Ok(AgentThreadProjectListResponse::new(items, sync))
}

#[tauri::command]
fn native_subagent_sync(
    state: tauri::State<'_, UsageService>,
    configuration: tauri::State<'_, ConfigurationService>,
    observer: tauri::State<'_, NativeObserverService>,
) -> Result<NativeSubagentSyncResponse, ApiError> {
    observer.sync(&state, &configuration)
}

#[tauri::command]
fn project_monitor_snapshot(
    state: tauri::State<'_, UsageService>,
    configuration: tauri::State<'_, ConfigurationService>,
    observer: tauri::State<'_, NativeObserverService>,
    request: ProjectMonitorSnapshotRequest,
) -> Result<ProjectMonitorSnapshotResponse, ApiError> {
    let sync = observer.sync(&state, &configuration)?;
    let projects = state.list_agent_thread_projects()?;
    let instances = if request.include_instances {
        state
            .list_agent_instances(AgentThreadInstanceListRequest::for_project(
                request.workspace_scope_key.clone(),
            ))?
            .into_items()
    } else {
        Vec::new()
    };
    let mode = configuration
        .project_monitor_mode(request.workspace_scope_key.as_deref())
        .map_err(ApiError::from)?;
    Ok(ProjectMonitorSnapshotResponse {
        projects,
        instances,
        sync,
        orchestration_enabled: mode.active_agent_count > 0,
        active_agent_count: mode.active_agent_count,
        project_excluded: request.include_instances && mode.project_excluded,
    })
}

#[tauri::command]
async fn project_monitor_open(app: tauri::AppHandle) -> Result<(), ApiError> {
    if let Some(window) = app.get_webview_window(PROJECT_MONITOR_WINDOW_LABEL) {
        window.show().map_err(|_| project_monitor_window_error())?;
        window
            .set_focus()
            .map_err(|_| project_monitor_window_error())?;
        return Ok(());
    }
    let window = WebviewWindowBuilder::new(
        &app,
        PROJECT_MONITOR_WINDOW_LABEL,
        WebviewUrl::App("index.html?window=project-monitor".into()),
    )
    .title("CAS 项目监控")
    .inner_size(380.0, 360.0)
    .min_inner_size(320.0, 260.0)
    .resizable(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .shadow(true)
    .build()
    .map_err(|_| project_monitor_window_error())?;
    let window_to_hide = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_to_hide.hide();
        }
    });
    Ok(())
}

#[tauri::command]
fn project_monitor_hide(app: tauri::AppHandle) -> Result<(), ApiError> {
    if let Some(window) = app.get_webview_window(PROJECT_MONITOR_WINDOW_LABEL) {
        window.hide().map_err(|_| project_monitor_window_error())?;
    }
    Ok(())
}

#[tauri::command]
fn project_monitor_set_always_on_top(
    app: tauri::AppHandle,
    always_on_top: bool,
) -> Result<(), ApiError> {
    if let Some(window) = app.get_webview_window(PROJECT_MONITOR_WINDOW_LABEL) {
        window
            .set_always_on_top(always_on_top)
            .map_err(|_| project_monitor_window_error())?;
    }
    Ok(())
}

#[tauri::command]
fn project_monitor_focus_main(app: tauri::AppHandle) -> Result<(), ApiError> {
    if let Some(window) = app.get_webview_window("main") {
        window.show().map_err(|_| project_monitor_window_error())?;
        window
            .set_focus()
            .map_err(|_| project_monitor_window_error())?;
    }
    Ok(())
}

fn project_monitor_window_error() -> ApiError {
    ApiError::new(
        "PROJECT_MONITOR_WINDOW_FAILED",
        "项目监控浮窗操作失败，请重试。",
        true,
        None,
    )
}

fn sync_native_subagents_once(
    state: &UsageService,
    configuration: &ConfigurationService,
) -> Result<NativeSubagentSyncResponse, ApiError> {
    Ok(match configuration.environment() {
        Ok(environment) => match environment.codex_home {
            Some(codex_home) => state.sync_native_subagents(std::path::Path::new(&codex_home))?,
            None => NativeSubagentSyncResponse::unavailable(
                "无法定位 CODEX_HOME；尚不能同步 Primary 原生子 Agent。",
            ),
        },
        Err(_) => NativeSubagentSyncResponse::unavailable(
            "读取 Codex 环境失败；尚不能同步 Primary 原生子 Agent。",
        ),
    })
}

#[tauri::command]
fn agent_thread_instance_set_workspace_scope(
    state: tauri::State<'_, UsageService>,
    request: AgentThreadInstanceWorkspaceScopeRequest,
) -> Result<AgentThreadInstanceResponse, ApiError> {
    state.set_agent_instance_workspace_scope(request)
}

#[tauri::command]
fn agent_thread_instance_set_reuse_state(
    state: tauri::State<'_, UsageService>,
    request: AgentThreadInstanceReuseStateRequest,
) -> Result<AgentThreadInstanceResponse, ApiError> {
    state.set_agent_instance_reuse_state(request)
}

#[tauri::command]
fn agent_thread_instance_cleanup(
    state: tauri::State<'_, UsageService>,
    configuration: tauri::State<'_, ConfigurationService>,
    observer: tauri::State<'_, NativeObserverService>,
    request: AgentThreadCleanupRequest,
) -> Result<AgentThreadCleanupResponse, ApiError> {
    observer.sync(&state, &configuration)?;
    state.cleanup_agent_instances(request)
}

#[tauri::command]
fn agent_thread_instance_recommend(
    state: tauri::State<'_, UsageService>,
    request: AgentThreadInstanceRecommendRequest,
) -> Result<AgentThreadInstanceRecommendation, ApiError> {
    state.recommend_agent_instance(request)
}

#[tauri::command]
fn agent_schedule_decision_list(
    state: tauri::State<'_, UsageService>,
    request: AgentScheduleDecisionListRequest,
) -> Result<Vec<ScheduleDecisionResponse>, ApiError> {
    state.list_schedule_decisions(request)
}

#[tauri::command]
fn runtime_enforcement_event_list(
    state: tauri::State<'_, UsageService>,
    request: RuntimeEnforcementEventListRequest,
) -> Result<Vec<RuntimeEnforcementEventResponse>, ApiError> {
    state.list_runtime_enforcement_events(request)
}

#[tauri::command]
fn agent_thread_instance_execute(
    bridge: tauri::State<'_, RuntimeBridgeService>,
    request: AgentThreadExecutionRequest,
) -> Result<AgentThreadExecutionResponse, ApiError> {
    bridge.execute_agent_thread(request)
}

#[tauri::command]
fn usage_monitor_start(
    bridge: tauri::State<'_, RuntimeBridgeService>,
    configuration: tauri::State<'_, ConfigurationService>,
) -> Result<RuntimeBridgeStatusResponse, ApiError> {
    let environment = configuration.environment().map_err(ApiError::from)?;
    let executable = environment.executable_path.ok_or_else(|| {
        ApiError::new(
            "CODEX_EXECUTABLE_NOT_FOUND",
            "未找到 Codex 可执行文件。",
            false,
            None,
        )
    })?;
    let codex_home = environment.codex_home.ok_or_else(|| {
        ApiError::new(
            "CODEX_HOME_UNRESOLVED",
            "无法定位 CODEX_HOME。",
            false,
            None,
        )
    })?;
    bridge.start(
        std::path::Path::new(&executable),
        std::path::Path::new(&codex_home),
        environment.version,
    )
}

#[tauri::command]
fn usage_monitor_stop(
    bridge: tauri::State<'_, RuntimeBridgeService>,
) -> Result<RuntimeBridgeStatusResponse, ApiError> {
    bridge.stop()
}

#[tauri::command]
fn usage_monitor_recover(
    bridge: tauri::State<'_, RuntimeBridgeService>,
) -> Result<RuntimeBridgeStatusResponse, ApiError> {
    bridge.recover()
}

#[tauri::command]
fn usage_monitor_status(
    bridge: tauri::State<'_, RuntimeBridgeService>,
) -> Result<RuntimeBridgeStatusResponse, ApiError> {
    bridge.status()
}

#[tauri::command]
fn usage_managed_session_start(
    bridge: tauri::State<'_, RuntimeBridgeService>,
    request: ManagedSessionStartRequest,
) -> Result<ManagedSessionResponse, ApiError> {
    bridge.managed_session_start(request)
}

#[tauri::command]
fn usage_managed_session_resume(
    bridge: tauri::State<'_, RuntimeBridgeService>,
    request: ManagedSessionResumeRequest,
) -> Result<ManagedSessionResponse, ApiError> {
    bridge.managed_session_resume(request)
}

#[tauri::command]
fn usage_managed_session_resolve_recovery(
    bridge: tauri::State<'_, RuntimeBridgeService>,
    request: ManagedSessionRecoveryRequest,
) -> Result<ManagedSessionResponse, ApiError> {
    bridge.managed_session_resolve_recovery(request)
}

#[tauri::command]
fn usage_managed_turn_start(
    bridge: tauri::State<'_, RuntimeBridgeService>,
    request: ManagedTurnStartRequest,
) -> Result<ManagedTurnStartResponse, ApiError> {
    bridge.managed_turn_start(request)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_home = app.path().app_local_data_dir()?;
            let database_path = data_home.join("cas.db");
            app.manage(ProviderService::open(&database_path)?);
            app.manage(ModelService::open(&database_path)?);
            app.manage(AgentService::open(&database_path)?);
            app.manage(UsageService::open(&database_path)?);
            app.manage(NativeObserverService::default());
            app.manage(RuntimeBridgeService::open(&database_path, &data_home)?);
            app.manage(ConfigurationService::open(&database_path, &data_home)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_get_bootstrap,
            codex_get_environment,
            codex_redetect,
            codex_mcp_server_list,
            settings_get,
            settings_update,
            provider_create,
            provider_list,
            provider_get,
            provider_update,
            provider_delete,
            model_list,
            model_get,
            model_add,
            model_update,
            model_set_enabled,
            model_delete,
            model_test_connection,
            agent_preset_list,
            agent_list,
            agent_get,
            agent_create,
            agent_update,
            agent_set_enabled,
            agent_set_model_binding,
            agent_remove_model_binding,
            agent_delete,
            configuration_get_status,
            configuration_preview_apply,
            configuration_apply,
            runtime_mode_get,
            runtime_mode_switch,
            runtime_mode_resolve_conflict,
            project_exclusion_list,
            project_exclusion_add,
            project_exclusion_delete,
            snapshot_list,
            snapshot_get,
            snapshot_restore,
            diagnostics_run,
            usage_get_summary,
            usage_list_records,
            native_subagent_sync,
            agent_thread_instance_list,
            agent_thread_project_list,
            project_monitor_snapshot,
            project_monitor_open,
            project_monitor_hide,
            project_monitor_set_always_on_top,
            project_monitor_focus_main,
            agent_schedule_decision_list,
            runtime_enforcement_event_list,
            agent_thread_instance_set_workspace_scope,
            agent_thread_instance_set_reuse_state,
            agent_thread_instance_cleanup,
            agent_thread_instance_recommend,
            agent_thread_instance_execute,
            usage_monitor_start,
            usage_monitor_stop,
            usage_monitor_recover,
            usage_monitor_status,
            usage_managed_session_start,
            usage_managed_session_resume,
            usage_managed_session_resolve_recovery,
            usage_managed_turn_start
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Codex Agent Switch")
        .run(|app, event| {
            // 关闭应用时清理 .codex 下的编排投影：与「切回 Default」走同一恢复路径
            // （按 baseline 还原 config.toml 片段与 AGENTS.md、删除 agents/cas-*.toml
            // 等托管资源），保证磁盘与 CAS 状态一致。
            if let tauri::RunEvent::ExitRequested { .. } = event
                && let Some(configuration) = app.try_state::<ConfigurationService>()
                && let Err(error) =
                    configuration.switch_runtime_mode(RuntimeModeSwitchRequest::default())
            {
                eprintln!("退出清理编排投影失败：{error}");
            }
        });
}
