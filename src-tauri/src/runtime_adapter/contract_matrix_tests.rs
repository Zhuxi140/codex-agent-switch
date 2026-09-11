//! V-01：多版本 Adapter 契约矩阵。
//!
//! 使用两个受支持 Profile（Modern 与较早别名 "legacy"）加一个 Unsupported
//! Schema Fixture，逐格验证事件规范化、Capability 与 Fail Closed，并输出
//! 可重复的矩阵报告。判定只依赖 Schema 能力与 wire contract 样本，
//! 禁止按版本字符串分支。

use super::*;

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

fn cell(profile: &str, scenario: &str, outcome: &str) -> String {
    format!("{profile:<10} | {scenario:<34} | {outcome}")
}

#[test]
fn adapter_contract_matrix_covers_supported_and_unsupported_profiles() {
    let mut report = vec![
        cell("profile", "scenario", "outcome"),
        cell("-", "-", "-"),
    ];

    for (profile, raw) in [("modern", MODERN_EVENTS), ("legacy", LEGACY_EVENTS)] {
        let events = events(raw);
        assert!(events.len() >= 5, "{profile} fixture incomplete");

        // 场景 1：thread/started 规范化必须给出非空 Thread ID 与 Profile。
        let thread = parse_event(&events[0]).unwrap().unwrap();
        assert!(
            matches!(&thread, NormalizedRuntimeEvent::ThreadStarted { thread_id, .. } if !thread_id.is_empty()),
            "{profile}: thread/started must normalize"
        );
        report.push(cell(profile, "thread/started normalize", "PASS"));

        // 场景 2：usage 事件规范化，保留 total 与 cached 来源证明。
        let usage = events
            .iter()
            .filter_map(|value| parse_event(value).unwrap())
            .find_map(|event| match event {
                NormalizedRuntimeEvent::Usage { usage, .. } => Some(usage),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{profile}: usage event missing"));
        assert!(usage.total_tokens > 0, "{profile}: usage total");
        assert!(
            !usage.cached_input_provided || usage.cached_input_tokens > 0,
            "{profile}: cached provenance inconsistent"
        );
        report.push(cell(profile, "usage normalize + provenance", "PASS"));

        // 场景 3：Turn 响应在缺 Turn ID 时必须 Fail Closed。
        assert!(parse_turn_response(&fixture("missing-turn-id.json")).is_err());
        report.push(cell(profile, "turn response missing id fail-closed", "PASS"));

        // 场景 4：已知方法的畸形事件 Fail Closed；未知事件保持向前兼容。
        assert!(parse_event(&fixture("unsupported-known-malformed.json")).is_err());
        assert!(parse_event(&fixture("unknown-event.json")).unwrap().is_none());
        report.push(cell(profile, "malformed fail-closed / unknown ignored", "PASS"));

        // 场景 5：未来附加字段不得破坏规范化（版本前瞻兼容）。
        let future = parse_event(&fixture("future-usage-event.json")).unwrap().unwrap();
        assert!(matches!(future, NormalizedRuntimeEvent::Usage { .. }));
        report.push(cell(profile, "future fields tolerated", "PASS"));
    }

    // Unsupported Profile：缺关键 ID 的响应必须 Fail Closed，不写入运行状态。
    let unsupported = parse_thread_response(&fixture("missing-thread-id.json"));
    assert!(matches!(
        unsupported,
        Err(ProtocolParseError::MissingField("thread.id"))
    ));
    report.push(cell("unsupported", "thread response missing id", "FAIL-CLOSED"));

    // 输出可重复报告：相同 Fixture 每次运行产生相同矩阵。
    let report_text = report.join("\n");
    println!("\n=== V-01 Adapter Contract Matrix ===\n{report_text}\n");
    assert_eq!(report.len(), 13, "matrix must cover all planned cells");
}
