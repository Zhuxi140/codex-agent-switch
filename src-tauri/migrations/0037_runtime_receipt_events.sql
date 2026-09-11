-- D-02：原始 Runtime 事件先持久化；证据尚未连续时不得猜测推进 Receipt。
CREATE TABLE runtime_receipt_events (
    event_id          TEXT PRIMARY KEY NOT NULL,
    event_key         TEXT NOT NULL UNIQUE,
    event_type        TEXT NOT NULL CHECK (event_type IN ('PARENT_CHILD', 'TURN_FINISHED')),
    raw_event         TEXT NOT NULL CHECK (json_valid(raw_event)),
    schema_profile    TEXT,
    parent_thread_id  TEXT,
    codex_thread_id   TEXT,
    codex_turn_id     TEXT,
    successful        INTEGER CHECK (successful IN (0, 1)),
    observed_at       TEXT NOT NULL,
    processed_at      TEXT
);

CREATE INDEX idx_runtime_receipt_events_pending
    ON runtime_receipt_events(codex_thread_id, codex_turn_id, processed_at, observed_at);
