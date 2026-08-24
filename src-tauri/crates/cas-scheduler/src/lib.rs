/// REUSE 短租约时长：覆盖「预检返回 REUSE」到「follow-up 使 Thread 进入 RUNNING」的窗口。
// ponytail: 固定 TTL，过短会在慢启动时放行第二个 REUSE；需按实测调整或改为显式释放。
pub const REUSE_CLAIM_TTL_SECONDS: i64 = 120;
pub const SPAWN_RESERVATION_TTL_SECONDS: i64 = 120;
const DELEGATED_AGENT_INSTRUCTIONS: &str = "你是由 Primary 委派的 Child Agent，不是 Primary。只完成 TASK 包中的一个可独立验收的工作单元，并把 GOAL、DECISIONS、ALLOW、DENY、TOOLS、CWD、ACCEPT、STOP 视为边界。不得扩展到相邻问题、额外重构、文档、提交、发布、依赖安装，或未明确授权的公开 API、数据模型与产品行为变更。缺少安全推进所需信息、需要未冻结决策、越过允许范围或磁盘事实冲突会改变方向时，立即停止并把控制权交还 Primary；不得猜测或重新执行 Primary 编排流程，也不得递归创建同职责子 Agent。首行必须返回 `RESULT: DONE`、`RESULT: NEEDS_DECISION`、`RESULT: PARTIAL` 或 `RESULT: BLOCKED`，且不得声称未实际验证的结果。";
const DELEGATED_TOOL_INSTRUCTIONS: &str = "工具契约：本地工具仍受阶段及 ALLOW/DENY 约束。外部 MCP、插件或连接器只可调用 TOOLS 明列且完成 ACCEPT 必需的项；`TOOLS: -` 表示禁用。任何外部写入、消息发送、发布、登录、授权或安装还必须由 ALLOW 明确许可，否则返回 `RESULT: NEEDS_DECISION`。Skill 只按任务匹配和已绑定规则加载，不得扫描或调用无关 Skill。";
const EXECUTION_PHASE_INSTRUCTIONS: &str = "阶段契约：EXECUTION。优先使用本地已有工具，只在 ALLOW 内做满足 ACCEPT 的最小实现及针对性验证；不负责需求规划、独立审查、发布或相邻清理。随后只报告适用的 `CHANGED`、`VERIFIED`、`REMAINING`、`EVIDENCE`、`NEXT`。";
const DISCOVERY_PHASE_INSTRUCTIONS: &str = "阶段契约：DISCOVERY。只读调查，不修改文件或外部状态，TOOLS 中的外部工具也仅可只读取证；证据应定位到文件、符号或行号，并区分事实、推断与未知项。随后只报告适用的 `EVIDENCE`、`INFERENCE`、`UNKNOWN`、`NEXT`。";
const REVIEW_PHASE_INSTRUCTIONS: &str = "阶段契约：REVIEW。只做独立审查，不修改文件或外部状态；仅报告可复现的正确性、安全性、回归或测试缺口，不报告纯风格偏好。每个 `FINDING` 包含严重度、位置、证据和影响；无问题时返回 `NO_FINDINGS`。";
const VERIFICATION_PHASE_INSTRUCTIONS: &str = "阶段契约：VERIFICATION。只复现或验证，不修改产品代码与文档；仅当 TASK 明确授权时可修改测试文件，测试和构建产物不视为产品修改。TOOLS 中的测试或浏览器工具仍不得造成未授权外部状态变更。随后只报告适用的 `COMMAND`、`OUTCOME`、`FAILURE`、`ARTIFACTS`、`NEXT`。";
const GENERIC_PHASE_INSTRUCTIONS: &str =
    "随后只报告适用的 `DONE`、`EVIDENCE`、`REMAINING`、`NEXT`。";

pub fn render_delegated_agent_instructions(instructions: &str) -> String {
    render_delegated_agent_instructions_for_phase(instructions, None)
}

pub fn render_delegated_agent_instructions_for_phase(
    instructions: &str,
    phase: Option<&str>,
) -> String {
    let phase_instructions = match phase {
        Some("EXECUTION") => EXECUTION_PHASE_INSTRUCTIONS,
        Some("DISCOVERY") => DISCOVERY_PHASE_INSTRUCTIONS,
        Some("REVIEW") => REVIEW_PHASE_INSTRUCTIONS,
        Some("VERIFICATION") => VERIFICATION_PHASE_INSTRUCTIONS,
        _ => GENERIC_PHASE_INSTRUCTIONS,
    };
    let delegated_instructions = format!(
        "{DELEGATED_AGENT_INSTRUCTIONS}\n{DELEGATED_TOOL_INSTRUCTIONS}\n{phase_instructions}"
    );
    if instructions.trim().is_empty() {
        delegated_instructions
    } else {
        format!("{}\n\n{}", instructions.trim_end(), delegated_instructions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub instance_id: String,
    pub thread_id: String,
    pub status: String,
    pub reuse_state: String,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub output_tokens: i64,
    pub total_tokens: i64,
    pub current_context_tokens: Option<i64>,
    pub context_window: Option<i64>,
    pub runtime_fingerprint: Option<String>,
    /// 距最近一次可证明的模型使用（`last_model_usage_at`）的秒数；
    /// `None` 表示只有观察时间、没有可证明的模型使用时间，缓存判定按未知处理。
    pub age_seconds: Option<i64>,
    /// 该 Thread 是否处于其他并发预检的有效 Claim 租约内。
    pub claimed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub reuse_strategy: String,
    pub orchestration_phase: Option<String>,
    pub cache_support: String,
    pub cache_retention_type: String,
    pub cache_retention_hint_seconds: Option<i64>,
    pub agent_cache_retention_override_seconds: Option<i64>,
    pub runtime_fingerprint: Option<String>,
}

pub fn runtime_fingerprint(fields: &[(&str, Vec<String>)]) -> String {
    let mut hash = Sha256::new();
    for (name, values) in fields {
        let mut values = values.clone();
        values.sort();
        values.dedup();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        for value in values {
            hash.update((value.len() as u64).to_le_bytes());
            hash.update(value.as_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}

pub fn skill_fingerprint_values(skill_keys: Vec<String>) -> Vec<String> {
    skill_keys
        .into_iter()
        .map(|key| {
            let revision = match key.as_str() {
                "caveman" => "full-v1",
                "caveman-slim" => "slim-v1",
                "ponytail" => "full-v1",
                "ponytail-slim" => "slim-v1",
                _ => "unknown-v1",
            };
            format!("{key}@{revision}")
        })
        .collect()
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            reuse_strategy: "AUTO".to_owned(),
            orchestration_phase: None,
            cache_support: "UNKNOWN".to_owned(),
            cache_retention_type: "UNKNOWN".to_owned(),
            cache_retention_hint_seconds: None,
            agent_cache_retention_override_seconds: None,
            runtime_fingerprint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recommendation {
    pub decision: &'static str,
    pub reason_code: &'static str,
    pub message: &'static str,
    pub workspace_scope_key: String,
    pub candidate_instance_id: Option<String>,
    pub candidate_thread_id: Option<String>,
    pub context_pressure_percent: Option<i64>,
    pub context_pressure_limit_percent: i64,
    pub reuse_strategy: String,
    pub effective_reuse_strategy: String,
    pub cache_support: String,
    pub cache_retention_type: String,
    pub cache_retention_hint_seconds: Option<i64>,
    pub cache_retention_source: &'static str,
    pub cache_hint: &'static str,
    pub candidate_age_seconds: Option<i64>,
}

pub fn normalize_workspace_scope_key(value: &str) -> Option<String> {
    let value = value
        .trim()
        .strip_prefix(r"\\?\")
        .or_else(|| value.trim().strip_prefix("//?/"))
        .unwrap_or(value.trim())
        .replace('\\', "/");
    let value = if value == "root" || value.starts_with("root/") {
        value
    } else if value == "unc" || value.starts_with("unc/") {
        value.to_lowercase()
    } else if value.starts_with("//") {
        format!("unc/{}", value.trim_start_matches('/')).to_lowercase()
    } else if value.starts_with('/') {
        format!("root/{}", value.trim_start_matches('/'))
    } else if value.len() >= 3 && value.as_bytes()[1] == b':' && value.as_bytes()[2] == b'/' {
        value.to_lowercase()
    } else if value.len() == 2 && value.as_bytes()[1] == b':' {
        value.to_lowercase()
    } else {
        return None;
    };
    let value = value.trim_end_matches('/').to_owned();
    (value != "unc"
        && !value.is_empty()
        && value.len() <= 200
        && !value.chars().any(char::is_control)
        && !value.contains("//"))
    .then_some(value)
}

pub fn normalize_task_scope_key(value: &str) -> Option<String> {
    let value = value.trim();
    let mut bytes = value.bytes();
    let valid = !value.is_empty()
        && value.len() <= 64
        && bytes
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    valid.then(|| value.to_owned())
}

pub fn effective_model_reasoning_efforts(
    reasoning_supported: Option<bool>,
    model_default_reasoning: Option<&str>,
    configured_efforts: &[String],
) -> Vec<String> {
    if reasoning_supported == Some(false) {
        return Vec::new();
    }
    if !configured_efforts.is_empty() {
        return configured_efforts.to_vec();
    }
    if reasoning_supported == Some(true) {
        return ["low", "medium", "high"]
            .into_iter()
            .map(str::to_owned)
            .collect();
    }
    vec![model_default_reasoning.unwrap_or("medium").to_owned()]
}

pub fn effective_model_default_reasoning<'a>(
    model_default_reasoning: Option<&str>,
    supported_efforts: &'a [String],
) -> Option<&'a str> {
    model_default_reasoning
        .and_then(|default| {
            supported_efforts
                .iter()
                .find(|effort| effort.as_str() == default)
        })
        .or_else(|| {
            supported_efforts
                .iter()
                .find(|effort| effort.as_str() == "medium")
        })
        .or_else(|| supported_efforts.first())
        .map(String::as_str)
}

pub fn resolve_agent_reasoning_effort(
    reasoning_policy: &str,
    model_default_reasoning: Option<&str>,
    supported_efforts: &[String],
) -> Option<String> {
    let requested_effort = match reasoning_policy {
        "LOW" => Some("low"),
        "MEDIUM" => Some("medium"),
        "HIGH" => Some("high"),
        _ => None,
    };
    let Some(requested_effort) = requested_effort else {
        return matches!(reasoning_policy, "MODEL_DEFAULT" | "INHERIT")
            .then(|| {
                effective_model_default_reasoning(model_default_reasoning, supported_efforts)
                    .map(str::to_owned)
            })
            .flatten();
    };
    if supported_efforts
        .iter()
        .any(|effort| effort == requested_effort)
    {
        return Some(requested_effort.to_owned());
    }
    let requested_rank = reasoning_effort_rank(requested_effort)?;
    supported_efforts
        .iter()
        .filter_map(|effort| {
            reasoning_effort_rank(effort)
                .filter(|rank| *rank <= requested_rank)
                .map(|rank| (rank, effort))
        })
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, effort)| effort.clone())
}

fn reasoning_effort_rank(effort: &str) -> Option<u8> {
    match effort {
        "minimal" => Some(0),
        "low" => Some(1),
        "medium" => Some(2),
        "high" => Some(3),
        "xhigh" => Some(4),
        "max" => Some(5),
        "ultra" => Some(6),
        _ => None,
    }
}

pub fn recommend(
    workspace_scope_key: String,
    candidates: Vec<Candidate>,
    profile: Profile,
) -> Recommendation {
    if let Some(candidate) = candidates.iter().find(|candidate| {
        candidate.reuse_state == "ACTIVE"
            && candidate.status == "IDLE"
            && !candidate.claimed
            && profile.runtime_fingerprint.is_some()
            && candidate.runtime_fingerprint == profile.runtime_fingerprint
            && context_pressure_percent(candidate).is_some_and(|percent| {
                percent < context_pressure_limit(&profile, candidate.age_seconds)
            })
    }) {
        return recommendation(
            "REUSE",
            "EXACT_WORKSPACE_SCOPE_IDLE",
            "存在同一 Agent、Workspace Scope 完全一致且符合当前复用偏好的空闲 Thread。",
            workspace_scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.reuse_state == "ACTIVE" && candidate.status == "IDLE")
    {
        if profile.runtime_fingerprint.is_none() || candidate.runtime_fingerprint.is_none() {
            return recommendation(
                "SPAWN",
                "RUNTIME_FINGERPRINT_UNKNOWN",
                "候选 Thread 或当前 Agent 的运行时配置未知，无法安全复用。",
                workspace_scope_key,
                Some(candidate),
                &profile,
            );
        }
        if candidate.runtime_fingerprint != profile.runtime_fingerprint {
            return recommendation(
                "SPAWN",
                if candidate.runtime_fingerprint.is_some() {
                    "RUNTIME_FINGERPRINT_MISMATCH"
                } else {
                    "RUNTIME_FINGERPRINT_UNKNOWN"
                },
                "候选 Thread 的运行时配置与当前 Agent 不一致或未知，无法安全复用。",
                workspace_scope_key,
                Some(candidate),
                &profile,
            );
        }
        if candidate.claimed {
            return recommendation(
                "SPAWN",
                "THREAD_CLAIMED",
                "同一 Workspace Scope 的空闲 Thread 刚被并发预检锁定，建议新建或稍后重试。",
                workspace_scope_key,
                Some(candidate),
                &profile,
            );
        }
        let base_limit = base_context_pressure_limit(effective_reuse_strategy(&profile));
        let limit = context_pressure_limit(&profile, candidate.age_seconds);
        let pressure = context_pressure_percent(candidate);
        if pressure.is_none() {
            return recommendation(
                "SPAWN",
                "CONTEXT_UNKNOWN",
                "当前 Context 或窗口未知，无法安全复用 Thread。",
                workspace_scope_key,
                Some(candidate),
                &profile,
            );
        }
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
                "同一 Workspace Scope 的空闲 Thread 当前 Context 已达到当前策略阈值，建议新建。"
            },
            workspace_scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates
        .iter()
        .find(|candidate| candidate.reuse_state == "ACTIVE")
    {
        return recommendation(
            "SPAWN",
            "NO_HEALTHY_IDLE_THREAD",
            "同一 Workspace Scope 的 Thread 当前不可安全复用，建议新建。",
            workspace_scope_key,
            Some(candidate),
            &profile,
        );
    }
    if let Some(candidate) = candidates.first() {
        return recommendation(
            "SPAWN",
            "CANDIDATE_RETIRED",
            "同一 Workspace Scope 的历史 Thread 已退出 CAS 复用池，建议新建。",
            workspace_scope_key,
            Some(candidate),
            &profile,
        );
    }
    recommendation(
        "SPAWN",
        "NO_WORKSPACE_SCOPE_MATCH",
        "没有同一 Agent 且 Workspace Scope 完全一致的 Thread，建议新建。",
        workspace_scope_key,
        None,
        &profile,
    )
}

fn recommendation(
    decision: &'static str,
    reason_code: &'static str,
    message: &'static str,
    workspace_scope_key: String,
    candidate: Option<&Candidate>,
    profile: &Profile,
) -> Recommendation {
    let age_seconds = candidate.map(|candidate| candidate.age_seconds).flatten();
    let (cache_retention_hint_seconds, cache_retention_source) = effective_cache_retention(profile);
    Recommendation {
        decision,
        reason_code,
        message,
        workspace_scope_key,
        candidate_instance_id: candidate.map(|candidate| candidate.instance_id.clone()),
        candidate_thread_id: candidate.map(|candidate| candidate.thread_id.clone()),
        context_pressure_percent: candidate.and_then(context_pressure_percent),
        context_pressure_limit_percent: context_pressure_limit(profile, age_seconds),
        reuse_strategy: profile.reuse_strategy.clone(),
        effective_reuse_strategy: effective_reuse_strategy(profile).to_owned(),
        cache_support: profile.cache_support.clone(),
        cache_retention_type: profile.cache_retention_type.clone(),
        cache_retention_hint_seconds,
        cache_retention_source,
        cache_hint: cache_hint(profile, age_seconds),
        candidate_age_seconds: age_seconds,
    }
}

pub fn context_pressure_limit(profile: &Profile, age_seconds: Option<i64>) -> i64 {
    let effective_strategy = effective_reuse_strategy(profile);
    let base = base_context_pressure_limit(effective_strategy);
    if effective_strategy == "HOT" {
        return base;
    }
    let (retention, _) = effective_cache_retention(profile);
    // 模型使用时间未知时无法证明仍在缓存窗口内，按已过窗口的保守方向处理。
    let cache_penalty = profile.cache_support == "UNSUPPORTED"
        || age_seconds.is_none_or(|age| retention.is_some_and(|retention| age > retention));
    if cache_penalty { base - 10 } else { base }
}

pub fn effective_reuse_strategy(profile: &Profile) -> &'static str {
    match profile.reuse_strategy.as_str() {
        "HOT" => "HOT",
        "COLD" => "COLD",
        _ => match profile.orchestration_phase.as_deref() {
            Some("DISCOVERY") => "HOT",
            Some("VERIFICATION" | "REVIEW") => "COLD",
            _ => "AUTO",
        },
    }
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
    candidate
        .current_context_tokens
        .zip(candidate.context_window)
        .map(|(tokens, window)| {
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

    #[test]
    fn delegated_agent_stops_at_one_reviewable_unit_and_returns_control() {
        let rendered = render_delegated_agent_instructions("保留角色规则。");

        assert!(rendered.starts_with("保留角色规则。"));
        assert!(rendered.contains("一个可独立验收的工作单元"));
        assert!(rendered.contains("GOAL、DECISIONS、ALLOW、DENY、TOOLS、CWD、ACCEPT、STOP"));
        assert!(rendered.contains("需要未冻结决策"));
        assert!(rendered.contains("RESULT: NEEDS_DECISION"));
        assert!(rendered.contains("`TOOLS: -` 表示禁用"));
        assert!(rendered.contains("外部写入、消息发送、发布、登录、授权或安装"));
        assert!(rendered.contains("不得扫描或调用无关 Skill"));
        assert!(rendered.contains("DONE"));
        assert!(rendered.contains("REMAINING"));
    }

    #[test]
    fn delegated_agent_contract_changes_with_orchestration_phase() {
        let execution =
            render_delegated_agent_instructions_for_phase("角色规则。", Some("EXECUTION"));
        let discovery =
            render_delegated_agent_instructions_for_phase("角色规则。", Some("DISCOVERY"));
        let review = render_delegated_agent_instructions_for_phase("角色规则。", Some("REVIEW"));
        let verification =
            render_delegated_agent_instructions_for_phase("角色规则。", Some("VERIFICATION"));

        assert!(execution.contains("阶段契约：EXECUTION"));
        assert!(execution.contains("优先使用本地已有工具"));
        assert!(execution.contains("最小实现及针对性验证"));
        assert!(discovery.contains("只读调查"));
        assert!(discovery.contains("外部工具也仅可只读取证"));
        assert!(discovery.contains("INFERENCE"));
        assert!(review.contains("不报告纯风格偏好"));
        assert!(review.contains("不修改文件或外部状态"));
        assert!(review.contains("NO_FINDINGS"));
        assert!(verification.contains("不修改产品代码与文档"));
        assert!(verification.contains("不得造成未授权外部状态变更"));
        assert!(verification.contains("COMMAND"));
    }

    fn candidate(status: &str, total_tokens: i64) -> Candidate {
        Candidate {
            instance_id: "instance-1".to_owned(),
            thread_id: "thread-1".to_owned(),
            status: status.to_owned(),
            reuse_state: "ACTIVE".to_owned(),
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: 0,
            total_tokens,
            current_context_tokens: Some(total_tokens),
            context_window: Some(100),
            runtime_fingerprint: Some("fingerprint".to_owned()),
            age_seconds: Some(10),
            claimed: false,
        }
    }

    #[test]
    fn retired_thread_is_never_reused() {
        let mut profile = Profile::default();
        profile.runtime_fingerprint = Some("fingerprint".to_owned());
        let mut retired = candidate("IDLE", 20);
        retired.reuse_state = "RETIRED".to_owned();

        let result = recommend("c:/workspace/project".to_owned(), vec![retired], profile);

        assert_eq!(result.decision, "SPAWN");
        assert_eq!(result.reason_code, "CANDIDATE_RETIRED");
    }

    #[test]
    fn claimed_idle_thread_spawns_until_lease_expires() {
        let mut profile = Profile::default();
        profile.runtime_fingerprint = Some("fingerprint".to_owned());
        let mut candidate = candidate("IDLE", 50);
        candidate.claimed = true;
        let result = recommend("c:/workspace/project".to_owned(), vec![candidate], profile);
        assert_eq!(result.decision, "SPAWN");
        assert_eq!(result.reason_code, "THREAD_CLAIMED");
    }

    #[test]
    fn reuses_healthy_exact_workspace_scope_thread() {
        let mut profile = Profile::default();
        profile.runtime_fingerprint = Some("fingerprint".to_owned());
        let mut candidate = candidate("IDLE", 50);
        candidate.runtime_fingerprint = Some("fingerprint".to_owned());
        let result = recommend("c:/workspace/project".to_owned(), vec![candidate], profile);
        assert_eq!(result.decision, "REUSE");
        assert_eq!(result.candidate_thread_id.as_deref(), Some("thread-1"));
        assert_eq!(result.context_pressure_percent, Some(50));
    }

    #[test]
    fn auto_reuse_strategy_is_role_aware_and_explicit_strategy_wins() {
        let mut profile = Profile::default();

        profile.orchestration_phase = Some("DISCOVERY".to_owned());
        assert_eq!(effective_reuse_strategy(&profile), "HOT");
        assert_eq!(context_pressure_limit(&profile, Some(10)), 90);

        profile.orchestration_phase = Some("EXECUTION".to_owned());
        assert_eq!(effective_reuse_strategy(&profile), "AUTO");
        assert_eq!(context_pressure_limit(&profile, Some(10)), 80);

        profile.orchestration_phase = Some("VERIFICATION".to_owned());
        assert_eq!(effective_reuse_strategy(&profile), "COLD");
        assert_eq!(context_pressure_limit(&profile, Some(10)), 60);

        profile.orchestration_phase = Some("REVIEW".to_owned());
        profile.reuse_strategy = "HOT".to_owned();
        assert_eq!(effective_reuse_strategy(&profile), "HOT");

        profile.orchestration_phase = Some("DISCOVERY".to_owned());
        profile.reuse_strategy = "COLD".to_owned();
        assert_eq!(effective_reuse_strategy(&profile), "COLD");
    }

    #[test]
    fn unknown_model_usage_age_is_unknown_cache_and_penalizes_limit() {
        let mut profile = Profile::default();
        profile.runtime_fingerprint = Some("fingerprint".to_owned());
        profile.cache_support = "SUPPORTED".to_owned();
        profile.cache_retention_hint_seconds = Some(300);
        let mut candidate = candidate("IDLE", 50);
        candidate.age_seconds = None;
        let result = recommend("c:/workspace/project".to_owned(), vec![candidate], profile);
        assert_eq!(result.decision, "REUSE");
        assert_eq!(result.cache_hint, "UNKNOWN");
        assert_eq!(result.candidate_age_seconds, None);
        assert_eq!(result.context_pressure_limit_percent, 70);

        let mut profile = Profile::default();
        profile.cache_support = "SUPPORTED".to_owned();
        profile.cache_retention_hint_seconds = Some(300);
        assert_eq!(context_pressure_limit(&profile, Some(10)), 80);
        assert_eq!(context_pressure_limit(&profile, None), 70);
        assert_eq!(cache_hint(&profile, Some(10)), "WITHIN_RETENTION_HINT");
        assert_eq!(cache_hint(&profile, None), "UNKNOWN");
    }

    #[test]
    fn unknown_or_mismatched_runtime_fingerprint_spawns() {
        let result = recommend(
            "c:/workspace/project".to_owned(),
            vec![candidate("IDLE", 50)],
            Profile {
                runtime_fingerprint: Some("current".to_owned()),
                ..Profile::default()
            },
        );
        assert_eq!(result.decision, "SPAWN");
        assert_eq!(result.reason_code, "RUNTIME_FINGERPRINT_MISMATCH");
    }

    #[test]
    fn runtime_fingerprint_is_stable_for_reordered_capabilities_and_changes_with_provider() {
        let first = runtime_fingerprint(&[
            (
                "provider",
                vec!["cas_a".to_owned(), "https://a.example/v1".to_owned()],
            ),
            (
                "capability",
                vec!["TOOLS:SUPPORTED".to_owned(), "JSON:SUPPORTED".to_owned()],
            ),
        ]);
        let reordered = runtime_fingerprint(&[
            (
                "provider",
                vec!["https://a.example/v1".to_owned(), "cas_a".to_owned()],
            ),
            (
                "capability",
                vec!["JSON:SUPPORTED".to_owned(), "TOOLS:SUPPORTED".to_owned()],
            ),
        ]);
        let changed = runtime_fingerprint(&[
            (
                "provider",
                vec!["cas_b".to_owned(), "https://b.example/v1".to_owned()],
            ),
            (
                "capability",
                vec!["JSON:SUPPORTED".to_owned(), "TOOLS:SUPPORTED".to_owned()],
            ),
        ]);
        assert_eq!(first, reordered);
        assert_ne!(first, changed);
    }

    #[test]
    fn skill_fingerprint_distinguishes_normal_slim_and_content_revision() {
        assert_eq!(
            skill_fingerprint_values(vec!["caveman".to_owned()]),
            vec!["caveman@full-v1"]
        );
        assert_eq!(
            skill_fingerprint_values(vec!["caveman-slim".to_owned()]),
            vec!["caveman-slim@slim-v1"]
        );
    }

    #[test]
    fn both_unknown_fingerprints_do_not_reuse() {
        let mut candidate = candidate("IDLE", 50);
        candidate.runtime_fingerprint = None;
        let result = recommend(
            "c:/workspace/project".to_owned(),
            vec![candidate],
            Profile::default(),
        );
        assert_eq!(result.reason_code, "RUNTIME_FINGERPRINT_UNKNOWN");
    }

    #[test]
    fn current_context_prevents_overfilled_reuse() {
        let result = recommend(
            "c:/workspace/project".to_owned(),
            vec![candidate("IDLE", 80)],
            Profile {
                runtime_fingerprint: Some("fingerprint".to_owned()),
                ..Profile::default()
            },
        );
        assert_eq!(result.decision, "SPAWN");
        assert_eq!(result.reason_code, "CONTEXT_PRESSURE");
    }

    #[test]
    fn cumulative_tokens_do_not_create_context_pressure() {
        let mut candidate = candidate("IDLE", 1_667_247);
        candidate.current_context_tokens = Some(50_000);
        candidate.context_window = Some(258_400);
        let result = recommend(
            "c:/workspace/project".to_owned(),
            vec![candidate],
            Profile {
                runtime_fingerprint: Some("fingerprint".to_owned()),
                ..Profile::default()
            },
        );
        assert_eq!(result.decision, "REUSE");
        assert_eq!(result.context_pressure_percent, Some(19));
    }

    #[test]
    fn compaction_reduces_pressure_even_when_cumulative_usage_grows() {
        let mut candidate = candidate("IDLE", 1_667_247);
        candidate.current_context_tokens = Some(50_000);
        candidate.context_window = Some(100_000);
        let compacted = recommend(
            "c:/workspace/project".to_owned(),
            vec![candidate],
            Profile {
                runtime_fingerprint: Some("fingerprint".to_owned()),
                ..Profile::default()
            },
        );
        assert_eq!(compacted.decision, "REUSE");
    }

    #[test]
    fn unknown_context_is_not_healthy_even_for_hot_strategy() {
        let mut candidate = candidate("IDLE", 1_667_247);
        candidate.current_context_tokens = None;
        let result = recommend(
            "c:/workspace/project".to_owned(),
            vec![candidate],
            Profile {
                reuse_strategy: "HOT".to_owned(),
                runtime_fingerprint: Some("fingerprint".to_owned()),
                ..Profile::default()
            },
        );
        assert_eq!(result.decision, "SPAWN");
        assert_eq!(result.reason_code, "CONTEXT_UNKNOWN");
        assert_eq!(result.context_pressure_percent, None);
    }

    #[test]
    fn workspace_scope_normalization_supports_real_cross_platform_paths() {
        assert_eq!(
            normalize_workspace_scope_key(r"\\?\C:\Workspace\Codex Agent Switch"),
            Some("c:/workspace/codex agent switch".to_owned())
        );
        assert_eq!(
            normalize_workspace_scope_key("/home/用户/My Project/"),
            Some("root/home/用户/My Project".to_owned())
        );
        assert_eq!(
            normalize_workspace_scope_key(r"\\server\share\Project"),
            Some("unc/server/share/project".to_owned())
        );
        for (input, expected) in [
            (r"C:\", "c:"),
            ("/", "root"),
            (r"\\Server\Share\", "unc/server/share"),
            ("/work/Foo", "root/work/Foo"),
        ] {
            let normalized = normalize_workspace_scope_key(input);
            assert_eq!(normalized.as_deref(), Some(expected));
            assert_eq!(
                normalized
                    .as_deref()
                    .and_then(normalize_workspace_scope_key),
                normalized
            );
        }
        assert_ne!(
            normalize_workspace_scope_key("/work/Foo"),
            normalize_workspace_scope_key("/work/foo")
        );
        assert_eq!(normalize_workspace_scope_key("order/refund"), None);
    }

    #[test]
    fn task_scope_keys_are_shell_safe_and_reasoning_downgrades_are_shared() {
        assert_eq!(
            normalize_task_scope_key("auth-oauth2"),
            Some("auth-oauth2".to_owned())
        );
        for invalid in ["Auth", "order/refund", "a b", "a;whoami", ""] {
            assert_eq!(normalize_task_scope_key(invalid), None);
        }
        assert_eq!(
            resolve_agent_reasoning_effort(
                "HIGH",
                Some("medium"),
                &["low".to_owned(), "medium".to_owned()]
            )
            .as_deref(),
            Some("medium")
        );
    }
}
use sha2::{Digest, Sha256};
