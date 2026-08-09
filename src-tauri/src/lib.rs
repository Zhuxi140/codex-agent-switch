mod agent;
mod codex_config;
mod codex_environment;
mod configuration;
mod domain;
mod model;
mod persistence;
mod provider;
mod settings;

use serde::Serialize;
use tauri::Manager;

use agent::{
    AgentBindingResponse, AgentCreateRequest, AgentDeleteRequest, AgentDetailResponse,
    AgentGetRequest, AgentListRequest, AgentPresetResponse, AgentRemoveModelBindingRequest,
    AgentService, AgentSetEnabledRequest, AgentSetModelBindingRequest, AgentSummary,
    AgentUpdateRequest,
};
use codex_environment::CodexEnvironment;
use configuration::{
    ConfigurationApplyPreview, ConfigurationApplyRequest, ConfigurationApplyResponse,
    ConfigurationService, ConfigurationStatus, ConfigurationStatusResponse, DiagnosticsResponse,
    DiagnosticsRunRequest, SnapshotDetailResponse, SnapshotGetRequest, SnapshotListRequest,
    SnapshotListResponse, SnapshotRestoreRequest, SnapshotRestoreResponse,
};
use model::{
    ModelAddRequest, ModelDetailResponse, ModelGetRequest, ModelListRequest, ModelService,
    ModelSummary,
};
use provider::{
    ApiError, ProviderCreateRequest, ProviderDetailResponse, ProviderGetRequest,
    ProviderListRequest, ProviderService, ProviderSummary,
};
use settings::{SettingsResponse, SettingsUpdateRequest};

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
        ipc_schema_version: 1,
        codex: CodexEnvironmentSummary {
            detected: environment.detected,
            version: environment.version,
            multi_agent_available: environment.multi_agent_available,
        },
        configuration_status: configuration.status,
        running_operation_id: state.running_operation_id(),
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
    state.environment().map_err(ApiError::from)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_home = app.path().app_local_data_dir()?;
            let database_path = data_home.join("cas.db");
            app.manage(ProviderService::open(&database_path)?);
            app.manage(ModelService::open(&database_path)?);
            app.manage(AgentService::open(&database_path)?);
            app.manage(ConfigurationService::open(&database_path, &data_home)?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_get_bootstrap,
            codex_get_environment,
            codex_redetect,
            settings_get,
            settings_update,
            provider_create,
            provider_list,
            provider_get,
            model_list,
            model_get,
            model_add,
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
            snapshot_list,
            snapshot_get,
            snapshot_restore,
            diagnostics_run
        ])
        .run(tauri::generate_context!())
        .expect("failed to run Codex Agent Switch");
}
