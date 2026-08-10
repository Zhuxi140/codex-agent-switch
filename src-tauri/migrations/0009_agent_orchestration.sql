ALTER TABLE agents
ADD COLUMN role_key TEXT;

ALTER TABLE agents
ADD COLUMN orchestration_phase TEXT
CHECK (
    orchestration_phase IS NULL
    OR orchestration_phase IN ('DISCOVERY', 'EXECUTION', 'VERIFICATION', 'REVIEW')
);

UPDATE agents
SET role_key = agent_key,
    orchestration_phase = CASE agent_key
        WHEN 'explorer' THEN 'DISCOVERY'
        WHEN 'executor' THEN 'EXECUTION'
        WHEN 'tester' THEN 'VERIFICATION'
        WHEN 'reviewer' THEN 'REVIEW'
    END
WHERE agent_key IN ('explorer', 'executor', 'tester', 'reviewer');

CREATE TABLE active_agent_bindings (
    role_key   TEXT PRIMARY KEY,
    agent_id   TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(agent_id)
        REFERENCES agents(id)
        ON DELETE RESTRICT
);

ALTER TABLE configuration_state
ADD COLUMN orchestration_baseline_json TEXT;
