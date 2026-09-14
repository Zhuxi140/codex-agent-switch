use super::*;
use crate::runtime_adapter::{
    NormalizedRuntimeEvent, ProtocolProfile, RecoveryTurnOutcome, recovery_turn_outcome,
};
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

fn session(
    thread_id: &str,
    status: ManagedSessionStatus,
    active_turn_id: Option<&str>,
) -> ManagedSessionState {
    ManagedSessionState {
        thread_id: thread_id.to_owned(),
        session_id: Some(format!("session-{thread_id}")),
        origin: ManagedSessionOrigin::Started,
        status,
        cwd: Some("C:\\workspace".to_owned()),
        active_turn_id: active_turn_id.map(str::to_owned),
        attached_at: "2026-09-09T00:00:00Z".to_owned(),
    }
}

fn state_with_sessions(sessions: Vec<ManagedSessionState>) -> RuntimeBridgeState {
    RuntimeBridgeState {
        status: RuntimeBridgeStatus::Running,
        managed_sessions: sessions
            .into_iter()
            .map(|session| (session.thread_id.clone(), session))
            .collect(),
        ..RuntimeBridgeState::default()
    }
}

fn bridge_with_sessions(
    status: RuntimeBridgeStatus,
    sessions: Vec<ManagedSessionState>,
) -> (RuntimeBridgeService, PathBuf) {
    let root = std::env::temp_dir().join(format!("cas-session-registry-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let bridge =
        RuntimeBridgeService::open(&root.join("cas.db"), &root, &root.join("cas-helper.exe"))
            .unwrap();
    let mut state = bridge.state().unwrap();
    state.status = status;
    state.managed_sessions = sessions
        .into_iter()
        .map(|session| (session.thread_id.clone(), session))
        .collect();
    drop(state);
    (bridge, root)
}

#[test]
fn terminal_event_changes_only_its_thread_when_both_are_running() {
    let state = Arc::new(Mutex::new(state_with_sessions(vec![
        session(
            "thread-completed",
            ManagedSessionStatus::Running,
            Some("turn-completed"),
        ),
        session(
            "thread-running",
            ManagedSessionStatus::Running,
            Some("turn-running"),
        ),
    ])));

    update_managed_session_from_event(
        &state,
        &NormalizedRuntimeEvent::TurnFinished {
            thread_id: "thread-completed".to_owned(),
            turn_id: "turn-completed".to_owned(),
            successful: true,
            failure_message: None,
            profile: ProtocolProfile::Modern,
        },
    );

    let state = state.lock().unwrap();
    let completed = state.managed_sessions.get("thread-completed").unwrap();
    let running = state.managed_sessions.get("thread-running").unwrap();
    assert_eq!(completed.status, ManagedSessionStatus::Idle);
    assert_eq!(completed.active_turn_id, None);
    assert_eq!(running.status, ManagedSessionStatus::Running);
    assert_eq!(running.active_turn_id.as_deref(), Some("turn-running"));
}

#[test]
fn second_turn_for_the_same_thread_is_rejected_before_requesting_runtime() {
    let (bridge, root) = bridge_with_sessions(
        RuntimeBridgeStatus::Running,
        vec![session(
            "thread-1",
            ManagedSessionStatus::Running,
            Some("turn-1"),
        )],
    );

    assert!(matches!(
        bridge.managed_turn_start_inner(ManagedTurnStartRequest {
            thread_id: "thread-1".to_owned(),
            input: "second turn".to_owned(),
            effort: None,
            approval_policy: None,
            sandbox_policy: None,
        }),
        Err(RuntimeBridgeError::TurnAlreadyRunning)
    ));
    let state = bridge.state().unwrap();
    let session = state.managed_sessions.get("thread-1").unwrap();
    assert_eq!(session.status, ManagedSessionStatus::Running);
    assert_eq!(session.active_turn_id.as_deref(), Some("turn-1"));
    drop(state);
    drop(bridge);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_turn_does_not_overwrite_another_thread_session_state() {
    let state = Arc::new(Mutex::new(state_with_sessions(vec![
        session(
            "thread-failed",
            ManagedSessionStatus::Running,
            Some("turn-failed"),
        ),
        session("thread-idle", ManagedSessionStatus::Idle, None),
    ])));

    update_managed_session_from_event(
        &state,
        &NormalizedRuntimeEvent::TurnFinished {
            thread_id: "thread-failed".to_owned(),
            turn_id: "turn-failed".to_owned(),
            successful: false,
            failure_message: Some("provider failed".to_owned()),
            profile: ProtocolProfile::Modern,
        },
    );

    let state = state.lock().unwrap();
    assert_eq!(
        state.managed_sessions.get("thread-failed").unwrap().status,
        ManagedSessionStatus::Failed
    );
    assert_eq!(
        state.managed_sessions.get("thread-idle").unwrap().status,
        ManagedSessionStatus::Idle
    );
}

#[test]
fn terminal_events_require_the_active_turn_and_are_idempotent() {
    let state = Arc::new(Mutex::new(state_with_sessions(vec![session(
        "thread-1",
        ManagedSessionStatus::Running,
        Some("turn-active"),
    )])));
    let terminal = |turn_id: &str, successful: bool| NormalizedRuntimeEvent::TurnFinished {
        thread_id: "thread-1".to_owned(),
        turn_id: turn_id.to_owned(),
        successful,
        failure_message: Some("should be ignored when unmatched".to_owned()),
        profile: ProtocolProfile::Modern,
    };

    update_managed_session_from_event(&state, &terminal("turn-stale", true));
    {
        let state = state.lock().unwrap();
        let session = state.managed_sessions.get("thread-1").unwrap();
        assert_eq!(session.status, ManagedSessionStatus::Running);
        assert_eq!(session.active_turn_id.as_deref(), Some("turn-active"));
    }

    update_managed_session_from_event(&state, &terminal("turn-active", true));
    update_managed_session_from_event(&state, &terminal("turn-active", false));
    let state = state.lock().unwrap();
    let session = state.managed_sessions.get("thread-1").unwrap();
    assert_eq!(session.status, ManagedSessionStatus::Idle);
    assert_eq!(session.active_turn_id, None);
    assert_eq!(state.last_error, None);
}

#[test]
fn stream_failure_marks_only_running_threads_as_recovery_required() {
    let state = Arc::new(Mutex::new(state_with_sessions(vec![
        session(
            "thread-running",
            ManagedSessionStatus::Running,
            Some("turn-running"),
        ),
        session("thread-idle", ManagedSessionStatus::Idle, None),
    ])));

    mark_stream_failure(&state, "stream closed".to_owned());

    let state = state.lock().unwrap();
    let running = state.managed_sessions.get("thread-running").unwrap();
    let idle = state.managed_sessions.get("thread-idle").unwrap();
    assert_eq!(state.status, RuntimeBridgeStatus::Failed);
    assert_eq!(running.status, ManagedSessionStatus::RecoveryRequired);
    assert_eq!(running.active_turn_id.as_deref(), Some("turn-running"));
    assert_eq!(idle.status, ManagedSessionStatus::Idle);
    assert_eq!(idle.active_turn_id, None);
}

#[test]
fn explicit_recovery_resolves_only_the_requested_thread() {
    let (bridge, root) = bridge_with_sessions(
        RuntimeBridgeStatus::Running,
        vec![
            session(
                "thread-resolved",
                ManagedSessionStatus::RecoveryRequired,
                Some("turn-resolved"),
            ),
            session(
                "thread-uncertain",
                ManagedSessionStatus::RecoveryRequired,
                Some("turn-uncertain"),
            ),
        ],
    );

    let response = bridge
        .managed_session_resolve_recovery_inner(ManagedSessionRecoveryRequest {
            thread_id: "thread-resolved".to_owned(),
            abandon_uncertain_turn: true,
        })
        .unwrap();
    assert_eq!(response.thread_id, "thread-resolved");
    assert_eq!(response.status, ManagedSessionStatus::Idle);
    assert_eq!(response.active_turn_id, None);

    let state = bridge.state().unwrap();
    let resolved = state.managed_sessions.get("thread-resolved").unwrap();
    let uncertain = state.managed_sessions.get("thread-uncertain").unwrap();
    assert_eq!(resolved.status, ManagedSessionStatus::Idle);
    assert_eq!(resolved.active_turn_id, None);
    assert_eq!(uncertain.status, ManagedSessionStatus::RecoveryRequired);
    assert_eq!(uncertain.active_turn_id.as_deref(), Some("turn-uncertain"));
    assert!(state.last_error.is_some());
    drop(state);
    drop(bridge);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unknown_turn_evidence_keeps_recovery_required_without_replay() {
    let unknown = json!({
        "thread": {"turns": [{"id": "turn-other", "status": "completed"}]}
    });
    assert_eq!(
        recovery_turn_outcome(&unknown, Some("turn-unknown")),
        RecoveryTurnOutcome::Unknown
    );

    let (bridge, root) = bridge_with_sessions(
        RuntimeBridgeStatus::Running,
        vec![session(
            "thread-unknown",
            ManagedSessionStatus::RecoveryRequired,
            Some("turn-unknown"),
        )],
    );
    assert!(matches!(
        bridge.managed_session_resolve_recovery_inner(ManagedSessionRecoveryRequest {
            thread_id: "thread-unknown".to_owned(),
            abandon_uncertain_turn: false,
        }),
        Err(RuntimeBridgeError::NotRunning)
    ));
    let state = bridge.state().unwrap();
    let session = state.managed_sessions.get("thread-unknown").unwrap();
    assert_eq!(session.status, ManagedSessionStatus::RecoveryRequired);
    assert_eq!(session.active_turn_id.as_deref(), Some("turn-unknown"));
    drop(state);
    drop(bridge);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn automatic_recovery_remains_limited_to_three_attempts() {
    let mut state = RuntimeBridgeState {
        status: RuntimeBridgeStatus::Failed,
        recovery_attempt_count: MAX_AUTO_RECOVERY_ATTEMPTS - 1,
        ..RuntimeBridgeState::default()
    };
    assert_eq!(MAX_AUTO_RECOVERY_ATTEMPTS, 3);
    assert!(auto_recovery_allowed(&state));

    state.recovery_attempt_count = MAX_AUTO_RECOVERY_ATTEMPTS;
    assert!(!auto_recovery_allowed(&state));
    let response = RuntimeBridgeStatusResponse::from(&state);
    assert_eq!(response.max_auto_recovery_attempts, 3);
    assert!(response.auto_recovery_exhausted);
}

#[test]
fn status_response_is_thread_bound_and_lists_the_complete_registry() {
    let mut state = state_with_sessions(vec![
        session("thread-a", ManagedSessionStatus::Idle, None),
        session(
            "thread-z",
            ManagedSessionStatus::RecoveryRequired,
            Some("turn-z"),
        ),
    ]);
    state.last_managed_thread_id = Some("thread-a".to_owned());

    let response = RuntimeBridgeStatusResponse::from(&state);
    assert_eq!(
        response.managed_session.as_ref().unwrap().thread_id,
        "thread-a"
    );
    assert_eq!(response.managed_sessions.len(), 2);
    let registry = response
        .managed_sessions
        .iter()
        .map(|session| {
            (
                session.thread_id.as_str(),
                (session.status, session.active_turn_id.as_deref()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        registry.get("thread-a"),
        Some(&(ManagedSessionStatus::Idle, None))
    );
    assert_eq!(
        registry.get("thread-z"),
        Some(&(ManagedSessionStatus::RecoveryRequired, Some("turn-z")))
    );
}
