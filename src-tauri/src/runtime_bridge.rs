use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::provider::ApiError;
use crate::usage::{
    AgentRuntimeProfile, AgentThreadExecutionPlan, AgentThreadExecutionRequest, UsageAttribution,
    UsageService, UsageServiceError, UsageSnapshot,
};

const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);
const PROTOCOL_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const SCHEMA_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct RuntimeBridgeService {
    data_home: PathBuf,
    usage: Arc<UsageService>,
    state: Arc<Mutex<RuntimeBridgeState>>,
    worker: Mutex<Option<BridgeWorker>>,
    managed_operation: Mutex<()>,
}

impl RuntimeBridgeService {
    pub(crate) fn open(database_path: &Path, data_home: &Path) -> Result<Self, RuntimeBridgeError> {
        Ok(Self {
            data_home: data_home.to_path_buf(),
            usage: Arc::new(UsageService::open(database_path)?),
            state: Arc::new(Mutex::new(RuntimeBridgeState::default())),
            worker: Mutex::new(None),
            managed_operation: Mutex::new(()),
        })
    }

    pub(crate) fn status(&self) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        self.status_inner().map_err(ApiError::from)
    }

    pub(crate) fn start(
        &self,
        executable: &Path,
        codex_home: &Path,
        codex_version: Option<String>,
    ) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        self.start_inner(executable, codex_home, codex_version)
            .map_err(ApiError::from)
    }

    pub(crate) fn stop(&self) -> Result<RuntimeBridgeStatusResponse, ApiError> {
        self.stop_inner().map_err(ApiError::from)
    }

    pub(crate) fn managed_session_start(
        &self,
        request: ManagedSessionStartRequest,
    ) -> Result<ManagedSessionResponse, ApiError> {
        self.managed_session_start_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn managed_session_resume(
        &self,
        request: ManagedSessionResumeRequest,
    ) -> Result<ManagedSessionResponse, ApiError> {
        self.managed_session_resume_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn managed_turn_start(
        &self,
        request: ManagedTurnStartRequest,
    ) -> Result<ManagedTurnStartResponse, ApiError> {
        self.managed_turn_start_inner(request)
            .map_err(ApiError::from)
    }

    pub(crate) fn execute_agent_thread(
        &self,
        request: AgentThreadExecutionRequest,
    ) -> Result<AgentThreadExecutionResponse, ApiError> {
        let _operation = self
            .managed_operation
            .lock()
            .map_err(|_| ApiError::from(RuntimeBridgeError::StateUnavailable))?;
        let plan = self.usage.prepare_agent_execution(request)?;
        self.execute_agent_thread_inner(plan)
            .map_err(ApiError::from)
    }

    fn start_inner(
        &self,
        executable: &Path,
        codex_home: &Path,
        codex_version: Option<String>,
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
        let schema_capabilities =
            probe_schema_capabilities(executable, &self.data_home).unwrap_or_default();
        {
            let mut state = self.state()?;
            let mut managed_session = state.managed_session.clone();
            if let Some(session) = managed_session.as_mut()
                && session.status == ManagedSessionStatus::Running
            {
                session.status = ManagedSessionStatus::RecoveryRequired;
                session.active_turn_id = None;
            }
            *state = RuntimeBridgeState {
                status: RuntimeBridgeStatus::Starting,
                schema_capability: schema_capabilities.usage,
                managed_session_capability: schema_capabilities.managed_session,
                agent_execution_capability: schema_capabilities.agent_execution,
                codex_version,
                started_at: Some(started_at),
                managed_session,
                ..RuntimeBridgeState::default()
            };
        }

        let mut child = Command::new(executable)
            .args(["app-server", "--listen", "stdio://"])
            .env("CODEX_HOME", codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(RuntimeBridgeError::Spawn)?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or(RuntimeBridgeError::MissingPipe("stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or(RuntimeBridgeError::MissingPipe("stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or(RuntimeBridgeError::MissingPipe("stderr"))?;
        let child = Arc::new(Mutex::new(child));
        let stopping = Arc::new(AtomicBool::new(false));
        let (initialize_tx, initialize_rx) = mpsc::sync_channel(1);
        let pending_responses = Arc::new(Mutex::new(HashMap::new()));

        let state = Arc::clone(&self.state);
        let usage = Arc::clone(&self.usage);
        let reader_stopping = Arc::clone(&stopping);
        let reader_pending_responses = Arc::clone(&pending_responses);
        let stdout_thread = thread::spawn(move || {
            read_app_server_stream(
                stdout,
                state,
                usage,
                reader_stopping,
                initialize_tx,
                reader_pending_responses,
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
        if let Err(error) = write_message(&mut stdin, &initialize) {
            cleanup_unstarted_worker(child, stopping, stdout_thread, stderr_thread);
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
            write_message(&mut stdin, &json!({"method": "initialized", "params": {}}))
        {
            cleanup_unstarted_worker(child, stopping, stdout_thread, stderr_thread);
            self.set_failed(error.to_string());
            return Err(error);
        }

        *worker_slot = Some(BridgeWorker {
            child,
            stdin: Some(stdin),
            stopping,
            stdout_thread: Some(stdout_thread),
            stderr_thread: Some(stderr_thread),
            pending_responses,
            next_request_id: 2,
        });
        self.state()?.status = RuntimeBridgeStatus::Running;
        self.status_inner()
    }

    fn stop_inner(&self) -> Result<RuntimeBridgeStatusResponse, RuntimeBridgeError> {
        let worker = self.worker()?.take();
        if let Some(mut worker) = worker {
            worker.stop()?;
        }
        let mut state = self.state()?;
        state.status = RuntimeBridgeStatus::Stopped;
        state.last_error = None;
        if let Some(session) = state.managed_session.as_mut() {
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
        let result = self.request("thread/start", json!({ "cwd": cwd }))?;
        let (thread_id, session_id) = parse_managed_thread(&result)?;
        self.bind_managed_session(
            thread_id,
            session_id,
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
        let mut params =
            serde_json::Map::from_iter([("threadId".to_owned(), Value::String(thread_id.clone()))]);
        if let Some(cwd) = cwd.as_ref() {
            params.insert("cwd".to_owned(), Value::String(cwd.clone()));
        }
        let result = self.request("thread/resume", Value::Object(params))?;
        let (response_thread_id, session_id) = parse_managed_thread(&result)?;
        if response_thread_id != thread_id {
            return Err(RuntimeBridgeError::UnexpectedThreadResponse);
        }
        let session = self.bind_managed_session(
            response_thread_id,
            session_id,
            ManagedSessionOrigin::Resumed,
            cwd,
        )?;
        self.usage.mark_agent_execution_idle_if_known(&thread_id)?;
        Ok(session)
    }

    fn managed_turn_start_inner(
        &self,
        request: ManagedTurnStartRequest,
    ) -> Result<ManagedTurnStartResponse, RuntimeBridgeError> {
        let thread_id = validate_thread_id(&request.thread_id)?;
        let input = validate_turn_input(&request.input)?;
        {
            let mut state = self.state()?;
            let Some(session) = state.managed_session.as_mut() else {
                return Err(RuntimeBridgeError::ThreadNotBound);
            };
            if session.thread_id != thread_id {
                return Err(RuntimeBridgeError::ThreadNotBound);
            }
            if session.status == ManagedSessionStatus::RecoveryRequired {
                return Err(RuntimeBridgeError::SessionRecoveryRequired);
            }
            if session.status == ManagedSessionStatus::Running {
                return Err(RuntimeBridgeError::TurnAlreadyRunning);
            }
            session.status = ManagedSessionStatus::Running;
            session.active_turn_id = None;
        }
        let result = self.request(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": input }],
                "effort": request.effort,
            }),
        );
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                if let Ok(mut state) = self.state()
                    && let Some(session) = state.managed_session.as_mut()
                {
                    session.status = ManagedSessionStatus::RecoveryRequired;
                    session.active_turn_id = None;
                }
                return Err(error);
            }
        };
        let turn = find_object(&result, &["turn"]).unwrap_or(&result);
        let Some(turn_id) = find_string(turn, &["id", "turnId", "turn_id"]) else {
            if let Ok(mut state) = self.state()
                && let Some(session) = state.managed_session.as_mut()
            {
                session.status = ManagedSessionStatus::RecoveryRequired;
                session.active_turn_id = None;
            }
            return Err(RuntimeBridgeError::InvalidProtocolResponse("turn.id"));
        };
        let mut state = self.state()?;
        let Some(session) = state.managed_session.as_mut() else {
            return Err(RuntimeBridgeError::ThreadNotBound);
        };
        if session.status == ManagedSessionStatus::Running {
            session.active_turn_id = Some(turn_id.clone());
        }
        Ok(ManagedTurnStartResponse {
            thread_id,
            turn_id,
            status: session.status,
        })
    }

    fn execute_agent_thread_inner(
        &self,
        plan: AgentThreadExecutionPlan,
    ) -> Result<AgentThreadExecutionResponse, RuntimeBridgeError> {
        let cwd = validate_cwd(&plan.cwd)?;
        let input = validate_turn_input(&plan.input)?;
        self.ensure_agent_execution_supported()?;
        if self
            .state()?
            .managed_session
            .as_ref()
            .is_some_and(|session| session.status == ManagedSessionStatus::Running)
        {
            return Err(RuntimeBridgeError::TurnAlreadyRunning);
        }
        let (thread_id, session_id, origin, action) = match plan.recommendation.decision {
            "REUSE" => {
                let thread_id = plan
                    .recommendation
                    .candidate_thread_id
                    .clone()
                    .ok_or(RuntimeBridgeError::UnexpectedThreadResponse)?;
                let mut params = agent_thread_params(&plan.profile, &cwd);
                params
                    .as_object_mut()
                    .expect("agent thread params are an object")
                    .insert("threadId".to_owned(), Value::String(thread_id.clone()));
                let result = self.request("thread/resume", params)?;
                let (response_thread_id, session_id) = parse_managed_thread(&result)?;
                if response_thread_id != thread_id {
                    return Err(RuntimeBridgeError::UnexpectedThreadResponse);
                }
                (
                    response_thread_id,
                    session_id,
                    ManagedSessionOrigin::Resumed,
                    AgentThreadExecutionAction::Reused,
                )
            }
            "SPAWN" => {
                let result =
                    self.request("thread/start", agent_thread_params(&plan.profile, &cwd))?;
                let (thread_id, session_id) = parse_managed_thread(&result)?;
                (
                    thread_id,
                    session_id,
                    ManagedSessionOrigin::Started,
                    AgentThreadExecutionAction::Spawned,
                )
            }
            _ => return Err(RuntimeBridgeError::UnexpectedThreadResponse),
        };
        self.bind_managed_session(thread_id.clone(), session_id, origin, Some(cwd.clone()))?;
        self.usage.register_agent_execution_thread(
            &plan.profile,
            &thread_id,
            &plan.workspace_scope_key,
        )?;
        let turn = self.managed_turn_start_inner(ManagedTurnStartRequest {
            thread_id: thread_id.clone(),
            input,
            effort: reasoning_effort(&plan.profile).map(str::to_owned),
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
        self.usage.mark_agent_execution_running(&thread_id)?;
        Ok(AgentThreadExecutionResponse {
            action,
            decision: plan.recommendation.decision,
            reason_code: plan.recommendation.reason_code,
            agent_id: plan.profile.agent_id,
            agent_name: plan.profile.agent_name,
            workspace_scope_key: plan.workspace_scope_key,
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
            SchemaCapability::NotDeclared | SchemaCapability::Incompatible => {
                Err(RuntimeBridgeError::ManagedSessionUnsupported)
            }
            SchemaCapability::Supported | SchemaCapability::Unavailable => Ok(()),
        }
    }

    fn ensure_agent_execution_supported(&self) -> Result<(), RuntimeBridgeError> {
        match self.state()?.agent_execution_capability {
            SchemaCapability::NotDeclared | SchemaCapability::Incompatible => {
                Err(RuntimeBridgeError::AgentExecutionUnsupported)
            }
            SchemaCapability::Supported | SchemaCapability::Unavailable => Ok(()),
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
        self.state()?.managed_session = Some(session.clone());
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
    stdin: Option<ChildStdin>,
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
        let Some(stdin) = self.stdin.as_mut() else {
            remove_pending_response(&self.pending_responses, request_id);
            return Err(RuntimeBridgeError::NotRunning);
        };
        if let Err(error) = write_message(stdin, &message) {
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
        self.stdin.take();
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

fn read_app_server_stream(
    stdout: impl std::io::Read,
    state: Arc<Mutex<RuntimeBridgeState>>,
    usage: Arc<UsageService>,
    stopping: Arc<AtomicBool>,
    initialize_tx: mpsc::SyncSender<Result<(), RuntimeBridgeError>>,
    pending_responses: Arc<Mutex<HashMap<i64, PendingResponse>>>,
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

        if message.get("id").and_then(Value::as_i64) == Some(1) {
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

        if resolve_pending_response(&message, &pending_responses) {
            continue;
        }

        match parse_bridge_event(&message) {
            Ok(Some(event)) => {
                update_managed_session_from_event(&state, &event);
                let profile = event.profile();
                if let Err(error) = observer.observe(event) {
                    mark_stream_failure(&state, error.to_string());
                    continue;
                }
                if let Ok(mut state) = state.lock() {
                    state.last_event_at = observer.last_event_at.clone();
                    state.status = RuntimeBridgeStatus::Running;
                    state.last_error = None;
                    if profile.is_usage() {
                        if profile == ProtocolProfile::UsageLegacy {
                            state.protocol_compatibility = ProtocolCompatibility::LegacyCompatible;
                        } else if state.protocol_compatibility
                            != ProtocolCompatibility::LegacyCompatible
                        {
                            state.protocol_compatibility = ProtocolCompatibility::Compatible;
                        }
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
        if let Some(session) = state.managed_session.as_mut() {
            session.status = ManagedSessionStatus::RecoveryRequired;
            session.active_turn_id = None;
        }
    }
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
        return true;
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

fn update_managed_session_from_event(state: &Arc<Mutex<RuntimeBridgeState>>, event: &BridgeEvent) {
    let Ok(mut state) = state.lock() else {
        return;
    };
    let Some(session) = state.managed_session.as_mut() else {
        return;
    };
    match event {
        BridgeEvent::ThreadStarted {
            thread_id,
            session_id,
            ..
        } if thread_id == &session.thread_id => {
            if session.session_id.is_none() {
                session.session_id = session_id.clone();
            }
        }
        BridgeEvent::TurnFinished {
            thread_id,
            successful,
            ..
        } if thread_id == &session.thread_id => {
            session.status = if *successful {
                ManagedSessionStatus::Idle
            } else {
                ManagedSessionStatus::Failed
            };
            session.active_turn_id = None;
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum RuntimeBridgeStatus {
    Stopped,
    Starting,
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum SchemaCapability {
    Supported,
    NotDeclared,
    Incompatible,
    #[default]
    Unavailable,
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
    managed_session: Option<ManagedSessionState>,
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
            managed_session: None,
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
    managed_session: Option<ManagedSessionResponse>,
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
            managed_session: state
                .managed_session
                .as_ref()
                .map(ManagedSessionResponse::from),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSessionStartRequest {
    cwd: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedSessionResumeRequest {
    thread_id: String,
    cwd: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedTurnStartRequest {
    thread_id: String,
    input: String,
    #[serde(default)]
    effort: Option<String>,
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
    decision: &'static str,
    reason_code: &'static str,
    agent_id: String,
    agent_name: String,
    workspace_scope_key: String,
    thread_id: String,
    turn_id: String,
    status: ManagedSessionStatus,
}

#[derive(Debug, Clone, Copy, Default)]
struct SchemaCapabilities {
    usage: SchemaCapability,
    managed_session: SchemaCapability,
    agent_execution: SchemaCapability,
}

fn probe_schema_capabilities(
    executable: &Path,
    data_home: &Path,
) -> Result<SchemaCapabilities, RuntimeBridgeError> {
    let probe_root = data_home
        .join("runtime-schema-probes")
        .join(Uuid::new_v4().to_string());
    fs::create_dir_all(&probe_root).map_err(RuntimeBridgeError::SchemaProbe)?;
    let result = (|| {
        let mut child = Command::new(executable)
            .args(["app-server", "generate-json-schema", "--out"])
            .arg(&probe_root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(RuntimeBridgeError::SchemaProbe)?;
        let deadline = Instant::now() + SCHEMA_PROBE_TIMEOUT;
        let succeeded = loop {
            match child.try_wait().map_err(RuntimeBridgeError::SchemaProbe)? {
                Some(status) => break status.success(),
                None if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                None => thread::sleep(Duration::from_millis(25)),
            }
        };
        if !succeeded {
            return Ok(SchemaCapabilities::default());
        }
        let usage = if find_schema_file(&probe_root, "ThreadTokenUsageUpdatedNotification.json")
            .is_some()
        {
            SchemaCapability::Supported
        } else {
            SchemaCapability::NotDeclared
        };
        let managed_session = managed_session_schema_capability(&probe_root);
        let agent_execution = agent_execution_schema_capability(&probe_root);
        Ok(SchemaCapabilities {
            usage,
            managed_session,
            agent_execution,
        })
    })();
    let _ = fs::remove_dir_all(&probe_root);
    result
}

fn managed_session_schema_capability(probe_root: &Path) -> SchemaCapability {
    let schemas = [
        ("ThreadStartParams.json", &[][..]),
        ("ThreadResumeParams.json", &["threadId"][..]),
        ("TurnStartParams.json", &["input", "threadId"][..]),
    ];
    let mut loaded = Vec::with_capacity(schemas.len());
    for (name, supported_required) in schemas {
        let Some(path) = find_schema_file(probe_root, name) else {
            return SchemaCapability::NotDeclared;
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return SchemaCapability::Incompatible;
        };
        let Ok(schema) = serde_json::from_str::<Value>(&contents) else {
            return SchemaCapability::Incompatible;
        };
        loaded.push((schema, supported_required));
    }
    if loaded
        .iter()
        .all(|(schema, supported)| schema_requires_only(schema, supported))
    {
        SchemaCapability::Supported
    } else {
        SchemaCapability::Incompatible
    }
}

fn agent_execution_schema_capability(probe_root: &Path) -> SchemaCapability {
    let managed_capability = managed_session_schema_capability(probe_root);
    if managed_capability != SchemaCapability::Supported {
        return managed_capability;
    }

    let schemas = [
        (
            "ThreadStartParams.json",
            &[
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
            ][..],
        ),
        (
            "ThreadResumeParams.json",
            &[
                "threadId",
                "cwd",
                "model",
                "modelProvider",
                "developerInstructions",
                "sandbox",
            ][..],
        ),
        ("TurnStartParams.json", &["threadId", "input", "effort"][..]),
    ];
    for (name, properties) in schemas {
        let Some(path) = find_schema_file(probe_root, name) else {
            return SchemaCapability::NotDeclared;
        };
        let Ok(contents) = fs::read_to_string(path) else {
            return SchemaCapability::Incompatible;
        };
        let Ok(schema) = serde_json::from_str::<Value>(&contents) else {
            return SchemaCapability::Incompatible;
        };
        if !properties.iter().all(|property| {
            schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|declared| declared.contains_key(*property))
        }) {
            return SchemaCapability::Incompatible;
        }
    }
    SchemaCapability::Supported
}

fn schema_requires_only(schema: &Value, supported_required: &[&str]) -> bool {
    schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .all(|field| {
            field
                .as_str()
                .is_some_and(|field| supported_required.contains(&field))
        })
}

fn find_schema_file(root: &Path, file_name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find_schema_file(&path, file_name)
        {
            return Some(found);
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolProfile {
    Modern,
    Legacy,
    UsageModern,
    UsageLegacy,
}

impl ProtocolProfile {
    fn is_usage(self) -> bool {
        matches!(self, Self::UsageModern | Self::UsageLegacy)
    }
}

#[derive(Debug, Clone)]
enum BridgeEvent {
    ThreadStarted {
        thread_id: String,
        session_id: Option<String>,
        parent_thread_id: Option<String>,
        profile: ProtocolProfile,
    },
    ParentChild {
        parent_thread_id: String,
        child_thread_ids: Vec<String>,
        model_slug: Option<String>,
        profile: ProtocolProfile,
    },
    AgentPath {
        thread_id: String,
        agent_key: String,
        profile: ProtocolProfile,
    },
    Usage {
        thread_id: String,
        usage: TokenUsageSnapshot,
        profile: ProtocolProfile,
    },
    TurnFinished {
        thread_id: String,
        successful: bool,
        profile: ProtocolProfile,
    },
}

impl BridgeEvent {
    fn profile(&self) -> ProtocolProfile {
        match self {
            Self::ThreadStarted { profile, .. }
            | Self::ParentChild { profile, .. }
            | Self::AgentPath { profile, .. }
            | Self::Usage { profile, .. }
            | Self::TurnFinished { profile, .. } => *profile,
        }
    }
}

#[derive(Debug, Clone)]
struct TokenUsageSnapshot {
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_input_tokens: i64,
    output_tokens: i64,
    reasoning_output_tokens: i64,
    total_tokens: i64,
    current_context_tokens: Option<i64>,
    model_context_window: Option<i64>,
    partial: bool,
}

fn parse_bridge_event(message: &Value) -> Result<Option<BridgeEvent>, ProtocolParseError> {
    let method = match message.get("method").and_then(Value::as_str) {
        Some(method) => method,
        None => return Ok(None),
    };
    let params = message.get("params").unwrap_or(&Value::Null);
    match method {
        "thread/tokenUsage/updated" => parse_usage_event(params, ProtocolProfile::UsageModern),
        "codex/event/token_count" | "codex/event/tokenCount" => {
            parse_usage_event(params, ProtocolProfile::UsageLegacy)
        }
        "thread/started" => parse_thread_started(params),
        "item/started" | "item/completed" => parse_item_event(params),
        "turn/completed" => parse_turn_finished(params),
        _ => Ok(None),
    }
}

fn parse_usage_event(
    params: &Value,
    method_profile: ProtocolProfile,
) -> Result<Option<BridgeEvent>, ProtocolParseError> {
    let thread_id = find_string(params, &["threadId", "thread_id", "conversationId"])
        .ok_or(ProtocolParseError::MissingField("threadId"))?;
    let envelope = find_object(params, &["tokenUsage", "token_usage", "info"]).unwrap_or(params);
    let total = find_object(envelope, &["total", "totalTokenUsage", "total_token_usage"])
        .unwrap_or(envelope);
    let (usage, used_legacy_fields) = parse_token_breakdown(total, envelope)?;
    let profile = if method_profile == ProtocolProfile::UsageLegacy || used_legacy_fields {
        ProtocolProfile::UsageLegacy
    } else {
        ProtocolProfile::UsageModern
    };
    Ok(Some(BridgeEvent::Usage {
        thread_id,
        usage,
        profile,
    }))
}

fn parse_token_breakdown(
    total: &Value,
    envelope: &Value,
) -> Result<(TokenUsageSnapshot, bool), ProtocolParseError> {
    let input = integer_alias(total, &["inputTokens", "input_tokens"]);
    let cached = integer_alias(total, &["cachedInputTokens", "cached_input_tokens"]);
    let cache_write = integer_alias(
        total,
        &["cacheWriteInputTokens", "cache_write_input_tokens"],
    );
    let output = integer_alias(total, &["outputTokens", "output_tokens"]);
    let reasoning = integer_alias(total, &["reasoningOutputTokens", "reasoning_output_tokens"]);
    let reported_total = integer_alias(total, &["totalTokens", "total_tokens"]);
    for value in [
        input,
        cached,
        cache_write,
        output,
        reasoning,
        reported_total,
    ]
    .into_iter()
    .flatten()
    {
        if value < 0 {
            return Err(ProtocolParseError::NegativeToken);
        }
    }
    if input.is_none() && output.is_none() && reported_total.is_none() {
        return Err(ProtocolParseError::MissingField("tokenUsage.total"));
    }
    let input_tokens = input.unwrap_or(0);
    let output_tokens = output.unwrap_or(0);
    let total_tokens = reported_total
        .or_else(|| input_tokens.checked_add(output_tokens))
        .ok_or(ProtocolParseError::TokenOverflow)?;
    let model_context_window =
        integer_alias(envelope, &["modelContextWindow", "model_context_window"])
            .filter(|value| *value > 0);
    let current_context_tokens =
        find_object(envelope, &["lastTokenUsage", "last_token_usage", "last"])
            .and_then(|usage| integer_alias(usage, &["totalTokens", "total_tokens"]))
            .filter(|value| *value >= 0);
    let partial = input.is_none()
        || cached.is_none()
        || output.is_none()
        || reasoning.is_none()
        || reported_total.is_none();
    let used_legacy_fields = has_any_key(
        total,
        &[
            "input_tokens",
            "cached_input_tokens",
            "cache_write_input_tokens",
            "output_tokens",
            "reasoning_output_tokens",
            "total_tokens",
        ],
    );
    Ok((
        TokenUsageSnapshot {
            input_tokens,
            cached_input_tokens: cached.unwrap_or(0),
            cache_write_input_tokens: cache_write.unwrap_or(0),
            output_tokens,
            reasoning_output_tokens: reasoning.unwrap_or(0),
            total_tokens,
            current_context_tokens,
            model_context_window,
            partial,
        },
        used_legacy_fields,
    ))
}

fn parse_thread_started(params: &Value) -> Result<Option<BridgeEvent>, ProtocolParseError> {
    let thread = find_object(params, &["thread"]).unwrap_or(params);
    let thread_id = find_string(thread, &["id", "threadId", "thread_id"])
        .ok_or(ProtocolParseError::MissingField("thread.id"))?;
    let session_id = find_string(thread, &["sessionId", "session_id"]);
    let parent_thread_id = find_string(thread, &["parentThreadId", "parent_thread_id"]);
    let profile = if has_any_key(thread, &["session_id", "parent_thread_id", "thread_id"]) {
        ProtocolProfile::Legacy
    } else {
        ProtocolProfile::Modern
    };
    Ok(Some(BridgeEvent::ThreadStarted {
        thread_id,
        session_id,
        parent_thread_id,
        profile,
    }))
}

fn parse_item_event(params: &Value) -> Result<Option<BridgeEvent>, ProtocolParseError> {
    let item = find_object(params, &["item"]).unwrap_or(params);
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
    if matches!(item_type, "collabAgentToolCall" | "collabToolCall") {
        let parent_thread_id = find_string(item, &["senderThreadId", "sender_thread_id"])
            .ok_or(ProtocolParseError::MissingField("senderThreadId"))?;
        let mut child_thread_ids =
            string_array_alias(item, &["receiverThreadIds", "receiver_thread_ids"]);
        if child_thread_ids.is_empty()
            && let Some(thread_id) = find_string(
                item,
                &[
                    "receiverThreadId",
                    "receiver_thread_id",
                    "newThreadId",
                    "new_thread_id",
                ],
            )
        {
            child_thread_ids.push(thread_id);
        }
        if child_thread_ids.is_empty() {
            return Ok(None);
        }
        let model_slug = find_string(item, &["model", "modelId", "model_id"]);
        let profile = if item_type == "collabToolCall"
            || has_any_key(item, &["sender_thread_id", "receiver_thread_ids"])
        {
            ProtocolProfile::Legacy
        } else {
            ProtocolProfile::Modern
        };
        return Ok(Some(BridgeEvent::ParentChild {
            parent_thread_id,
            child_thread_ids,
            model_slug,
            profile,
        }));
    }
    if item_type == "subAgentActivity" {
        let thread_id = find_string(item, &["agentThreadId", "agent_thread_id"])
            .ok_or(ProtocolParseError::MissingField("agentThreadId"))?;
        let agent_path = find_string(item, &["agentPath", "agent_path"])
            .ok_or(ProtocolParseError::MissingField("agentPath"))?;
        let agent_key = agent_key_from_path(&agent_path)
            .ok_or(ProtocolParseError::InvalidField("agentPath"))?;
        let profile = if has_any_key(item, &["agent_thread_id", "agent_path"]) {
            ProtocolProfile::Legacy
        } else {
            ProtocolProfile::Modern
        };
        return Ok(Some(BridgeEvent::AgentPath {
            thread_id,
            agent_key,
            profile,
        }));
    }
    Ok(None)
}

fn parse_turn_finished(params: &Value) -> Result<Option<BridgeEvent>, ProtocolParseError> {
    let thread_id = match find_string(params, &["threadId", "thread_id", "conversationId"]) {
        Some(thread_id) => thread_id,
        None => return Ok(None),
    };
    let status = params
        .pointer("/turn/status")
        .and_then(Value::as_str)
        .or_else(|| params.get("status").and_then(Value::as_str))
        .unwrap_or("completed");
    let profile = if has_any_key(params, &["thread_id", "conversationId"]) {
        ProtocolProfile::Legacy
    } else {
        ProtocolProfile::Modern
    };
    Ok(Some(BridgeEvent::TurnFinished {
        thread_id,
        successful: status == "completed",
        profile,
    }))
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

fn agent_thread_params(profile: &AgentRuntimeProfile, cwd: &str) -> Value {
    let mut params = json!({
        "cwd": cwd,
        "model": profile.model_slug,
        "developerInstructions": profile.instruction,
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

fn reasoning_effort(profile: &AgentRuntimeProfile) -> Option<&'static str> {
    match profile.reasoning_policy.as_str() {
        "LOW" => Some("low"),
        "MEDIUM" => Some("medium"),
        "HIGH" => Some("high"),
        _ => None,
    }
}

fn parse_managed_thread(result: &Value) -> Result<(String, Option<String>), RuntimeBridgeError> {
    let thread = find_object(result, &["thread"]).unwrap_or(result);
    let thread_id = find_string(thread, &["id", "threadId", "thread_id"])
        .ok_or(RuntimeBridgeError::InvalidProtocolResponse("thread.id"))?;
    let session_id = find_string(thread, &["sessionId", "session_id"]);
    Ok((thread_id, session_id))
}

fn find_object<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    if let Some(object) = value.as_object() {
        for key in keys {
            if let Some(candidate) = object.get(*key)
                && candidate.is_object()
            {
                return Some(candidate);
            }
        }
        for candidate in object.values() {
            if let Some(found) = find_object(candidate, keys) {
                return Some(found);
            }
        }
    } else if let Some(array) = value.as_array() {
        for candidate in array {
            if let Some(found) = find_object(candidate, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    if let Some(object) = value.as_object() {
        for key in keys {
            if let Some(candidate) = object.get(*key).and_then(Value::as_str) {
                return Some(candidate.to_owned());
            }
        }
        for candidate in object.values() {
            if let Some(found) = find_string(candidate, keys) {
                return Some(found);
            }
        }
    } else if let Some(array) = value.as_array() {
        for candidate in array {
            if let Some(found) = find_string(candidate, keys) {
                return Some(found);
            }
        }
    }
    None
}

fn integer_alias(value: &Value, keys: &[&str]) -> Option<i64> {
    let object = value.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_i64))
}

fn string_array_alias(value: &Value, keys: &[&str]) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_array))
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn has_any_key(value: &Value, keys: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| keys.iter().any(|key| object.contains_key(*key)))
}

fn agent_key_from_path(path: &str) -> Option<String> {
    let normalized = path.replace('\\', "/");
    let file_name = normalized.rsplit('/').next()?;
    let stem = file_name.strip_suffix(".toml").unwrap_or(file_name);
    stem.strip_prefix("cas-")
        .or_else(|| stem.strip_prefix("cas_"))
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
}

#[derive(Default)]
struct ObservedThread {
    identity_known: bool,
    session_id: Option<String>,
    parent_thread_id: Option<String>,
    agent_key: Option<String>,
    model_slug: Option<String>,
    started_at: Option<String>,
    latest_usage: Option<TokenUsageSnapshot>,
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

    fn observe(&mut self, event: BridgeEvent) -> Result<(), RuntimeBridgeError> {
        self.last_event_at = Some(self.usage.current_timestamp()?);
        match event {
            BridgeEvent::ThreadStarted {
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
                let thread = self.threads.entry(thread_id.clone()).or_default();
                thread.identity_known = true;
                thread.session_id = resolved_session_id;
                thread.parent_thread_id = parent_thread_id;
                self.persist(&thread_id, None)?;
            }
            BridgeEvent::ParentChild {
                parent_thread_id,
                child_thread_ids,
                model_slug,
                ..
            } => {
                let session_id = self.root_thread_id(&parent_thread_id);
                for child_thread_id in child_thread_ids {
                    let thread = self.threads.entry(child_thread_id.clone()).or_default();
                    thread.identity_known = true;
                    thread.session_id = Some(session_id.clone());
                    thread.parent_thread_id = Some(parent_thread_id.clone());
                    if thread.model_slug.is_none() {
                        thread.model_slug = model_slug.clone();
                    }
                    self.persist(&child_thread_id, None)?;
                }
            }
            BridgeEvent::AgentPath {
                thread_id,
                agent_key,
                ..
            } => {
                self.threads.entry(thread_id.clone()).or_default().agent_key = Some(agent_key);
                self.persist(&thread_id, None)?;
            }
            BridgeEvent::Usage {
                thread_id, usage, ..
            } => {
                let thread = self.threads.entry(thread_id.clone()).or_default();
                thread.latest_usage = Some(usage);
                if thread.started_at.is_none() {
                    thread.started_at = self.last_event_at.clone();
                }
                self.persist(&thread_id, None)?;
            }
            BridgeEvent::TurnFinished {
                thread_id,
                successful,
                ..
            } => {
                let thread = self.threads.entry(thread_id.clone()).or_default();
                if !thread.identity_known {
                    thread.identity_known = true;
                    thread.session_id = Some(thread_id.clone());
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
    usage: TokenUsageSnapshot,
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
            RuntimeBridgeError::ManagedSessionUnsupported => (
                "APP_SERVER_MANAGED_SESSION_UNSUPPORTED",
                "当前 Codex App Server Schema 不支持 CAS 托管会话。",
                false,
            ),
            RuntimeBridgeError::AgentExecutionUnsupported => (
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
            RuntimeBridgeError::StateUnavailable => (
                "USAGE_MONITOR_STATE_UNAVAILABLE",
                "Token Usage 监控状态当前不可用。",
                true,
            ),
            RuntimeBridgeError::Usage(_) => (
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
    ManagedSessionUnsupported,
    AgentExecutionUnsupported,
    InvalidCwd,
    InvalidThreadId,
    InvalidInput,
    ThreadNotBound,
    SessionRecoveryRequired,
    TurnAlreadyRunning,
    ProtocolWrite(std::io::Error),
    ProtocolRejected(String),
    ProtocolTimeout(String),
    InvalidProtocolResponse(&'static str),
    UnexpectedThreadResponse,
    StreamClosed,
    RequestIdExhausted,
    Process(std::io::Error),
    SchemaProbe(std::io::Error),
    Usage(UsageServiceError),
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
            Self::ManagedSessionUnsupported => {
                formatter.write_str("managed sessions are not supported by the detected schema")
            }
            Self::AgentExecutionUnsupported => formatter
                .write_str("managed agent execution is not supported by the detected schema"),
            Self::InvalidCwd => formatter.write_str("managed session cwd is invalid"),
            Self::InvalidThreadId => formatter.write_str("managed session thread id is invalid"),
            Self::InvalidInput => formatter.write_str("managed turn input is invalid"),
            Self::ThreadNotBound => formatter.write_str("thread is not bound to runtime bridge"),
            Self::SessionRecoveryRequired => {
                formatter.write_str("managed session recovery is required")
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
            Self::StreamClosed => formatter.write_str("app server event stream closed"),
            Self::RequestIdExhausted => formatter.write_str("app server request id exhausted"),
            Self::Process(error) => write!(formatter, "app server process failed: {error}"),
            Self::SchemaProbe(error) => write!(formatter, "schema probe failed: {error}"),
            Self::Usage(error) => write!(formatter, "usage operation failed: {error}"),
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

#[derive(Debug)]
enum ProtocolParseError {
    MissingField(&'static str),
    InvalidField(&'static str),
    NegativeToken,
    TokenOverflow,
}

impl fmt::Display for ProtocolParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingField(field) => write!(formatter, "App Server 事件缺少字段：{field}"),
            Self::InvalidField(field) => write!(formatter, "App Server 事件字段无效：{field}"),
            Self::NegativeToken => formatter.write_str("App Server 返回了负数 Token"),
            Self::TokenOverflow => formatter.write_str("App Server Token 总数溢出"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usage::UsageListRequest;

    #[test]
    fn managed_session_schema_rejects_new_unknown_required_fields() {
        assert!(schema_requires_only(
            &json!({"required": ["input", "threadId"]}),
            &["input", "threadId"],
        ));
        assert!(!schema_requires_only(
            &json!({"required": ["input", "threadId", "futureRequired"]}),
            &["input", "threadId"],
        ));
    }

    #[test]
    fn agent_profile_maps_to_exact_app_server_overrides() {
        let profile = AgentRuntimeProfile {
            agent_id: "agent-1".to_owned(),
            agent_key: "executor".to_owned(),
            agent_name: "Executor".to_owned(),
            instruction: "只执行已明确的实现任务。".to_owned(),
            sandbox_policy: "WORKSPACE_WRITE".to_owned(),
            reasoning_policy: "HIGH".to_owned(),
            model_slug: "deepseek-v4-flash".to_owned(),
            model_provider: Some("cas_deepseek".to_owned()),
            runtime_fingerprint: "test".to_owned(),
        };

        assert_eq!(
            agent_thread_params(&profile, "C:\\workspace\\project"),
            json!({
                "cwd": "C:\\workspace\\project",
                "model": "deepseek-v4-flash",
                "modelProvider": "cas_deepseek",
                "developerInstructions": "只执行已明确的实现任务。",
                "sandbox": "workspace-write",
            })
        );
        assert_eq!(reasoning_effort(&profile), Some("high"));
    }

    #[test]
    fn native_agent_profile_omits_model_provider_override() {
        let profile = AgentRuntimeProfile {
            agent_id: "agent-native".to_owned(),
            agent_key: "native-executor".to_owned(),
            agent_name: "Native Executor".to_owned(),
            instruction: "执行任务。".to_owned(),
            sandbox_policy: "WORKSPACE_WRITE".to_owned(),
            reasoning_policy: "HIGH".to_owned(),
            model_slug: "gpt-5.6-luna".to_owned(),
            model_provider: None,
            runtime_fingerprint: "test".to_owned(),
        };

        let params = agent_thread_params(&profile, "C:\\workspace\\project");
        assert_eq!(params.get("model"), Some(&json!("gpt-5.6-luna")));
        assert!(params.get("modelProvider").is_none());
    }

    #[test]
    fn managed_thread_parser_accepts_current_and_legacy_identity_fields() {
        assert_eq!(
            parse_managed_thread(&json!({
                "thread": {"id": "thread-1", "sessionId": "session-1", "future": true}
            }))
            .unwrap(),
            ("thread-1".to_owned(), Some("session-1".to_owned())),
        );
        assert_eq!(
            parse_managed_thread(&json!({
                "thread": {"thread_id": "thread-2", "session_id": "session-2"}
            }))
            .unwrap(),
            ("thread-2".to_owned(), Some("session-2".to_owned())),
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
    }

    #[test]
    fn stream_failure_requires_explicit_managed_session_recovery() {
        let state = Arc::new(Mutex::new(RuntimeBridgeState {
            managed_session: Some(ManagedSessionState {
                thread_id: "thread-1".to_owned(),
                session_id: Some("session-1".to_owned()),
                origin: ManagedSessionOrigin::Started,
                status: ManagedSessionStatus::Running,
                cwd: Some("C:\\workspace".to_owned()),
                active_turn_id: Some("turn-1".to_owned()),
                attached_at: "2026-08-11T00:00:00Z".to_owned(),
            }),
            ..RuntimeBridgeState::default()
        }));
        mark_stream_failure(&state, "closed".to_owned());
        let state = state.lock().unwrap();
        let session = state.managed_session.as_ref().unwrap();
        assert_eq!(state.status, RuntimeBridgeStatus::Failed);
        assert_eq!(session.status, ManagedSessionStatus::RecoveryRequired);
        assert!(session.active_turn_id.is_none());
    }

    #[test]
    fn parses_current_usage_and_ignores_future_fields() {
        let event = parse_bridge_event(&json!({
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
        let BridgeEvent::Usage { usage, profile, .. } = event else {
            panic!("expected usage event");
        };
        assert_eq!(profile, ProtocolProfile::UsageModern);
        assert_eq!(usage.total_tokens, 120);
        assert_eq!(usage.current_context_tokens, None);
        assert_eq!(usage.cache_write_input_tokens, 4);
        assert!(!usage.partial);
    }

    #[test]
    fn parses_legacy_snake_case_usage_as_partial_without_fabricating_fields() {
        let event = parse_bridge_event(&json!({
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
        let BridgeEvent::Usage { usage, profile, .. } = event else {
            panic!("expected usage event");
        };
        assert_eq!(profile, ProtocolProfile::UsageLegacy);
        assert_eq!(usage.total_tokens, 35);
        assert_eq!(usage.current_context_tokens, Some(12));
        assert_eq!(usage.reasoning_output_tokens, 0);
        assert!(usage.partial);
    }

    #[test]
    fn keeps_cumulative_and_current_context_usage_separate() {
        let event = parse_bridge_event(&json!({
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
        let BridgeEvent::Usage { usage, .. } = event else {
            panic!("expected usage event");
        };
        assert_eq!(usage.total_tokens, 1_667_248);
        assert_eq!(usage.current_context_tokens, Some(50_000));
        assert_eq!(usage.model_context_window, Some(258_400));
    }

    #[test]
    fn parses_both_collaboration_item_names_and_agent_path() {
        for item_type in ["collabAgentToolCall", "collabToolCall"] {
            let event = parse_bridge_event(&json!({
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
            assert!(matches!(event, BridgeEvent::ParentChild { .. }));
        }

        let event = parse_bridge_event(&json!({
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
            BridgeEvent::AgentPath { agent_key, .. } if agent_key == "executor"
        ));
    }

    #[test]
    fn malformed_recognized_usage_is_rejected_but_unknown_events_are_ignored() {
        assert!(
            parse_bridge_event(&json!({
                "method": "thread/tokenUsage/updated",
                "params": {"threadId": "thread", "tokenUsage": {"total": {}}}
            }))
            .is_err()
        );
        assert!(
            parse_bridge_event(&json!({
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
            .observe(BridgeEvent::ThreadStarted {
                thread_id: "root".to_owned(),
                session_id: Some("root".to_owned()),
                parent_thread_id: None,
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(BridgeEvent::ParentChild {
                parent_thread_id: "root".to_owned(),
                child_thread_ids: vec!["child".to_owned()],
                model_slug: Some("deepseek-v4-flash".to_owned()),
                profile: ProtocolProfile::Modern,
            })
            .unwrap();
        observer
            .observe(BridgeEvent::Usage {
                thread_id: "child".to_owned(),
                usage: TokenUsageSnapshot {
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
                profile: ProtocolProfile::UsageModern,
            })
            .unwrap();
        observer
            .observe(BridgeEvent::TurnFinished {
                thread_id: "child".to_owned(),
                successful: true,
                profile: ProtocolProfile::Modern,
            })
            .unwrap();

        let records = usage.list(UsageListRequest::default()).unwrap();
        let records = serde_json::to_value(records).unwrap();
        assert_eq!(records[0]["codexSessionId"], "root");
        assert_eq!(records[0]["parentThreadId"], "root");
        assert_eq!(records[0]["usageStatus"], "FINAL");
        assert_eq!(records[0]["totalTokens"], 120);
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
        let bridge = RuntimeBridgeService::open(&database_path, &test_root).unwrap();

        bridge
            .start_inner(Path::new(&executable), Path::new(&codex_home), None)
            .unwrap();
        let session = bridge
            .managed_session_start_inner(ManagedSessionStartRequest { cwd })
            .unwrap();
        bridge
            .managed_turn_start_inner(ManagedTurnStartRequest {
                thread_id: session.thread_id,
                input: prompt,
                effort: None,
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
