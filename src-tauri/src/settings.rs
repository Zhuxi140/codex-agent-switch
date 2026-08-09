use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Deserializer, Serialize};

use crate::persistence::{PersistenceError, open_database};
use crate::provider::ApiError;

const DEFAULT_UPDATE_CHANNEL: &str = "STABLE";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(crate) enum Appearance {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsResponse {
    pub(crate) appearance: Appearance,
    pub(crate) auto_backup_enabled: bool,
    pub(crate) update_channel: String,
    pub(crate) custom_codex_home: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsUpdateRequest {
    appearance: Option<Appearance>,
    auto_backup_enabled: Option<bool>,
    update_channel: Option<String>,
    #[serde(default)]
    custom_codex_home: NullableStringUpdate,
}

#[derive(Debug, Default)]
enum NullableStringUpdate {
    #[default]
    Missing,
    Null,
    Value(String),
}

impl<'de> Deserialize<'de> for NullableStringUpdate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(value) => Self::Value(value),
            None => Self::Null,
        })
    }
}

pub(crate) fn get_settings(database_path: &Path) -> Result<SettingsResponse, SettingsError> {
    let connection = open_database(database_path)?;
    read_settings(&connection)
}

pub(crate) fn update_settings(
    database_path: &Path,
    request: SettingsUpdateRequest,
) -> Result<SettingsResponse, SettingsError> {
    if request.auto_backup_enabled == Some(false) {
        return Err(SettingsError::InvalidField("autoBackupEnabled"));
    }
    let update_channel = request
        .update_channel
        .as_deref()
        .map(validate_update_channel)
        .transpose()?;
    let custom_codex_home = match request.custom_codex_home {
        NullableStringUpdate::Missing => None,
        NullableStringUpdate::Null => Some(None),
        NullableStringUpdate::Value(value) => Some(Some(validate_codex_home(&value)?)),
    };

    let mut connection = open_database(database_path)?;
    let transaction = connection.transaction()?;
    if let Some(appearance) = request.appearance {
        upsert_setting(
            &transaction,
            "appearance",
            appearance_value(appearance),
            "STRING",
        )?;
    }
    if request.auto_backup_enabled == Some(true) {
        upsert_setting(&transaction, "auto_backup_enabled", "true", "BOOLEAN")?;
    }
    if let Some(update_channel) = update_channel {
        upsert_setting(&transaction, "update_channel", update_channel, "STRING")?;
    }
    if let Some(value) = custom_codex_home {
        match value {
            Some(path) => upsert_setting(&transaction, "custom_codex_home", &path, "PATH")?,
            None => {
                transaction.execute(
                    "DELETE FROM application_settings WHERE setting_key = 'custom_codex_home'",
                    [],
                )?;
            }
        }
    }
    transaction.commit()?;
    read_settings(&connection)
}

pub(crate) fn read_custom_codex_home(
    connection: &Connection,
) -> Result<Option<PathBuf>, rusqlite::Error> {
    Ok(connection
        .query_row(
            "SELECT setting_value FROM application_settings
             WHERE setting_key = 'custom_codex_home'",
            [],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .map(PathBuf::from))
}

fn read_settings(connection: &Connection) -> Result<SettingsResponse, SettingsError> {
    let mut response = SettingsResponse {
        appearance: Appearance::System,
        auto_backup_enabled: true,
        update_channel: DEFAULT_UPDATE_CHANNEL.to_owned(),
        custom_codex_home: None,
    };
    let mut statement = connection.prepare(
        "SELECT setting_key, setting_value FROM application_settings ORDER BY setting_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        match key.as_str() {
            "appearance" => {
                response.appearance = parse_appearance(value.as_deref())?;
            }
            "auto_backup_enabled" => {
                response.auto_backup_enabled = match value.as_deref() {
                    Some("true") => true,
                    Some("false") => false,
                    _ => return Err(SettingsError::InvalidStoredValue),
                };
            }
            "update_channel" => {
                response.update_channel = value.ok_or(SettingsError::InvalidStoredValue)?;
            }
            "custom_codex_home" => response.custom_codex_home = value,
            _ => {}
        }
    }
    Ok(response)
}

fn upsert_setting(
    connection: &Connection,
    key: &str,
    value: &str,
    value_type: &str,
) -> Result<(), rusqlite::Error> {
    connection.execute(
        "INSERT INTO application_settings (
            setting_key, setting_value, value_type, source, updated_at
         ) VALUES (?1, ?2, ?3, 'USER', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
         ON CONFLICT(setting_key) DO UPDATE SET
            setting_value = excluded.setting_value,
            value_type = excluded.value_type,
            source = excluded.source,
            updated_at = excluded.updated_at",
        params![key, value, value_type],
    )?;
    Ok(())
}

fn validate_codex_home(value: &str) -> Result<String, SettingsError> {
    let value = value.trim();
    let path = Path::new(value);
    if value.is_empty() || value.len() > 4096 || !path.is_absolute() || !path.is_dir() {
        return Err(SettingsError::InvalidField("customCodexHome"));
    }
    Ok(path.to_string_lossy().into_owned())
}

fn validate_update_channel(value: &str) -> Result<&str, SettingsError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 32
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(SettingsError::InvalidField("updateChannel"));
    }
    Ok(value)
}

fn parse_appearance(value: Option<&str>) -> Result<Appearance, SettingsError> {
    match value {
        Some("SYSTEM") => Ok(Appearance::System),
        Some("LIGHT") => Ok(Appearance::Light),
        Some("DARK") => Ok(Appearance::Dark),
        _ => Err(SettingsError::InvalidStoredValue),
    }
}

fn appearance_value(appearance: Appearance) -> &'static str {
    match appearance {
        Appearance::System => "SYSTEM",
        Appearance::Light => "LIGHT",
        Appearance::Dark => "DARK",
    }
}

#[derive(Debug)]
pub(crate) enum SettingsError {
    InvalidField(&'static str),
    InvalidStoredValue,
    Persistence(PersistenceError),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidField(_) => "invalid settings field",
            Self::InvalidStoredValue => "invalid stored setting",
            Self::Persistence(_) => "settings persistence failed",
            Self::Sqlite(_) => "settings database operation failed",
        })
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persistence(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PersistenceError> for SettingsError {
    fn from(error: PersistenceError) -> Self {
        Self::Persistence(error)
    }
}

impl From<rusqlite::Error> for SettingsError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl From<SettingsError> for ApiError {
    fn from(error: SettingsError) -> Self {
        match error {
            SettingsError::InvalidField(field) => ApiError::new(
                "VALIDATION_ERROR",
                "Settings 字段无效。",
                false,
                Some(BTreeMap::from([("field", field.to_owned())])),
            ),
            SettingsError::Persistence(PersistenceError::SchemaTooNew) => ApiError::new(
                "DATABASE_SCHEMA_TOO_NEW",
                "数据库版本高于当前应用支持版本。",
                false,
                None,
            ),
            SettingsError::InvalidStoredValue => {
                ApiError::new("SETTINGS_INVALID", "CAS 设置数据无效。", false, None)
            }
            SettingsError::Persistence(_) | SettingsError::Sqlite(_) => ApiError::new(
                "DATABASE_OPERATION_FAILED",
                "CAS 设置保存失败。",
                true,
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use uuid::Uuid;

    use super::*;

    #[test]
    fn settings_round_trip_and_clear_codex_home() {
        let root = std::env::temp_dir().join(format!("cas-settings-{}", Uuid::new_v4()));
        let codex_home = root.join("codex");
        fs::create_dir_all(&codex_home).unwrap();
        let database = root.join("cas.db");

        assert_eq!(
            get_settings(&database).unwrap().appearance,
            Appearance::System
        );
        let updated = update_settings(
            &database,
            SettingsUpdateRequest {
                appearance: Some(Appearance::Dark),
                auto_backup_enabled: Some(true),
                update_channel: Some("BETA".to_owned()),
                custom_codex_home: NullableStringUpdate::Value(
                    codex_home.to_string_lossy().into_owned(),
                ),
            },
        )
        .unwrap();
        assert_eq!(updated.appearance, Appearance::Dark);
        assert_eq!(updated.custom_codex_home.as_deref(), codex_home.to_str());

        let cleared = update_settings(
            &database,
            SettingsUpdateRequest {
                appearance: None,
                auto_backup_enabled: None,
                update_channel: None,
                custom_codex_home: NullableStringUpdate::Null,
            },
        )
        .unwrap();
        assert_eq!(cleared.custom_codex_home, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_backup_cannot_be_disabled() {
        let root = std::env::temp_dir().join(format!("cas-settings-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let database = root.join("cas.db");
        let error = update_settings(
            &database,
            SettingsUpdateRequest {
                appearance: None,
                auto_backup_enabled: Some(false),
                update_channel: None,
                custom_codex_home: NullableStringUpdate::Missing,
            },
        )
        .unwrap_err();

        assert!(matches!(
            error,
            SettingsError::InvalidField("autoBackupEnabled")
        ));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_home_patch_distinguishes_missing_and_null() {
        let missing: SettingsUpdateRequest = serde_json::from_str("{}").unwrap();
        let null: SettingsUpdateRequest =
            serde_json::from_str(r#"{"customCodexHome":null}"#).unwrap();

        assert!(matches!(
            missing.custom_codex_home,
            NullableStringUpdate::Missing
        ));
        assert!(matches!(null.custom_codex_home, NullableStringUpdate::Null));
    }
}
