use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cas_scheduler::hard_gates::{CacheRequirement, Capability};
use cas_scheduler::normalize_workspace_scope_key;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::codex_config::render_delegated_agent_instructions_for_phase;
use crate::codex_schema_probe::{SchemaCapability, probe_schema_capabilities};
use crate::orchestration_contract::{
    ExecutionKind, IdempotencyOutcome, OrchestrationError, OrchestrationErrorCode, RouteAction,
    TaskPacket,
};
use crate::orchestration_job::{
    AtomicScheduleOutcome, AtomicScheduleRequest, DispatchAdmission, DispatchAgentProfile,
    DispatchPermit, OrchestrationJobCreateResponse, OrchestrationJobService, ScheduleStop,
};
use crate::orchestration_receipt::{ManagedTurnAcceptedEvidence, OrchestrationReceiptEventService};
use crate::provider::ApiError;
use crate::runtime_adapter::{
    AppServerMethod, NormalizedRuntimeEvent, NormalizedUsage, ProtocolParseError, ProtocolProfile,
    RecoveryTurnOutcome, parse_event, parse_thread_response, parse_turn_response,
    recovery_turn_outcome,
};
use crate::usage::{
    AgentRuntimeProfile, UsageAttribution, UsageService, UsageServiceError, UsageSnapshot,
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const PROTOCOL_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_AUTO_RECOVERY_ATTEMPTS: u8 = 3;

pub(crate) struct RuntimeBridgeService {
    data_home: PathBuf,
    database_path: PathBuf,
    helper_path: PathBuf,
    usage: Arc<UsageService>,
    receipt_events: Arc<OrchestrationReceiptEventService>,
    state: Arc<Mutex<RuntimeBridgeState>>,
    worker: Mutex<Option<BridgeWorker>>,
    launch: Mutex<Option<RuntimeBridgeLaunch>>,
    managed_operation: Mutex<()>,
}

impl RuntimeBridgeService {
    pub(crate) fn open(
        database_path: &Path,
        data_home: &Path,
        helper_path: &Path,
    ) -> Result<Self, RuntimeBridgeError> {
        Ok(Self {
            data_home: data_home.to_path_buf(),
            database_path: database_path.to_path_buf(),
            helper_path: helper_path.to_path_buf(),
            usage: Arc::new(UsageService::open(database_path)?),
            receipt_events: Arc::new(
                OrchestrationReceiptEventService::open(database_path)
                    .map_err(|_| RuntimeBridgeError::StateUnavailable)?,
            ),
            state: Arc::new(Mutex::new(RuntimeBridgeState::default())),
            worker: Mutex::new(None),
            launch: Mutex::new(None),
            managed_operation: Mutex::new(()),
        })
    }

    pub(crate) fn status(&self) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        match self.recover_inner(false) {
            Ok(status) => Ok(status),
            Err(_) => self.status_inner().map_err(ApiError::from),
        }
    }

    pub(crate) fn start(
        &self,
        executable: &Path,
        codex_home: &Path,
        codex_version: Option<String>,
    ) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.start_inner(executable, codex_home, codex_version)
            .map_err(ApiError::from)
    }

    pub(crate) fn stop(&self) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.stop_inner().map_err(ApiError::from)
    }

    pub(crate) fn recover(&self) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.recover_inner(true).map_err(ApiError::from)
    }

    pub(crate) fn managed_session_start(
        &self,
        request: ManagedSessionStartRequest,
    ) -> Result<ManagedSessionResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.managed_session_start_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn managed_session_resume(
        &self,
        request: ManagedSessionResumeRequest,
    ) -> Result<ManagedSessionResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.managed_session_resume_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn managed_session_resolve_recovery(
        &self,
        request: ManagedSessionRecoveryRequest,
    ) -> Result<ManagedSessionResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.managed_session_resolve_recovery_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn managed_turn_start(
        &self,
        request: ManagedTurnStartRequest,
    ) -> Result<ManagedTurnStartResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        self.managed_turn_start_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn execute_agent_thread(
        &self,
        orchestration: &OrchestrationJobService,
        request: AgentThreadExecutionRequest,
    ) -> Result<AgentThreadExecutionResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        let cwd = validate_cwd(&request.cwd).map_err(ApiError::from)?;
        let input = validate_turn_input(&request.input).map_err(ApiError::from)?;
        if normalize_workspace_scope_key(&cwd).as_deref()
            != Some(request.task_packet.workspace_scope_key.as_str())
        {
            return Err(orchestration_error_to_api(OrchestrationError {
                code: OrchestrationErrorCode::TaskPacketScopeMismatch,
                message: "TaskPacket 的 workspace_scope_key 与 cwd 不一致。".to_owned(),
                field_path: Some("workspace_scope_key".to_owned()),
                job_id: Some(request.task_packet.job_id.clone()),
                attempt_id: None,
            }));
        }
        let admission = self.dispatch_admission().map_err(ApiError::from)?;
        let outcome = orchestration
            .schedule_atomic(AtomicScheduleRequest {
                task_packet: request.task_packet,
                expected_decision: request.expected_decision,
                expected_candidate_thread_id: request.expected_candidate_thread_id,
                planned_execution_kind: ExecutionKind::ManagedWorker,
                admission,
            })
            .map_err(orchestration_error_to_api)?;
        let permit = match outcome {
            AtomicScheduleOutcome::Ready(permit) => permit,
            AtomicScheduleOutcome::Waiting(stop) => {
                return Err(schedule_stop_to_api(stop, true));
            }
            AtomicScheduleOutcome::Blocked(stop) => {
                return Err(schedule_stop_to_api(stop, false));
            }
            AtomicScheduleOutcome::Existing(existing) => {
                return Err(existing_job_to_api(existing));
            }
        };

        // DISPATCH_RECORDED 与 Job=DISPATCHED 必须先提交；下行函数才可触碰 App Server。
        orchestration
            .authorize_dispatch(&permit)
            .map_err(orchestration_error_to_api)?;
        self.execute_agent_thread_inner(permit, cwd, input)
            .map_err(ApiError::from)
    }

    fn dispatch_admission(&self) -> Result<DispatchAdmission, RuntimeBridgeError> {
        let state = self.state()?;
        Ok(DispatchAdmission {
            runtime: match state.status {
                RuntimeBridgeStatus::Running | RuntimeBridgeStatus::Degraded => {
                    Capability::Supported
                }
                RuntimeBridgeStatus::Starting | RuntimeBridgeStatus::Recovering => {
                    Capability::Unknown
                }
                RuntimeBridgeStatus::Stopped | RuntimeBridgeStatus::Failed => {
                    Capability::Unsupported
                }
            },
            agent_execution: scheduler_capability(state.agent_execution_capability),
            event: scheduler_capability(state.managed_session_capability),
            runtime_healthy: state.status == RuntimeBridgeStatus::Running,
            permission_allowed: true,
            scope_allowed: true,
            schedule_certain: true,
            lease_certain: true,
            receipt_certain: true,
            schema_verified: state.managed_session_capability == SchemaCapability::Supported,
            global_concurrency_available: true,
            workspace_excluded: false,
            conversation_excluded: false,
            cache_requirement: CacheRequirement::NotRequired,
        })
    }

    fn start_inner(
        &self,
        executable: &Path,
        codex_home: &Path,
        codex_version: Option<String>,
    ) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        self.start_inner_with_hook_trust(executable, codex_home, codex_version, false)
    }

    #[cfg(test)]
    fn start_inner_for_e2e(
        &self,
        executable: &Path,
        codex_home: &Path,
        codex_version: Option<String>,
    ) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        self.start_inner_with_hook_trust(executable, codex_home, codex_version, true)
    }

    fn start_inner_with_hook_trust(
        &self,
        executable: &Path,
        codex_home: &Path,
        codex_version: Option<String>,
        bypass_hook_trust: bool,
    ) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        let mut worker_slot = self.worker()?;
        if let Some(worker) = worker_slot.as_mut() {
            if worker.is_running()? {
                return Err(RuntimeBridgeError::AlreadyRunning);
            }
            let finished = worker_slot.take().expect("worker exists");
            finished.join();
        }

        let started_at = self.usage.current_timestamp()?;
        let schema_capabilities = probe_schema_capabilities(executable, &self.data_home);
        {
            let mut state = self.state()?;
            let mut managed_sessions = state.managed_sessions.clone();
            for session in managed_sessions.values_mut() {
                if session.status == ManagedSessionStatus::Running {
                    session.status = ManagedSessionStatus::RecoveryRequired;
                }
            }
            *state = RuntimeBridgeState {
                status: RuntimeBridgeStatus::Starting,
                schema_capability: schema_capabilities.usage,
                managed_session_capability: schema_capabilities.managed_session,
                agent_execution_capability: schema_capabilities.agent_execution,
                codex_version: codex_version.clone(),
                started_at: Some(started_at),
                last_managed_thread_id: state.last_managed_thread_id.clone(),
                managed_sessions,
                ..RuntimeBridgeState::default()
            };
        }

        let mut command = Command::new(executable);
        if bypass_hook_trust {
            command.arg("--dangerously-bypass-hook-trust");
        }
        let mut child = match command
            .args(["app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => {
                let error = RuntimeBridgeError::Spawn(error);
                self.set_failed(error.to_string());
                return Err(error);
            }
        };
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                let error = RuntimeBridgeError::MissingPipe("stdin");
                let _ = child.kill();
                let _ = child.wait();
                self.set_failed(error.to_string());
                return Err(error);
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                let error = RuntimeBridgeError::MissingPipe("stdout");
                let _ = child.kill();
                let _ = child.wait();
                self.set_failed(error.to_string());
                return Err(error);
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                let error = RuntimeBridgeError::MissingPipe("stderr");
                let _ = child.kill();
                let _ = child.wait();
                self.set_failed(error.to_string());
                return Err(error);
            }
        };
        let child = Arc::new(Mutex::new(child));
        let stopping = Arc::new(AtomicBool::new(false));
        let (initialize_tx, initialize_rx) = mpsc::sync_channel(1);
        let pending_responses = Arc::new(Mutex::new(HashMap::new()));
        let stdin = Arc::new(Mutex::new(Some(stdin)));

        let state = Arc::clone(&self.state);
        let usage = Arc::clone(&self.usage);
        let receipt_events = Arc::clone(&self.receipt_events);
        let reader_stopping = Arc::clone(&stopping);
        let reader_pending_responses = Arc::clone(&pending_responses);
        let reader_stdin = Arc::clone(&stdin);
        let helper_path = self.helper_path.clone();
        let database_path = self.database_path.clone();
        let stdout_thread = thread::spawn(move || {
            read_app_server_stream(
                stdout,
                state,
                usage,
                receipt_events,
                reader_stopping,
                initialize_tx,
                reader_pending_responses,
                reader_stdin,
                helper_path,
                database_path,
            );
        });
        let stderr_thread = thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                if line.is_err() {
                    break;
                }
            }
        });

        let initialize = json!({
            "id": 1,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "codex_agent_switch",
                    "title": "Codex Agent Switch",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        });
        if let Err(error) = write_shared_message(&stdin, &initialize) {
            cleanup_unstarted_worker(child, stopping, stdout_thread, stderr_thread);
            self.set_failed(error.to_string());
            return Err(error);
        }

        let initialize_result = initialize_rx
            .recv_timeout(INITIALIZE_TIMEOUT)
            .map_err(|_| RuntimeBridgeError::InitializeTimeout)
            .and_then(|result| result);
        if let Err(error) = initialize_result {
            cleanup_unstarted_worker(child, stopping, stdout_thread, stderr_thread);
            self.set_failed(error.to_string());
            return Err(error);
        }
        if let Err(error) =
            write_shared_message(&stdin, &json!({"method": "initialized", "params": {}}))
        {
            cleanup_unstarted_worker(child, stopping, stdout_thread, stderr_thread);
            self.set_failed(error.to_string());
            return Err(error);
        }

        *worker_slot = Some(BridgeWorker {
            child,
            stdin,
            stopping,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            pending_responses,
            next_request_id: 2,
        });
        *self.launch()? = Some(RuntimeBridgeLaunch {
            executable: executable.to_path_buf(),
            codex_home: codex_home.to_path_buf(),
            codex_version,
        });
        self.state()?.status = RuntimeBridgeStatus::Running;
        self.status_inner()
    }

    fn recover_inner(
        &self,
        force: bool,
    ) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        let (attempt, previous_sessions) = {
            let mut state = self.state()?;
            if state.status != RuntimeBridgeStatus::Failed
                || (!force && !auto_recovery_allowed(&state))
            {
                return Ok(RuntimeBridgeStatusResponse::from(&*state));
            }
            if force {
                state.recovery_attempt_count = 0;
            }
            state.recovery_attempt_count += 1;
            state.status = RuntimeBridgeStatus::Recovering;
            state.last_error = None;
            (state.recovery_attempt_count, state.managed_sessions.clone())
        };
        let launch = match self.launch()?.clone() {
            Some(launch) => launch,
            None => {
                let error = RuntimeBridgeError::RecoveryLaunchUnavailable;
                self.record_recovery_failure(attempt, &error);
                return Err(error);
            }
        };
        if let Some(mut worker) = self.worker()?.take()
            && let Err(error) = worker.stop()
        {
            self.record_recovery_failure(attempt, &error);
            return Err(error);
        }
        if let Err(error) =
            self.start_inner(&launch.executable, &launch.codex_home, launch.codex_version)
        {
            self.record_recovery_failure(attempt, &error);
            return Err(error);
        }

        let recovered = (|| {
            let recovered_at = self.usage.current_timestamp()?;
            for previous in previous_sessions.into_values() {
                let mut params = serde_json::Map::from_iter([(
                    "threadId".to_owned(),
                    Value::String(previous.thread_id.clone()),
                )]);
                if let Some(cwd) = previous.cwd.as_ref() {
                    params.insert("cwd".to_owned(), Value::String(cwd.clone()));
                }
                let result = self.request(
                    AppServerMethod::ThreadResume.as_str(),
                    Value::Object(params),
                )?;
                let thread = parse_thread_response(&result).map_err(runtime_response_error)?;
                if thread.thread_id != previous.thread_id {
                    return Err(RuntimeBridgeError::UnexpectedThreadResponse);
                }
                let was_uncertain = matches!(
                    previous.status,
                    ManagedSessionStatus::Running | ManagedSessionStatus::RecoveryRequired
                );
                let recovered_status = if was_uncertain {
                    match recovery_turn_outcome(&result, previous.active_turn_id.as_deref()) {
                        RecoveryTurnOutcome::Terminal => ManagedSessionStatus::Idle,
                        RecoveryTurnOutcome::Running | RecoveryTurnOutcome::Unknown => {
                            ManagedSessionStatus::RecoveryRequired
                        }
                    }
                } else {
                    ManagedSessionStatus::Idle
                };
                let active_turn_id = (recovered_status == ManagedSessionStatus::RecoveryRequired)
                    .then_some(previous.active_turn_id)
                    .flatten();
                self.state()?.managed_sessions.insert(
                    thread.thread_id.clone(),
                    ManagedSessionState {
                        thread_id: thread.thread_id.clone(),
                        session_id: thread.session_id,
                        origin: ManagedSessionOrigin::Resumed,
                        status: recovered_status,
                        cwd: previous.cwd,
                        active_turn_id,
                        attached_at: recovered_at.clone(),
                    },
                );
                if recovered_status == ManagedSessionStatus::Idle {
                    self.usage
                        .mark_agent_execution_idle_if_known(&thread.thread_id)?;
                } else {
                    self.usage
                        .mark_agent_execution_recovery_required_if_known(&thread.thread_id)?;
                }
            }
            let mut state = self.state()?;
            state.status = RuntimeBridgeStatus::Running;
            state.recovery_attempt_count = 0;
            state.last_recovery_at = Some(recovered_at);
            state.last_error = state
                .managed_sessions
                .values()
                .any(|session| session.status == ManagedSessionStatus::RecoveryRequired)
                .then(|| {
                    "Bridge 已恢复，但中断时的 Turn 结果仍不确定；CAS 不会自动重放。".to_owned()
                });
            Ok(RuntimeBridgeStatusResponse::from(&*state))
        })();
        if let Err(error) = &recovered {
            if let Ok(mut workers) = self.worker()
                && let Some(mut worker) = workers.take()
            {
                let _ = worker.stop();
            }
            self.record_recovery_failure(attempt, error);
        }
        recovered
    }

    fn stop_inner(&self) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        let worker = self.worker()?.take();
        if let Some(mut worker) = worker {
            worker.stop()?;
        }
        let mut state = self.state()?;
        state.status = RuntimeBridgeStatus::Stopped;
        state.last_error = None;
        state.recovery_attempt_count = 0;
        for session in state.managed_sessions.values_mut() {
            session.status = ManagedSessionStatus::Detached;
            session.active_turn_id = None;
        }
        Ok(RuntimeBridgeStatusResponse::from(&*state))
    }

    fn managed_session_start_inner(
        &self,
        request: ManagedSessionStartRequest,
    ) -> Result<ManagedSessionResponse, RuntimeBridgeError> {
        let cwd = validate_cwd(&request.cwd)?;
        self.ensure_managed_session_supported()?;
        let mut params =
            serde_json::Map::from_iter([("cwd".to_owned(), Value::String(cwd.clone()))]);
        if let Some(approval_policy) = request.approval_policy {
            params.insert("approvalPolicy".to_owned(), Value::String(approval_policy));
        }
        if let Some(sandbox) = request.sandbox {
            params.insert("sandbox".to_owned(), Value::String(sandbox));
        }
        let result = self.request(AppServerMethod::ThreadStart.as_str(), Value::Object(params))?;
        let thread = parse_thread_response(&result).map_err(runtime_response_error)?;
        self.bind_managed_session(
            thread.thread_id,
            thread.session_id,
            ManagedSessionOrigin::Started,
            Some(cwd),
        )
    }

    fn managed_session_resume_inner(
        &self,
        request: ManagedSessionResumeRequest,
    ) -> Result<ManagedSessionResponse, RuntimeBridgeError> {
        let thread_id = validate_thread_id(&request.thread_id)?;
        let cwd = request.cwd.as_deref().map(validate_cwd).transpose()?;
        self.ensure_managed_session_supported()?;
        if let Some(session) = self.state()?.managed_sessions.get(&thread_id) {
            match session.status {
                ManagedSessionStatus::Running => {
                    return Err(RuntimeBridgeError::TurnAlreadyRunning);
                }
                ManagedSessionStatus::RecoveryRequired => {
                    return Err(RuntimeBridgeError::SessionRecoveryRequired);
                }
                _ => {}
            }
        }
        let mut params =
            serde_json::Map::from_iter([("threadId".to_owned(), Value::String(thread_id.clone()))]);
        if let Some(cwd) = cwd.as_ref() {
            params.insert("cwd".to_owned(), Value::String(cwd.clone()));
        }
        let result = self.request(
            AppServerMethod::ThreadResume.as_str(),
            Value::Object(params),
        )?;
        let thread = parse_thread_response(&result).map_err(runtime_response_error)?;
        if thread.thread_id != thread_id {
            return Err(RuntimeBridgeError::UnexpectedThreadResponse);
        }
        let session = self.bind_managed_session(
            thread.thread_id,
            thread.session_id,
            ManagedSessionOrigin::Resumed,
            cwd,
        )?;
        self.usage.mark_agent_execution_idle_if_known(&thread_id)?;
        Ok(session)
    }

    fn managed_session_resolve_recovery_inner(
        &self,
        request: ManagedSessionRecoveryRequest,
    ) -> Result<ManagedSessionResponse, RuntimeBridgeError> {
        let thread_id = validate_thread_id(&request.thread_id)?;
        let active_turn_id = {
            let state = self.state()?;
            if state.status != RuntimeBridgeStatus::Running {
                return Err(RuntimeBridgeError::NotRunning);
            }
            let session = state
                .managed_sessions
                .get(&thread_id)
                .ok_or(RuntimeBridgeError::ThreadNotBound)?;
            if session.status != ManagedSessionStatus::RecoveryRequired {
                return Ok(ManagedSessionResponse::from(session));
            }
            session.active_turn_id.clone()
        };
        if !request.abandon_uncertain_turn {
            let result = self.request(
                AppServerMethod::ThreadRead.as_str(),
                json!({"threadId": thread_id, "includeTurns": true}),
            )?;
            match recovery_turn_outcome(&result, active_turn_id.as_deref()) {
                RecoveryTurnOutcome::Terminal => {}
                RecoveryTurnOutcome::Running => {
                    return Err(RuntimeBridgeError::RecoveryTurnStillRunning);
                }
                RecoveryTurnOutcome::Unknown => {
                    return Err(RuntimeBridgeError::RecoveryOutcomeUnknown);
                }
            }
        }
        let response = {
            let mut state = self.state()?;
            let response = {
                let session = state
                    .managed_sessions
                    .get_mut(&thread_id)
                    .ok_or(RuntimeBridgeError::ThreadNotBound)?;
                session.status = ManagedSessionStatus::Idle;
                session.active_turn_id = None;
                ManagedSessionResponse::from(&*session)
            };
            state.last_error = state
                .managed_sessions
                .values()
                .any(|session| session.status == ManagedSessionStatus::RecoveryRequired)
                .then(|| {
                    "Bridge 已恢复，但中断时的 Turn 结果仍不确定；CAS 不会自动重放。".to_owned()
                });
            response
        };
        self.usage.mark_agent_execution_idle_if_known(&thread_id)?;
        Ok(response)
    }

    fn managed_turn_start_inner(
        &self,
        request: ManagedTurnStartRequest,
    ) -> Result<ManagedTurnStartResponse, RuntimeBridgeError> {
        let thread_id = validate_thread_id(&request.thread_id)?;
        let input = validate_turn_input(&request.input)?;
        {
            let mut state = self.state()?;
            let session = state
                .managed_sessions
                .get_mut(&thread_id)
                .ok_or(RuntimeBridgeError::ThreadNotBound)?;
            if session.status == ManagedSessionStatus::RecoveryRequired {
                return Err(RuntimeBridgeError::SessionRecoveryRequired);
            }
            if session.status == ManagedSessionStatus::Running {
                return Err(RuntimeBridgeError::TurnAlreadyRunning);
            }
            session.status = ManagedSessionStatus::Running;
            session.active_turn_id = None;
        }
        let mut params = serde_json::Map::from_iter([
            ("threadId".to_owned(), Value::String(thread_id.clone())),
            (
                "input".to_owned(),
                json!([{ "type": "text", "text": input }]),
            ),
            (
                "effort".to_owned(),
                request.effort.map(Value::String).unwrap_or(Value::Null),
            ),
        ]);
        if let Some(approval_policy) = request.approval_policy {
            params.insert("approvalPolicy".to_owned(), Value::String(approval_policy));
        }
        if let Some(sandbox_policy) = request.sandbox_policy {
            params.insert("sandboxPolicy".to_owned(), sandbox_policy);
        }
        let result = self.request(AppServerMethod::TurnStart.as_str(), Value::Object(params));
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if let Ok(mut state) = self.state()
                    && let Some(session) = state.managed_sessions.get_mut(&thread_id)
                {
                    session.status = ManagedSessionStatus::RecoveryRequired;
                    session.active_turn_id = None;
                }
                return Err(error);
            }
        };
        let turn = match parse_turn_response(&result) {
            Ok(turn) => turn,
            Err(error) => {
                if let Ok(mut state) = self.state()
                    && let Some(session) = state.managed_sessions.get_mut(&thread_id)
                {
                    session.status = ManagedSessionStatus::RecoveryRequired;
                    session.active_turn_id = None;
                }
                return Err(runtime_response_error(error));
            }
        };
        let mut state = self.state()?;
        let session = state
            .managed_sessions
            .get_mut(&thread_id)
            .ok_or(RuntimeBridgeError::ThreadNotBound)?;
        if session.status == ManagedSessionStatus::Running {
            session.active_turn_id = Some(turn.turn_id.clone());
        }
        Ok(ManagedTurnStartResponse {
            thread_id,
            turn_id: turn.turn_id,
            status: session.status,
        })
    }

    fn execute_agent_thread_inner(
        &self,
        permit: DispatchPermit,
        cwd: String,
        input: String,
    ) -> Result<AgentThreadExecutionResponse, RuntimeBridgeError> {
        let profile = runtime_profile_from_dispatch(permit.profile());
        let route_action = permit.attempt().route_action;
        let job_id = permit.job().job_id.clone();
        let attempt_id = permit.attempt().attempt_id.clone();
        let reason_code = permit.reason_code().to_owned();
        let workspace_scope_key = permit.job().workspace_scope_key.clone();
        let parent_thread_id = permit.job().parent_thread_id.clone();
        let task_scope_key = permit.job().task_scope_key.clone();
        let (thread_id, session_id, origin, action) = match route_action {
            RouteAction::Reuse => {
                let thread_id = permit
                    .candidate_thread_id()
                    .map(str::to_owned)
                    .ok_or(RuntimeBridgeError::UnexpectedThreadResponse)?;
                if let Some(session) = self.state()?.managed_sessions.get(&thread_id) {
                    match session.status {
                        ManagedSessionStatus::Running => {
                            return Err(RuntimeBridgeError::TurnAlreadyRunning);
                        }
                        ManagedSessionStatus::RecoveryRequired => {
                            return Err(RuntimeBridgeError::SessionRecoveryRequired);
                        }
                        _ => {}
                    }
                }
                let mut params = agent_thread_params(&profile, &cwd);
                params
                    .as_object_mut()
                    .expect("agent thread params are an object")
                    .insert("threadId".to_owned(), Value::String(thread_id.clone()));
                let result = self.request(AppServerMethod::ThreadResume.as_str(), params)?;
                let thread = parse_thread_response(&result).map_err(runtime_response_error)?;
                if thread.thread_id != thread_id {
                    return Err(RuntimeBridgeError::UnexpectedThreadResponse);
                }
                (
                    thread.thread_id,
                    thread.session_id,
                    ManagedSessionOrigin::Resumed,
                    AgentThreadExecutionAction::Reused,
                )
            }
            RouteAction::Spawn => {
                let result = self.request(
                    AppServerMethod::ThreadStart.as_str(),
                    agent_thread_params(&profile, &cwd),
                )?;
                let thread = parse_thread_response(&result).map_err(runtime_response_error)?;
                (
                    thread.thread_id,
                    thread.session_id,
                    ManagedSessionOrigin::Started,
                    AgentThreadExecutionAction::Spawned,
                )
            }
        };
        self.bind_managed_session(thread_id.clone(), session_id, origin, Some(cwd.clone()))?;
        let turn = self.managed_turn_start_inner(ManagedTurnStartRequest {
            thread_id: thread_id.clone(),
            input,
            effort: profile.reasoning_effort.clone(),
            approval_policy: None,
            sandbox_policy: None,
        });
        let turn = match turn {
            Ok(turn) => turn,
            Err(error) => {
                let _ = self
                    .usage
                    .mark_agent_execution_recovery_required(&thread_id);
                return Err(error);
            }
        };
        self.receipt_events
            .accept_managed(ManagedTurnAcceptedEvidence {
                job_id: job_id.clone(),
                attempt_id: attempt_id.clone(),
                agent_id: profile.agent_id.clone(),
                agent_name: profile.agent_name.clone(),
                parent_thread_id: parent_thread_id.clone(),
                workspace_scope_key: workspace_scope_key.clone(),
                task_scope_key: task_scope_key.clone(),
                runtime_fingerprint: profile.runtime_fingerprint.clone(),
                thread_id: thread_id.clone(),
                turn_id: turn.turn_id.clone(),
                evidence_ref: format!(
                    "{}+{}:{thread_id}:{}",
                    match route_action {
                        RouteAction::Reuse => "thread/resume",
                        RouteAction::Spawn => "thread/start",
                    },
                    AppServerMethod::TurnStart.as_str(),
                    turn.turn_id
                ),
            })
            .map_err(RuntimeBridgeError::Receipt)?;
        Ok(AgentThreadExecutionResponse {
            action,
            decision: route_action,
            reason_code,
            job_id,
            attempt_id,
            agent_id: profile.agent_id,
            agent_name: profile.agent_name,
            workspace_scope_key,
            thread_id,
            turn_id: turn.turn_id,
            status: turn.status,
        })
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, RuntimeBridgeError> {
        let mut worker_slot = self.worker()?;
        let worker = worker_slot.as_mut().ok_or(RuntimeBridgeError::NotRunning)?;
        if !worker.is_running()? {
            return Err(RuntimeBridgeError::NotRunning);
        }
        worker.request(method, params)
    }

    fn ensure_managed_session_supported(&self) -> Result<(), RuntimeBridgeError> {
        match self.state()?.managed_session_capability {
            SchemaCapability::Supported => Ok(()),
            capability => Err(RuntimeBridgeError::ManagedSessionUnsupported(capability)),
        }
    }

    fn ensure_agent_execution_supported(&self) -> Result<(), RuntimeBridgeError> {
        match self.state()?.agent_execution_capability {
            SchemaCapability::Supported => Ok(()),
            capability => Err(RuntimeBridgeError::AgentExecutionUnsupported(capability)),
        }
    }

    fn bind_managed_session(
        &self,
        thread_id: String,
        session_id: Option<String>,
        origin: ManagedSessionOrigin,
        cwd: Option<String>,
    ) -> Result<ManagedSessionResponse, RuntimeBridgeError> {
        let attached_at = self.usage.current_timestamp()?;
        let session = ManagedSessionState {
            thread_id,
            session_id,
            origin,
            status: ManagedSessionStatus::Idle,
            cwd,
            active_turn_id: None,
            attached_at,
        };
        let mut state = self.state()?;
        if let Some(existing) = state.managed_sessions.get(&session.thread_id) {
            match existing.status {
                ManagedSessionStatus::Running => {
                    return Err(RuntimeBridgeError::TurnAlreadyRunning);
                }
                ManagedSessionStatus::RecoveryRequired => {
                    return Err(RuntimeBridgeError::SessionRecoveryRequired);
                }
                _ => {}
            }
        }
        state.last_managed_thread_id = Some(session.thread_id.clone());
        state
            .managed_sessions
            .insert(session.thread_id.clone(), session.clone());
        Ok(ManagedSessionResponse::from(&session))
    }

    fn status_inner(&self) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        Ok(RuntimeBridgeStatusResponse::from(&*self.state()?))
    }

    fn worker(&self) -> Result<MutexGuard<'_, Option<BridgeWorker>>, RuntimeBridgeError> {
        self.worker
            .lock()
            .map_err(|_| RuntimeBridgeError::StateUnavailable)
    }

    fn launch(&self) -> Result<MutexGuard<'_, Option<RuntimeBridgeLaunch>>, RuntimeBridgeError> {
        self.launch
            .lock()
            .map_err(|_| RuntimeBridgeError::StateUnavailable)
    }

    fn state(&self) -> Result<MutexGuard<'_, RuntimeBridgeState>, RuntimeBridgeError> {
        self.state
            .lock()
            .map_err(|_| RuntimeBridgeError::StateUnavailable)
    }

    fn set_failed(&self, message: String) {
        if let Ok(mut state) = self.state.lock() {
            state.status = RuntimeBridgeStatus::Failed;
            state.last_error = Some(message);
        }
    }

    fn record_recovery_failure(&self, attempt: u8, error: &RuntimeBridgeError) {
        let recovered_at = self.usage.current_timestamp().ok();
        if let Ok(mut state) = self.state.lock() {
            state.status = RuntimeBridgeStatus::Failed;
            state.recovery_attempt_count = attempt;
            state.last_recovery_at = recovered_at;
            state.last_error = Some(format!(
                "自动恢复 {attempt}/{MAX_AUTO_RECOVERY_ATTEMPTS} 失败：{error}"
            ));
            for session in state.managed_sessions.values_mut() {
                if session.status == ManagedSessionStatus::Running {
                    session.status = ManagedSessionStatus::RecoveryRequired;
                }
            }
        }
    }
}

impl Drop for RuntimeBridgeService {
    fn drop(&mut self) {
        if let Ok(slot) = self.worker.get_mut()
            && let Some(mut worker) = slot.take()
        {
            let _ = worker.stop();
        }
    }
}

struct BridgeWorker {
    child: Arc<Mutex<Child>>,
    stdin: SharedStdin,
    stopping: Arc<AtomicBool>,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
    pending_responses: Arc<Mutex<HashMap<i64, PendingResponse>>>,
    next_request_id: i64,
}

impl BridgeWorker {
    fn is_running(&mut self) -> Result<bool, RuntimeBridgeError> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| RuntimeBridgeError::StateUnavailable)?;
        child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(RuntimeBridgeError::Process)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, RuntimeBridgeError> {
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_add(1)
            .ok_or(RuntimeBridgeError::RequestIdExhausted)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        self.pending_responses
            .lock()
            .map_err(|_| RuntimeBridgeError::StateUnavailable)?
            .insert(request_id, sender);
        let message = json!({
            "id": request_id,
            "method": method,
            "params": params
        });
        if let Err(error) = write_shared_message(&self.stdin, &message) {
            remove_pending_response(&self.pending_responses, request_id);
            return Err(error);
        }
        match receiver.recv_timeout(PROTOCOL_REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(_) => {
                remove_pending_response(&self.pending_responses, request_id);
                Err(RuntimeBridgeError::ProtocolTimeout(method.to_owned()))
            }
        }
    }

    fn stop(&mut self) -> Result<(), RuntimeBridgeError> {
        self.stopping.store(true, Ordering::Release);
        if let Ok(mut stdin) = self.stdin.lock() {
            stdin.take();
        }
        {
            let mut child = self
                .child
                .lock()
                .map_err(|_| RuntimeBridgeError::StateUnavailable)?;
            if child
                .try_wait()
                .map_err(RuntimeBridgeError::Process)?
                .is_none()
            {
                child.kill().map_err(RuntimeBridgeError::Process)?;
            }
            child.wait().map_err(RuntimeBridgeError::Process)?;
        }
        self.join_threads();
        Ok(())
    }

    fn join(mut self) {
        self.join_threads();
    }

    fn join_threads(&mut self) {
        if let Some(thread) = self.stdout_thread.take() {
            let _ = thread.join();
        }
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
    }
}

fn cleanup_unstarted_worker(
    child: Arc<Mutex<Child>>,
    stopping: Arc<AtomicBool>,
    stdout_thread: JoinHandle<()>,
    stderr_thread: JoinHandle<()>,
) {
    stopping.store(true, Ordering::Release);
    if let Ok(mut child) = child.lock() {
        let _ = child.kill();
        let _ = child.wait();
    }
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
}

fn write_message(stdin: &mut ChildStdin, message: &Value) -> Result<(), RuntimeBridgeError> {
    writeln!(stdin, "{message}")
        .and_then(|_| stdin.flush())
        .map_err(RuntimeBridgeError::ProtocolWrite)
}

type PendingResponse = mpsc::SyncSender<Result<Value, RuntimeBridgeError>>;
type SharedStdin = Arc<Mutex<Option<ChildStdin>>>;

fn is_initialize_response(message: &Value, initialization_pending: bool) -> bool {
    initialization_pending
        && message.get("id").and_then(Value::as_i64) == Some(1)
        && message.get("method").is_none()
}

fn write_shared_message(stdin: &SharedStdin, message: &Value) -> Result<(), RuntimeBridgeError> {
    let mut stdin = stdin
        .lock()
        .map_err(|_| RuntimeBridgeError::StateUnavailable)?;
    let stdin = stdin.as_mut().ok_or(RuntimeBridgeError::NotRunning)?;
    write_message(stdin, message)
}

fn read_app_server_stream(
    stdout: impl std::io::Read,
    state: Arc<Mutex<RuntimeBridgeState>>,
    usage: Arc<UsageService>,
    receipt_events: Arc<OrchestrationReceiptEventService>,
    stopping: Arc<AtomicBool>,
    initialize_tx: mpsc::SyncSender<Result<(), RuntimeBridgeError>>,
    pending_responses: Arc<Mutex<HashMap<i64, PendingResponse>>>,
    stdin: SharedStdin,
    helper_path: PathBuf,
    database_path: PathBuf,
) {
    let mut initialize_tx = Some(initialize_tx);
    let mut observer = RuntimeObserver::new(usage);
    for line in BufReader::new(stdout).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                mark_stream_failure(&state, format!("读取 App Server 事件失败：{error}"));
                break;
            }
        };
        let message: Value = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(_) => {
                mark_malformed_event(&state, "App Server 返回了无法解析的 JSONL。");
                continue;
            }
        };
        if is_initialize_response(&message, initialize_tx.is_some()) {
            if let Some(sender) = initialize_tx.take() {
                if let Some(error) = message.get("error") {
                    let message = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("未知初始化错误")
                        .to_owned();
                    let _ = sender.send(Err(RuntimeBridgeError::InitializeRejected(message)));
                } else {
                    if let Ok(mut state) = state.lock() {
                        state.server_user_agent = message
                            .pointer("/result/userAgent")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                    let _ = sender.send(Ok(()));
                }
            }
            continue;
        }

        match respond_to_server_request(&message, &stdin, &helper_path, &database_path) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                mark_stream_failure(&state, error.to_string());
                continue;
            }
        }

        if resolve_pending_response(&message, &pending_responses) {
            continue;
        }

        match parse_event(&message) {
            Ok(Some(event)) => {
                update_managed_session_from_event(&state, &event);
                let is_usage = event.is_usage();
                let profile = event.profile();
                let failure_message = event.failure_message().map(str::to_owned);
                if let Err(error) =
                    receipt_events.observe_runtime_event(&event, &message.to_string())
                {
                    mark_stream_failure(&state, error.message);
                    continue;
                }
                if let Err(error) = observer.observe(event) {
                    mark_stream_failure(&state, error.to_string());
                    continue;
                }
                if let Ok(mut state) = state.lock() {
                    state.last_event_at = observer.last_event_at.clone();
                    state.status = RuntimeBridgeStatus::Running;
                    state.last_error = failure_message;
                    if profile == ProtocolProfile::Legacy {
                        state.protocol_compatibility = ProtocolCompatibility::LegacyCompatible;
                    } else if state.protocol_compatibility
                        != ProtocolCompatibility::LegacyCompatible
                    {
                        state.protocol_compatibility = ProtocolCompatibility::Compatible;
                    }
                    if is_usage {
                        state.usage_event_count += 1;
                    }
                }
            }
            Ok(None) => {}
            Err(error) => mark_malformed_event(&state, &error.to_string()),
        }
    }

    if let Some(sender) = initialize_tx {
        let _ = sender.send(Err(RuntimeBridgeError::InitializeRejected(
            "App Server 在初始化前关闭了事件流。".to_owned(),
        )));
    }
    fail_pending_responses(&pending_responses);
    if !stopping.load(Ordering::Acquire) {
        observer.mark_live_records_partial();
        mark_stream_failure(&state, "App Server 事件流意外关闭。".to_owned());
    }
}

fn mark_malformed_event(state: &Arc<Mutex<RuntimeBridgeState>>, message: &str) {
    if let Ok(mut state) = state.lock() {
        state.status = RuntimeBridgeStatus::Degraded;
        state.protocol_compatibility = ProtocolCompatibility::Degraded;
        state.malformed_event_count += 1;
        state.last_error = Some(message.to_owned());
    }
}

fn mark_stream_failure(state: &Arc<Mutex<RuntimeBridgeState>>, message: String) {
    if let Ok(mut state) = state.lock() {
        state.status = RuntimeBridgeStatus::Failed;
        state.last_error = Some(message);
        for session in state.managed_sessions.values_mut() {
            if session.status == ManagedSessionStatus::Running {
                session.status = ManagedSessionStatus::RecoveryRequired;
            }
        }
    }
}

fn auto_recovery_allowed(state: &RuntimeBridgeState) -> bool {
    state.status == RuntimeBridgeStatus::Failed
        && state.recovery_attempt_count < MAX_AUTO_RECOVERY_ATTEMPTS
}

fn respond_to_server_request(
    message: &Value,
    stdin: &SharedStdin,
    helper_path: &Path,
    database_path: &Path,
) -> Result<bool, RuntimeBridgeError> {
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Ok(false);
    };
    let Some(request_id) = message.get("id").cloned() else {
        return Ok(false);
    };
    let decision = match method {
        "item/commandExecution/requestApproval" => {
            if is_cas_control_plane_request(message, helper_path, database_path)
                || is_isolated_e2e_workspace_request(message)
            {
                "accept"
            } else {
                "decline"
            }
        }
        "item/fileChange/requestApproval" => {
            if is_isolated_e2e_file_change_request(message) {
                "accept"
            } else {
                "decline"
            }
        }
        _ => return Ok(false),
    };
    write_shared_message(
        stdin,
        &json!({
            "id": request_id,
            "result": { "decision": decision }
        }),
    )?;
    Ok(true)
}

#[cfg(test)]
fn is_isolated_e2e_file_change_request(message: &Value) -> bool {
    std::env::var_os("CAS_E2E_ROOT").is_some()
        && message
            .pointer("/params/grantRoot")
            .is_none_or(Value::is_null)
}

#[cfg(not(test))]
fn is_isolated_e2e_file_change_request(_message: &Value) -> bool {
    false
}

#[cfg(test)]
fn is_isolated_e2e_workspace_request(message: &Value) -> bool {
    let Some(root) = std::env::var_os("CAS_E2E_ROOT").map(PathBuf::from) else {
        return false;
    };
    is_isolated_e2e_workspace_request_for_root(message, &root)
}

#[cfg(test)]
fn is_isolated_e2e_workspace_request_for_root(message: &Value, root: &Path) -> bool {
    if message
        .pointer("/params/networkApprovalContext")
        .is_some_and(|value| !value.is_null())
        || message
            .pointer("/params/additionalPermissions")
            .is_some_and(|value| !value.is_null())
    {
        return false;
    }
    let Some(cwd) = message.pointer("/params/cwd").and_then(Value::as_str) else {
        return false;
    };
    Path::new(cwd).starts_with(root.join("workspace"))
}

#[cfg(not(test))]
fn is_isolated_e2e_workspace_request(_message: &Value) -> bool {
    false
}

fn is_cas_control_plane_request(message: &Value, helper_path: &Path, database_path: &Path) -> bool {
    let Some(actions) = message
        .pointer("/params/commandActions")
        .and_then(Value::as_array)
    else {
        return false;
    };
    if actions.len() != 1 {
        return false;
    }
    let expected_helper = normalize_workspace_scope_key(&helper_path.to_string_lossy());
    let expected_database = normalize_workspace_scope_key(&database_path.to_string_lossy());
    actions.iter().all(|action| {
        let Some(command) = action.get("command").and_then(Value::as_str) else {
            return false;
        };
        let Some(command) = command.strip_prefix("& ") else {
            return false;
        };
        let Some(arguments) = parse_cas_control_plane_arguments(command) else {
            return false;
        };
        let Some((helper, arguments)) = arguments.split_first() else {
            return false;
        };
        if expected_helper.as_deref() != normalize_workspace_scope_key(helper).as_deref() {
            return false;
        }
        let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
        let valid_argument = |value: &str| {
            !value.is_empty()
                && value.len() <= 64
                && value.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
                })
        };
        let valid_database = |value: &str| {
            expected_database.as_deref() == normalize_workspace_scope_key(value).as_deref()
        };
        let valid_workspace = |value: &str| {
            Path::new(value).is_absolute() && normalize_workspace_scope_key(value).is_some()
        };
        match arguments.as_slice() {
            ["job-schedule", database, agent_key, workspace] => {
                valid_database(database) && valid_argument(agent_key) && valid_workspace(workspace)
            }
            [
                "job-bind" | "job-observe",
                database,
                job_id,
                attempt_id,
                thread_id,
                workspace,
            ] => {
                valid_database(database)
                    && valid_argument(job_id)
                    && valid_argument(attempt_id)
                    && valid_argument(thread_id)
                    && valid_workspace(workspace)
            }
            ["job-review", database, job_id, attempt_id] => {
                valid_database(database) && valid_argument(job_id) && valid_argument(attempt_id)
            }
            ["schedule", agent_key] => valid_argument(agent_key),
            ["schedule", agent_key, task_key] => {
                valid_argument(agent_key) && valid_argument(task_key)
            }
            ["bind", agent_key, thread_id] => {
                valid_argument(agent_key) && valid_argument(thread_id)
            }
            ["bind", agent_key, thread_id, task_key] => {
                valid_argument(agent_key) && valid_argument(thread_id) && valid_argument(task_key)
            }
            ["schedule", database, agent_key, workspace] => {
                valid_database(database) && valid_argument(agent_key) && valid_workspace(workspace)
            }
            ["schedule", database, agent_key, workspace, task_key] => {
                valid_database(database)
                    && valid_argument(agent_key)
                    && valid_workspace(workspace)
                    && valid_argument(task_key)
            }
            ["bind", database, agent_key, thread_id, workspace] => {
                valid_database(database)
                    && valid_argument(agent_key)
                    && valid_argument(thread_id)
                    && valid_workspace(workspace)
            }
            ["bind", database, agent_key, thread_id, workspace, task_key] => {
                valid_database(database)
                    && valid_argument(agent_key)
                    && valid_argument(thread_id)
                    && valid_workspace(workspace)
                    && valid_argument(task_key)
            }
            _ => false,
        }
    })
}

fn parse_cas_control_plane_arguments(input: &str) -> Option<Vec<String>> {
    let mut arguments = Vec::new();
    let mut offset = 0;
    while offset < input.len() {
        while offset < input.len() {
            let current = input[offset..].chars().next()?;
            if !current.is_ascii_whitespace() {
                break;
            }
            offset += current.len_utf8();
        }
        if offset == input.len() {
            break;
        }
        let opening = input[offset..].chars().next()?;
        if matches!(opening, '"' | '\'') {
            let value_start = offset + 1;
            let closing = input[value_start..].find(opening)? + value_start;
            let value = &input[value_start..closing];
            if value.is_empty()
                || value.chars().any(|character| {
                    character.is_control() || (opening == '"' && matches!(character, '`' | '$'))
                })
            {
                return None;
            }
            offset = closing + 1;
            if offset < input.len() && !input[offset..].chars().next()?.is_ascii_whitespace() {
                return None;
            }
            arguments.push(value.to_owned());
        } else {
            let start = offset;
            while offset < input.len() {
                let current = input[offset..].chars().next()?;
                if current.is_ascii_whitespace() {
                    break;
                }
                if current.is_control()
                    || matches!(
                        current,
                        '`' | '$' | '"' | '\'' | ';' | '&' | '|' | '<' | '>' | '#' | ','
                    )
                {
                    return None;
                }
                offset += current.len_utf8();
            }
            arguments.push(input[start..offset].to_owned());
        }
    }
    (!arguments.is_empty()).then_some(arguments)
}

fn resolve_pending_response(
    message: &Value,
    pending_responses: &Arc<Mutex<HashMap<i64, PendingResponse>>>,
) -> bool {
    let Some(request_id) = message.get("id").and_then(Value::as_i64) else {
        return false;
    };
    let sender = pending_responses
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(&request_id));
    let Some(sender) = sender else {
        return false;
    };
    let result = if let Some(error) = message.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("App Server 返回未知协议错误。")
            .to_owned();
        Err(RuntimeBridgeError::ProtocolRejected(message))
    } else {
        message
            .get("result")
            .cloned()
            .ok_or(RuntimeBridgeError::InvalidProtocolResponse("result"))
    };
    let _ = sender.send(result);
    true
}

fn remove_pending_response(
    pending_responses: &Arc<Mutex<HashMap<i64, PendingResponse>>>,
    request_id: i64,
) {
    if let Ok(mut pending) = pending_responses.lock() {
        pending.remove(&request_id);
    }
}

fn fail_pending_responses(pending_responses: &Arc<Mutex<HashMap<i64, PendingResponse>>>) {
    let senders = pending_responses
        .lock()
        .map(|mut pending| {
            pending
                .drain()
                .map(|(_, sender)| sender)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for sender in senders {
        let _ = sender.send(Err(RuntimeBridgeError::StreamClosed));
    }
}

fn update_managed_session_from_event(
    state: &Arc<Mutex<RuntimeBridgeState>>,
    event: &NormalizedRuntimeEvent,
) {
    let Ok(mut state) = state.lock() else {
        return;
    };
    match event {
        NormalizedRuntimeEvent::ThreadStarted {
            thread_id,
            session_id,
            ..
        } => {
            let Some(session) = state.managed_sessions.get_mut(thread_id) else {
                return;
            };
            if session.session_id.is_none() {
                session.session_id = session_id.clone();
            }
        }
        NormalizedRuntimeEvent::TurnFinished {
            thread_id,
            turn_id,
            successful,
            failure_message,
            ..
        } => {
            let Some(session) = state.managed_sessions.get_mut(thread_id) else {
                return;
            };
            if session.active_turn_id.as_deref() != Some(turn_id.as_str()) {
                return;
            }
            session.status = if *successful {
                ManagedSessionStatus::Idle
            } else {
                ManagedSessionStatus::Failed
            };
            session.active_turn_id = None;
            if !successful {
                state.last_error = failure_message.clone();
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum RuntimeBridgeStatus {
    Stopped,
    Starting,
    Recovering,
    Running,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ProtocolCompatibility {
    Unverified,
    Compatible,
    LegacyCompatible,
    Degraded,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ManagedSessionStatus {
    Idle,
    Running,
    Detached,
    RecoveryRequired,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum ManagedSessionOrigin {
    Started,
    Resumed,
}

#[derive(Debug, Clone)]
struct ManagedSessionState {
    thread_id: String,
    session_id: Option<String>,
    origin: ManagedSessionOrigin,
    status: ManagedSessionStatus,
    cwd: Option<String>,
    active_turn_id: Option<String>,
    attached_at: String,
}

#[derive(Debug, Clone)]
struct RuntimeBridgeLaunch {
    executable: PathBuf,
    codex_home: PathBuf,
    codex_version: Option<String>,
}

#[derive(Debug, Clone)]
struct RuntimeBridgeState {
    status: RuntimeBridgeStatus,
    protocol_compatibility: ProtocolCompatibility,
    schema_capability: SchemaCapability,
    managed_session_capability: SchemaCapability,
    agent_execution_capability: SchemaCapability,
    codex_version: Option<String>,
    server_user_agent: Option<String>,
    usage_event_count: u64,
    malformed_event_count: u64,
    started_at: Option<String>,
    last_event_at: Option<String>,
    last_error: Option<String>,
    recovery_attempt_count: u8,
    last_recovery_at: Option<String>,
    last_managed_thread_id: Option<String>,
    managed_sessions: BTreeMap<String, ManagedSessionState>,
}

impl Default for RuntimeBridgeState {
    fn default() -> Self {
        Self {
            status: RuntimeBridgeStatus::Stopped,
            protocol_compatibility: ProtocolCompatibility::Unverified,
            schema_capability: SchemaCapability::Unavailable,
            managed_session_capability: SchemaCapability::Unavailable,
            agent_execution_capability: SchemaCapability::Unavailable,
            codex_version: None,
            server_user_agent: None,
            usage_event_count: 0,
            malformed_event_count: 0,
            started_at: None,
            last_event_at: None,
            last_error: None,
            recovery_attempt_count: 0,
            last_recovery_at: None,
            last_managed_thread_id: None,
            managed_sessions: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeBridgeStatusResponse {
    status: RuntimeBridgeStatus,
    protocol_compatibility: ProtocolCompatibility,
    schema_capability: SchemaCapability,
    managed_session_capability: SchemaCapability,
    agent_execution_capability: SchemaCapability,
    codex_version: Option<String>,
    server_user_agent: Option<String>,
    usage_event_count: u64,
    malformed_event_count: u64,
    started_at: Option<String>,
    last_event_at: Option<String>,
    last_error: Option<String>,
    recovery_attempt_count: u8,
    max_auto_recovery_attempts: u8,
    auto_recovery_exhausted: bool,
    last_recovery_at: Option<String>,
    managed_session: Option<ManagedSessionResponse>,
    managed_sessions: Vec<ManagedSessionResponse>,
}

impl From<&RuntimeBridgeState> for RuntimeBridgeStatusResponse {
    fn from(state: &RuntimeBridgeState) -> Self {
        Self {
            status: state.status,
            protocol_compatibility: state.protocol_compatibility,
            schema_capability: state.schema_capability,
            managed_session_capability: state.managed_session_capability,
            agent_execution_capability: state.agent_execution_capability,
            codex_version: state.codex_version.clone(),
            server_user_agent: state.server_user_agent.clone(),
            usage_event_count: state.usage_event_count,
            malformed_event_count: state.malformed_event_count,
            started_at: state.started_at.clone(),
            last_event_at: state.last_event_at.clone(),
            last_error: state.last_error.clone(),
            recovery_attempt_count: state.recovery_attempt_count,
            max_auto_recovery_attempts: MAX_AUTO_RECOVERY_ATTEMPTS,
            auto_recovery_exhausted: state.status == RuntimeBridgeStatus::Failed
                && state.recovery_attempt_count >= MAX_AUTO_RECOVERY_ATTEMPTS,
            last_recovery_at: state.last_recovery_at.clone(),
            managed_session: state
                .last_managed_thread_id
                .as_ref()
                .and_then(|thread_id| state.managed_sessions.get(thread_id))
                .or_else(|| state.managed_sessions.values().next_back())
                .map(ManagedSessionResponse::from),
            managed_sessions: state
                .managed_sessions
                .values()
                .map(ManagedSessionResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSessionStartRequest {
    cwd: String,
    #[serde(default)]
    approval_policy: Option<String>,
    #[serde(default)]
    sandbox: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSessionResumeRequest {
    thread_id: String,
    cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSessionRecoveryRequest {
    thread_id: String,
    #[serde(default)]
    abandon_uncertain_turn: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTurnStartRequest {
    thread_id: String,
    input: String,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    approval_policy: Option<String>,
    #[serde(default)]
    sandbox_policy: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadExecutionRequest {
    task_packet: TaskPacket,
    cwd: String,
    input: String,
    expected_decision: RouteAction,
    expected_candidate_thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSessionResponse {
    thread_id: String,
    session_id: Option<String>,
    origin: ManagedSessionOrigin,
    status: ManagedSessionStatus,
    cwd: Option<String>,
    active_turn_id: Option<String>,
    attached_at: String,
}

impl From<&ManagedSessionState> for ManagedSessionResponse {
    fn from(session: &ManagedSessionState) -> Self {
        Self {
            thread_id: session.thread_id.clone(),
            session_id: session.session_id.clone(),
            origin: session.origin,
            status: session.status,
            cwd: session.cwd.clone(),
            active_turn_id: session.active_turn_id.clone(),
            attached_at: session.attached_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTurnStartResponse {
    thread_id: String,
    turn_id: String,
    status: ManagedSessionStatus,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum AgentThreadExecutionAction {
    Reused,
    Spawned,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentThreadExecutionResponse {
    action: AgentThreadExecutionAction,
    decision: RouteAction,
    reason_code: String,
    job_id: String,
    attempt_id: String,
    agent_id: String,
    agent_name: String,
    workspace_scope_key: String,
    thread_id: String,
    turn_id: String,
    status: ManagedSessionStatus,
}

fn validate_cwd(cwd: &str) -> Result<String, RuntimeBridgeError> {
    let cwd = cwd.trim();
    let path = Path::new(cwd);
    if cwd.is_empty() || !path.is_absolute() || !path.is_dir() {
        return Err(RuntimeBridgeError::InvalidCwd);
    }
    Ok(cwd.to_owned())
}

fn validate_thread_id(thread_id: &str) -> Result<String, RuntimeBridgeError> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() || thread_id.len() > 256 {
        return Err(RuntimeBridgeError::InvalidThreadId);
    }
    Ok(thread_id.to_owned())
}

fn validate_turn_input(input: &str) -> Result<String, RuntimeBridgeError> {
    let input = input.trim();
    if input.is_empty() || input.len() > 100_000 {
        return Err(RuntimeBridgeError::InvalidInput);
    }
    Ok(input.to_owned())
}

fn scheduler_capability(capability: SchemaCapability) -> Capability {
    match capability {
        SchemaCapability::Supported => Capability::Supported,
        SchemaCapability::NotDeclared | SchemaCapability::Incompatible => Capability::Unsupported,
        SchemaCapability::Unavailable => Capability::Unknown,
    }
}

fn runtime_profile_from_dispatch(profile: &DispatchAgentProfile) -> AgentRuntimeProfile {
    AgentRuntimeProfile {
        agent_id: profile.agent_id.clone(),
        agent_key: profile.agent_key.clone(),
        agent_name: profile.agent_name.clone(),
        instruction: profile.instruction.clone(),
        orchestration_phase: profile.orchestration_phase.clone(),
        sandbox_policy: profile.sandbox_policy.clone(),
        reasoning_effort: profile.reasoning_effort.clone(),
        model_slug: profile.model_slug.clone(),
        model_provider: profile.model_provider.clone(),
        runtime_fingerprint: profile.runtime_fingerprint.clone(),
    }
}

fn orchestration_error_to_api(error: OrchestrationError) -> ApiError {
    let retryable = matches!(
        error.code,
        OrchestrationErrorCode::RuntimeUnavailable
            | OrchestrationErrorCode::SchemaUnverified
            | OrchestrationErrorCode::ConcurrencyLimitReached
            | OrchestrationErrorCode::StaleExpectedDecision
            | OrchestrationErrorCode::StaleExpectedCandidate
            | OrchestrationErrorCode::DispatchOutcomeUnknown
            | OrchestrationErrorCode::RecoveryRequired
            | OrchestrationErrorCode::PersistenceError
    );
    let mut details = BTreeMap::from([("reason", error.message)]);
    if let Some(field_path) = error.field_path {
        details.insert("fieldPath", field_path);
    }
    if let Some(job_id) = error.job_id {
        details.insert("jobId", job_id);
    }
    if let Some(attempt_id) = error.attempt_id {
        details.insert("attemptId", attempt_id);
    }
    ApiError::new(
        error.code.as_str(),
        "CAS 原子调度未授权此次派发。",
        retryable,
        Some(details),
    )
}

fn schedule_stop_to_api(stop: ScheduleStop, retryable: bool) -> ApiError {
    ApiError::new(
        stop.error_code.as_str(),
        "CAS 调度已停止，未调用 App Server。",
        retryable,
        Some(BTreeMap::from([
            ("jobId", stop.job.job_id),
            ("scheduleDecisionId", stop.schedule_decision_id),
            (
                "admissionDecision",
                stop.admission_decision.as_str().to_owned(),
            ),
            ("reasonCode", stop.reason_code),
        ])),
    )
}

fn existing_job_to_api(existing: OrchestrationJobCreateResponse) -> ApiError {
    let code = match existing.outcome {
        IdempotencyOutcome::ExistingUncertain => OrchestrationErrorCode::DispatchOutcomeUnknown,
        IdempotencyOutcome::KeyConflict => OrchestrationErrorCode::IdempotencyKeyConflict,
        IdempotencyOutcome::Created
        | IdempotencyOutcome::ExistingNotDispatched
        | IdempotencyOutcome::ExistingKnown => OrchestrationErrorCode::AttemptNotCurrent,
    };
    let outcome = serde_json::to_value(existing.outcome)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "EXISTING_KNOWN".to_owned());
    ApiError::new(
        code.as_str(),
        "该幂等请求已有 Job；CAS 不会创建第二个 Turn。",
        false,
        Some(BTreeMap::from([
            ("jobId", existing.job.job_id),
            ("idempotencyOutcome", outcome),
        ])),
    )
}

fn agent_thread_params(profile: &AgentRuntimeProfile, cwd: &str) -> Value {
    let mut params = json!({
        "cwd": cwd,
        "model": profile.model_slug,
        "developerInstructions": render_delegated_agent_instructions_for_phase(
            &profile.instruction,
            Some(&profile.orchestration_phase),
        ),
        "sandbox": match profile.sandbox_policy.as_str() {
            "READ_ONLY" => Some("read-only"),
            "WORKSPACE_WRITE" => Some("workspace-write"),
            "DANGER_FULL_ACCESS" => Some("danger-full-access"),
            _ => None,
        },
    });
    if let Some(model_provider) = &profile.model_provider {
        params
            .as_object_mut()
            .expect("agent thread params are an object")
            .insert(
                "modelProvider".to_owned(),
                Value::String(model_provider.clone()),
            );
    }
    params
}

fn runtime_response_error(error: ProtocolParseError) -> RuntimeBridgeError {
    match error {
        ProtocolParseError::MissingField(field) | ProtocolParseError::InvalidField(field) => {
            RuntimeBridgeError::InvalidProtocolResponse(field)
        }
        ProtocolParseError::NegativeToken | ProtocolParseError::TokenOverflow => {
            RuntimeBridgeError::InvalidProtocolResponse("tokenUsage.total")
        }
    }
}

#[derive(Default)]
struct ObservedThread {
    identity_known: bool,
    session_id: Option<String>,
    parent_thread_id: Option<String>,
    execution_kind: Option<ExecutionKind>,
    agent_key: Option<String>,
    model_slug: Option<String>,
    started_at: Option<String>,
    latest_usage: Option<NormalizedUsage>,
}

struct RuntimeObserver {
    usage: Arc<UsageService>,
    threads: HashMap<String, ObservedThread>,
    last_event_at: Option<String>,
}

impl RuntimeObserver {
    fn new(usage: Arc<UsageService>) -> Self {
        Self {
            usage,
            threads: HashMap::new(),
            last_event_at: None,
        }
    }

    fn observe(&mut self, event: NormalizedRuntimeEvent) -> Result<(), RuntimeBridgeError> {
        self.last_event_at = Some(self.usage.current_timestamp()?);
        match event {
            NormalizedRuntimeEvent::ThreadStarted {
                thread_id,
                session_id,
                parent_thread_id,
                ..
            } => {
                let resolved_session_id = session_id.or_else(|| {
                    parent_thread_id
                        .as_deref()
                        .map(|parent| self.root_thread_id(parent))
                        .or(Some(thread_id.clone()))
                });
                validate_observed_execution_kind(
                    self.threads.get(&thread_id),
                    ExecutionKind::ManagedWorker,
                )?;
                let thread = self.threads.entry(thread_id.clone()).or_default();
                thread.identity_known = true;
                thread.session_id = resolved_session_id;
                thread.parent_thread_id = parent_thread_id;
                thread.execution_kind = Some(ExecutionKind::ManagedWorker);
                self.persist(&thread_id, None)?;
            }
            NormalizedRuntimeEvent::ParentChild {
                parent_thread_id,
                child_thread_ids,
                model_slug,
                ..
            } => {
                let session_id = self.root_thread_id(&parent_thread_id);
                for child_thread_id in &child_thread_ids {
                    validate_observed_execution_kind(
                        self.threads.get(child_thread_id),
                        ExecutionKind::ObservedExternal,
                    )?;
                }
                for child_thread_id in child_thread_ids {
                    let thread = self.threads.entry(child_thread_id.clone()).or_default();
                    thread.identity_known = true;
                    thread.session_id = Some(session_id.clone());
                    thread.parent_thread_id = Some(parent_thread_id.clone());
                    thread.execution_kind = Some(ExecutionKind::ObservedExternal);
                    if thread.model_slug.is_none() {
                        thread.model_slug = model_slug.clone();
                    }
                    self.persist(&child_thread_id, None)?;
                }
            }
            NormalizedRuntimeEvent::AgentPath {
                thread_id,
                agent_key,
                ..
            } => {
                self.threads.entry(thread_id.clone()).or_default().agent_key = Some(agent_key);
                self.persist(&thread_id, None)?;
            }
            NormalizedRuntimeEvent::Usage {
                thread_id, usage, ..
            } => {
                let thread = self.threads.entry(thread_id.clone()).or_default();
                thread.latest_usage = Some(usage);
                if thread.started_at.is_none() {
                    thread.started_at = self.last_event_at.clone();
                }
                self.persist(&thread_id, None)?;
            }
            NormalizedRuntimeEvent::TurnFinished {
                thread_id,
                successful,
                ..
            } => {
                let thread = self.threads.entry(thread_id.clone()).or_default();
                if !thread.identity_known {
                    thread.identity_known = true;
                    thread.session_id = Some(thread_id.clone());
                }
                if thread.execution_kind.is_none() {
                    thread.execution_kind = Some(ExecutionKind::ObservedExternal);
                }
                self.persist(&thread_id, Some(successful))?;
            }
        }
        Ok(())
    }

    fn persist(
        &mut self,
        thread_id: &str,
        completion: Option<bool>,
    ) -> Result<(), RuntimeBridgeError> {
        let Some(thread) = self.threads.get(thread_id) else {
            return Ok(());
        };
        if !thread.identity_known {
            return Ok(());
        }
        let Some(latest) = thread.latest_usage.clone() else {
            return Ok(());
        };
        let timestamp = self.usage.current_timestamp()?;
        let session_id = thread
            .session_id
            .clone()
            .unwrap_or_else(|| self.root_thread_id(thread_id));
        let attribution = self.usage.resolve_attribution(
            Some(thread_id),
            thread.agent_key.as_deref(),
            thread.model_slug.as_deref(),
        )?;
        let status = match completion {
            Some(true) if !latest.partial => "FINAL",
            Some(_) => "PARTIAL",
            None if latest.partial => "PARTIAL",
            None => "LIVE",
        };
        let snapshot = usage_snapshot(
            session_id,
            thread_id,
            thread,
            latest,
            attribution.as_ref(),
            status,
            &timestamp,
        );
        self.usage.upsert_snapshot(snapshot)?;
        Ok(())
    }

    fn root_thread_id(&self, thread_id: &str) -> String {
        let mut current = thread_id;
        for _ in 0..64 {
            let Some(parent) = self
                .threads
                .get(current)
                .and_then(|thread| thread.parent_thread_id.as_deref())
            else {
                return current.to_owned();
            };
            current = parent;
        }
        thread_id.to_owned()
    }

    fn mark_live_records_partial(&mut self) {
        let thread_ids = self.threads.keys().cloned().collect::<Vec<_>>();
        for thread_id in thread_ids {
            let _ = self.persist(&thread_id, Some(false));
        }
    }
}

fn usage_snapshot(
    session_id: String,
    thread_id: &str,
    thread: &ObservedThread,
    usage: NormalizedUsage,
    attribution: Option<&UsageAttribution>,
    status: &str,
    timestamp: &str,
) -> UsageSnapshot {
    UsageSnapshot {
        codex_session_id: session_id,
        codex_thread_id: thread_id.to_owned(),
        parent_thread_id: thread.parent_thread_id.clone(),
        agent_id: attribution.map(|value| value.agent_id.clone()),
        agent_name_snapshot: attribution.map(|value| value.agent_name.clone()),
        provider_id: attribution.map(|value| value.provider_id.clone()),
        provider_name_snapshot: attribution.map(|value| value.provider_name.clone()),
        model_id: attribution.map(|value| value.model_id.clone()),
        model_name_snapshot: attribution.map(|value| value.model_name.clone()),
        input_tokens: usage.input_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cached_input_provided: Some(usage.cached_input_provided),
        cache_write_input_tokens: usage.cache_write_input_tokens,
        output_tokens: usage.output_tokens,
        reasoning_output_tokens: usage.reasoning_output_tokens,
        total_tokens: usage.total_tokens,
        current_context_tokens: usage.current_context_tokens,
        model_context_window: usage.model_context_window,
        usage_status: status.to_owned(),
        source: "CODEX_APP_SERVER".to_owned(),
        started_at: thread
            .started_at
            .clone()
            .unwrap_or_else(|| timestamp.to_owned()),
        completed_at: matches!(status, "FINAL" | "PARTIAL").then(|| timestamp.to_owned()),
        updated_at: timestamp.to_owned(),
        execution_kind: thread
            .execution_kind
            .unwrap_or(ExecutionKind::ObservedExternal),
    }
}

fn validate_observed_execution_kind(
    thread: Option<&ObservedThread>,
    execution_kind: ExecutionKind,
) -> Result<(), RuntimeBridgeError> {
    match thread.and_then(|thread| thread.execution_kind) {
        Some(existing) if existing != execution_kind => {
            Err(RuntimeBridgeError::ExecutionKindConflict)
        }
        _ => Ok(()),
    }
}

impl From<RuntimeBridgeError> for ApiError {
    fn from(error: RuntimeBridgeError) -> Self {
        let (code, message, retryable) = match error {
            RuntimeBridgeError::AlreadyRunning => (
                "USAGE_MONITOR_ALREADY_RUNNING",
                "Token Usage 监控已经在运行。",
                false,
            ),
            RuntimeBridgeError::InitializeTimeout => (
                "APP_SERVER_INITIALIZE_TIMEOUT",
                "Codex App Server 初始化超时。",
                true,
            ),
            RuntimeBridgeError::InitializeRejected(_) => (
                "APP_SERVER_INITIALIZE_REJECTED",
                "Codex App Server 拒绝初始化。",
                false,
            ),
            RuntimeBridgeError::NotRunning => (
                "USAGE_MONITOR_NOT_RUNNING",
                "请先启动 Token Usage 监控。",
                false,
            ),
            RuntimeBridgeError::ManagedSessionUnsupported(SchemaCapability::Unavailable) => (
                "APP_SERVER_MANAGED_SESSION_UNPROVED",
                "无法证明当前 Codex App Server 支持 CAS 托管会话，请重新检测 Codex。",
                true,
            ),
            RuntimeBridgeError::ManagedSessionUnsupported(_) => (
                "APP_SERVER_MANAGED_SESSION_UNSUPPORTED",
                "当前 Codex App Server Schema 不支持 CAS 托管会话。",
                false,
            ),
            RuntimeBridgeError::AgentExecutionUnsupported(SchemaCapability::Unavailable) => (
                "APP_SERVER_AGENT_EXECUTION_UNPROVED",
                "无法证明当前 Codex App Server 支持安全指定 Agent，请重新检测 Codex。",
                true,
            ),
            RuntimeBridgeError::AgentExecutionUnsupported(_) => (
                "APP_SERVER_AGENT_EXECUTION_UNSUPPORTED",
                "当前 Codex App Server Schema 不支持安全指定 Agent 的 Provider、Model 与 Instructions。",
                false,
            ),
            RuntimeBridgeError::InvalidCwd => (
                "MANAGED_SESSION_CWD_INVALID",
                "托管会话工作目录必须是已存在的绝对目录。",
                false,
            ),
            RuntimeBridgeError::InvalidThreadId => (
                "MANAGED_SESSION_THREAD_ID_INVALID",
                "托管会话 Thread ID 无效。",
                false,
            ),
            RuntimeBridgeError::InvalidInput => (
                "MANAGED_TURN_INPUT_INVALID",
                "托管 Turn 输入不能为空且不能超过 100000 个字符。",
                false,
            ),
            RuntimeBridgeError::ThreadNotBound => (
                "MANAGED_SESSION_NOT_BOUND",
                "该 Thread 尚未绑定到当前 CAS Runtime Bridge。",
                false,
            ),
            RuntimeBridgeError::SessionRecoveryRequired => (
                "MANAGED_SESSION_RECOVERY_REQUIRED",
                "App Server 曾异常退出，请先恢复该托管会话。",
                false,
            ),
            RuntimeBridgeError::RecoveryTurnStillRunning => (
                "MANAGED_TURN_RECOVERY_STILL_RUNNING",
                "中断前的 Turn 仍在运行，CAS 不会启动重复任务。",
                true,
            ),
            RuntimeBridgeError::RecoveryOutcomeUnknown => (
                "MANAGED_TURN_RECOVERY_UNKNOWN",
                "无法证明中断前 Turn 已结束；请稍后核验，或显式放弃该不确定状态。",
                true,
            ),
            RuntimeBridgeError::TurnAlreadyRunning => (
                "MANAGED_TURN_ALREADY_RUNNING",
                "当前托管会话已有 Turn 正在运行。",
                false,
            ),
            RuntimeBridgeError::ProtocolRejected(_) => (
                "APP_SERVER_REQUEST_REJECTED",
                "Codex App Server 拒绝了托管会话请求。",
                false,
            ),
            RuntimeBridgeError::ProtocolTimeout(_) => (
                "APP_SERVER_REQUEST_TIMEOUT",
                "Codex App Server 请求超时。",
                true,
            ),
            RuntimeBridgeError::InvalidProtocolResponse(_)
            | RuntimeBridgeError::UnexpectedThreadResponse => (
                "APP_SERVER_PROTOCOL_INCOMPATIBLE",
                "当前 Codex App Server 返回了无法安全识别的会话响应。",
                false,
            ),
            RuntimeBridgeError::ExecutionKindConflict => (
                "EXECUTION_KIND_MISMATCH",
                "Thread 已有不可变的执行身份，拒绝用冲突事件改写。",
                false,
            ),
            RuntimeBridgeError::StreamClosed => (
                "APP_SERVER_STREAM_CLOSED",
                "Codex App Server 事件流已关闭。",
                true,
            ),
            RuntimeBridgeError::Spawn(_) | RuntimeBridgeError::MissingPipe(_) => (
                "APP_SERVER_START_FAILED",
                "无法启动 Codex App Server。",
                true,
            ),
            RuntimeBridgeError::RecoveryLaunchUnavailable => (
                "APP_SERVER_RECOVERY_UNAVAILABLE",
                "缺少上一次成功启动参数，无法自动恢复 Codex App Server。",
                false,
            ),
            RuntimeBridgeError::StateUnavailable => (
                "USAGE_MONITOR_STATE_UNAVAILABLE",
                "Token Usage 监控状态当前不可用。",
                true,
            ),
            RuntimeBridgeError::Usage(_) | RuntimeBridgeError::Receipt(_) => (
                "USAGE_DATABASE_OPERATION_FAILED",
                "Token Usage 数据操作失败。",
                true,
            ),
            RuntimeBridgeError::SchemaProbe(_)
            | RuntimeBridgeError::ProtocolWrite(_)
            | RuntimeBridgeError::Process(_)
            | RuntimeBridgeError::RequestIdExhausted => (
                "APP_SERVER_OPERATION_FAILED",
                "Codex App Server 操作失败。",
                true,
            ),
        };
        let details = match error {
            RuntimeBridgeError::InitializeRejected(details) => {
                Some(BTreeMap::from([("reason", details)]))
            }
            RuntimeBridgeError::ProtocolRejected(details) => {
                Some(BTreeMap::from([("reason", details)]))
            }
            RuntimeBridgeError::ProtocolTimeout(method) => {
                Some(BTreeMap::from([("method", method)]))
            }
            RuntimeBridgeError::InvalidProtocolResponse(field) => {
                Some(BTreeMap::from([("field", field.to_owned())]))
            }
            RuntimeBridgeError::ManagedSessionUnsupported(capability)
            | RuntimeBridgeError::AgentExecutionUnsupported(capability) => {
                Some(BTreeMap::from([(
                    "capability",
                    format!("{capability:?}").to_ascii_uppercase(),
                )]))
            }
            RuntimeBridgeError::Spawn(details)
            | RuntimeBridgeError::Process(details)
            | RuntimeBridgeError::ProtocolWrite(details)
            | RuntimeBridgeError::SchemaProbe(details) => {
                Some(BTreeMap::from([("reason", details.to_string())]))
            }
            _ => None,
        };
        ApiError::new(code, message, retryable, details)
    }
}

#[derive(Debug)]
pub(crate) enum RuntimeBridgeError {
    AlreadyRunning,
    Spawn(std::io::Error),
    MissingPipe(&'static str),
    InitializeTimeout,
    InitializeRejected(String),
    NotRunning,
    ManagedSessionUnsupported(SchemaCapability),
    AgentExecutionUnsupported(SchemaCapability),
    InvalidCwd,
    InvalidThreadId,
    InvalidInput,
    ThreadNotBound,
    SessionRecoveryRequired,
    RecoveryTurnStillRunning,
    RecoveryOutcomeUnknown,
    TurnAlreadyRunning,
    ProtocolWrite(std::io::Error),
    ProtocolRejected(String),
    ProtocolTimeout(String),
    InvalidProtocolResponse(&'static str),
    UnexpectedThreadResponse,
    ExecutionKindConflict,
    StreamClosed,
    RecoveryLaunchUnavailable,
    RequestIdExhausted,
    Process(std::io::Error),
    SchemaProbe(std::io::Error),
    Usage(UsageServiceError),
    Receipt(OrchestrationError),
    StateUnavailable,
}

impl fmt::Display for RuntimeBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning => formatter.write_str("runtime bridge already running"),
            Self::Spawn(error) => write!(formatter, "app server spawn failed: {error}"),
            Self::MissingPipe(pipe) => write!(formatter, "app server missing {pipe} pipe"),
            Self::InitializeTimeout => formatter.write_str("app server initialize timed out"),
            Self::InitializeRejected(message) => {
                write!(formatter, "app server initialize rejected: {message}")
            }
            Self::NotRunning => formatter.write_str("runtime bridge is not running"),
            Self::ManagedSessionUnsupported(capability) => {
                write!(
                    formatter,
                    "managed sessions are not supported by the detected schema: {capability:?}"
                )
            }
            Self::AgentExecutionUnsupported(capability) => write!(
                formatter,
                "managed agent execution is not supported by the detected schema: {capability:?}"
            ),
            Self::InvalidCwd => formatter.write_str("managed session cwd is invalid"),
            Self::InvalidThreadId => formatter.write_str("managed session thread id is invalid"),
            Self::InvalidInput => formatter.write_str("managed turn input is invalid"),
            Self::ThreadNotBound => formatter.write_str("thread is not bound to runtime bridge"),
            Self::SessionRecoveryRequired => {
                formatter.write_str("managed session recovery is required")
            }
            Self::RecoveryTurnStillRunning => {
                formatter.write_str("managed turn is still running after recovery")
            }
            Self::RecoveryOutcomeUnknown => {
                formatter.write_str("managed turn recovery outcome is unknown")
            }
            Self::TurnAlreadyRunning => formatter.write_str("managed turn is already running"),
            Self::ProtocolWrite(error) => write!(formatter, "protocol write failed: {error}"),
            Self::ProtocolRejected(message) => {
                write!(formatter, "app server request rejected: {message}")
            }
            Self::ProtocolTimeout(method) => {
                write!(formatter, "app server request timed out: {method}")
            }
            Self::InvalidProtocolResponse(field) => {
                write!(
                    formatter,
                    "app server response missing or invalid field: {field}"
                )
            }
            Self::UnexpectedThreadResponse => {
                formatter.write_str("app server returned an unexpected thread")
            }
            Self::ExecutionKindConflict => formatter.write_str("thread execution kind conflict"),
            Self::StreamClosed => formatter.write_str("app server event stream closed"),
            Self::RecoveryLaunchUnavailable => {
                formatter.write_str("runtime bridge recovery launch is unavailable")
            }
            Self::RequestIdExhausted => formatter.write_str("app server request id exhausted"),
            Self::Process(error) => write!(formatter, "app server process failed: {error}"),
            Self::SchemaProbe(error) => write!(formatter, "schema probe failed: {error}"),
            Self::Usage(error) => write!(formatter, "usage operation failed: {error}"),
            Self::Receipt(error) => write!(
                formatter,
                "receipt event operation failed: {}",
                error.message
            ),
            Self::StateUnavailable => formatter.write_str("runtime bridge state unavailable"),
        }
    }
}

impl std::error::Error for RuntimeBridgeError {}

impl From<UsageServiceError> for RuntimeBridgeError {
    fn from(error: UsageServiceError) -> Self {
        Self::Usage(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration_contract::{
        ExecutionKindPolicy, OutputContract, PermissionPolicy, ReviewPolicy,
        TASK_PACKET_SCHEMA_VERSION,
    };
    use crate::usage::UsageListRequest;
    use std::fs;
    use std::time::Instant;
    use uuid::Uuid;

    fn seed_dispatch_agent(database_path: &Path) {
        let connection = rusqlite::Connection::open(database_path).unwrap();
        connection
            .execute_batch(
                "INSERT INTO providers (
                    id, provider_key, name, provider_type, base_url, protocol, auth_type,
                    enabled, source, preset_id, created_at, updated_at
                 ) VALUES (
                    'provider-dispatch', 'openai', 'OpenAI', 'PRESET',
                    'https://api.example/v1', 'RESPONSES', 'BEARER_TOKEN', 1,
                    'BUILT_IN', 'codex-native', '2026-09-09T00:00:00Z',
                    '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO models (
                    id, provider_id, model_id, display_name, enabled, source,
                    created_at, updated_at
                 ) VALUES (
                    'model-dispatch', 'provider-dispatch', 'gpt-test', 'GPT Test', 1,
                    'PRESET', '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO agents (
                    id, agent_key, name, description, instruction, agent_type, enabled,
                    sandbox_policy, reasoning_policy, source, managed, role_key,
                    orchestration_phase, created_at, updated_at
                 ) VALUES (
                    'agent-dispatch', 'executor', 'Executor', 'test', '执行任务', 'CUSTOM', 1,
                    'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, 'executor', 'EXECUTION',
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO agent_model_bindings (
                    id, agent_id, model_id, enabled, priority, source, created_at, updated_at
                 ) VALUES (
                    'binding-dispatch', 'agent-dispatch', 'model-dispatch', 1, 0, 'CAS',
                    '2026-09-09T00:00:00Z', '2026-09-09T00:00:00Z'
                 );
                 INSERT INTO active_agent_bindings (role_key, agent_id, created_at, updated_at)
                 VALUES (
                    'executor', 'agent-dispatch', '2026-09-09T00:00:00Z',
                    '2026-09-09T00:00:00Z'
                 );",
            )
            .unwrap();
    }

    fn dispatch_request(
        root: &Path,
        job_id: &str,
        idempotency_key: &str,
    ) -> AgentThreadExecutionRequest {
        AgentThreadExecutionRequest {
            task_packet: TaskPacket {
                schema_version: TASK_PACKET_SCHEMA_VERSION,
                job_id: job_id.to_owned(),
                idempotency_key: idempotency_key.to_owned(),
                agent_id: "agent-dispatch".to_owned(),
                parent_thread_id: "parent-dispatch".to_owned(),
                workspace_scope_key: normalize_workspace_scope_key(root.to_string_lossy().as_ref())
                    .unwrap(),
                task_scope_key: job_id.to_owned(),
                objective: "验证派发边界".to_owned(),
                allowed_scope: vec![root.to_string_lossy().into_owned()],
                constraints: Vec::new(),
                success_criteria: vec!["派发前审计已提交".to_owned()],
                allowed_tools: Vec::new(),
                permission_policy: PermissionPolicy::WorkspaceWrite,
                execution_kind_policy: ExecutionKindPolicy::ManagedWorkerRequired,
                context_references: Vec::new(),
                output_contract: OutputContract::StandardV1,
                review_policy: ReviewPolicy::PrimaryRequired,
            },
            cwd: root.to_string_lossy().into_owned(),
            input: "执行测试".to_owned(),
            expected_decision: RouteAction::Spawn,
            expected_candidate_thread_id: None,
        }
    }

    #[test]
    fn agent_profile_maps_to_exact_app_server_overrides() {
        let profile = AgentRuntimeProfile {
            agent_id: "agent-1".to_owned(),
            agent_key: "executor".to_owned(),
            agent_name: "Executor".to_owned(),
            instruction: "只执行已明确的实现任务。".to_owned(),
            orchestration_phase: "EXECUTION".to_owned(),
            sandbox_policy: "WORKSPACE_WRITE".to_owned(),
            reasoning_effort: Some("high".to_owned()),
            model_slug: "deepseek-v4-flash".to_owned(),
            model_provider: Some("cas_deepseek".to_owned()),
            runtime_fingerprint: "test".to_owned(),
        };

        let params = agent_thread_params(&profile, "C:\\workspace\\project");
        assert_eq!(params.get("cwd"), Some(&json!("C:\\workspace\\project")));
        assert_eq!(params.get("model"), Some(&json!("deepseek-v4-flash")));
        assert_eq!(params.get("modelProvider"), Some(&json!("cas_deepseek")));
        assert_eq!(params.get("sandbox"), Some(&json!("workspace-write")));
        let instructions = params
            .get("developerInstructions")
            .and_then(Value::as_str)
            .unwrap();
        assert!(instructions.contains("只执行已明确的实现任务。"));
        assert!(instructions.contains("你是由 Primary 委派的 Child Agent"));
        assert!(instructions.contains("阶段契约：EXECUTION"));
        assert!(instructions.contains("`TOOLS: -` 表示禁用"));
    }

    #[test]
    fn native_agent_profile_omits_model_provider_override() {
        let profile = AgentRuntimeProfile {
            agent_id: "agent-native".to_owned(),
            agent_key: "native-executor".to_owned(),
            agent_name: "Native Executor".to_owned(),
            instruction: "执行任务。".to_owned(),
            orchestration_phase: "EXECUTION".to_owned(),
            sandbox_policy: "WORKSPACE_WRITE".to_owned(),
            reasoning_effort: Some("high".to_owned()),
            model_slug: "gpt-5.6-luna".to_owned(),
            model_provider: None,
            runtime_fingerprint: "test".to_owned(),
        };

        let params = agent_thread_params(&profile, "C:\\workspace\\project");
        assert_eq!(params.get("model"), Some(&json!("gpt-5.6-luna")));
        assert!(params.get("modelProvider").is_none());
    }

    #[test]
    fn atomic_audit_gates_app_server_dispatch_without_real_provider() {
        let root = std::env::temp_dir().join(format!("cas-runtime-dispatch-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        let bridge =
            RuntimeBridgeService::open(&database_path, &root, &root.join("cas-helper.exe"))
                .unwrap();
        let orchestration = OrchestrationJobService::open(&database_path).unwrap();
        seed_dispatch_agent(&database_path);
        {
            let mut state = bridge.state().unwrap();
            state.status = RuntimeBridgeStatus::Running;
            state.managed_session_capability = SchemaCapability::Supported;
            state.agent_execution_capability = SchemaCapability::Supported;
        }

        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TRIGGER fail_atomic_lease
                 BEFORE INSERT ON runtime_delegation_leases
                 BEGIN SELECT RAISE(ABORT, 'injected lease failure'); END;",
            )
            .unwrap();
        drop(connection);
        let error = bridge
            .execute_agent_thread(
                &orchestration,
                dispatch_request(&root, "job-failed", "dispatch-failed"),
            )
            .unwrap_err();
        assert_eq!(error.code(), "PERSISTENCE_ERROR");

        let connection = rusqlite::Connection::open(&database_path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM orchestration_jobs", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        connection
            .execute_batch("DROP TRIGGER fail_atomic_lease;")
            .unwrap();
        drop(connection);

        let error = bridge
            .execute_agent_thread(
                &orchestration,
                dispatch_request(&root, "job-dispatched", "dispatch-audited"),
            )
            .unwrap_err();
        assert_eq!(error.code(), "USAGE_MONITOR_NOT_RUNNING");
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        let audit: (String, String, i64, i64, i64) = connection
            .query_row(
                "SELECT job.state, attempt.state,
                        attempt.dispatch_recorded_at IS NOT NULL,
                        (SELECT COUNT(*) FROM agent_schedule_decisions WHERE job_id = job.job_id),
                        (SELECT COUNT(*) FROM runtime_delegation_leases WHERE id = attempt.lease_id)
                 FROM orchestration_jobs job
                 JOIN job_attempts attempt ON attempt.job_id = job.job_id
                 WHERE job.job_id = 'job-dispatched'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            audit,
            ("DISPATCHED".to_owned(), "DISPATCHING".to_owned(), 1, 2, 1)
        );

        drop(connection);
        drop(orchestration);
        drop(bridge);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_thread_parser_accepts_current_and_legacy_identity_fields() {
        assert_eq!(
            parse_thread_response(&json!({
                "thread": {"id": "thread-1", "sessionId": "session-1", "future": true}
            }))
            .unwrap(),
            crate::runtime_adapter::NormalizedThread {
                thread_id: "thread-1".to_owned(),
                session_id: Some("session-1".to_owned()),
            },
        );
        assert_eq!(
            parse_thread_response(&json!({
                "thread": {"thread_id": "thread-2", "session_id": "session-2"}
            }))
            .unwrap(),
            crate::runtime_adapter::NormalizedThread {
                thread_id: "thread-2".to_owned(),
                session_id: Some("session-2".to_owned()),
            },
        );
    }

    #[test]
    fn protocol_response_router_delivers_result_without_exposing_notifications() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (sender, receiver) = mpsc::sync_channel(1);
        pending.lock().unwrap().insert(2, sender);
        assert!(resolve_pending_response(
            &json!({"id": 2, "result": {"thread": {"id": "thread-1"}}}),
            &pending,
        ));
        assert_eq!(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .unwrap()["thread"]["id"],
            "thread-1",
        );
        assert!(!resolve_pending_response(
            &json!({"method": "thread/started", "params": {}}),
            &pending,
        ));
        assert!(!resolve_pending_response(
            &json!({
                "id": 99,
                "method": "item/commandExecution/requestApproval",
                "params": {}
            }),
            &pending,
        ));
    }

    #[test]
    fn initialize_response_does_not_swallow_later_server_request_id() {
        assert!(is_initialize_response(
            &json!({"id": 1, "result": {}}),
            true
        ));
        assert!(!is_initialize_response(
            &json!({
                "id": 1,
                "method": "item/commandExecution/requestApproval",
                "params": {}
            }),
            false,
        ));
    }

    #[test]
    fn only_exact_cas_control_plane_commands_are_auto_approved() {
        let helper = Path::new("C:\\Program Files\\CAS\\cas-helper.exe");
        let database = Path::new("C:\\CAS Data\\cas.db");
        let request = |command: &str| {
            json!({
                "params": {
                    "commandActions": [{"type": "unknown", "command": command}]
                }
            })
        };

        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" job-schedule \"C:\\CAS Data\\cas.db\" executor \"C:\\Work Space\""
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" job-bind \"C:\\CAS Data\\cas.db\" job-1 attempt-1 019ffb28-1234 \"C:\\Work Space\""
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" job-observe \"C:\\CAS Data\\cas.db\" job-1 attempt-1 019ffb28-1234 \"C:\\Work Space\""
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" job-review \"C:\\CAS Data\\cas.db\" job-1 attempt-1"
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request("& \"C:\\Program Files\\CAS\\cas-helper.exe\" schedule executor stable-task"),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" bind executor 019ffb28-1234 stable-task"
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" schedule \"C:\\CAS Data\\cas.db\" executor \"C:\\Work Space\" stable-task"
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" bind \"C:\\CAS Data\\cas.db\" executor 019ffb28-1234 \"C:\\Work Space\" stable-task"
            ),
            helper,
            database,
        ));
        assert!(is_cas_control_plane_request(
            &request(
                "& 'C:\\Program Files\\CAS\\cas-helper.exe' schedule 'C:\\CAS Data\\cas.db' executor 'C:\\Work Space' stable-task"
            ),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &request("& \"C:\\Program Files\\CAS\\cas-helper.exe\" token credential-id"),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" schedule executor; Remove-Item victim"
            ),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" job-review \"C:\\CAS Data\\cas.db\" job-1 attempt-1; Remove-Item victim"
            ),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &request("& \"C:\\Other\\cas-helper.exe\" schedule executor stable-task"),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" schedule \"C:\\Other\\cas.db\" executor \"C:\\Work Space\" stable-task"
            ),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &request(
                "& \"C:\\Program Files\\CAS\\cas-helper.exe\" schedule \"C:\\CAS Data\\cas.db\" executor relative-workspace stable-task"
            ),
            helper,
            database,
        ));
        assert!(!is_cas_control_plane_request(
            &json!({
                "params": {
                    "commandActions": [
                        {
                            "type": "unknown",
                            "command": "& \"C:\\Program Files\\CAS\\cas-helper.exe\" schedule executor stable-task"
                        },
                        {"type": "unknown", "command": "Remove-Item victim"}
                    ]
                }
            }),
            helper,
            database,
        ));
    }

    #[test]
    fn e2e_auto_approval_stays_inside_isolated_workspace_without_escalation() {
        let root = Path::new("C:\\Temp\\cas-rc1-test");
        let request = |cwd: &str| json!({"params": {"cwd": cwd}});

        assert!(is_isolated_e2e_workspace_request_for_root(
            &request("C:\\Temp\\cas-rc1-test\\workspace"),
            root,
        ));
        assert!(!is_isolated_e2e_workspace_request_for_root(
            &request("C:\\Temp\\cas-rc1-test\\cas-data"),
            root,
        ));
        assert!(!is_isolated_e2e_workspace_request_for_root(
            &json!({
                "params": {
                    "cwd": "C:\\Temp\\cas-rc1-test\\workspace",
                    "additionalPermissions": {"network": {"enabled": true}}
                }
            }),
            root,
        ));
    }

    #[test]
    fn stream_failure_preserves_uncertain_turn_for_safe_recovery() {
        let state = Arc::new(Mutex::new(RuntimeBridgeState {
            managed_sessions: BTreeMap::from([(
                "thread-1".to_owned(),
                ManagedSessionState {
                    thread_id: "thread-1".to_owned(),
                    session_id: Some("session-1".to_owned()),
                    origin: ManagedSessionOrigin::Started,
                    status: ManagedSessionStatus::Running,
                    cwd: Some("C:\\workspace".to_owned()),
                    active_turn_id: Some("turn-1".to_owned()),
                    attached_at: "2026-08-11T00:00:00Z".to_owned(),
                },
            )]),
            ..RuntimeBridgeState::default()
        }));
        mark_stream_failure(&state, "closed".to_owned());
        let state = state.lock().unwrap();
        let session = state.managed_sessions.get("thread-1").unwrap();
        assert_eq!(state.status, RuntimeBridgeStatus::Failed);
        assert_eq!(session.status, ManagedSessionStatus::RecoveryRequired);
        assert_eq!(session.active_turn_id.as_deref(), Some("turn-1"));
    }

    #[test]
    fn idle_stream_failure_does_not_invent_an_uncertain_turn() {
        let state = Arc::new(Mutex::new(RuntimeBridgeState {
            managed_sessions: BTreeMap::from([(
                "thread-1".to_owned(),
                ManagedSessionState {
                    thread_id: "thread-1".to_owned(),
                    session_id: Some("session-1".to_owned()),
                    origin: ManagedSessionOrigin::Started,
                    status: ManagedSessionStatus::Idle,
                    cwd: Some("C:\\workspace".to_owned()),
                    active_turn_id: None,
                    attached_at: "2026-08-11T00:00:00Z".to_owned(),
                },
            )]),
            ..RuntimeBridgeState::default()
        }));
        mark_stream_failure(&state, "closed".to_owned());
        let state = state.lock().unwrap();
        let session = state.managed_sessions.get("thread-1").unwrap();
        assert_eq!(state.status, RuntimeBridgeStatus::Failed);
        assert_eq!(session.status, ManagedSessionStatus::Idle);
        assert_eq!(session.active_turn_id, None);
    }

    #[test]
    fn recovery_only_clears_a_proven_terminal_turn_and_has_a_retry_ceiling() {
        let thread = |status: &str| {
            json!({
                "thread": {
                    "id": "thread-1",
                    "turns": [{"id": "turn-1", "status": status}]
                }
            })
        };
        assert_eq!(
            recovery_turn_outcome(&thread("completed"), Some("turn-1")),
            RecoveryTurnOutcome::Terminal
        );
        assert_eq!(
            recovery_turn_outcome(&thread("inProgress"), Some("turn-1")),
            RecoveryTurnOutcome::Running
        );
        assert_eq!(
            recovery_turn_outcome(&thread("completed"), Some("turn-other")),
            RecoveryTurnOutcome::Unknown
        );
        assert_eq!(
            recovery_turn_outcome(&thread("completed"), None),
            RecoveryTurnOutcome::Unknown
        );

        let mut state = RuntimeBridgeState {
            status: RuntimeBridgeStatus::Failed,
            recovery_attempt_count: MAX_AUTO_RECOVERY_ATTEMPTS - 1,
            ..RuntimeBridgeState::default()
        };
        assert!(auto_recovery_allowed(&state));
        state.recovery_attempt_count = MAX_AUTO_RECOVERY_ATTEMPTS;
        assert!(!auto_recovery_allowed(&state));
        state.status = RuntimeBridgeStatus::Stopped;
        state.recovery_attempt_count = 0;
        assert!(!auto_recovery_allowed(&state));
    }

    #[test]
    fn unavailable_schema_fails_closed_for_managed_sessions_and_agent_execution() {
        let root = std::env::temp_dir().join(format!("cas-runtime-schema-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let bridge =
            RuntimeBridgeService::open(&root.join("cas.db"), &root, &root.join("cas-helper.exe"))
                .unwrap();
        {
            let mut state = bridge.state().unwrap();
            state.managed_session_capability = SchemaCapability::Unavailable;
            state.agent_execution_capability = SchemaCapability::Unavailable;
        }

        assert!(matches!(
            bridge.ensure_managed_session_supported(),
            Err(RuntimeBridgeError::ManagedSessionUnsupported(
                SchemaCapability::Unavailable
            ))
        ));
        assert!(matches!(
            bridge.ensure_agent_execution_supported(),
            Err(RuntimeBridgeError::AgentExecutionUnsupported(
                SchemaCapability::Unavailable
            ))
        ));

        drop(bridge);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn startup_failure_does_not_leave_bridge_starting() {
        let root = std::env::temp_dir().join(format!("cas-runtime-start-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let bridge =
            RuntimeBridgeService::open(&root.join("cas.db"), &root, &root.join("cas-helper.exe"))
                .unwrap();
        let missing = root.join("missing-codex.exe");

        assert!(matches!(
            bridge.start_inner(&missing, &root, Some("missing".to_owned())),
            Err(RuntimeBridgeError::Spawn(_))
        ));
        let status = bridge.status_inner().unwrap();
        assert_eq!(status.status, RuntimeBridgeStatus::Failed);
        assert!(status.last_error.is_some());
        assert!(bridge.launch().unwrap().is_none());

        drop(bridge);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn consecutive_recovery_failures_stop_at_the_retry_ceiling() {
        let root = std::env::temp_dir().join(format!("cas-runtime-storm-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let bridge =
            RuntimeBridgeService::open(&root.join("cas.db"), &root, &root.join("cas-helper.exe"))
                .unwrap();
        *bridge.launch().unwrap() = Some(RuntimeBridgeLaunch {
            executable: root.join("missing-codex.exe"),
            codex_home: root.clone(),
            codex_version: Some("missing".to_owned()),
        });
        bridge.state().unwrap().status = RuntimeBridgeStatus::Failed;

        for expected_attempt in 1..=MAX_AUTO_RECOVERY_ATTEMPTS {
            assert!(matches!(
                bridge.recover_inner(false),
                Err(RuntimeBridgeError::Spawn(_))
            ));
            let status = bridge.status_inner().unwrap();
            assert_eq!(status.status, RuntimeBridgeStatus::Failed);
            assert_eq!(status.recovery_attempt_count, expected_attempt);
        }
        let exhausted = bridge.recover_inner(false).unwrap();
        assert_eq!(exhausted.status, RuntimeBridgeStatus::Failed);
        assert_eq!(exhausted.recovery_attempt_count, MAX_AUTO_RECOVERY_ATTEMPTS);
        assert!(exhausted.auto_recovery_exhausted);

        drop(bridge);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_turn_preserves_app_server_error_message() {
        let event = parse_event(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "status": "failed",
                    "error": {"message": "provider rejected tool output"}
                }
            }
        }))
        .unwrap()
        .unwrap();
        let NormalizedRuntimeEvent::TurnFinished {
            successful,
            failure_message,
            ..
        } = event
        else {
            panic!("expected turn completion event");
        };
        assert!(!successful);
        assert_eq!(
            failure_message.as_deref(),
            Some("provider rejected tool output")
        );
    }

    #[test]
    fn parses_current_usage_and_ignores_future_fields() {
        let event = parse_event(&json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "child-1",
                "turnId": "turn-1",
                "tokenUsage": {
                    "total": {
                        "inputTokens": 100,
                        "cachedInputTokens": 80,
                        "cacheWriteInputTokens": 4,
                        "outputTokens": 20,
                        "reasoningOutputTokens": 5,
                        "totalTokens": 120,
                        "futureField": "ignored"
                    },
                    "last": {},
                    "modelContextWindow": 1000000,
                    "futureEnvelope": true
                }
            }
        }))
        .unwrap()
        .unwrap();
        let NormalizedRuntimeEvent::Usage { usage, profile, .. } = event else {
            panic!("expected usage event");
        };
        assert_eq!(profile, ProtocolProfile::Modern);
        assert_eq!(usage.total_tokens, 120);
        assert_eq!(usage.current_context_tokens, None);
        assert_eq!(usage.cache_write_input_tokens, 4);
        assert!(!usage.partial);
    }

    #[test]
    fn parses_legacy_snake_case_usage_as_partial_without_fabricating_fields() {
        let event = parse_event(&json!({
            "method": "codex/event/token_count",
            "params": {
                "conversationId": "legacy-thread",
                "msg": {
                    "info": {
                        "total_token_usage": {
                            "input_tokens": 30,
                            "cached_input_tokens": 10,
                            "output_tokens": 5,
                            "total_tokens": 35
                        },
                        "last_token_usage": {
                            "total_tokens": 12
                        },
                        "model_context_window": 200000
                    }
                }
            }
        }))
        .unwrap()
        .unwrap();
        let NormalizedRuntimeEvent::Usage { usage, profile, .. } = event else {
            panic!("expected usage event");
        };
        assert_eq!(profile, ProtocolProfile::Legacy);
        assert_eq!(usage.total_tokens, 35);
        assert_eq!(usage.current_context_tokens, Some(12));
        assert_eq!(usage.reasoning_output_tokens, 0);
        assert!(usage.partial);
    }

    #[test]
    fn keeps_cumulative_and_current_context_usage_separate() {
        let event = parse_event(&json!({
            "method": "thread/tokenUsage/updated",
            "params": {
                "threadId": "thread-1",
                "tokenUsage": {
                    "totalTokenUsage": {
                        "inputTokens": 1_667_247,
                        "cachedInputTokens": 0,
                        "outputTokens": 1,
                        "reasoningOutputTokens": 0,
                        "totalTokens": 1_667_248
                    },
                    "lastTokenUsage": {"totalTokens": 50_000},
                    "modelContextWindow": 258_400,
                    "future": true
                }
            }
        }))
        .unwrap()
        .unwrap();
        let NormalizedRuntimeEvent::Usage { usage, .. } = event else {
            panic!("expected usage event");
        };
        assert_eq!(usage.total_tokens, 1_667_248);
        assert_eq!(usage.current_context_tokens, Some(50_000));
        assert_eq!(usage.model_context_window, Some(258_400));
    }

    #[test]
    fn parses_both_collaboration_item_names_and_agent_path() {
        for item_type in ["collabAgentToolCall", "collabToolCall"] {
            let event = parse_event(&json!({
                "method": "item/completed",
                "params": {
                    "item": {
                        "type": item_type,
                        "senderThreadId": "root",
                        "receiverThreadIds": ["child"],
                        "model": "deepseek-v4-flash"
                    }
                }
            }))
            .unwrap()
            .unwrap();
            assert!(matches!(event, NormalizedRuntimeEvent::ParentChild { .. }));
        }

        let event = parse_event(&json!({
            "method": "item/started",
            "params": {
                "item": {
                    "type": "subAgentActivity",
                    "agentThreadId": "child",
                    "agentPath": "C:\\Users\\test\\.codex\\agents\\cas-executor.toml"
                }
            }
        }))
        .unwrap()
        .unwrap();
        assert!(matches!(
            event,
            NormalizedRuntimeEvent::AgentPath { agent_key, .. } if agent_key == "executor"
        ));
    }

    #[test]
    fn malformed_recognized_usage_is_rejected_but_unknown_events_are_ignored() {
        assert!(
            parse_event(&json!({
                "method": "thread/tokenUsage/updated",
                "params": {"threadId": "thread", "tokenUsage": {"total": {}}}
            }))
            .is_err()
        );
        assert!(
            parse_event(&json!({
                "method": "future/event",
                "params": {"anything": true}
            }))
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn observer_persists_child_usage_with_root_session_identity() {
        let usage = Arc::new(UsageService::in_memory());
        let mut observer = RuntimeObserver::new(Arc::clone(&usage));
        observer
            .observe(NormalizedRuntimeEvent::ThreadStarted {
                thread_id: "root".to_owned(),
                session_id: Some("root".to_owned()),
                parent_thread_id: None,
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(NormalizedRuntimeEvent::ParentChild {
                parent_thread_id: "root".to_owned(),
                child_thread_ids: vec!["child".to_owned()],
                model_slug: Some("deepseek-v4-flash".to_owned()),
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(NormalizedRuntimeEvent::Usage {
                thread_id: "child".to_owned(),
                usage: NormalizedUsage {
                    cached_input_provided: true,
                    input_tokens: 100,
                    cached_input_tokens: 80,
                    cache_write_input_tokens: 0,
                    output_tokens: 20,
                    reasoning_output_tokens: 5,
                    total_tokens: 120,
                    current_context_tokens: Some(100),
                    model_context_window: Some(1_000_000),
                    partial: false,
                },
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(NormalizedRuntimeEvent::TurnFinished {
                thread_id: "child".to_owned(),
                turn_id: "turn-1".to_owned(),
                successful: true,
                failure_message: None,
                profile: ProtocolProfile::Modern,
            })
            .unwrap();

        let records = usage.list(UsageListRequest::default()).unwrap();
        let records = serde_json::to_value(records).unwrap();
        assert_eq!(records[0]["codexSessionId"], "root");
        assert_eq!(records[0]["parentThreadId"], "root");
        assert_eq!(records[0]["usageStatus"], "FINAL");
        assert_eq!(records[0]["totalTokens"], 120);
        assert_eq!(records[0]["executionKind"], "OBSERVED_EXTERNAL");
    }

    #[test]
    fn observer_preserves_authoritative_native_child_binding() {
        let root = std::env::temp_dir().join(format!("cas-native-usage-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database_path = root.join("cas.db");
        let usage = Arc::new(UsageService::open(&database_path).unwrap());
        seed_dispatch_agent(&database_path);
        let connection = rusqlite::Connection::open(&database_path).unwrap();
        connection
            .execute(
                "INSERT INTO agent_thread_instances (
                    id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                    status, created_at, last_used_at, execution_kind
                 ) VALUES (
                    'native-child-instance', 'agent-dispatch', 'Executor', 'native-child',
                    'root', 'RUNNING', '2026-09-13T00:00:00Z',
                    '2026-09-13T00:00:00Z', 'NATIVE_CHILD'
                 )",
                [],
            )
            .unwrap();
        drop(connection);

        let mut observer = RuntimeObserver::new(Arc::clone(&usage));
        observer
            .observe(NormalizedRuntimeEvent::ParentChild {
                parent_thread_id: "root".to_owned(),
                child_thread_ids: vec!["native-child".to_owned()],
                model_slug: Some("gpt-test".to_owned()),
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(NormalizedRuntimeEvent::Usage {
                thread_id: "native-child".to_owned(),
                usage: NormalizedUsage {
                    cached_input_provided: true,
                    input_tokens: 100,
                    cached_input_tokens: 80,
                    cache_write_input_tokens: 0,
                    output_tokens: 20,
                    reasoning_output_tokens: 5,
                    total_tokens: 120,
                    current_context_tokens: Some(100),
                    model_context_window: Some(1_000_000),
                    partial: false,
                },
                profile: ProtocolProfile::Modern,
            })
            .unwrap();

        let records =
            serde_json::to_value(usage.list(UsageListRequest::default()).unwrap()).unwrap();
        assert_eq!(records[0]["codexThreadId"], "native-child");
        assert_eq!(records[0]["executionKind"], "NATIVE_CHILD");
        drop(observer);
        drop(usage);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_thread_started_is_not_upgraded_by_parent_child_evidence() {
        let usage = Arc::new(UsageService::in_memory());
        let mut observer = RuntimeObserver::new(Arc::clone(&usage));
        observer
            .observe(NormalizedRuntimeEvent::ThreadStarted {
                thread_id: "managed-thread".to_owned(),
                session_id: Some("managed-thread".to_owned()),
                parent_thread_id: None,
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(NormalizedRuntimeEvent::Usage {
                thread_id: "managed-thread".to_owned(),
                usage: NormalizedUsage {
                    cached_input_provided: true,
                    input_tokens: 10,
                    cached_input_tokens: 0,
                    cache_write_input_tokens: 0,
                    output_tokens: 5,
                    reasoning_output_tokens: 0,
                    total_tokens: 15,
                    current_context_tokens: Some(15),
                    model_context_window: Some(128_000),
                    partial: false,
                },
                profile: ProtocolProfile::Modern,
            })
            .unwrap();

        assert!(matches!(
            observer.observe(NormalizedRuntimeEvent::ParentChild {
                parent_thread_id: "parent".to_owned(),
                child_thread_ids: vec!["managed-thread".to_owned()],
                model_slug: None,
                profile: ProtocolProfile::Modern,
            }),
            Err(RuntimeBridgeError::ExecutionKindConflict)
        ));
        assert_eq!(
            observer.threads["managed-thread"].execution_kind,
            Some(ExecutionKind::ManagedWorker)
        );
        assert!(observer.threads["managed-thread"].identity_known);
        assert_eq!(
            observer.threads["managed-thread"].session_id.as_deref(),
            Some("managed-thread")
        );
        assert_eq!(observer.threads["managed-thread"].parent_thread_id, None);
        let records =
            serde_json::to_value(usage.list(UsageListRequest::default()).unwrap()).unwrap();
        assert_eq!(records.as_array().unwrap().len(), 1);
        assert!(records[0]["parentThreadId"].is_null());
        assert_eq!(records[0]["executionKind"], "MANAGED_WORKER");
    }

    #[test]
    #[ignore = "requires CAS_E2E_CODEX_HOME and a configured Responses provider"]
    fn managed_session_real_e2e_persists_subagent_usage() {
        let executable =
            std::env::var("CAS_E2E_CODEX_EXECUTABLE").unwrap_or_else(|_| "codex".to_owned());
        let codex_home =
            std::env::var("CAS_E2E_CODEX_HOME").expect("CAS_E2E_CODEX_HOME is required");
        let cwd = std::env::var("CAS_E2E_CWD").expect("CAS_E2E_CWD is required");
        let prompt = std::env::var("CAS_E2E_PROMPT").unwrap_or_else(|_| {
            "必须调用 executor 子 Agent 完成任务。让它只读取当前工作目录 package.json 的 name 字段，不得修改文件；等待它完成后，只输出 EXECUTOR_OK:<name>。".to_owned()
        });
        let test_root = std::env::temp_dir().join(format!("cas-runtime-e2e-{}", Uuid::new_v4()));
        fs::create_dir_all(&test_root).unwrap();
        let database_path = test_root.join("cas.db");
        let bridge = RuntimeBridgeService::open(
            &database_path,
            &test_root,
            &test_root.join("cas-helper.exe"),
        )
        .unwrap();

        bridge
            .start_inner(Path::new(&executable), Path::new(&codex_home), None)
            .unwrap();
        let session = bridge
            .managed_session_start_inner(ManagedSessionStartRequest {
                cwd,
                approval_policy: None,
                sandbox: None,
            })
            .unwrap();
        bridge
            .managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: session.thread_id,
                input: prompt,
                effort: None,
                approval_policy: None,
                sandbox_policy: None,
            })
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(70);
        let outcome = loop {
            let status = bridge.status_inner().unwrap();
            let session = status.managed_session.expect("managed session exists");
            if session.status != ManagedSessionStatus::Running {
                break if session.status == ManagedSessionStatus::Idle {
                    Ok(())
                } else {
                    Err(format!("managed turn ended as {:?}", session.status))
                };
            }
            if Instant::now() >= deadline {
                break Err(format!(
                    "managed turn timed out; last_event_at={:?}, usage_events={}",
                    status.last_event_at, status.usage_event_count
                ));
            }
            thread::sleep(Duration::from_millis(100));
        };
        let records =
            serde_json::to_value(bridge.usage.list(UsageListRequest::default()).unwrap()).unwrap();
        let usage_persisted = records.as_array().is_some_and(|records| {
            !records.is_empty()
                && records
                    .iter()
                    .any(|record| !record["parentThreadId"].is_null())
        });
        let _ = bridge.stop_inner();
        drop(bridge);
        let _ = fs::remove_dir_all(test_root);
        outcome.unwrap();
        assert!(usage_persisted, "no subagent usage record persisted");
    }
}

#[cfg(test)]
mod rc_e2e;

#[cfg(test)]
#[path = "runtime_bridge/session_registry_tests.rs"]
mod session_registry_tests;
