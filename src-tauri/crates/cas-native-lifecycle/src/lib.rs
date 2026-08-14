use std::fs::File;
use std::io::{self, BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

const ROLLOUT_TAIL_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Closed,
    Idle,
    Running,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RolloutState {
    pub lifecycle: Option<ThreadState>,
    pub current_context_tokens: Option<i64>,
    pub model_context_window: Option<i64>,
}

pub fn thread_state(edge_status: &str, rollout_path: &Path) -> ThreadState {
    let rollout = rollout_state(rollout_path).ok();
    thread_state_from_rollout(edge_status, rollout.as_ref())
}

pub fn thread_state_from_rollout(edge_status: &str, rollout: Option<&RolloutState>) -> ThreadState {
    if edge_status.eq_ignore_ascii_case("closed") {
        return ThreadState::Closed;
    }
    if !edge_status.eq_ignore_ascii_case("open") {
        return ThreadState::Unknown;
    }
    rollout
        .and_then(|state| state.lifecycle)
        .unwrap_or(ThreadState::Unknown)
}

#[derive(Clone, Copy)]
enum LifecycleEvent {
    TaskStarted,
    TurnEnded,
}

pub fn rollout_state(rollout_path: &Path) -> io::Result<RolloutState> {
    let mut file = File::open(rollout_path)?;
    let length = file.metadata()?.len();
    let offset = length.saturating_sub(ROLLOUT_TAIL_BYTES);
    file.seek(SeekFrom::Start(offset))?;

    let mut reader = BufReader::new(file);
    if offset > 0 {
        let mut discarded = Vec::new();
        reader.read_until(b'\n', &mut discarded)?;
    }

    let mut lifecycle = None;
    let mut current_context_tokens = None;
    let mut model_context_window = None;
    let mut line = Vec::new();
    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            return Ok(RolloutState {
                lifecycle: lifecycle.map(|event| match event {
                    LifecycleEvent::TurnEnded => ThreadState::Idle,
                    LifecycleEvent::TaskStarted => ThreadState::Running,
                }),
                current_context_tokens,
                model_context_window,
            });
        }
        if let Some(value) = serde_json::from_slice::<Value>(&line).ok() {
            if let Some(event) = lifecycle_event(&value) {
                lifecycle = Some(event);
            }
            if let Some((tokens, window)) = token_count(&value) {
                current_context_tokens = tokens;
                model_context_window = window;
            }
        }
    }
}

fn lifecycle_event(value: &Value) -> Option<LifecycleEvent> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    match value
        .get("payload")
        .and_then(Value::as_object)?
        .get("type")
        .and_then(Value::as_str)
    {
        Some("task_complete") | Some("turn_aborted") => Some(LifecycleEvent::TurnEnded),
        Some("task_started") => Some(LifecycleEvent::TaskStarted),
        _ => None,
    }
}

fn token_count(value: &Value) -> Option<(Option<i64>, Option<i64>)> {
    if value.get("type").and_then(Value::as_str) != Some("event_msg") {
        return None;
    }
    let payload = value.get("payload")?.as_object()?;
    if payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let envelope = payload
        .get("info")
        .or_else(|| payload.get("tokenUsage"))
        .or_else(|| payload.get("token_usage"))
        .unwrap_or(&Value::Null);
    let last = ["lastTokenUsage", "last_token_usage"]
        .into_iter()
        .find_map(|key| envelope.get(key))
        .and_then(Value::as_object);
    let tokens = last
        .and_then(|usage| {
            usage
                .get("totalTokens")
                .or_else(|| usage.get("total_tokens"))
        })
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0);
    let window = envelope
        .get("modelContextWindow")
        .or_else(|| envelope.get("model_context_window"))
        .and_then(Value::as_i64)
        .filter(|value| *value > 0);
    Some((tokens, window))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn completed_open_thread_is_idle_even_when_its_writer_lock_remains() {
        let root = temporary_directory("completed");
        fs::create_dir_all(root.join("thread-writer-locks")).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n",
        )
        .unwrap();
        fs::write(root.join("thread-writer-locks/thread-child.lock"), "").unwrap();

        assert_eq!(thread_state("open", &rollout), ThreadState::Idle);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn latest_started_turn_is_running() {
        let root = temporary_directory("running");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
        )
        .unwrap();

        assert_eq!(thread_state("open", &rollout), ThreadState::Running);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn lifecycle_events_allow_reordered_keys_and_extra_fields() {
        let root = temporary_directory("reordered");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"payload\":{\"extra\":true,\"type\":\"task_started\"},\"type\":\"event_msg\"}\n\
             {\"payload\":{\"type\":\"task_complete\",\"extra\":\"value\"},\"timestamp\":\"now\",\"type\":\"event_msg\"}\n",
        )
        .unwrap();

        assert_eq!(thread_state("open", &rollout), ThreadState::Idle);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn aborted_turn_is_idle_and_can_be_followed_up() {
        let root = temporary_directory("aborted");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"turn_aborted\"}}\n",
        )
        .unwrap();

        assert_eq!(thread_state("open", &rollout), ThreadState::Idle);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn similar_text_outside_a_lifecycle_event_is_ignored() {
        let root = temporary_directory("similar-text");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"content\":\"task_complete\"}}\n\
             {\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"note\":\"task_started\"}}\n",
        )
        .unwrap();

        assert_eq!(thread_state("open", &rollout), ThreadState::Unknown);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn closed_edge_is_closed_without_reading_rollout() {
        assert_eq!(
            thread_state("closed", Path::new("does-not-exist.jsonl")),
            ThreadState::Closed
        );
    }

    #[test]
    fn unreadable_or_unrecognizable_open_rollout_is_unknown() {
        let root = temporary_directory("unknown");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\"}}\n",
        )
        .unwrap();

        assert_eq!(thread_state("open", &rollout), ThreadState::Unknown);
        assert_eq!(
            thread_state("open", Path::new("does-not-exist.jsonl")),
            ThreadState::Unknown
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollout_state_uses_latest_context_with_camel_and_snake_aliases() {
        let root = temporary_directory("context");
        fs::create_dir_all(&root).unwrap();
        let rollout = root.join("rollout.jsonl");
        fs::write(
            &rollout,
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"total_tokens\":90000},\"model_context_window\":258400}}}\n\
             {\"extra\":true,\"type\":\"event_msg\",\"payload\":{\"future\":true,\"type\":\"token_count\",\"info\":{\"lastTokenUsage\":{\"totalTokens\":50000},\"modelContextWindow\":258400}}}\n",
        )
        .unwrap();

        assert_eq!(
            rollout_state(&rollout).unwrap(),
            RolloutState {
                lifecycle: None,
                current_context_tokens: Some(50_000),
                model_context_window: Some(258_400),
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    fn temporary_directory(prefix: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nonce}", std::process::id()))
    }
}
