use super::*;
use serde_json::Value;

const MODERN_EVENTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/runtime-adapter/modern-events.json"
));
const LEGACY_EVENTS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/test-fixtures/runtime-adapter/legacy-events.json"
));

fn fixture(name: &str) -> Value {
    let path = format!(
        "{}/test-fixtures/runtime-adapter/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn events(raw: &str) -> Vec<Value> {
    serde_json::from_str(raw).unwrap()
}

#[test]
fn supported_wire_contract_variants_normalize_thread_turn_and_events() {
    // “legacy” 仅表示较早字段别名的同一现代 RPC wire contract 样本，
    // 不据此宣称存在或支持独立的 Legacy RPC method/schema。
    let cases = [("modern", MODERN_EVENTS), ("legacy", LEGACY_EVENTS)];
    for (profile, raw) in cases {
        let values = events(raw);
        let thread = parse_event(&values[0]).unwrap().unwrap();
        assert!(matches!(
            thread,
            NormalizedRuntimeEvent::ThreadStarted { .. }
        ));
        assert_eq!(
            thread.profile(),
            if profile == "modern" {
                ProtocolProfile::Modern
            } else {
                ProtocolProfile::Legacy
            }
        );

        let parent_child = parse_event(&values[1]).unwrap().unwrap();
        assert!(
            matches!(parent_child, NormalizedRuntimeEvent::ParentChild { child_thread_ids, .. } if child_thread_ids.len() == 1)
        );
        let agent = parse_event(&values[2]).unwrap().unwrap();
        assert!(
            matches!(agent, NormalizedRuntimeEvent::AgentPath { ref agent_key, .. } if agent_key == if profile == "modern" { "reviewer" } else { "legacy" })
        );
        let usage = parse_event(&values[3]).unwrap().unwrap();
        assert!(
            matches!(usage, NormalizedRuntimeEvent::Usage { ref usage, .. } if usage.total_tokens == if profile == "modern" { 130 } else { 100 })
        );
        let finished = parse_event(&values[4]).unwrap().unwrap();
        assert!(
            matches!(finished, NormalizedRuntimeEvent::TurnFinished { successful, .. } if successful == (profile == "modern"))
        );
    }
}

#[test]
fn response_aliases_normalize_modern_and_legacy_ids() {
    let cases = [
        (
            "modern-thread-response.json",
            "thread-modern-001",
            "modern-turn-response.json",
            "turn-modern-001",
        ),
        (
            "legacy-thread-response.json",
            "thread-legacy-001",
            "legacy-turn-response.json",
            "turn-legacy-001",
        ),
    ];
    for (thread_file, thread_id, turn_file, turn_id) in cases {
        assert_eq!(
            parse_thread_response(&fixture(thread_file))
                .unwrap()
                .thread_id,
            thread_id
        );
        assert_eq!(
            parse_turn_response(&fixture(turn_file)).unwrap().turn_id,
            turn_id
        );
    }
}

#[test]
fn future_fields_are_ignored_without_losing_known_usage() {
    let event = parse_event(&fixture("future-usage-event.json"))
        .unwrap()
        .unwrap();
    assert!(
        matches!(event, NormalizedRuntimeEvent::Usage { ref usage, .. } if usage.input_tokens == 1 && usage.output_tokens == 2 && usage.total_tokens == 3 && usage.partial)
    );
}

#[test]
fn cached_input_provenance_tracks_provider_field_presence() {
    // F-02：事件缺少 cachedInputTokens 时数值为 0 但必须标记为「未提供」。
    let missing = parse_event(&fixture("future-usage-event.json"))
        .unwrap()
        .unwrap();
    assert!(
        matches!(missing, NormalizedRuntimeEvent::Usage { ref usage, .. } if usage.cached_input_tokens == 0 && !usage.cached_input_provided)
    );

    let present = events(MODERN_EVENTS)
        .iter()
        .filter_map(|value| parse_event(value).unwrap())
        .find_map(|event| match event {
            NormalizedRuntimeEvent::Usage { usage, .. } if usage.input_tokens == 100 => Some(usage),
            _ => None,
        })
        .unwrap();
    assert!(present.cached_input_tokens == 20 && present.cached_input_provided);
}

#[test]
fn missing_critical_ids_fail_closed() {
    assert_eq!(
        parse_thread_response(&fixture("missing-thread-id.json")),
        Err(ProtocolParseError::MissingField("thread.id"))
    );
    assert_eq!(
        parse_turn_response(&fixture("missing-turn-id.json")),
        Err(ProtocolParseError::MissingField("turn.id"))
    );
    let failures = [
        "unsupported-known-malformed.json",
        "legacy-missing-turn-id.json",
    ];
    for name in failures {
        assert!(
            matches!(
                parse_event(&fixture(name)),
                Err(ProtocolParseError::MissingField("turn.id"))
            ),
            "fixture {name} must fail closed"
        );
    }
}

#[test]
fn unsupported_unknown_events_are_ignored_but_known_malformed_events_are_rejected() {
    assert!(
        parse_event(&fixture("unknown-event.json"))
            .unwrap()
            .is_none()
    );
    assert!(parse_event(&fixture("unsupported-known-malformed.json")).is_err());
}

#[test]
fn recovery_and_turn_evidence_require_the_expected_turn_id() {
    let result = serde_json::json!({"thread":{"turns":[{"id":"turn-modern-001","status":"completed"},{"id":"other","status":"running"} ]}});
    assert_eq!(
        recovery_turn_outcome(&result, Some("turn-modern-001")),
        RecoveryTurnOutcome::Terminal
    );
    assert_eq!(
        recovery_turn_outcome(&result, Some("missing")),
        RecoveryTurnOutcome::Unknown
    );
    assert_eq!(
        thread_turn_evidence(&result, "turn-modern-001"),
        Some((2, 1))
    );
    assert_eq!(thread_turn_evidence(&result, "missing"), Some((2, 0)));
}
