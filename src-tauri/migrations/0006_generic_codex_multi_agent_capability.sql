INSERT OR IGNORE INTO model_capabilities (
    model_id,
    capability,
    status,
    source,
    confidence,
    verified_at,
    evidence_version,
    details_json
)
SELECT
    model_id,
    'CODEX_MULTI_AGENT',
    status,
    source,
    confidence,
    verified_at,
    evidence_version,
    details_json
FROM model_capabilities
WHERE capability = 'CODEX_MULTI_AGENT_V2';

DELETE FROM model_capabilities
WHERE capability = 'CODEX_MULTI_AGENT_V2';
