use std::fmt;

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppServerMethod {
    ThreadStart,
    ThreadResume,
    ThreadRead,
    TurnStart,
}

impl AppServerMethod {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ThreadStart => "thread/start",
            Self::ThreadResume => "thread/resume",
            Self::ThreadRead => "thread/read",
            Self::TurnStart => "turn/start",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProtocolProfile {
    Modern,
    Legacy,
}

impl ProtocolProfile {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Modern => "MODERN",
            Self::Legacy => "LEGACY",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedThread {
    pub(crate) thread_id: String,
    pub(crate) session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedTurn {
    pub(crate) turn_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedUsage {
    pub(crate) input_tokens: i64,
    pub(crate) cached_input_tokens: i64,
    /// F-02：Provider 事件是否真的提供了 Cached Input 字段。缺失时数值为 0
    /// 但此标志为 false，存储与展示不得把「未提供」冒充为「0」。
    pub(crate) cached_input_provided: bool,
    pub(crate) cache_write_input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) reasoning_output_tokens: i64,
    pub(crate) total_tokens: i64,
    pub(crate) current_context_tokens: Option<i64>,
    pub(crate) model_context_window: Option<i64>,
    pub(crate) partial: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum NormalizedRuntimeEvent {
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
        usage: NormalizedUsage,
        profile: ProtocolProfile,
    },
    TurnFinished {
        thread_id: String,
        turn_id: String,
        successful: bool,
        failure_message: Option<String>,
        profile: ProtocolProfile,
    },
}

impl NormalizedRuntimeEvent {
    pub(crate) fn profile(&self) -> ProtocolProfile {
        match self {
            Self::ThreadStarted { profile, .. }
            | Self::ParentChild { profile, .. }
            | Self::AgentPath { profile, .. }
            | Self::Usage { profile, .. }
            | Self::TurnFinished { profile, .. } => *profile,
        }
    }

    pub(crate) fn is_usage(&self) -> bool {
        matches!(self, Self::Usage { .. })
    }

    pub(crate) fn failure_message(&self) -> Option<&str> {
        match self {
            Self::TurnFinished {
                failure_message, ..
            } => failure_message.as_deref(),
            _ => None,
        }
    }
}

pub(crate) fn parse_event(
    message: &Value,
) -> Result<Option<NormalizedRuntimeEvent>, ProtocolParseError> {
    let method = match message.get("method").and_then(Value::as_str) {
        Some(method) => method,
        None => return Ok(None),
    };
    let params = message.get("params").unwrap_or(&Value::Null);
    match method {
        "thread/tokenUsage/updated" => parse_usage_event(params, ProtocolProfile::Modern),
        "codex/event/token_count" | "codex/event/tokenCount" => {
            parse_usage_event(params, ProtocolProfile::Legacy)
        }
        "thread/started" => parse_thread_started(params),
        "item/started" | "item/completed" => parse_item_event(params),
        "turn/completed" => parse_turn_finished(params),
        _ => Ok(None),
    }
}

pub(crate) fn parse_thread_response(
    result: &Value,
) -> Result<NormalizedThread, ProtocolParseError> {
    let thread = object_field(result, "thread").unwrap_or(result);
    let thread_id = string_alias(thread, &["id", "threadId", "thread_id"])
        .ok_or(ProtocolParseError::MissingField("thread.id"))?;
    Ok(NormalizedThread {
        thread_id,
        session_id: string_alias(thread, &["sessionId", "session_id"]),
    })
}

pub(crate) fn parse_turn_response(result: &Value) -> Result<NormalizedTurn, ProtocolParseError> {
    let turn = object_field(result, "turn").unwrap_or(result);
    let turn_id = string_alias(turn, &["id", "turnId", "turn_id"])
        .ok_or(ProtocolParseError::MissingField("turn.id"))?;
    Ok(NormalizedTurn { turn_id })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryTurnOutcome {
    Terminal,
    Running,
    Unknown,
}

pub(crate) fn recovery_turn_outcome(
    result: &Value,
    active_turn_id: Option<&str>,
) -> RecoveryTurnOutcome {
    let Some(active_turn_id) = active_turn_id else {
        return RecoveryTurnOutcome::Unknown;
    };
    let thread = object_field(result, "thread").unwrap_or(result);
    let Some(turn) = thread
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|turn| {
            string_alias(turn, &["id", "turnId", "turn_id"]).as_deref() == Some(active_turn_id)
        })
    else {
        return RecoveryTurnOutcome::Unknown;
    };
    let Some(status) = string_alias(turn, &["status"]) else {
        return RecoveryTurnOutcome::Unknown;
    };
    let status = status.to_ascii_lowercase().replace(['_', '-'], "");
    if matches!(
        status.as_str(),
        "completed" | "failed" | "interrupted" | "cancelled" | "canceled"
    ) {
        RecoveryTurnOutcome::Terminal
    } else if matches!(status.as_str(), "inprogress" | "running") {
        RecoveryTurnOutcome::Running
    } else {
        RecoveryTurnOutcome::Unknown
    }
}

#[cfg(test)]
pub(crate) fn thread_turn_evidence(result: &Value, turn_id: &str) -> Option<(usize, usize)> {
    let thread = object_field(result, "thread").unwrap_or(result);
    let turns = thread.get("turns").and_then(Value::as_array)?;
    let occurrences = turns
        .iter()
        .filter(|turn| string_alias(turn, &["id", "turnId", "turn_id"]).as_deref() == Some(turn_id))
        .count();
    Some((turns.len(), occurrences))
}

fn parse_usage_event(
    params: &Value,
    method_profile: ProtocolProfile,
) -> Result<Option<NormalizedRuntimeEvent>, ProtocolParseError> {
    let thread_id = string_alias(params, &["threadId", "thread_id", "conversationId"])
        .ok_or(ProtocolParseError::MissingField("threadId"))?;
    let envelope = object_alias(params, &["tokenUsage", "token_usage", "info"])
        .or_else(|| {
            params
                .pointer("/msg/info")
                .filter(|value| value.is_object())
        })
        .unwrap_or(params);
    let total = object_alias(envelope, &["total", "totalTokenUsage", "total_token_usage"])
        .unwrap_or(envelope);
    let (usage, used_legacy_fields) = parse_token_breakdown(total, envelope)?;
    let profile = if method_profile == ProtocolProfile::Legacy || used_legacy_fields {
        ProtocolProfile::Legacy
    } else {
        ProtocolProfile::Modern
    };
    Ok(Some(NormalizedRuntimeEvent::Usage {
        thread_id,
        usage,
        profile,
    }))
}

fn parse_token_breakdown(
    total: &Value,
    envelope: &Value,
) -> Result<(NormalizedUsage, bool), ProtocolParseError> {
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
        object_alias(envelope, &["lastTokenUsage", "last_token_usage", "last"])
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
        NormalizedUsage {
            input_tokens,
            cached_input_tokens: cached.unwrap_or(0),
            cached_input_provided: cached.is_some(),
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

fn parse_thread_started(
    params: &Value,
) -> Result<Option<NormalizedRuntimeEvent>, ProtocolParseError> {
    let thread = object_field(params, "thread").unwrap_or(params);
    let thread_id = string_alias(thread, &["id", "threadId", "thread_id"])
        .ok_or(ProtocolParseError::MissingField("thread.id"))?;
    let session_id = string_alias(thread, &["sessionId", "session_id"]);
    let parent_thread_id = string_alias(thread, &["parentThreadId", "parent_thread_id"]);
    let profile = if has_any_key(thread, &["session_id", "parent_thread_id", "thread_id"]) {
        ProtocolProfile::Legacy
    } else {
        ProtocolProfile::Modern
    };
    Ok(Some(NormalizedRuntimeEvent::ThreadStarted {
        thread_id,
        session_id,
        parent_thread_id,
        profile,
    }))
}

fn parse_item_event(params: &Value) -> Result<Option<NormalizedRuntimeEvent>, ProtocolParseError> {
    let item = object_field(params, "item").unwrap_or(params);
    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
    if matches!(item_type, "collabAgentToolCall" | "collabToolCall") {
        let parent_thread_id = string_alias(item, &["senderThreadId", "sender_thread_id"])
            .ok_or(ProtocolParseError::MissingField("senderThreadId"))?;
        let mut child_thread_ids =
            string_array_alias(item, &["receiverThreadIds", "receiver_thread_ids"]);
        if child_thread_ids.is_empty()
            && let Some(thread_id) = string_alias(
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
            return Err(ProtocolParseError::MissingField("receiverThreadIds"));
        }
        let model_slug = string_alias(item, &["model", "modelId", "model_id"]);
        let profile = if item_type == "collabToolCall"
            || has_any_key(item, &["sender_thread_id", "receiver_thread_ids"])
        {
            ProtocolProfile::Legacy
        } else {
            ProtocolProfile::Modern
        };
        return Ok(Some(NormalizedRuntimeEvent::ParentChild {
            parent_thread_id,
            child_thread_ids,
            model_slug,
            profile,
        }));
    }
    if item_type == "subAgentActivity" {
        let thread_id = string_alias(item, &["agentThreadId", "agent_thread_id"])
            .ok_or(ProtocolParseError::MissingField("agentThreadId"))?;
        let agent_path = string_alias(item, &["agentPath", "agent_path"])
            .ok_or(ProtocolParseError::MissingField("agentPath"))?;
        let agent_key = agent_key_from_path(&agent_path)
            .ok_or(ProtocolParseError::InvalidField("agentPath"))?;
        let profile = if has_any_key(item, &["agent_thread_id", "agent_path"]) {
            ProtocolProfile::Legacy
        } else {
            ProtocolProfile::Modern
        };
        return Ok(Some(NormalizedRuntimeEvent::AgentPath {
            thread_id,
            agent_key,
            profile,
        }));
    }
    Ok(None)
}

fn parse_turn_finished(
    params: &Value,
) -> Result<Option<NormalizedRuntimeEvent>, ProtocolParseError> {
    let thread_id = string_alias(params, &["threadId", "thread_id", "conversationId"])
        .ok_or(ProtocolParseError::MissingField("threadId"))?;
    let turn = object_field(params, "turn").unwrap_or(params);
    let turn_id = string_alias(turn, &["id", "turnId", "turn_id"])
        .ok_or(ProtocolParseError::MissingField("turn.id"))?;
    let status = turn
        .get("status")
        .and_then(Value::as_str)
        .or_else(|| params.get("status").and_then(Value::as_str))
        .unwrap_or("completed");
    let profile = if has_any_key(params, &["thread_id", "conversationId"])
        || has_any_key(turn, &["turn_id"])
    {
        ProtocolProfile::Legacy
    } else {
        ProtocolProfile::Modern
    };
    let successful = status == "completed";
    let failure_message = (!successful).then(|| {
        turn.get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .or_else(|| params.pointer("/error/message").and_then(Value::as_str))
            .or_else(|| turn.get("error").and_then(Value::as_str))
            .or_else(|| params.get("error").and_then(Value::as_str))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("Turn ended with status {status}"))
    });
    Ok(Some(NormalizedRuntimeEvent::TurnFinished {
        thread_id,
        turn_id,
        successful,
        failure_message,
        profile,
    }))
}

fn object_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).filter(|candidate| candidate.is_object())
}

fn object_alias<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object_field(value, key))
}

fn string_alias(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .map(str::to_owned)
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProtocolParseError {
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
#[path = "runtime_adapter/fixture_tests.rs"]
mod fixture_tests;

#[cfg(test)]
#[path = "runtime_adapter/contract_matrix_tests.rs"]
mod contract_matrix_tests;
