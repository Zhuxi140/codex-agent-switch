ALTER TABLE models ADD COLUMN last_test_status TEXT
    CHECK (last_test_status IS NULL OR last_test_status IN (
        'SUCCESS',
        'CREDENTIAL_MISSING',
        'AUTH_FAILED',
        'MODEL_NOT_FOUND',
        'RATE_LIMITED',
        'PROTOCOL_ERROR',
        'UNREACHABLE',
        'SERVER_ERROR'
    ));

ALTER TABLE models ADD COLUMN last_tested_at TEXT;

ALTER TABLE models ADD COLUMN last_test_latency_ms INTEGER
    CHECK (last_test_latency_ms IS NULL OR last_test_latency_ms >= 0);
