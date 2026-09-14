use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use uuid::Uuid;

use crate::delivery_receipt::{
    DeliveryReceiptAppendRequest, ReceiptEvidence, append_in_transaction,
};
use crate::orchestration_contract::{
    ExecutionKind, OrchestrationError, OrchestrationErrorCode, ReceiptEvidenceSource, ReceiptStage,
};
use crate::persistence::{PersistenceError, open_database};
use crate::runtime_adapter::{NormalizedRuntimeEvent, ProtocolProfile};

/// D-02 的唯一写入口。身份、Lease、Attempt/Job 与 Receipt 总是在同一立即事务中变化。
pub(crate) struct OrchestrationReceiptEventService {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManagedTurnAcceptedEvidence {
    pub(crate) job_id: String,
    pub(crate) attempt_id: String,
    pub(crate) agent_id: String,
    pub(crate) agent_name: String,
    pub(crate) parent_thread_id: String,
    pub(crate) workspace_scope_key: String,
    pub(crate) task_scope_key: String,
    pub(crate) runtime_fingerprint: String,
    pub(crate) thread_id: String,
    pub(crate) turn_id: String,
    pub(crate) evidence_ref: String,
}

impl OrchestrationReceiptEventService {
    pub(crate) fn open(database_path: &Path) -> Result<Self, PersistenceError> {
        Ok(Self {
            connection: Mutex::new(open_database(database_path)?),
        })
    }

    /// 仅在已核验 thread/start|resume 与 turn/start 的响应后调用。
    pub(crate) fn accept_managed(
        &self,
        evidence: ManagedTurnAcceptedEvidence,
    ) -> Result<(), OrchestrationError> {
        if [
            &evidence.job_id,
            &evidence.attempt_id,
            &evidence.agent_id,
            &evidence.parent_thread_id,
            &evidence.workspace_scope_key,
            &evidence.task_scope_key,
            &evidence.runtime_fingerprint,
            &evidence.thread_id,
            &evidence.turn_id,
            &evidence.evidence_ref,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return Err(field_error());
        }
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let now = timestamp(&transaction)?;
        let identity = managed_identity(&transaction, &evidence)?;
        let Some(identity) = identity else {
            return Err(attempt_not_current(&evidence.attempt_id));
        };
        if identity
            .execution_kind
            .is_some_and(|kind| kind != "MANAGED_WORKER")
            || identity.planned_execution_kind != "MANAGED_WORKER"
            || identity
                .thread_id
                .as_deref()
                .is_some_and(|id| id != evidence.thread_id)
            || identity
                .lease_thread_id
                .as_deref()
                .is_some_and(|id| id != evidence.thread_id)
            || identity
                .turn_id
                .as_deref()
                .is_some_and(|id| id != evidence.turn_id)
        {
            return Err(execution_kind_mismatch(
                &evidence.job_id,
                &evidence.attempt_id,
            ));
        }
        if identity.state == "DISPATCHING"
            && identity.lease_state == "PENDING"
            && identity.job_state == "DISPATCHED"
        {
            let instance_id = bind_managed_instance(
                &transaction,
                &evidence,
                identity.native_revision_resume_allowed,
                &now,
            )?;
            let changed = transaction.execute(
                "UPDATE job_attempts SET execution_kind = 'MANAGED_WORKER', thread_instance_id = ?2,
                    codex_turn_id = ?3, state = 'ACCEPTED', accepted_at = COALESCE(accepted_at, ?4),
                    updated_at = ?4 WHERE attempt_id = ?1 AND state = 'DISPATCHING'",
                params![evidence.attempt_id, instance_id, evidence.turn_id, now],
            ).map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE runtime_delegation_leases
                     SET codex_agent_id = ?2, state = 'ACTIVE',
                         admitted_at = COALESCE(admitted_at, ?3),
                         admission_confirmed_at = COALESCE(admission_confirmed_at, ?3),
                         updated_at = ?3
                     WHERE id = ?1 AND state = 'PENDING'
                       AND (codex_agent_id IS NULL OR codex_agent_id = ?2)",
                    params![identity.lease_id, evidence.thread_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            let changed = transaction
                .execute(
                    "UPDATE orchestration_jobs SET state = 'RUNNING', updated_at = ?2
                 WHERE job_id = ?1 AND state = 'DISPATCHED'",
                    params![evidence.job_id, now],
                )
                .map_err(|_| persistence_error())?;
            if changed != 1 {
                return Err(invariant_error(&evidence.job_id, &evidence.attempt_id));
            }
            transaction
                .execute(
                    "DELETE FROM agent_spawn_reservations WHERE lease_id = ?1",
                    [&identity.lease_id],
                )
                .map_err(|_| persistence_error())?;
        } else if !matches!(
            (identity.state.as_str(), identity.lease_state.as_str()),
            ("ACCEPTED", "ACTIVE") | ("SUCCEEDED", "ACTIVE")
        ) {
            return Err(attempt_not_current(&evidence.attempt_id));
        }
        append_in_transaction(
            &transaction,
            DeliveryReceiptAppendRequest {
                job_id: evidence.job_id.clone(),
                attempt_id: evidence.attempt_id.clone(),
                evidence: vec![ReceiptEvidence {
                    stage: ReceiptStage::TurnAccepted,
                    execution_kind: Some(ExecutionKind::ManagedWorker),
                    evidence_source: ReceiptEvidenceSource::AppServerResponse,
                    evidence_ref: evidence.evidence_ref,
                    parent_thread_id: evidence.parent_thread_id,
                    codex_thread_id: Some(evidence.thread_id.clone()),
                    codex_turn_id: Some(evidence.turn_id.clone()),
                    schema_profile: Some("APP_SERVER".to_owned()),
                    evidence_at: now.clone(),
                }],
            },
        )?;
        observe_pending_success(
            &transaction,
            &evidence.thread_id,
            Some(&evidence.turn_id),
            &now,
        )?;
        transaction.commit().map_err(|_| persistence_error())
    }

    /// Runtime 事件先耐久化；缺少连续 Receipt 或原生身份证据时仅保留 UNKNOWN。
    pub(crate) fn observe_runtime_event(
        &self,
        event: &NormalizedRuntimeEvent,
        raw_event: &str,
    ) -> Result<(), OrchestrationError> {
        let mut connection = self.connection()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| persistence_error())?;
        let now = timestamp(&transaction)?;
        match event {
            NormalizedRuntimeEvent::ParentChild {
                parent_thread_id,
                child_thread_ids,
                profile,
                ..
            } => {
                for child_thread_id in child_thread_ids {
                    store_event(
                        &transaction,
                        &format!("PARENT_CHILD:{parent_thread_id}:{child_thread_id}"),
                        "PARENT_CHILD",
                        raw_event,
                        *profile,
                        Some(parent_thread_id),
                        Some(child_thread_id),
                        None,
                        None,
                        &now,
                    )?;
                }
            }
            NormalizedRuntimeEvent::TurnFinished {
                thread_id,
                turn_id,
                successful,
                profile,
                ..
            } => {
                let event_key = format!("TURN_FINISHED:{thread_id}:{turn_id}:{successful}");
                store_event(
                    &transaction,
                    &event_key,
                    "TURN_FINISHED",
                    raw_event,
                    *profile,
                    None,
                    Some(thread_id),
                    Some(turn_id),
                    Some(*successful),
                    &now,
                )?;
            }
            _ => {}
        }
        reconcile_pending_native_children(&transaction, &now)?;
        if let NormalizedRuntimeEvent::TurnFinished {
            thread_id,
            turn_id,
            successful: true,
            ..
        } = event
        {
            observe_pending_success(&transaction, thread_id, Some(turn_id), &now)?;
        }
        transaction.commit().map_err(|_| persistence_error())
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, OrchestrationError> {
        self.connection.lock().map_err(|_| persistence_error())
    }
}

struct AttemptIdentity {
    state: String,
    job_state: String,
    lease_state: String,
    lease_id: String,
    planned_execution_kind: String,
    execution_kind: Option<String>,
    thread_id: Option<String>,
    lease_thread_id: Option<String>,
    turn_id: Option<String>,
    native_revision_resume_allowed: bool,
}

fn managed_identity(
    tx: &Transaction<'_>,
    e: &ManagedTurnAcceptedEvidence,
) -> Result<Option<AttemptIdentity>, OrchestrationError> {
    tx.query_row(
        "SELECT attempt.state, job.state, lease.state, attempt.lease_id,
                attempt.planned_execution_kind, attempt.execution_kind,
                instance.codex_thread_id, lease.codex_agent_id, attempt.codex_turn_id,
                EXISTS (
                    SELECT 1
                    FROM job_attempts previous
                    JOIN review_decisions review
                      ON review.job_id = previous.job_id
                     AND review.attempt_id = previous.attempt_id
                    WHERE attempt.route_action = 'REUSE'
                      AND attempt.planned_execution_kind = 'MANAGED_WORKER'
                      AND previous.job_id = attempt.job_id
                      AND previous.attempt_id = attempt.previous_attempt_id
                      AND previous.attempt_no + 1 = attempt.attempt_no
                      AND previous.state = 'SUCCEEDED'
                      AND previous.execution_kind = 'NATIVE_CHILD'
                      AND previous.thread_instance_id = attempt.thread_instance_id
                      AND review.decision = 'REVISION_REQUIRED'
                      AND instance.reuse_state = 'HELD_FOR_REVIEW'
                      AND instance.claim_lease_id = attempt.lease_id
                )
         FROM job_attempts attempt JOIN orchestration_jobs job ON job.job_id = attempt.job_id
         JOIN runtime_delegation_leases lease ON lease.id = attempt.lease_id
         LEFT JOIN agent_thread_instances instance ON instance.id = attempt.thread_instance_id
         WHERE attempt.job_id = ?1 AND attempt.attempt_id = ?2 AND job.agent_id = ?3
           AND job.parent_thread_id = ?4 AND job.workspace_scope_key = ?5 AND job.task_scope_key = ?6",
        params![e.job_id, e.attempt_id, e.agent_id, e.parent_thread_id, e.workspace_scope_key, e.task_scope_key],
        |row| Ok(AttemptIdentity {
            state: row.get(0)?,
            job_state: row.get(1)?,
            lease_state: row.get(2)?,
            lease_id: row.get(3)?,
            planned_execution_kind: row.get(4)?,
            execution_kind: row.get(5)?,
            thread_id: row.get(6)?,
            lease_thread_id: row.get(7)?,
            turn_id: row.get(8)?,
            native_revision_resume_allowed: row.get(9)?,
        }),
    ).optional().map_err(|_| persistence_error())
}

fn bind_managed_instance(
    tx: &Transaction<'_>,
    e: &ManagedTurnAcceptedEvidence,
    native_revision_resume_allowed: bool,
    now: &str,
) -> Result<String, OrchestrationError> {
    let existing = tx.query_row("SELECT id, agent_id, parent_thread_id, scope_key, task_scope_key, runtime_fingerprint, execution_kind FROM agent_thread_instances WHERE codex_thread_id = ?1", [&e.thread_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, String>(6)?))
    }).optional().map_err(|_| persistence_error())?;
    if let Some((id, agent, parent, scope, task, fingerprint, kind)) = existing {
        if agent.as_deref() != Some(&e.agent_id)
            || parent.as_deref() != Some(&e.parent_thread_id)
            || scope.as_deref() != Some(&e.workspace_scope_key)
            || task.as_deref() != Some(&e.task_scope_key)
            || fingerprint.as_deref() != Some(&e.runtime_fingerprint)
            || (!matches!(kind.as_str(), "MANAGED_WORKER" | "OBSERVED_EXTERNAL")
                && !(kind == "NATIVE_CHILD" && native_revision_resume_allowed))
        {
            return Err(execution_kind_mismatch(&e.job_id, &e.attempt_id));
        }
        let changed = tx
            .execute(
                "UPDATE agent_thread_instances
             SET execution_kind = 'MANAGED_WORKER', status = 'RUNNING',
                 last_used_at = ?2, last_observed_at = ?2
             WHERE id = ?1
               AND (execution_kind IN ('MANAGED_WORKER', 'OBSERVED_EXTERNAL')
                    OR (?3 = 1 AND execution_kind = 'NATIVE_CHILD'))",
                params![id, now, i64::from(native_revision_resume_allowed)],
            )
            .map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(invariant_error(&e.job_id, &e.attempt_id));
        }
        return Ok(id);
    }
    let id = Uuid::new_v4().to_string();
    tx.execute("INSERT INTO agent_thread_instances (id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id, scope_key, status, input_tokens, cached_input_tokens, output_tokens, total_tokens, current_context_tokens, context_window, runtime_fingerprint, created_at, last_used_at, closed_at, last_model_usage_at, last_observed_at, task_scope_key, reuse_state, reuse_state_reason, execution_kind) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'RUNNING', 0,0,0,0,NULL,NULL,?7,?8,?8,NULL,NULL,?8,?9,'ACTIVE',NULL,'MANAGED_WORKER')", params![id, e.agent_id, e.agent_name, e.thread_id, e.parent_thread_id, e.workspace_scope_key, e.runtime_fingerprint, now, e.task_scope_key]).map_err(|_| persistence_error())?;
    Ok(id)
}

fn store_event(
    tx: &Transaction<'_>,
    key: &str,
    kind: &str,
    raw: &str,
    profile: ProtocolProfile,
    parent: Option<&String>,
    thread: Option<&String>,
    turn: Option<&String>,
    successful: Option<bool>,
    now: &str,
) -> Result<(), OrchestrationError> {
    tx.execute("INSERT INTO runtime_receipt_events (event_id,event_key,event_type,raw_event,schema_profile,parent_thread_id,codex_thread_id,codex_turn_id,successful,observed_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(event_key) DO NOTHING", params![Uuid::new_v4().to_string(),key,kind,raw,profile.as_str(),parent,thread,turn,successful.map(i64::from),now]).map_err(|_| persistence_error())?;
    Ok(())
}

fn accept_native_child(
    tx: &Transaction<'_>,
    parent: &str,
    child: &str,
    profile: ProtocolProfile,
    event_key: &str,
    now: &str,
) -> Result<bool, OrchestrationError> {
    let candidates = {
        let mut statement = tx.prepare(
        "SELECT attempt.attempt_id, job.job_id, attempt.lease_id, instance.id
         FROM job_attempts attempt JOIN orchestration_jobs job ON job.job_id = attempt.job_id
         JOIN runtime_delegation_leases lease ON lease.id = attempt.lease_id
         JOIN agent_schedule_decisions decision ON decision.id = attempt.schedule_decision_id
         JOIN agent_thread_instances instance ON instance.codex_thread_id = ?1
         WHERE job.parent_thread_id = ?2 AND attempt.state = 'DISPATCHING' AND lease.state = 'PENDING'
           AND attempt.execution_kind IS NULL AND attempt.planned_execution_kind = 'NATIVE_CHILD'
           AND (lease.codex_agent_id IS NULL OR lease.codex_agent_id = ?1)
           AND instance.agent_id = job.agent_id AND instance.parent_thread_id = job.parent_thread_id
           AND instance.scope_key = job.workspace_scope_key AND instance.task_scope_key = job.task_scope_key
           AND instance.runtime_fingerprint = decision.runtime_fingerprint
         LIMIT 2",
        ).map_err(|_| persistence_error())?;
        statement
            .query_map(params![child, parent], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| persistence_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| persistence_error())?
    };
    let [(attempt_id, job_id, lease_id, instance_id)] = candidates.as_slice() else {
        return Ok(false);
    };
    let changed = tx.execute("UPDATE agent_thread_instances SET execution_kind='NATIVE_CHILD', status='RUNNING', last_observed_at=?2 WHERE id=?1 AND execution_kind IN ('OBSERVED_EXTERNAL', 'NATIVE_CHILD')", params![instance_id,now]).map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(invariant_error(job_id, attempt_id));
    }
    let changed = tx.execute("UPDATE job_attempts SET execution_kind='NATIVE_CHILD', thread_instance_id=?2, state='ACCEPTED', accepted_at=COALESCE(accepted_at,?3), updated_at=?3 WHERE attempt_id=?1 AND state='DISPATCHING'", params![attempt_id,instance_id,now]).map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(invariant_error(job_id, attempt_id));
    }
    let changed = tx
        .execute(
            "UPDATE runtime_delegation_leases
         SET codex_agent_id=?2, state='ACTIVE',
             admitted_at=COALESCE(admitted_at,?3),
             admission_confirmed_at=COALESCE(admission_confirmed_at,?3),
             updated_at=?3
         WHERE id=?1 AND state='PENDING'
           AND (codex_agent_id IS NULL OR codex_agent_id=?2)",
            params![lease_id, child, now],
        )
        .map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(invariant_error(job_id, attempt_id));
    }
    let changed = tx.execute("UPDATE orchestration_jobs SET state='RUNNING', updated_at=?2 WHERE job_id=?1 AND state='DISPATCHED'", params![job_id,now]).map_err(|_| persistence_error())?;
    if changed != 1 {
        return Err(invariant_error(job_id, attempt_id));
    }
    tx.execute(
        "DELETE FROM agent_spawn_reservations WHERE lease_id=?1",
        [lease_id],
    )
    .map_err(|_| persistence_error())?;
    append_in_transaction(
        tx,
        DeliveryReceiptAppendRequest {
            job_id: job_id.clone(),
            attempt_id: attempt_id.clone(),
            evidence: vec![ReceiptEvidence {
                stage: ReceiptStage::TurnAccepted,
                execution_kind: Some(ExecutionKind::NativeChild),
                evidence_source: ReceiptEvidenceSource::NativeParentChildEvent,
                evidence_ref: event_key.to_owned(),
                parent_thread_id: parent.to_owned(),
                codex_thread_id: Some(child.to_owned()),
                codex_turn_id: None,
                schema_profile: Some(profile.as_str().to_owned()),
                evidence_at: now.to_owned(),
            }],
        },
    )?;
    Ok(true)
}

/// ParentChild 证据可能先于 Hook/同步完成到达。保留原始事件，直到刚好一条 Pending
/// Attempt 的 Agent、Scope、Task 与 Fingerprint 全部可核验；歧义和缺证据都不消费事件。
fn reconcile_pending_native_children(
    tx: &Transaction<'_>,
    now: &str,
) -> Result<(), OrchestrationError> {
    let events = {
        let mut statement = tx
            .prepare(
                "SELECT event_id, event_key, schema_profile, parent_thread_id, codex_thread_id
             FROM runtime_receipt_events
             WHERE event_type = 'PARENT_CHILD' AND processed_at IS NULL
             ORDER BY observed_at, event_id",
            )
            .map_err(|_| persistence_error())?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            })
            .map_err(|_| persistence_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| persistence_error())?
    };
    for (event_id, event_key, profile, parent, child) in events {
        let (Some(parent), Some(child), Some(profile)) = (parent, child, profile) else {
            continue;
        };
        let profile = match profile.as_str() {
            "MODERN" => ProtocolProfile::Modern,
            "LEGACY" => ProtocolProfile::Legacy,
            _ => continue,
        };
        if accept_native_child(tx, &parent, &child, profile, &event_key, now)? {
            tx.execute(
                "UPDATE runtime_receipt_events SET processed_at = ?2 WHERE event_id = ?1",
                params![event_id, now],
            )
            .map_err(|_| persistence_error())?;
            observe_pending_success(tx, &child, None, now)?;
        }
    }
    Ok(())
}

fn observe_pending_success(
    tx: &Transaction<'_>,
    thread: &str,
    turn: Option<&str>,
    now: &str,
) -> Result<(), OrchestrationError> {
    let events = {
        let mut statement = tx.prepare("SELECT event_id, event_key, schema_profile, codex_turn_id, observed_at FROM runtime_receipt_events WHERE event_type='TURN_FINISHED' AND successful=1 AND processed_at IS NULL AND codex_thread_id=?1 ORDER BY observed_at, event_id").map_err(|_| persistence_error())?;
        statement
            .query_map([thread], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| persistence_error())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| persistence_error())?
    };
    for (event_id, event_key, profile, event_turn, observed_at) in events {
        if turn.is_some_and(|expected| event_turn.as_deref() != Some(expected)) {
            continue;
        }
        // Native Acceptance 没有 Turn ID；至少以 Attempt 的派发时刻隔离同一 Child 的旧终态。
        let candidate = tx.query_row("SELECT attempt.attempt_id, attempt.job_id, attempt.execution_kind, attempt.codex_turn_id, job.parent_thread_id, instance.id FROM job_attempts attempt JOIN orchestration_jobs job ON job.job_id=attempt.job_id JOIN agent_thread_instances instance ON instance.id=attempt.thread_instance_id JOIN runtime_delegation_leases lease ON lease.id=attempt.lease_id WHERE instance.codex_thread_id=?1 AND attempt.state IN ('ACCEPTED','RUNNING') AND attempt.execution_kind IS NOT NULL AND job.state='RUNNING' AND lease.state='ACTIVE' AND lease.codex_agent_id=?1 AND (attempt.execution_kind='MANAGED_WORKER' OR (attempt.execution_kind='NATIVE_CHILD' AND attempt.dispatch_recorded_at IS NOT NULL AND julianday(?2) > julianday(attempt.dispatch_recorded_at))) LIMIT 1", params![thread, observed_at], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,Option<String>>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?))).optional().map_err(|_| persistence_error())?;
        let Some((attempt_id, job_id, kind, expected_turn, parent, instance_id)) = candidate else {
            continue;
        };
        if kind == "MANAGED_WORKER" && expected_turn.as_deref() != event_turn.as_deref() {
            continue;
        }
        let execution_kind = if kind == "MANAGED_WORKER" {
            ExecutionKind::ManagedWorker
        } else {
            ExecutionKind::NativeChild
        };
        append_in_transaction(
            tx,
            DeliveryReceiptAppendRequest {
                job_id: job_id.clone(),
                attempt_id: attempt_id.clone(),
                evidence: vec![ReceiptEvidence {
                    stage: ReceiptStage::ResultObserved,
                    execution_kind: Some(execution_kind),
                    evidence_source: ReceiptEvidenceSource::RuntimeEvent,
                    evidence_ref: event_key,
                    parent_thread_id: parent,
                    codex_thread_id: Some(thread.to_owned()),
                    codex_turn_id: event_turn.clone(),
                    schema_profile: profile,
                    evidence_at: now.to_owned(),
                }],
            },
        )?;
        let changed = tx.execute("UPDATE job_attempts SET state='SUCCEEDED', codex_turn_id=COALESCE(codex_turn_id,?3), terminal_at=COALESCE(terminal_at,?2), updated_at=?2 WHERE attempt_id=?1 AND state IN ('ACCEPTED','RUNNING') AND (codex_turn_id IS NULL OR codex_turn_id=?3)", params![attempt_id,now,event_turn]).map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(invariant_error(&job_id, &attempt_id));
        }
        let changed = tx
            .execute(
                "UPDATE agent_thread_instances
             SET status='IDLE', reuse_state='HELD_FOR_REVIEW', last_observed_at=?2
             WHERE id=?1 AND codex_thread_id=?3",
                params![instance_id, now, thread],
            )
            .map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(invariant_error(&job_id, &attempt_id));
        }
        let changed = tx.execute("UPDATE orchestration_jobs SET state='RESULT_RECEIVED', updated_at=?2 WHERE job_id=?1 AND state='RUNNING'", params![job_id,now]).map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(invariant_error(&job_id, &attempt_id));
        }
        let changed = tx.execute("UPDATE orchestration_jobs SET state='REVIEW_PENDING', updated_at=?2 WHERE job_id=?1 AND state='RESULT_RECEIVED'", params![job_id,now]).map_err(|_| persistence_error())?;
        if changed != 1 {
            return Err(invariant_error(&job_id, &attempt_id));
        }
        tx.execute(
            "UPDATE runtime_receipt_events SET processed_at=?2 WHERE event_id=?1",
            params![event_id, now],
        )
        .map_err(|_| persistence_error())?;
    }
    Ok(())
}

fn timestamp(connection: &Connection) -> Result<String, OrchestrationError> {
    connection
        .query_row("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", [], |row| {
            row.get(0)
        })
        .map_err(|_| persistence_error())
}
fn field_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::TaskPacketFieldInvalid,
        message: "Receipt 身份证据无效。".to_owned(),
        field_path: Some("receipt_identity".to_owned()),
        job_id: None,
        attempt_id: None,
    }
}
fn attempt_not_current(attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::AttemptNotCurrent,
        message: "Attempt 不再是可接收的派发对象。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: Some(attempt_id.to_owned()),
    }
}
fn execution_kind_mismatch(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::ExecutionKindMismatch,
        message: "Runtime 身份证据与 Job/Attempt 不一致。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}
fn invariant_error(job_id: &str, attempt_id: &str) -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::InternalInvariantViolation,
        message: "Receipt 身份链与持久化状态不一致。".to_owned(),
        field_path: None,
        job_id: Some(job_id.to_owned()),
        attempt_id: Some(attempt_id.to_owned()),
    }
}
fn persistence_error() -> OrchestrationError {
    OrchestrationError {
        code: OrchestrationErrorCode::PersistenceError,
        message: "Receipt 事件持久化失败。".to_owned(),
        field_path: None,
        job_id: None,
        attempt_id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    const NOW: &str = "2026-09-10T00:00:00.000Z";

    fn database_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("cas-receipt-{}.sqlite", Uuid::new_v4()))
    }

    fn seed(path: &Path, native_instance: bool) {
        let connection = open_database(path).unwrap();
        connection.execute(
            "INSERT INTO providers (id, provider_key, name, provider_type, base_url, protocol, auth_type, enabled, source, preset_id, created_at, updated_at) VALUES ('provider-1', 'openai', 'OpenAI', 'PRESET', 'https://api.example/v1', 'RESPONSES', 'BEARER_TOKEN', 1, 'BUILT_IN', 'codex-native', ?1, ?1)",
            [NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO models (id, provider_id, model_id, display_name, enabled, source, created_at, updated_at) VALUES ('model-1', 'provider-1', 'gpt-test', 'GPT Test', 1, 'PRESET', ?1, ?1)",
            [NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO agents (id, agent_key, name, description, instruction, agent_type, enabled, sandbox_policy, reasoning_policy, source, managed, role_key, orchestration_phase, created_at, updated_at) VALUES ('agent-1', 'executor', 'Executor', 'test', 'test', 'CUSTOM', 1, 'WORKSPACE_WRITE', 'MEDIUM', 'CAS', 1, 'executor', 'EXECUTION', ?1, ?1)",
            [NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO orchestration_jobs (job_id, idempotency_key, task_packet, task_packet_hash, agent_id, parent_thread_id, workspace_scope_key, task_scope_key, state, created_at, updated_at) VALUES ('job-1', 'key-1', '{}', ?1, 'agent-1', 'parent-1', 'c:/workspace', 'task-1', 'DISPATCHED', ?2, ?2)",
            params!["0".repeat(64), NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO agent_schedule_decisions (id, created_at, source, agent_id, agent_name_snapshot, workspace_scope_key, parent_thread_id, decision, reason_code, runtime_fingerprint, cache_hint, claimed, task_scope_key, job_id) VALUES ('decision-1', ?1, 'CAS', 'agent-1', 'Executor', 'c:/workspace', 'parent-1', 'SPAWN', 'TEST', 'fingerprint-1', 'UNKNOWN', 0, 'task-1', 'job-1')",
            [NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO runtime_delegation_leases (id, created_at, updated_at, agent_id, parent_thread_id, workspace_scope_key, task_scope_key, schedule_decision_id, state, expires_at, agent_type) VALUES ('lease-1', ?1, ?1, 'agent-1', 'parent-1', 'c:/workspace', 'task-1', 'decision-1', 'PENDING', '2099-01-01T00:00:00.000Z', 'executor')",
            [NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO job_attempts (attempt_id, job_id, attempt_no, schedule_decision_id, lease_id, route_action, planned_execution_kind, state, created_at, updated_at, dispatch_recorded_at) VALUES ('attempt-1', 'job-1', 1, 'decision-1', 'lease-1', 'SPAWN', ?1, 'DISPATCHING', ?2, ?2, ?2)",
            params![if native_instance { "NATIVE_CHILD" } else { "MANAGED_WORKER" }, NOW],
        ).unwrap();
        connection.execute(
            "INSERT INTO delivery_receipts (receipt_id, job_id, attempt_id, stage, evidence_source, evidence_ref, parent_thread_id, evidence_at, created_at) VALUES ('receipt-dispatch', 'job-1', 'attempt-1', 'DISPATCH_RECORDED', 'CAS_TRANSACTION', 'decision-1', 'parent-1', ?1, ?1)",
            [NOW],
        ).unwrap();
        if native_instance {
            connection.execute(
                "INSERT INTO agent_thread_instances (id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id, scope_key, status, input_tokens, cached_input_tokens, output_tokens, total_tokens, runtime_fingerprint, created_at, last_used_at, last_observed_at, task_scope_key, reuse_state, execution_kind) VALUES ('instance-1', 'agent-1', 'Executor', 'child-1', 'parent-1', 'c:/workspace', 'UNKNOWN', 0, 0, 0, 0, 'fingerprint-1', ?1, ?1, ?1, 'task-1', 'ACTIVE', 'OBSERVED_EXTERNAL')",
                [NOW],
            ).unwrap();
        }
    }

    fn managed_evidence() -> ManagedTurnAcceptedEvidence {
        ManagedTurnAcceptedEvidence {
            job_id: "job-1".to_owned(),
            attempt_id: "attempt-1".to_owned(),
            agent_id: "agent-1".to_owned(),
            agent_name: "Executor".to_owned(),
            parent_thread_id: "parent-1".to_owned(),
            workspace_scope_key: "c:/workspace".to_owned(),
            task_scope_key: "task-1".to_owned(),
            runtime_fingerprint: "fingerprint-1".to_owned(),
            thread_id: "thread-1".to_owned(),
            turn_id: "turn-1".to_owned(),
            evidence_ref: "thread/start+turn/start:thread-1:turn-1".to_owned(),
        }
    }

    fn insert_native_thread(connection: &Connection) {
        connection
            .execute(
                "INSERT INTO agent_thread_instances (
                id, agent_id, agent_name_snapshot, codex_thread_id, parent_thread_id,
                scope_key, status, input_tokens, cached_input_tokens, output_tokens,
                total_tokens, runtime_fingerprint, created_at, last_used_at,
                last_observed_at, task_scope_key, reuse_state, execution_kind
             ) VALUES (
                'instance-1', 'agent-1', 'Executor', 'thread-1', 'parent-1',
                'c:/workspace', 'IDLE', 0, 0, 0, 0, 'fingerprint-1', ?1, ?1,
                ?1, 'task-1', 'HELD_FOR_REVIEW', 'NATIVE_CHILD'
             )",
                [NOW],
            )
            .unwrap();
    }

    fn seed_native_revision(path: &Path) -> ManagedTurnAcceptedEvidence {
        seed(path, false);
        let connection = open_database(path).unwrap();
        insert_native_thread(&connection);
        connection
            .execute(
                "UPDATE job_attempts
             SET planned_execution_kind='NATIVE_CHILD', execution_kind='NATIVE_CHILD',
                 thread_instance_id='instance-1', state='SUCCEEDED', accepted_at=?1,
                 terminal_at=?1, updated_at=?1
             WHERE attempt_id='attempt-1'",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE runtime_delegation_leases
             SET codex_agent_id='thread-1', state='RELEASED', released_at=?1,
                 release_reason='REVISION_REQUIRED', updated_at=?1
             WHERE id='lease-1'",
                [NOW],
            )
            .unwrap();
        for (receipt_id, stage, source, reference) in [
            (
                "receipt-accepted-old",
                "TURN_ACCEPTED",
                "NATIVE_PARENT_CHILD_EVENT",
                "native-parent-child:thread-1",
            ),
            (
                "receipt-result-old",
                "RESULT_OBSERVED",
                "RUNTIME_EVENT",
                "native-result:thread-1",
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO delivery_receipts (
                    receipt_id, job_id, attempt_id, stage, execution_kind,
                    evidence_source, evidence_ref, parent_thread_id, codex_thread_id,
                    schema_profile, evidence_at, created_at
                 ) VALUES (
                    ?1, 'job-1', 'attempt-1', ?2, 'NATIVE_CHILD', ?3, ?4,
                    'parent-1', 'thread-1', 'TEST', ?5, ?5
                 )",
                    params![receipt_id, stage, source, reference, NOW],
                )
                .unwrap();
        }
        connection
            .execute(
                "UPDATE orchestration_jobs SET state='REVIEW_PENDING', updated_at=?1
             WHERE job_id='job-1'",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO review_decisions (
                review_id, job_id, attempt_id, decision, reviewer_thread_id,
                reason, evidence_refs, created_at
             ) VALUES (
                'review-1', 'job-1', 'attempt-1', 'REVISION_REQUIRED', 'parent-1',
                '补充修订', '[\"native-result:thread-1\",\"review:test\"]', ?1
             )",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO agent_schedule_decisions (
                id, created_at, source, agent_id, agent_name_snapshot,
                workspace_scope_key, parent_thread_id, candidate_thread_id,
                decision, reason_code, runtime_fingerprint, cache_hint, claimed,
                task_scope_key, job_id, supersedes_decision_id
             ) VALUES (
                'decision-2', ?1, 'CAS', 'agent-1', 'Executor', 'c:/workspace',
                'parent-1', 'thread-1', 'REUSE', 'REVISION_EXACT_THREAD',
                'fingerprint-1', 'UNKNOWN', 1, 'task-1', 'job-1', 'decision-1'
             )",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO runtime_delegation_leases (
                id, created_at, updated_at, agent_id, parent_thread_id,
                codex_agent_id, workspace_scope_key, task_scope_key,
                schedule_decision_id, state, expires_at, agent_type
             ) VALUES (
                'lease-2', ?1, ?1, 'agent-1', 'parent-1', 'thread-1',
                'c:/workspace', 'task-1', 'decision-2', 'PENDING',
                '2099-01-01T00:00:00.000Z', 'executor'
             )",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO job_attempts (
                attempt_id, job_id, attempt_no, previous_attempt_id,
                schedule_decision_id, lease_id, route_action,
                planned_execution_kind, thread_instance_id, state, created_at,
                updated_at, dispatch_recorded_at
             ) VALUES (
                'attempt-2', 'job-1', 2, 'attempt-1', 'decision-2', 'lease-2',
                'REUSE', 'MANAGED_WORKER', 'instance-1', 'DISPATCHING', ?1, ?1, ?1
             )",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO delivery_receipts (
                receipt_id, job_id, attempt_id, stage, evidence_source,
                evidence_ref, parent_thread_id, evidence_at, created_at
             ) VALUES (
                'receipt-dispatch-2', 'job-1', 'attempt-2', 'DISPATCH_RECORDED',
                'CAS_TRANSACTION', 'decision-2', 'parent-1', ?1, ?1
             )",
                [NOW],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE agent_thread_instances
             SET claim_lease_id='lease-2', claimed_until='2099-01-01T00:00:00.000Z'
             WHERE id='instance-1'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "UPDATE orchestration_jobs SET state='DISPATCHED', updated_at=?1
             WHERE job_id='job-1'",
                [NOW],
            )
            .unwrap();
        drop(connection);

        let mut evidence = managed_evidence();
        evidence.attempt_id = "attempt-2".to_owned();
        evidence.turn_id = "turn-2".to_owned();
        evidence.evidence_ref = "thread/resume+turn/start:thread-1:turn-2".to_owned();
        evidence
    }

    #[test]
    fn managed_acceptance_reconciles_an_earlier_success_event_idempotently() {
        let path = database_path();
        seed(&path, false);
        let service = OrchestrationReceiptEventService::open(&path).unwrap();
        service
            .observe_runtime_event(
                &NormalizedRuntimeEvent::TurnFinished {
                    thread_id: "thread-1".to_owned(),
                    turn_id: "turn-1".to_owned(),
                    successful: true,
                    failure_message: None,
                    profile: ProtocolProfile::Modern,
                },
                r#"{"method":"turn/completed"}"#,
            )
            .unwrap();
        service.accept_managed(managed_evidence()).unwrap();
        service.accept_managed(managed_evidence()).unwrap();
        let connection = open_database(&path).unwrap();
        let stages = connection
            .prepare(
                "SELECT stage FROM delivery_receipts WHERE attempt_id = 'attempt-1' ORDER BY rowid",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            stages,
            ["DISPATCH_RECORDED", "TURN_ACCEPTED", "RESULT_OBSERVED"]
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM job_attempts WHERE attempt_id = 'attempt-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "SUCCEEDED"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT codex_agent_id FROM runtime_delegation_leases WHERE id = 'lease-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "thread-1"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM orchestration_jobs WHERE job_id = 'job-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "REVIEW_PENDING"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT status || ':' || reuse_state FROM agent_thread_instances
                     WHERE codex_thread_id = 'thread-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "IDLE:HELD_FOR_REVIEW"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM runtime_delegation_leases WHERE id = 'lease-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "ACTIVE"
        );
        drop(connection);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn native_revision_thread_can_resume_as_a_new_managed_attempt() {
        let path = database_path();
        let evidence = seed_native_revision(&path);
        let service = OrchestrationReceiptEventService::open(&path).unwrap();

        service.accept_managed(evidence).unwrap();

        let connection = open_database(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT execution_kind || ':' || state
                 FROM job_attempts WHERE attempt_id='attempt-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "NATIVE_CHILD:SUCCEEDED"
        );
        let previous_receipts = connection
            .prepare(
                "SELECT stage || ':' || COALESCE(execution_kind, 'NULL') || ':' || evidence_ref
                 FROM delivery_receipts WHERE attempt_id='attempt-1' ORDER BY rowid",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            previous_receipts,
            [
                "DISPATCH_RECORDED:NULL:decision-1",
                "TURN_ACCEPTED:NATIVE_CHILD:native-parent-child:thread-1",
                "RESULT_OBSERVED:NATIVE_CHILD:native-result:thread-1",
            ]
        );
        assert_eq!(
            connection.query_row(
                "SELECT execution_kind || ':' || state || ':' || thread_instance_id || ':' || codex_turn_id
                 FROM job_attempts WHERE attempt_id='attempt-2'",
                [],
                |row| row.get::<_, String>(0),
            ).unwrap(),
            "MANAGED_WORKER:ACCEPTED:instance-1:turn-2"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT execution_kind || ':' || status
                 FROM agent_thread_instances WHERE id='instance-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "MANAGED_WORKER:RUNNING"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM delivery_receipts
                 WHERE attempt_id='attempt-2' AND stage='TURN_ACCEPTED'
                   AND execution_kind='MANAGED_WORKER' AND codex_turn_id='turn-2'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        drop(connection);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn native_thread_without_exact_revision_chain_cannot_become_managed() {
        let path = database_path();
        seed(&path, false);
        let connection = open_database(&path).unwrap();
        insert_native_thread(&connection);
        connection
            .execute(
                "UPDATE job_attempts SET thread_instance_id='instance-1'
             WHERE attempt_id='attempt-1'",
                [],
            )
            .unwrap();
        drop(connection);
        let service = OrchestrationReceiptEventService::open(&path).unwrap();

        let error = service.accept_managed(managed_evidence()).unwrap_err();

        assert_eq!(error.code, OrchestrationErrorCode::ExecutionKindMismatch);
        let connection = open_database(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT execution_kind FROM agent_thread_instances WHERE id='instance-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "NATIVE_CHILD"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM job_attempts WHERE attempt_id='attempt-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "DISPATCHING"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM delivery_receipts
                 WHERE attempt_id='attempt-1' AND stage='TURN_ACCEPTED'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        drop(connection);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn native_parent_child_requires_full_identity_and_consumes_raw_event_after_binding() {
        let path = database_path();
        seed(&path, true);
        let service = OrchestrationReceiptEventService::open(&path).unwrap();
        service
            .observe_runtime_event(
                &NormalizedRuntimeEvent::ParentChild {
                    parent_thread_id: "parent-1".to_owned(),
                    child_thread_ids: vec!["child-1".to_owned()],
                    model_slug: None,
                    profile: ProtocolProfile::Modern,
                },
                r#"{"method":"item/started"}"#,
            )
            .unwrap();
        let connection = open_database(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT execution_kind FROM job_attempts WHERE attempt_id = 'attempt-1'",
                    [],
                    |row| row.get::<_, Option<String>>(0)
                )
                .unwrap()
                .as_deref(),
            Some("NATIVE_CHILD")
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM runtime_delegation_leases WHERE id = 'lease-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "ACTIVE"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT codex_agent_id FROM runtime_delegation_leases WHERE id = 'lease-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "child-1"
        );
        assert_eq!(connection.query_row("SELECT COUNT(*) FROM delivery_receipts WHERE attempt_id = 'attempt-1' AND stage = 'TURN_ACCEPTED'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
        drop(connection);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn native_attempt_does_not_consume_a_terminal_event_older_than_dispatch() {
        let path = database_path();
        seed(&path, true);
        let connection = open_database(&path).unwrap();
        connection
            .execute(
                "UPDATE job_attempts
                 SET dispatch_recorded_at='2000-01-02T00:00:00.000Z'
                 WHERE attempt_id='attempt-1'",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO runtime_receipt_events (
                    event_id, event_key, event_type, raw_event, schema_profile,
                    codex_thread_id, codex_turn_id, successful, observed_at
                 ) VALUES (
                    'event-old', 'TURN_FINISHED:child-1:turn-old:true',
                    'TURN_FINISHED', '{}', 'MODERN', 'child-1', 'turn-old', 1,
                    '2000-01-01T00:00:00.000Z'
                 )",
                [],
            )
            .unwrap();
        drop(connection);
        let service = OrchestrationReceiptEventService::open(&path).unwrap();

        service
            .observe_runtime_event(
                &NormalizedRuntimeEvent::ParentChild {
                    parent_thread_id: "parent-1".to_owned(),
                    child_thread_ids: vec!["child-1".to_owned()],
                    model_slug: None,
                    profile: ProtocolProfile::Modern,
                },
                r#"{"method":"item/started"}"#,
            )
            .unwrap();

        let connection = open_database(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM job_attempts WHERE attempt_id='attempt-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "ACCEPTED"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM delivery_receipts
                     WHERE attempt_id='attempt-1' AND stage='RESULT_OBSERVED'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        drop(connection);

        service
            .observe_runtime_event(
                &NormalizedRuntimeEvent::TurnFinished {
                    thread_id: "child-1".to_owned(),
                    turn_id: "turn-new".to_owned(),
                    successful: true,
                    failure_message: None,
                    profile: ProtocolProfile::Modern,
                },
                r#"{"method":"turn/completed"}"#,
            )
            .unwrap();

        let connection = open_database(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT state || ':' || codex_turn_id FROM job_attempts
                     WHERE attempt_id='attempt-1'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "SUCCEEDED:turn-new"
        );
        assert!(
            connection
                .query_row(
                    "SELECT processed_at IS NULL FROM runtime_receipt_events
                     WHERE event_id='event-old'",
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap()
        );
        drop(connection);
        drop(service);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn native_parent_child_with_missing_task_scope_stays_unknown() {
        let path = database_path();
        seed(&path, true);
        let connection = open_database(&path).unwrap();
        connection
            .execute(
                "UPDATE agent_thread_instances SET task_scope_key = NULL WHERE id = 'instance-1'",
                [],
            )
            .unwrap();
        drop(connection);
        let service = OrchestrationReceiptEventService::open(&path).unwrap();
        service
            .observe_runtime_event(
                &NormalizedRuntimeEvent::ParentChild {
                    parent_thread_id: "parent-1".to_owned(),
                    child_thread_ids: vec!["child-1".to_owned()],
                    model_slug: None,
                    profile: ProtocolProfile::Modern,
                },
                r#"{"method":"item/started"}"#,
            )
            .unwrap();
        let connection = open_database(&path).unwrap();
        assert_eq!(
            connection
                .query_row(
                    "SELECT execution_kind FROM job_attempts WHERE attempt_id = 'attempt-1'",
                    [],
                    |row| row.get::<_, Option<String>>(0)
                )
                .unwrap(),
            None
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT state FROM runtime_delegation_leases WHERE id = 'lease-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "PENDING"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM delivery_receipts WHERE attempt_id = 'attempt-1'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        drop(connection);
        drop(service);
        let _ = std::fs::remove_file(path);
    }
}
