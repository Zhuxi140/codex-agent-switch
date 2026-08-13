#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub instance_id: String,
    pub thread_id: String,
    pub status: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub context_window: Option<i64>,
    pub age_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub reuse_strategy: String,
    pub cache_support: String,
    pub cache_retention_type: String,
    pub cache_retention_hint_seconds: Option<i64>,
    pub agent_cache_retention_override_seconds: Option<i64>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            reuse_strategy: "AUTO".to_owned(),
            cache_support: "UNKNOWN".to_owned(),
            cache_retention_type: "UNKNOWN".to_owned(),
            cache_retention_hint_seconds: None,
            agent_cache_retention_override_seconds: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub decision: &'static str,
    pub reason_code: &'static str,
    pub message: &'static str,
    pub scope_key: String,
    pub candidate_instance_id: Option<String>,
    pub candidate_thread_id: Option<String>,
    pub context_pressure_percent: Option<i64>,
    pub context_pressure_limit_percent: i64,
    pub reuse_strategy: String,
    pub cache_support: String,
    pub cache_retention_type: String,
    pub cache_retention_hint_seconds: Option<i64>,
    pub cache_retention_source: &'static str,
    pub cache_hint: &'static str,
    pub candidate_age_seconds: Option<i64>,
}

pub fn normalize_scope_key(value: &str) -> Option<String> {
    let value = value
        .trim()
        .strip_prefix(r"\\?\")
        .or_else(|| value.trim().strip_prefix("//?/"))
        .unwrap_or(value.trim())
        .replace('\\', "/")
        .to_lowercase();
    let value = if value.starts_with("//") {
        format!("unc/{}", value.trim_start_matches('/'))
    } else if value.starts_with('/') {
        format!("root/{}", value.trim_start_matches('/'))
    } else {
        value
    };
    let value = value.trim_end_matches('/').to_owned();
    (!value.is_empty()
        && value.len() <= 200
        && !value.chars().any(char::is_control)
        && !value.contains("//"))
    .then_some(value)
}

pub fn recommend(
    scope_key: String,
    candidates: Vec<Candidate>,
    profile: Profile,
) -> Recommendation {
    if let Some(candidate) = candidates.iter().find(|candidate| {
        candidate.status == "IDLE"
            && context_pressure_percent(candidate).map_or(true, |percent| {
                percent < context_pressure_limit(&profile, candidate.age_seconds)
            })
    }) {
        return recommendation(
            "REUSE",
            "EXACT_SCOPE_IDLE",
            "存在同一 Agent、Scope 完全一致且符合当前复用偏好的空闲 Thread。",
            scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.status == "IDLE")
    {
        let base_limit = base_context_pressure_limit(&profile.reuse_strategy);
        let limit = context_pressure_limit(&profile, candidate.age_seconds);
        let pressure = context_pressure_percent(candidate);
        let cache_adjusted = limit < base_limit && pressure.is_some_and(|value| value < base_limit);
        return recommendation(
            "SPAWN",
            if cache_adjusted {
                "CACHE_HINT_PRESSURE"
            } else {
                "CONTEXT_PRESSURE"
            },
            if cache_adjusted {
                "Provider 缓存提示已降低复用倾向，当前 Context 压力建议新建 Thread。"
            } else {
                "同 Scope 的空闲 Thread 累计输入已达到当前策略阈值，建议新建。"
            },
            scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates.first() {
        return recommendation(
            "SPAWN",
            "NO_HEALTHY_IDLE_THREAD",
            "同 Scope Thread 当前不可安全复用，建议新建。",
            scope_key,
            Some(candidate),
            &profile,
        );
    }
    recommendation(
        "SPAWN",
        "NO_SCOPE_MATCH",
        "没有同一 Agent 且 Scope 完全一致的 Thread，建议新建。",
        scope_key,
        None,
        &profile,
    )
}

fn recommendation(
    decision: &'static str,
    reason_code: &'static str,
    message: &'static str,
    scope_key: String,
    candidate: Option<&Candidate>,
    profile: &Profile,
) -> Recommendation {
    let age_seconds = candidate.map(|candidate| candidate.age_seconds);
    let (cache_retention_hint_seconds, cache_retention_source) = effective_cache_retention(profile);
    Recommendation {
        decision,
        reason_code,
        message,
        scope_key,
        candidate_instance_id: candidate.map(|candidate| candidate.instance_id.clone()),
        candidate_thread_id: candidate.map(|candidate| candidate.thread_id.clone()),
        context_pressure_percent: candidate.and_then(context_pressure_percent),
        context_pressure_limit_percent: context_pressure_limit(
            profile,
            age_seconds.unwrap_or_default(),
        ),
        reuse_strategy: profile.reuse_strategy.clone(),
        cache_support: profile.cache_support.clone(),
        cache_retention_type: profile.cache_retention_type.clone(),
        cache_retention_hint_seconds,
        cache_retention_source,
        cache_hint: cache_hint(profile, age_seconds),
        candidate_age_seconds: age_seconds,
    }
}

pub fn context_pressure_limit(profile: &Profile, age_seconds: i64) -> i64 {
    let base = base_context_pressure_limit(&profile.reuse_strategy);
    if profile.reuse_strategy == "HOT" {
        return base;
    }
    let (retention, _) = effective_cache_retention(profile);
    let cache_penalty = profile.cache_support == "UNSUPPORTED"
        || retention.is_some_and(|retention| age_seconds > retention);
    if cache_penalty { base - 10 } else { base }
}

pub fn cache_hint(profile: &Profile, age_seconds: Option<i64>) -> &'static str {
    if profile.cache_support == "UNSUPPORTED" {
        return "UNSUPPORTED";
    }
    match (effective_cache_retention(profile).0, age_seconds) {
        (Some(retention), Some(age)) if age <= retention => "WITHIN_RETENTION_HINT",
        (Some(_), Some(_)) => "OUTSIDE_RETENTION_HINT",
        (None, _) if profile.cache_support == "SUPPORTED" => "SUPPORTED_NO_RETENTION_HINT",
        _ => "UNKNOWN",
    }
}

pub fn effective_cache_retention(profile: &Profile) -> (Option<i64>, &'static str) {
    if profile.cache_support == "UNSUPPORTED" {
        return (None, "PROVIDER");
    }
    match (
        profile.agent_cache_retention_override_seconds,
        profile.cache_retention_hint_seconds,
    ) {
        (Some(agent), Some(provider)) if agent <= provider => (Some(agent), "AGENT_OVERRIDE"),
        (Some(_), Some(provider)) => (Some(provider), "PROVIDER"),
        (Some(agent), None) => (Some(agent), "AGENT_OVERRIDE"),
        (None, Some(provider)) => (Some(provider), "PROVIDER"),
        (None, None) => (None, "NONE"),
    }
}

fn base_context_pressure_limit(strategy: &str) -> i64 {
    match strategy {
        "HOT" => 90,
        "COLD" => 60,
        _ => 80,
    }
}

fn context_pressure_percent(candidate: &Candidate) -> Option<i64> {
    candidate.context_window.map(|window| {
        let tokens =
            if candidate.input_tokens + candidate.cached_input_tokens + candidate.output_tokens > 0
            {
                candidate.input_tokens
            } else {
                candidate.total_tokens
            };
        tokens
            .saturating_mul(100)
            .checked_div(window)
            .unwrap_or(100)
            .clamp(0, 100)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(status: &str, total_tokens: i64) -> Candidate {
        Candidate {
            instance_id: "instance-1".to_owned(),
            thread_id: "thread-1".to_owned(),
            status: status.to_owned(),
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            total_tokens,
            context_window: Some(100),
            age_seconds: 10,
        }
    }

    #[test]
    fn reuses_healthy_exact_scope_thread() {
        let result = recommend(
            "project/a".to_owned(),
            vec![candidate("IDLE", 50)],
            Profile::default(),
        );
        assert_eq!(result.decision, "REUSE");
        assert_eq!(result.candidate_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(result.context_pressure_percent, Some(50));
    }

    #[test]
    fn native_total_tokens_prevent_overfilled_reuse() {
        let result = recommend(
            "project/a".to_owned(),
            vec![candidate("IDLE", 80)],
            Profile::default(),
        );
        assert_eq!(result.decision, "SPAWN");
        assert_eq!(result.reason_code, "CONTEXT_PRESSURE");
    }

    #[test]
    fn scope_normalization_supports_real_cross_platform_paths() {
        assert_eq!(
            normalize_scope_key(r"\\?\C:\Workspace\Codex Agent Switch"),
            Some("c:/workspace/codex agent switch".to_owned())
        );
        assert_eq!(
            normalize_scope_key("/home/用户/My Project/"),
            Some("root/home/用户/my project".to_owned())
        );
        assert_eq!(
            normalize_scope_key(r"\\server\share\Project"),
            Some("unc/server/share/project".to_owned())
        );
        assert_eq!(normalize_scope_key("order//refund"), None);
    }
}
