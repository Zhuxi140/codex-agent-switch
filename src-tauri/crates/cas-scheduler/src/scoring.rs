//! 复用候选的纯函数软评分。硬门槛始终先于此模块中的任何分数。

use std::cmp::Reverse;

use crate::hard_gates::{
    CacheState, CandidateHardGateInput, ContextPressure, HardGateContext, HardGateDisposition,
    HardGateReasonCode, evaluate_hard_gates, evaluate_spawn_hard_gates,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReuseStrategy {
    Hot,
    Auto,
    Cold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoringPolicy {
    pub reuse_strategy: ReuseStrategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheEvidenceSource {
    AgentOverride,
    Provider,
    Observed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoringCandidate {
    pub hard_gate: CandidateHardGateInput,
    /// 距最后一次可证明模型使用的秒数；未知时不伪造新鲜度。
    pub age_seconds: Option<u64>,
    /// 运行时已观察到的 cached input token 数；未知时保持 `None`。
    pub cached_input_tokens: Option<u64>,
    pub cache_evidence_source: CacheEvidenceSource,
    /// 同一 `thread_id` 的持久化实例或调用方提供的稳定最终键。
    pub stable_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScoredReuseCandidate {
    pub thread_id: String,
    pub stable_key: String,
    pub score: i64,
    pub age_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReuseSelection {
    Reuse(ScoredReuseCandidate),
    SpawnAllowed { reason: Option<HardGateReasonCode> },
    Blocked { reason: HardGateReasonCode },
}

/// 策略权重：HOT=`recent*3 + cache*3 + cached_input*2 + context`，
/// AUTO 各分量权重均为 1，COLD=`recent + cache + cached_input + context*3`。
/// 最终键固定为 `score DESC, age_seconds ASC (unknown last), thread_id ASC, stable_key ASC`。
pub fn select_reuse(
    context: &HardGateContext,
    policy: ScoringPolicy,
    candidates: &[ScoringCandidate],
) -> ReuseSelection {
    let mut eligible = Vec::new();
    let mut rejected = Vec::new();
    let mut blocked = Vec::new();

    for candidate in candidates {
        let evaluation = evaluate_hard_gates(context, &candidate.hard_gate);
        match evaluation.disposition {
            HardGateDisposition::EligibleForReuse => eligible.push(ScoredReuseCandidate {
                thread_id: candidate.hard_gate.thread_id.clone(),
                stable_key: candidate.stable_key.clone(),
                score: soft_score(candidate, policy),
                age_seconds: candidate.age_seconds,
            }),
            HardGateDisposition::CandidateRejected { .. } => {
                if let Some(reason) = evaluation.reason_code {
                    rejected.push((reason, candidate_sort_key(candidate)));
                }
            }
            HardGateDisposition::Blocked { .. } => {
                if let Some(reason) = evaluation.reason_code {
                    blocked.push((reason, candidate_sort_key(candidate)));
                }
            }
            HardGateDisposition::SpawnAllowed => {}
        }
    }

    if !blocked.is_empty() {
        blocked.sort_by(|left, right| left.1.cmp(&right.1));
        return ReuseSelection::Blocked {
            reason: blocked[0].0,
        };
    }
    if eligible.is_empty() {
        rejected.sort_by(|left, right| left.1.cmp(&right.1));
        let spawn = evaluate_spawn_hard_gates(context);
        return match spawn.disposition {
            HardGateDisposition::SpawnAllowed => ReuseSelection::SpawnAllowed {
                reason: rejected.first().map(|(reason, _)| *reason),
            },
            HardGateDisposition::Blocked { .. } => ReuseSelection::Blocked {
                reason: spawn.reason_code.expect("blocked spawn gate has a reason"),
            },
            _ => unreachable!("spawn gate only returns SpawnAllowed or Blocked"),
        };
    }

    eligible.sort_by_key(|candidate| {
        (
            Reverse(candidate.score),
            candidate.age_seconds.is_none(),
            candidate.age_seconds.unwrap_or(u64::MAX),
            candidate.thread_id.clone(),
            candidate.stable_key.clone(),
        )
    });
    ReuseSelection::Reuse(eligible.remove(0))
}

fn soft_score(candidate: &ScoringCandidate, policy: ScoringPolicy) -> i64 {
    let (recency_weight, cache_weight, cached_input_weight, context_weight) =
        match policy.reuse_strategy {
            ReuseStrategy::Hot => (3, 3, 2, 1),
            ReuseStrategy::Auto => (1, 1, 1, 1),
            ReuseStrategy::Cold => (1, 1, 1, 3),
        };
    recency_score(candidate.age_seconds) * recency_weight
        + cache_score(
            candidate.hard_gate.cache_state,
            candidate.cache_evidence_source,
        ) * cache_weight
        + cached_input_score(candidate.cached_input_tokens) * cached_input_weight
        + context_pressure_score(candidate.hard_gate.context_pressure) * context_weight
}

fn candidate_sort_key(candidate: &ScoringCandidate) -> (String, String) {
    (
        candidate.hard_gate.thread_id.clone(),
        candidate.stable_key.clone(),
    )
}

fn recency_score(age_seconds: Option<u64>) -> i64 {
    age_seconds
        .map(|age| 100_i64.saturating_sub(i64::try_from(age.min(100)).unwrap_or(100)))
        .unwrap_or(0)
}

fn cache_score(state: CacheState, source: CacheEvidenceSource) -> i64 {
    match state {
        CacheState::WithinRequiredWindow => 30 + cache_source_score(source),
        CacheState::OutsideRequiredWindow => -10,
        CacheState::Unknown => -5,
        CacheState::NotObserved => -15,
    }
}

fn cache_source_score(source: CacheEvidenceSource) -> i64 {
    match source {
        CacheEvidenceSource::Observed => 12,
        CacheEvidenceSource::Provider => 8,
        CacheEvidenceSource::AgentOverride => 4,
        CacheEvidenceSource::Unknown => 0,
    }
}

fn cached_input_score(tokens: Option<u64>) -> i64 {
    tokens
        .map(|value| (value / 1_000).min(20) as i64)
        .unwrap_or(0)
}

fn context_pressure_score(pressure: ContextPressure) -> i64 {
    match pressure {
        ContextPressure::Known {
            observed_percent, ..
        } => i64::from(100_u8.saturating_sub(observed_percent) / 5),
        ContextPressure::Unknown => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hard_gates::{
        AgentExecutionInput, CacheRequirement, CandidateHardGateInput, CandidateScope, Capability,
        ContextHealth, ExclusionInput, ExecutionCapabilityInput, ExecutionKind, RequiredIdentity,
        RequiredScope, ThreadState,
    };

    fn context() -> HardGateContext {
        HardGateContext {
            agent: AgentExecutionInput {
                active: true,
                enabled: true,
                model_available: true,
                provider_available: true,
            },
            exclusions: ExclusionInput {
                workspace_excluded: false,
                project_excluded: false,
                conversation_excluded: false,
            },
            execution: ExecutionCapabilityInput {
                selected_kind: ExecutionKind::ManagedWorker,
                runtime: Capability::Supported,
                agent_execution: Capability::Supported,
                event: Capability::Supported,
            },
            required_identity: RequiredIdentity {
                agent_id: "agent".into(),
                runtime_fingerprint: Some("fingerprint".into()),
            },
            required_scope: RequiredScope {
                workspace_scope_key: "workspace".into(),
                parent_thread_id: "parent".into(),
                task_scope_key: "task".into(),
            },
            cache_requirement: CacheRequirement::NotRequired,
            requested_agent_type: "worker".into(),
            conflicting_agent_type_lease: false,
        }
    }

    fn policy() -> ScoringPolicy {
        ScoringPolicy {
            reuse_strategy: ReuseStrategy::Auto,
        }
    }

    fn candidate(thread_id: &str) -> ScoringCandidate {
        ScoringCandidate {
            hard_gate: CandidateHardGateInput {
                thread_id: thread_id.into(),
                agent_id: "agent".into(),
                runtime_fingerprint: Some("fingerprint".into()),
                execution_kind: ExecutionKind::ManagedWorker,
                scope: CandidateScope {
                    workspace_scope_key: "workspace".into(),
                    parent_thread_id: "parent".into(),
                    task_scope_key: "task".into(),
                },
                thread_state: ThreadState::Idle,
                context_health: ContextHealth::Healthy,
                active_claim: false,
                active_lease: false,
                recovery_required: false,
                review_pending: false,
                context_pressure: ContextPressure::Known {
                    observed_percent: 20,
                    limit_percent: 80,
                },
                cache_state: CacheState::WithinRequiredWindow,
            },
            age_seconds: Some(10),
            cached_input_tokens: Some(5_000),
            cache_evidence_source: CacheEvidenceSource::Provider,
            stable_key: format!("instance-{thread_id}"),
        }
    }

    fn selected_thread(result: ReuseSelection) -> String {
        match result {
            ReuseSelection::Reuse(candidate) => candidate.thread_id,
            other => panic!("expected reuse, got {other:?}"),
        }
    }

    #[test]
    fn high_score_cannot_overcome_a_hard_gate_failure() {
        let mut rejected = candidate("rejected");
        rejected.age_seconds = Some(0);
        rejected.cached_input_tokens = Some(1_000_000);
        rejected.hard_gate.thread_state = ThreadState::Active;
        let mut eligible = candidate("eligible");
        eligible.age_seconds = Some(100);
        eligible.cached_input_tokens = None;

        assert_eq!(
            selected_thread(select_reuse(&context(), policy(), &[rejected, eligible])),
            "eligible"
        );
    }

    #[test]
    fn each_soft_factor_can_change_the_order() {
        let hard_context = context();
        let baseline = candidate("baseline");
        let mut recency = baseline.clone();
        recency.hard_gate.thread_id = "recency".into();
        recency.age_seconds = Some(0);
        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                policy(),
                &[baseline.clone(), recency]
            )),
            "recency"
        );

        let mut cache = baseline.clone();
        cache.hard_gate.thread_id = "cache".into();
        cache.hard_gate.cache_state = CacheState::OutsideRequiredWindow;
        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                policy(),
                &[baseline.clone(), cache]
            )),
            "baseline"
        );

        let mut cached_input = baseline.clone();
        cached_input.hard_gate.thread_id = "cached".into();
        cached_input.cached_input_tokens = Some(20_000);
        let mut no_cached_input = baseline.clone();
        no_cached_input.cached_input_tokens = None;
        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                policy(),
                &[no_cached_input, cached_input]
            )),
            "cached"
        );

        let mut context = baseline.clone();
        context.hard_gate.thread_id = "context".into();
        context.hard_gate.context_pressure = ContextPressure::Known {
            observed_percent: 0,
            limit_percent: 80,
        };
        let mut pressured = baseline.clone();
        pressured.hard_gate.context_pressure = ContextPressure::Known {
            observed_percent: 79,
            limit_percent: 80,
        };
        assert_eq!(
            selected_thread(select_reuse(&hard_context, policy(), &[pressured, context])),
            "context"
        );
    }

    #[test]
    fn choice_is_independent_of_input_order_and_uses_stable_ties() {
        let mut older = candidate("z-thread");
        older.age_seconds = Some(20);
        let mut newer = candidate("z-thread");
        newer.age_seconds = Some(10);
        newer.stable_key = "z".into();
        let mut equal = candidate("a-thread");
        equal.age_seconds = Some(10);
        equal.stable_key = "a".into();
        let first = selected_thread(select_reuse(
            &context(),
            policy(),
            &[older.clone(), newer.clone(), equal.clone()],
        ));
        let second = selected_thread(select_reuse(&context(), policy(), &[equal, newer, older]));
        assert_eq!(first, "a-thread");
        assert_eq!(first, second);
    }

    #[test]
    fn expired_cache_best_effort_reuses_but_cache_required_does_not_score() {
        let mut expired = candidate("expired");
        expired.hard_gate.cache_state = CacheState::OutsideRequiredWindow;
        assert_eq!(
            selected_thread(select_reuse(&context(), policy(), &[expired.clone()])),
            "expired"
        );
        let mut cache_required = context();
        cache_required.cache_requirement = CacheRequirement::CacheRequired;
        assert_eq!(
            select_reuse(&cache_required, policy(), &[expired]),
            ReuseSelection::SpawnAllowed {
                reason: Some(HardGateReasonCode::CacheRequiredUnavailable)
            }
        );
    }

    #[test]
    fn no_eligible_candidate_allows_spawn_but_request_block_wins() {
        let mut rejected = candidate("rejected");
        rejected.hard_gate.thread_state = ThreadState::Active;
        assert!(matches!(
            select_reuse(&context(), policy(), &[rejected.clone()]),
            ReuseSelection::SpawnAllowed { .. }
        ));
        let mut blocked = context();
        blocked.exclusions.workspace_excluded = true;
        assert_eq!(
            select_reuse(&blocked, policy(), &[rejected]),
            ReuseSelection::Blocked {
                reason: HardGateReasonCode::WorkspaceExcluded
            }
        );
    }

    #[test]
    fn unknown_recency_and_cache_are_stable_and_safe() {
        let mut unknown = candidate("unknown");
        unknown.age_seconds = None;
        unknown.hard_gate.cache_state = CacheState::Unknown;
        unknown.cache_evidence_source = CacheEvidenceSource::Unknown;
        let mut known = candidate("known");
        known.age_seconds = Some(10);
        known.hard_gate.cache_state = CacheState::NotObserved;
        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                policy(),
                &[unknown.clone(), known.clone()]
            )),
            "known"
        );
        assert_eq!(
            selected_thread(select_reuse(&context(), policy(), &[known, unknown])),
            "known"
        );
    }

    #[test]
    fn strategy_weights_change_selection() {
        let mut fresh_cache_hint = candidate("fresh-cache-hint");
        fresh_cache_hint.age_seconds = Some(0);
        fresh_cache_hint.cached_input_tokens = None;
        fresh_cache_hint.cache_evidence_source = CacheEvidenceSource::Unknown;
        fresh_cache_hint.hard_gate.context_pressure = ContextPressure::Known {
            observed_percent: 75,
            limit_percent: 80,
        };
        let mut low_pressure = candidate("low-pressure");
        low_pressure.age_seconds = Some(30);
        low_pressure.cached_input_tokens = None;
        low_pressure.cache_evidence_source = CacheEvidenceSource::Observed;
        low_pressure.hard_gate.context_pressure = ContextPressure::Known {
            observed_percent: 0,
            limit_percent: 80,
        };

        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                ScoringPolicy {
                    reuse_strategy: ReuseStrategy::Hot,
                },
                &[fresh_cache_hint.clone(), low_pressure.clone()],
            )),
            "fresh-cache-hint"
        );
        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                ScoringPolicy {
                    reuse_strategy: ReuseStrategy::Cold,
                },
                &[fresh_cache_hint, low_pressure],
            )),
            "low-pressure"
        );
    }

    #[test]
    fn cache_evidence_trust_independently_changes_order() {
        let mut observed = candidate("observed");
        observed.cache_evidence_source = CacheEvidenceSource::Observed;
        let mut provider = candidate("provider");
        provider.cache_evidence_source = CacheEvidenceSource::Provider;
        let mut override_hint = candidate("override");
        override_hint.cache_evidence_source = CacheEvidenceSource::AgentOverride;
        let mut unknown = candidate("unknown");
        unknown.cache_evidence_source = CacheEvidenceSource::Unknown;

        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                policy(),
                &[unknown, override_hint, provider, observed],
            )),
            "observed"
        );
    }

    #[test]
    fn ties_use_age_then_thread_then_stable_key_across_permutations() {
        let mut age_101 = candidate("z-thread");
        age_101.age_seconds = Some(101);
        let mut age_200 = candidate("a-thread");
        age_200.age_seconds = Some(200);
        assert_eq!(
            selected_thread(select_reuse(
                &context(),
                policy(),
                &[age_200.clone(), age_101.clone()]
            )),
            "z-thread"
        );

        let mut thread_a = age_101.clone();
        thread_a.hard_gate.thread_id = "a-thread".into();
        let mut stable_z = thread_a.clone();
        stable_z.stable_key = "z".into();
        let mut stable_a = thread_a;
        stable_a.stable_key = "a".into();
        let first = select_reuse(
            &context(),
            policy(),
            &[stable_z.clone(), age_101.clone(), stable_a.clone()],
        );
        let second = select_reuse(
            &context(),
            policy(),
            &[age_101, stable_a.clone(), stable_z.clone()],
        );
        let third = select_reuse(&context(), policy(), &[stable_a, stable_z]);
        for result in [first, second, third] {
            match result {
                ReuseSelection::Reuse(candidate) => {
                    assert_eq!(candidate.thread_id, "a-thread");
                    assert_eq!(candidate.stable_key, "a");
                }
                other => panic!("expected reuse, got {other:?}"),
            }
        }
    }

    #[test]
    fn empty_candidates_use_spawn_gate() {
        assert_eq!(
            select_reuse(&context(), policy(), &[]),
            ReuseSelection::SpawnAllowed { reason: None }
        );
        let mut blocked = context();
        blocked.conflicting_agent_type_lease = true;
        assert_eq!(
            select_reuse(&blocked, policy(), &[]),
            ReuseSelection::Blocked {
                reason: HardGateReasonCode::AgentTypeLeaseConflict
            }
        );
    }
}
