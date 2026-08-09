CREATE TABLE application_settings (
    setting_key   TEXT PRIMARY KEY,
    setting_value TEXT,
    value_type    TEXT NOT NULL,
    source        TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

INSERT INTO application_settings (
    setting_key, setting_value, value_type, source, updated_at
) VALUES
    ('appearance', 'SYSTEM', 'STRING', 'DEFAULT', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('auto_backup_enabled', 'true', 'BOOLEAN', 'DEFAULT', strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    ('update_channel', 'STABLE', 'STRING', 'DEFAULT', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
