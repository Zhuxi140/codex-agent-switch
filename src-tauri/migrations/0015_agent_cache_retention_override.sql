ALTER TABLE agents
ADD COLUMN cache_retention_override_seconds INTEGER
CHECK (
    cache_retention_override_seconds IS NULL
    OR (
        cache_retention_override_seconds > 0
        AND cache_retention_override_seconds <= 31536000
    )
);
