INSERT OR IGNORE INTO agent_required_capabilities (agent_id, capability)
SELECT agent_id, 'CODEX_MULTI_AGENT'
FROM agent_required_capabilities
WHERE capability = 'CODEX_MULTI_AGENT_V2';

DELETE FROM agent_required_capabilities
WHERE capability = 'CODEX_MULTI_AGENT_V2';

INSERT OR IGNORE INTO agent_preferred_capabilities (agent_id, capability)
SELECT agent_id, 'CODEX_MULTI_AGENT'
FROM agent_preferred_capabilities
WHERE capability = 'CODEX_MULTI_AGENT_V2';

DELETE FROM agent_preferred_capabilities
WHERE capability = 'CODEX_MULTI_AGENT_V2';
