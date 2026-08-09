CREATE TABLE agents (
    id                     TEXT PRIMARY KEY,
    agent_key              TEXT NOT NULL UNIQUE,
    name                   TEXT NOT NULL,
    description            TEXT NOT NULL,
    instruction            TEXT NOT NULL,
    agent_type             TEXT NOT NULL CHECK (agent_type IN ('PRESET', 'CUSTOM', 'IMPORTED')),
    enabled                INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    sandbox_policy         TEXT NOT NULL CHECK (sandbox_policy IN ('READ_ONLY', 'WORKSPACE_WRITE', 'DANGER_FULL_ACCESS', 'INHERIT')),
    reasoning_policy       TEXT NOT NULL CHECK (reasoning_policy IN ('INHERIT', 'LOW', 'MEDIUM', 'HIGH', 'MODEL_DEFAULT')),
    source                 TEXT NOT NULL CHECK (source IN ('CAS', 'USER', 'IMPORTED')),
    managed                INTEGER NOT NULL DEFAULT 1 CHECK (managed IN (0, 1)),
    minimum_context_window INTEGER CHECK (minimum_context_window IS NULL OR minimum_context_window > 0),
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL
);

CREATE TABLE agent_required_capabilities (
    agent_id   TEXT NOT NULL,
    capability TEXT NOT NULL,

    PRIMARY KEY(agent_id, capability),

    FOREIGN KEY(agent_id)
        REFERENCES agents(id)
        ON DELETE CASCADE
);

CREATE TABLE agent_preferred_capabilities (
    agent_id   TEXT NOT NULL,
    capability TEXT NOT NULL,

    PRIMARY KEY(agent_id, capability),

    FOREIGN KEY(agent_id)
        REFERENCES agents(id)
        ON DELETE CASCADE
);

CREATE TABLE agent_model_bindings (
    id         TEXT PRIMARY KEY,
    agent_id   TEXT NOT NULL UNIQUE,
    model_id   TEXT NOT NULL,
    enabled    INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    priority   INTEGER NOT NULL DEFAULT 0,
    source     TEXT NOT NULL CHECK (source IN ('CAS', 'USER', 'IMPORTED')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,

    FOREIGN KEY(agent_id)
        REFERENCES agents(id)
        ON DELETE CASCADE,

    FOREIGN KEY(model_id)
        REFERENCES models(id)
        ON DELETE RESTRICT
);

CREATE INDEX idx_agents_enabled ON agents(enabled);
CREATE INDEX idx_agent_bindings_model ON agent_model_bindings(model_id);
