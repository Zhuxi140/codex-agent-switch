//! 受限的 Responses ↔ Chat Completions 语义转换。
//! 未覆盖的字段必须显式拒绝，不能悄悄丢失 Codex 的上下文或工具信息。

use serde_json::{Map, Value, json};
use url::Url;
use uuid::Uuid;

pub(crate) fn chat_endpoint(base_url: &str) -> Result<Url, ()> {
    let mut url = Url::parse(base_url).map_err(|_| ())?;
    url.set_query(None);
    url.set_fragment(None);
    if url
        .path()
        .trim_end_matches('/')
        .ends_with("/chat/completions")
    {
        return Ok(url);
    }
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    url.join("chat/completions").map_err(|_| ())
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChatCompatError {
    Invalid(&'static str),
    Unsupported(&'static str),
}

pub struct PreparedChatRequest {
    pub body: Value,
    pub wants_stream: bool,
}

pub fn responses_to_chat(request: &Value) -> Result<PreparedChatRequest, ChatCompatError> {
    let source = request
        .as_object()
        .ok_or(ChatCompatError::Invalid("Responses 请求必须是对象"))?;
    for key in source.keys() {
        if !matches!(
            key.as_str(),
            "model"
                | "input"
                | "instructions"
                | "tools"
                | "tool_choice"
                | "parallel_tool_calls"
                | "max_output_tokens"
                | "temperature"
                | "top_p"
                | "stream"
                | "store"
                | "reasoning"
                | "include"
                | "prompt_cache_key"
                | "client_metadata"
        ) {
            return Err(ChatCompatError::Unsupported(
                "请求包含尚未支持的 Responses 字段",
            ));
        }
    }
    if source.get("store").is_some_and(|value| value != false) {
        return Err(ChatCompatError::Unsupported("有状态 Responses 会话"));
    }
    if let Some(reasoning) = source.get("reasoning") {
        let reasoning = reasoning
            .as_object()
            .ok_or(ChatCompatError::Invalid("reasoning 必须是对象"))?;
        if reasoning
            .keys()
            .any(|key| !matches!(key.as_str(), "effort" | "summary"))
            || reasoning
                .get("effort")
                .is_some_and(|value| !value.is_null() && value != "none")
        {
            return Err(ChatCompatError::Unsupported(
                "Chat Completions 无法保留 reasoning effort",
            ));
        }
    }
    if let Some(include) = source.get("include") {
        let include = include
            .as_array()
            .ok_or(ChatCompatError::Invalid("include 必须是数组"))?;
        if include
            .iter()
            .any(|item| item.as_str() != Some("reasoning.encrypted_content"))
        {
            return Err(ChatCompatError::Unsupported("include 类型"));
        }
    }
    if source
        .get("prompt_cache_key")
        .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err(ChatCompatError::Invalid("prompt_cache_key 必须是字符串"));
    }
    if source
        .get("client_metadata")
        .is_some_and(|value| !value.is_object())
    {
        return Err(ChatCompatError::Invalid("client_metadata 必须是对象"));
    }
    let model = source
        .get("model")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(ChatCompatError::Invalid("缺少 model"))?;
    let wants_stream = match source.get("stream") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(ChatCompatError::Invalid("stream 必须是布尔值")),
    };
    let mut messages = Vec::new();
    if let Some(instructions) = source.get("instructions") {
        let text = instructions
            .as_str()
            .ok_or(ChatCompatError::Unsupported("非文本 instructions"))?;
        messages.push(json!({"role":"system","content":text}));
    }
    match source.get("input") {
        Some(Value::String(text)) => messages.push(json!({"role":"user","content":text})),
        Some(Value::Array(items)) => {
            for item in items {
                messages.push(input_item_to_chat(item)?);
            }
        }
        _ => return Err(ChatCompatError::Invalid("缺少可转换的 input")),
    }
    let mut body = Map::new();
    body.insert("model".to_owned(), json!(model));
    body.insert("messages".to_owned(), Value::Array(messages));
    // 首版缓冲上游完整结果，再按调用方要求生成 Responses JSON 或 SSE。
    body.insert("stream".to_owned(), Value::Bool(false));
    for (from, to) in [
        ("parallel_tool_calls", "parallel_tool_calls"),
        ("max_output_tokens", "max_completion_tokens"),
        ("temperature", "temperature"),
        ("top_p", "top_p"),
    ] {
        if let Some(value) = source.get(from) {
            body.insert(to.to_owned(), value.clone());
        }
    }
    if let Some(tools) = source.get("tools") {
        let tools = tools
            .as_array()
            .ok_or(ChatCompatError::Invalid("tools 必须是数组"))?;
        let mut converted = Vec::with_capacity(tools.len());
        for tool in tools {
            // Codex 默认还会发送 Responses 内置/命名空间工具；Chat 上游只暴露
            // 独立 function 工具，其他能力在兼容模式中明确停用。
            match tool.get("type").and_then(Value::as_str) {
                Some("namespace")
                    if tool.get("name").and_then(Value::as_str).is_some()
                        && tool.get("tools").and_then(Value::as_array).is_some() =>
                {
                    continue;
                }
                Some("web_search") => continue,
                _ => {}
            }
            let object = tool
                .as_object()
                .filter(|value| value.get("type").and_then(Value::as_str) == Some("function"))
                .ok_or(ChatCompatError::Unsupported("非 function 工具"))?;
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or(ChatCompatError::Invalid("function 工具缺少 name"))?;
            if object
                .get("strict")
                .is_some_and(|value| !value.is_null() && value != false)
            {
                return Err(ChatCompatError::Unsupported("strict function 工具"));
            }
            let mut function = Map::new();
            function.insert("name".to_owned(), json!(name));
            for field in ["description", "parameters"] {
                if let Some(value) = object.get(field) {
                    function.insert(field.to_owned(), value.clone());
                }
            }
            if object.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "type" | "name" | "description" | "parameters" | "strict"
                )
            }) {
                return Err(ChatCompatError::Unsupported("function 工具含未支持字段"));
            }
            converted.push(json!({"type":"function","function":function}));
        }
        if !converted.is_empty() {
            body.insert("tools".to_owned(), Value::Array(converted));
        } else {
            body.remove("parallel_tool_calls");
        }
    }
    if let Some(choice) = source.get("tool_choice") {
        let mapped = match choice {
            Value::String(mode) if matches!(mode.as_str(), "auto" | "none" | "required") => {
                choice.clone()
            }
            Value::Object(value)
                if value.get("type").and_then(Value::as_str) == Some("function") =>
            {
                let name = value
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or(ChatCompatError::Invalid("tool_choice 缺少 name"))?;
                json!({"type":"function","function":{"name":name}})
            }
            _ => return Err(ChatCompatError::Unsupported("tool_choice 类型")),
        };
        if let Some(name) = mapped.pointer("/function/name").and_then(Value::as_str) {
            let present = body
                .get("tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| {
                    tools.iter().any(|tool| {
                        tool.pointer("/function/name").and_then(Value::as_str) == Some(name)
                    })
                });
            if !present {
                return Err(ChatCompatError::Unsupported("tool_choice 引用了未映射工具"));
            }
        }
        if mapped == "required"
            && body
                .get("tools")
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
        {
            return Err(ChatCompatError::Unsupported("没有可用的 function 工具"));
        }
        if body.contains_key("tools") || !matches!(mapped.as_str(), Some("auto" | "none")) {
            body.insert("tool_choice".to_owned(), mapped);
        }
    }
    Ok(PreparedChatRequest {
        body: Value::Object(body),
        wants_stream,
    })
}

fn input_item_to_chat(item: &Value) -> Result<Value, ChatCompatError> {
    let object = item
        .as_object()
        .ok_or(ChatCompatError::Invalid("input Item 必须是对象"))?;
    match object.get("type").and_then(Value::as_str) {
        Some("function_call") => {
            let call_id = required_string(object, "call_id")?;
            let name = required_string(object, "name")?;
            let arguments = required_string(object, "arguments")?;
            Ok(json!({"role":"assistant","content":null,"tool_calls":[{
                "id":call_id,"type":"function",
                "function":{"name":name,"arguments":arguments}
            }]}))
        }
        Some("function_call_output") => Ok(json!({
            "role":"tool",
            "tool_call_id":required_string(object, "call_id")?,
            "content":required_string(object, "output")?
        })),
        None | Some("message") => {
            let role = required_string(object, "role")?;
            if !matches!(role, "system" | "developer" | "user" | "assistant") {
                return Err(ChatCompatError::Unsupported("消息角色"));
            }
            let content = match object.get("content") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Array(parts)) => {
                    let mut text = String::new();
                    for part in parts {
                        let part = part
                            .as_object()
                            .ok_or(ChatCompatError::Invalid("文本片段"))?;
                        if !matches!(
                            part.get("type").and_then(Value::as_str),
                            Some("input_text" | "output_text")
                        ) {
                            return Err(ChatCompatError::Unsupported("非文本消息片段"));
                        }
                        text.push_str(required_string(part, "text")?);
                    }
                    text
                }
                _ => return Err(ChatCompatError::Unsupported("非文本消息")),
            };
            Ok(json!({"role":role,"content":content}))
        }
        _ => Err(ChatCompatError::Unsupported("Responses Item 类型")),
    }
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &'static str,
) -> Result<&'a str, ChatCompatError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or(ChatCompatError::Invalid(key))
}

pub fn chat_to_responses(completion: &Value) -> Result<Value, ChatCompatError> {
    let choice = completion
        .get("choices")
        .and_then(Value::as_array)
        .filter(|choices| choices.len() == 1)
        .and_then(|choices| choices.first())
        .ok_or(ChatCompatError::Invalid(
            "Chat Completion 必须只有一个 choice",
        ))?;
    if !matches!(
        choice.get("finish_reason").and_then(Value::as_str),
        Some("stop" | "tool_calls")
    ) {
        return Err(ChatCompatError::Unsupported("上游未正常完成，或输出被截断"));
    }
    let message = choice
        .get("message")
        .ok_or(ChatCompatError::Invalid("缺少 assistant message"))?;
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return Err(ChatCompatError::Invalid("上游消息不是 assistant"));
    }
    let mut output = Vec::new();
    if message
        .get("content")
        .is_some_and(|value| !value.is_null() && !value.is_string())
    {
        return Err(ChatCompatError::Unsupported("上游非文本内容"));
    }
    if let Some(text) = message.get("content").and_then(Value::as_str) {
        if !text.is_empty() {
            output.push(json!({
                "id":format!("msg_{}", Uuid::new_v4().simple()),
                "type":"message","status":"completed","role":"assistant",
                "content":[{"type":"output_text","text":text,"annotations":[]}]
            }));
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            if call.get("type").and_then(Value::as_str) != Some("function") {
                return Err(ChatCompatError::Unsupported("上游非 function 工具调用"));
            }
            output.push(json!({
                "id":format!("fc_{}", Uuid::new_v4().simple()),
                "type":"function_call","status":"completed",
                "call_id":call.get("id").and_then(Value::as_str).ok_or(ChatCompatError::Invalid("tool call id"))?,
                "name":call.pointer("/function/name").and_then(Value::as_str).ok_or(ChatCompatError::Invalid("tool name"))?,
                "arguments":call.pointer("/function/arguments").and_then(Value::as_str).ok_or(ChatCompatError::Invalid("tool arguments"))?
            }));
        }
    }
    if output.is_empty() {
        return Err(ChatCompatError::Invalid("上游未返回文本或工具调用"));
    }
    let usage = if let Some(usage) = completion.get("usage").filter(|value| !value.is_null()) {
        let input_tokens = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .ok_or(ChatCompatError::Invalid("prompt_tokens"))?;
        let output_tokens = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .ok_or(ChatCompatError::Invalid("completion_tokens"))?;
        let total_tokens = input_tokens
            .checked_add(output_tokens)
            .ok_or(ChatCompatError::Invalid("token 用量溢出"))?;
        json!({"input_tokens":input_tokens,"output_tokens":output_tokens,"total_tokens":total_tokens})
    } else {
        Value::Null
    };
    Ok(json!({
        "id":format!("resp_{}", Uuid::new_v4().simple()),
        "object":"response","created_at":completion.get("created").and_then(Value::as_u64).ok_or(ChatCompatError::Invalid("created"))?,
        "status":"completed","model":completion.get("model").and_then(Value::as_str).ok_or(ChatCompatError::Invalid("model"))?,
        "output":output,
        "usage":usage
    }))
}

pub(crate) fn responses_to_sse(response: &Value) -> Vec<u8> {
    let mut stream = Vec::new();
    let mut sequence = 0_u64;
    let mut started = response.clone();
    started["status"] = json!("in_progress");
    started["output"] = json!([]);
    started["usage"] = Value::Null;
    push_event(
        &mut stream,
        &mut sequence,
        "response.created",
        json!({"response":started}),
    );
    for (output_index, item) in response["output"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let item_id = item["id"].clone();
        let mut added = item.clone();
        added["status"] = json!("in_progress");
        match item["type"].as_str() {
            Some("message") => added["content"] = json!([]),
            Some("function_call") => added["arguments"] = json!(""),
            _ => continue,
        }
        push_event(
            &mut stream,
            &mut sequence,
            "response.output_item.added",
            json!({"output_index":output_index,"item":added}),
        );
        if item["type"] == "message" {
            let part = &item["content"][0];
            let text = part["text"].as_str().unwrap_or_default();
            push_event(
                &mut stream,
                &mut sequence,
                "response.content_part.added",
                json!({"item_id":item_id,"output_index":output_index,"content_index":0,
                    "part":{"type":"output_text","text":"","annotations":[]}}),
            );
            push_event(
                &mut stream,
                &mut sequence,
                "response.output_text.delta",
                json!({"item_id":item_id,"output_index":output_index,"content_index":0,"delta":text}),
            );
            push_event(
                &mut stream,
                &mut sequence,
                "response.output_text.done",
                json!({"item_id":item_id,"output_index":output_index,"content_index":0,"text":text}),
            );
            push_event(
                &mut stream,
                &mut sequence,
                "response.content_part.done",
                json!({"item_id":item_id,"output_index":output_index,"content_index":0,"part":part}),
            );
        } else {
            let arguments = item["arguments"].as_str().unwrap_or_default();
            push_event(
                &mut stream,
                &mut sequence,
                "response.function_call_arguments.delta",
                json!({"item_id":item_id,"output_index":output_index,"delta":arguments}),
            );
            push_event(
                &mut stream,
                &mut sequence,
                "response.function_call_arguments.done",
                json!({"item_id":item_id,"output_index":output_index,"arguments":arguments}),
            );
        }
        push_event(
            &mut stream,
            &mut sequence,
            "response.output_item.done",
            json!({"output_index":output_index,"item":item}),
        );
    }
    push_event(
        &mut stream,
        &mut sequence,
        "response.completed",
        json!({"response":response}),
    );
    stream.extend_from_slice(b"data: [DONE]\n\n");
    stream
}

fn push_event(stream: &mut Vec<u8>, sequence: &mut u64, kind: &str, mut payload: Value) {
    payload["type"] = json!(kind);
    payload["sequence_number"] = json!(*sequence);
    *sequence += 1;
    stream.extend_from_slice(format!("event: {kind}\ndata: {payload}\n\n").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_endpoint_accepts_base_and_full_path() {
        assert_eq!(
            chat_endpoint("https://provider.example/v1")
                .unwrap()
                .as_str(),
            "https://provider.example/v1/chat/completions"
        );
        assert_eq!(
            chat_endpoint("https://provider.example/v1/chat/completions")
                .unwrap()
                .as_str(),
            "https://provider.example/v1/chat/completions"
        );
    }

    #[test]
    fn buffered_responses_stream_emits_text_and_completion() {
        let response = json!({"id":"resp_test","status":"completed","output":[{
            "id":"msg_test","type":"message","status":"completed","role":"assistant",
            "content":[{"type":"output_text","text":"ok","annotations":[]}]
        }]});
        let stream = String::from_utf8(responses_to_sse(&response)).unwrap();
        assert!(stream.contains("event: response.output_text.delta"));
        assert!(stream.contains("event: response.completed"));
        assert!(stream.ends_with("data: [DONE]\n\n"));
    }

    #[test]
    fn converts_text_and_function_round_trip() {
        let request = json!({
            "model":"chat-model","stream":true,"store":false,
            "instructions":"Follow instructions",
            "input":[
                {"role":"user","content":[{"type":"input_text","text":"Run test"}]},
                {"type":"function_call","call_id":"call-1","name":"shell","arguments":"{}"},
                {"type":"function_call_output","call_id":"call-1","output":"ok"}
            ],
            "tools":[{"type":"function","name":"shell","parameters":{"type":"object"}}]
        });
        let prepared = responses_to_chat(&request).unwrap();
        assert!(prepared.wants_stream);
        assert_eq!(prepared.body["stream"], false);
        assert_eq!(
            prepared.body["messages"][2]["tool_calls"][0]["id"],
            "call-1"
        );
        assert_eq!(prepared.body["messages"][3]["tool_call_id"], "call-1");
        assert_eq!(prepared.body["tools"][0]["function"]["name"], "shell");

        let completion = json!({
            "created":123,"model":"chat-model",
            "choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,
                "tool_calls":[{"id":"call-2","type":"function",
                    "function":{"name":"shell","arguments":"{\"cmd\":\"pwd\"}"}}]}}],
            "usage":{"prompt_tokens":12,"completion_tokens":3}
        });
        let converted = chat_to_responses(&completion).unwrap();
        assert_eq!(converted["output"][0]["type"], "function_call");
        assert_eq!(converted["output"][0]["call_id"], "call-2");
        assert_eq!(converted["usage"]["total_tokens"], 15);
    }

    #[test]
    fn unsupported_state_and_modalities_fail_closed() {
        assert_eq!(
            responses_to_chat(&json!({"model":"m","input":"hi","previous_response_id":"resp-1"}))
                .err(),
            Some(ChatCompatError::Unsupported(
                "请求包含尚未支持的 Responses 字段"
            ))
        );
        assert!(matches!(
            responses_to_chat(&json!({"model":"m","input":"hi","tools":[{"type":"computer_use"}]})),
            Err(ChatCompatError::Unsupported(_))
        ));
        assert!(matches!(
            chat_to_responses(
                &json!({"model":"m","choices":[{"finish_reason":"length","message":{"content":"partial"}}]})
            ),
            Err(ChatCompatError::Unsupported(_))
        ));
    }

    #[test]
    fn codex_request_keeps_functions_and_disables_untranslatable_tools() {
        let request = json!({
            "model":"chat-model","store":false,"stream":true,
            "input":[{"type":"message","id":"msg_input","role":"user",
                "content":[{"type":"input_text","text":"hello"}]}],
            "reasoning":{"summary":"auto"},
            "include":["reasoning.encrypted_content"],
            "prompt_cache_key":"cache-key",
            "client_metadata":{"thread_id":"test"},
            "tools":[
                {"type":"function","name":"exec_command","strict":false,
                    "parameters":{"type":"object"}},
                {"type":"namespace","name":"mcp_example","tools":[]},
                {"type":"web_search","external_web_access":true}
            ]
        });
        let prepared = responses_to_chat(&request).unwrap();
        assert!(prepared.wants_stream);
        assert_eq!(prepared.body["tools"].as_array().unwrap().len(), 1);
        assert_eq!(
            prepared.body["tools"][0]["function"]["name"],
            "exec_command"
        );
        assert!(
            prepared.body["tools"][0]["function"]
                .get("strict")
                .is_none()
        );
        assert!(
            responses_to_chat(&json!({
                "model":"chat-model","input":"hello","reasoning":{"effort":"high"}
            }))
            .is_err()
        );
        assert!(
            responses_to_chat(&json!({
                "model":"chat-model","input":"hello",
                "tools":[{"type":"function","name":"strict_tool","strict":true}]
            }))
            .is_err()
        );
    }
}
