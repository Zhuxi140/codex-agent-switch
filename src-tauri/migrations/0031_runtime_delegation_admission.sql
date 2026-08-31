ALTER TABLE runtime_delegation_leases ADD COLUMN admission_tool_use_id TEXT;
ALTER TABLE runtime_delegation_leases ADD COLUMN admitted_at TEXT;

CREATE INDEX idx_runtime_delegation_leases_admission
    ON runtime_delegation_leases(admission_tool_use_id, state, expires_at);
