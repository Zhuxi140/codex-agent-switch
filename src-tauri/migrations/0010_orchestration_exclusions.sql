CREATE TABLE project_orchestration_exclusions (
    id              TEXT PRIMARY KEY,
    project_path    TEXT NOT NULL,
    normalized_path TEXT NOT NULL COLLATE NOCASE UNIQUE,
    config_existed  INTEGER NOT NULL CHECK (config_existed IN (0, 1)),
    baseline_json   TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
