ALTER TABLE agents
ADD COLUMN reuse_strategy TEXT NOT NULL DEFAULT 'AUTO'
CHECK (reuse_strategy IN ('AUTO', 'HOT', 'COLD'));

ALTER TABLE providers
ADD COLUMN cache_support TEXT NOT NULL DEFAULT 'UNKNOWN'
CHECK (cache_support IN ('UNKNOWN', 'SUPPORTED', 'UNSUPPORTED'));

ALTER TABLE providers
ADD COLUMN cache_retention_type TEXT NOT NULL DEFAULT 'UNKNOWN'
CHECK (cache_retention_type IN ('UNKNOWN', 'APPROXIMATE', 'GUARANTEED'));

ALTER TABLE providers
ADD COLUMN cache_retention_hint_seconds INTEGER
CHECK (
    cache_retention_hint_seconds IS NULL
    OR cache_retention_hint_seconds > 0
);

ALTER TABLE providers
ADD COLUMN cache_profile_source TEXT;

ALTER TABLE providers
ADD COLUMN cache_profile_verified_at TEXT;
