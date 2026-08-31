ALTER TABLE runtime_delegation_leases ADD COLUMN admission_confirmed_at TEXT;

CREATE INDEX idx_runtime_delegation_leases_confirmation
    ON runtime_delegation_leases(admission_confirmed_at, admitted_at, state);
