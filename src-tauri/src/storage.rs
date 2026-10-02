use std::collections::BTreeSet;
use std::error::Error;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use uuid::Uuid;

use crate::{persistence::open_database, provider::ProviderService};

/// 使用当前用户目录，不依赖 AppData、安装位置或启动进程的 MSIX 虚拟化。
pub fn data_home(user_home: &Path, identifier: &str) -> io::Result<PathBuf> {
    if !user_home.is_absolute()
        || !identifier.starts_with(|c: char| c.is_ascii_alphanumeric())
        || !identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(io::Error::other("CAS 用户目录或应用标识无效"));
    }
    Ok(user_home.join(".codex-agent-switch").join(identifier))
}

/// 只读选取旧库；通过 SQLite 快照迁移 WAL，并在全部成功后发布新目录。
pub(crate) fn prepare_data_home(
    user_home: &Path,
    identifier: &str,
    legacy_homes: &[PathBuf],
) -> Result<PathBuf, Box<dyn Error>> {
    let destination = data_home(user_home, identifier)?;
    if destination.exists() {
        if !destination.join("cas.db").is_file() {
            return Err(io::Error::other(format!(
                "CAS 固定目录已有文件但缺少 cas.db，请先核对，未自动覆盖：{}",
                destination.display()
            ))
            .into());
        }
        return Ok(destination);
    }

    let mut candidates = legacy_homes.to_vec();
    #[cfg(windows)]
    for legacy in legacy_homes {
        let Some(parent) = legacy.parent() else {
            continue;
        };
        let packages = parent.join("Packages");
        if packages.is_dir() {
            for package in fs::read_dir(packages)? {
                let cache = package?.path().join("LocalCache");
                for kind in ["Local", "Roaming"] {
                    candidates.push(cache.join(kind).join(identifier));
                }
            }
        }
    }
    let source = select_legacy(&candidates)?;
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("CAS 目录无父目录"))?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(".{}-迁移-{}", identifier, Uuid::new_v4()));
    fs::create_dir(&staging)?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        let database = staging.join("cas.db");
        if let Some(source) = &source {
            let connection = open_read_only(&source.join("cas.db"))?;
            connection.backup(rusqlite::MAIN_DB, &database, None)?;
            copy_backups(&source.join("backups"), &staging.join("backups"))?;
        }
        let connection = open_database(&database)?;
        if let Some(source) = &source {
            let snapshots = connection
                .prepare("SELECT id, snapshot_path FROM configuration_snapshots")?
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (id, path) in snapshots {
                if let Ok(relative) = Path::new(&path).strip_prefix(source.join("backups"))
                    && relative
                        .components()
                        .all(|part| matches!(part, Component::Normal(_)))
                {
                    connection.execute(
                        "UPDATE configuration_snapshots SET snapshot_path = ?1 WHERE id = ?2",
                        [
                            destination
                                .join("backups")
                                .join(relative)
                                .to_string_lossy()
                                .as_ref(),
                            &id,
                        ],
                    )?;
                }
            }
        }
        drop(connection);
        ProviderService::initialize_native(&database)?;
        // SQLite 连接已全部关闭；连同备份一次发布，不覆盖任何既有目录。
        if destination.exists() {
            return Err(io::Error::other("CAS 数据目录已由另一实例建立，请重新启动").into());
        }
        fs::rename(&staging, &destination)?;
        Ok(())
    })();
    if result.is_err() {
        // staging 是本次 create_dir 新建的 UUID 子目录，只清除此范围。
        if staging.parent() == Some(parent)
            && fs::symlink_metadata(&staging)
                .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
        {
            let _ = fs::remove_dir_all(&staging);
        }
    }
    result?;
    if let Some(source) = source {
        eprintln!("CAS 已复制旧库（原文件保留）：{}", source.display());
    }
    Ok(destination)
}

fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
}

fn select_legacy(candidates: &[PathBuf]) -> Result<Option<PathBuf>, Box<dyn Error>> {
    let mut seen = BTreeSet::new();
    let mut valid = Vec::new();
    let mut populated = Vec::new();
    for home in candidates {
        let database = home.join("cas.db");
        if !database.is_file() || !seen.insert(fs::canonicalize(&database)?) {
            continue;
        }
        let connection = open_read_only(&database)?;
        let tables = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<BTreeSet<_>, _>>()?;
        if tables.is_empty() {
            continue; // 旧的 4 KiB 无表文件不是可恢复的 CAS 配置。
        }
        if !tables.contains("schema_migrations") || !tables.contains("providers") {
            return Err(io::Error::other(format!(
                "旧库不是有效 CAS 数据库，未自动替换：{}",
                database.display()
            ))
            .into());
        }
        let mut records = 0_i64;
        for table in ["providers", "models", "agents"] {
            if tables.contains(table) {
                records +=
                    connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                        r.get::<_, i64>(0)
                    })?;
            }
        }
        valid.push(home.clone());
        if records > 0 {
            populated.push(home.clone());
        }
    }
    if populated.len() > 1 {
        return Err(io::Error::other(format!(
            "发现多个含配置的旧库，需先选择来源，未合并或覆盖：{}",
            populated
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("；")
        ))
        .into());
    }
    Ok(populated.pop().or_else(|| valid.into_iter().next()))
}

fn copy_backups(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("旧备份包含链接，未自动迁移"));
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_backups(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        fs::copy(source, destination)?;
    } else {
        return Err(io::Error::other("旧备份包含非普通文件，未自动迁移"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentListRequest, AgentService};
    use crate::model::{ModelListRequest, ModelService};
    use crate::provider::ProviderListRequest;
    use crate::runtime_bridge::RuntimeBridgeService;
    use serde_json::{Value, json, to_value};

    const IDENTIFIER: &str = "com.codexagentswitch.desktop";

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("cas-storage-{}", Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn prepare(&self, legacy: &[PathBuf]) -> Result<PathBuf, Box<dyn Error>> {
            prepare_data_home(&self.0.join("用户 home"), IDENTIFIER, legacy)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            assert!(self.0.starts_with(std::env::temp_dir()));
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn records(database: &Path) -> Value {
        json!({
            "providers": ProviderService::open(database).unwrap().list(ProviderListRequest::default()).unwrap(),
            "models": ModelService::open(database).unwrap().list(ModelListRequest::default()).unwrap(),
            "agents": AgentService::open(database).unwrap().list(AgentListRequest::default()).unwrap(),
        })
    }

    fn add_agent(database: &Path) {
        let catalog = to_value(
            ModelService::open(database)
                .unwrap()
                .list(ModelListRequest::default())
                .unwrap(),
        )
        .unwrap();
        let model = catalog
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["modelId"] == "gpt-6-luna")
            .unwrap();
        AgentService::open(database).unwrap().create(serde_json::from_value(json!({
            "agentKey": "storage-worker", "name": "持久化测试",
                "description": "验证落盘和绑定", "instruction": "仅执行指定任务", "enabled": true,
            "sandboxPolicy": "WORKSPACE_WRITE", "reasoningPolicy": "MEDIUM",
            "modelId": model["id"], "roleKey": "storage-worker", "orchestrationPhase": "EXECUTION"
        })).unwrap()).unwrap();
    }

    #[test]
    fn path_uses_current_user_and_rejects_escape() {
        let fixture = Fixture::new();
        for name in ["用户甲", "another-user"] {
            let user = fixture.0.join(name);
            assert_eq!(
                data_home(&user, IDENTIFIER).unwrap(),
                user.join(".codex-agent-switch").join(IDENTIFIER)
            );
        }
        assert_ne!(
            data_home(&fixture.0, IDENTIFIER).unwrap(),
            data_home(&fixture.0, "com.codexagentswitch.test").unwrap()
        );
        for identifier in ["", "..", "../other", "C:\\other", "/other"] {
            assert!(data_home(&fixture.0, identifier).is_err());
        }
        assert!(data_home(Path::new("relative"), IDENTIFIER).is_err());
    }

    #[test]
    fn fresh_native_and_agent_survive_service_shutdown_and_restart() {
        let fixture = Fixture::new();
        let home = fixture.prepare(&[]).unwrap();
        let database = home.join("cas.db");
        let initial = records(&database);
        assert_eq!(initial["providers"].as_array().unwrap().len(), 1);
        assert_eq!(initial["models"].as_array().unwrap().len(), 6);
        assert!(initial["agents"].as_array().unwrap().is_empty());
        add_agent(&database);
        let expected = records(&database);
        let runtime = RuntimeBridgeService::open(&database, &home, &home.join("helper")).unwrap();
        runtime.stop().unwrap();
        drop(runtime); // 覆盖应用退出时实际调用的 Drop；不启动任何 Codex 进程。
        for _ in 0..2 {
            assert_eq!(fixture.prepare(&[]).unwrap(), home);
            assert_eq!(records(&database), expected);
        }
    }

    #[test]
    fn restart_does_not_recreate_user_deleted_native() {
        let fixture = Fixture::new();
        let home = fixture.prepare(&[]).unwrap();
        let database = home.join("cas.db");
        let initial = records(&database);
        for model in initial["models"].as_array().unwrap() {
            ModelService::open(&database)
                .unwrap()
                .delete(serde_json::from_value(json!({ "modelId": model["id"] })).unwrap())
                .unwrap();
        }
        let provider_id = initial["providers"][0]["id"].clone();
        ProviderService::open(&database)
            .unwrap()
            .delete(serde_json::from_value(json!({ "providerId": provider_id })).unwrap())
            .unwrap();
        fixture.prepare(&[]).unwrap();
        assert!(
            records(&database)["providers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn migration_keeps_wal_bindings_backups_and_source_unchanged() {
        let fixture = Fixture::new();
        let legacy = fixture.0.join("legacy");
        let database = legacy.join("cas.db");
        ProviderService::initialize_native(&database).unwrap();
        let connection = open_database(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA wal_autocheckpoint = 0; UPDATE providers SET name = '旧供应商的 WAL 数据';",
            )
            .unwrap();
        add_agent(&database);
        let expected = records(&database);
        let backup = legacy.join("backups/snapshot-1");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("config.toml"), b"snapshot fixture").unwrap();
        connection.execute(
            "INSERT INTO configuration_snapshots (id, reason, codex_home, snapshot_path, status, created_at)
             VALUES ('snapshot-1', 'test', 'test', ?1, 'COMPLETED', 'test')",
            [backup.to_string_lossy().as_ref()],
        ).unwrap();
        let main_before = fs::read(&database).unwrap();
        let wal = legacy.join("cas.db-wal");
        let wal_before = fs::read(&wal).unwrap();
        assert!(!wal_before.is_empty());
        let home = fixture.prepare(&[legacy.clone(), legacy.clone()]).unwrap();
        assert_eq!(records(&home.join("cas.db")), expected);
        assert_eq!(fs::read(&database).unwrap(), main_before);
        assert_eq!(fs::read(&wal).unwrap(), wal_before);
        assert_eq!(
            fs::read(home.join("backups/snapshot-1/config.toml")).unwrap(),
            b"snapshot fixture"
        );
        assert_eq!(
            open_database(&home.join("cas.db"))
                .unwrap()
                .query_row(
                    "SELECT snapshot_path FROM configuration_snapshots WHERE id = 'snapshot-1'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            home.join("backups").join("snapshot-1").to_string_lossy()
        );
        drop(connection);
        assert_eq!(records(&home.join("cas.db")), expected);
    }

    #[test]
    fn empty_legacy_stub_is_not_modified() {
        let fixture = Fixture::new();
        let legacy = fixture.0.join("legacy");
        fs::create_dir(&legacy).unwrap();
        let database = legacy.join("cas.db");
        Connection::open(&database)
            .unwrap()
            .execute_batch("VACUUM")
            .unwrap();
        let before = fs::read(&database).unwrap();
        let home = fixture.prepare(&[legacy]).unwrap();
        assert_eq!(fs::read(&database).unwrap(), before);
        assert_eq!(
            records(&home.join("cas.db"))["models"]
                .as_array()
                .unwrap()
                .len(),
            6
        );
    }

    #[test]
    fn conflicting_populated_databases_are_not_arbitrarily_selected() {
        let fixture = Fixture::new();
        let sources = [fixture.0.join("legacy-1"), fixture.0.join("legacy-2")];
        for source in &sources {
            ProviderService::initialize_native(&source.join("cas.db")).unwrap();
        }
        assert!(
            fixture
                .prepare(&sources)
                .unwrap_err()
                .to_string()
                .contains("多个")
        );
        assert!(
            !data_home(&fixture.0.join("用户 home"), IDENTIFIER)
                .unwrap()
                .exists()
        );
        for source in &sources {
            assert_eq!(
                records(&source.join("cas.db"))["providers"]
                    .as_array()
                    .unwrap()
                    .len(),
                1
            );
        }
    }

    #[test]
    fn failed_upgrade_is_not_published_and_does_not_modify_source() {
        let fixture = Fixture::new();
        let legacy = fixture.0.join("legacy");
        let database = legacy.join("cas.db");
        let connection = open_database(&database).unwrap();
        connection.execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (999, 'future', 'future')", []
        ).unwrap();
        drop(connection);
        let before = fs::read(&database).unwrap();
        assert!(fixture.prepare(&[legacy]).is_err());
        let home = data_home(&fixture.0.join("用户 home"), IDENTIFIER).unwrap();
        assert!(!home.exists());
        assert_eq!(fs::read_dir(home.parent().unwrap()).unwrap().count(), 0);
        assert_eq!(fs::read(&database).unwrap(), before);
    }

    #[test]
    fn existing_incomplete_fixed_directory_is_not_overwritten() {
        let fixture = Fixture::new();
        let home = data_home(&fixture.0.join("用户 home"), IDENTIFIER).unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("keep.txt"), b"existing user data").unwrap();
        assert!(fixture.prepare(&[]).is_err());
        assert_eq!(
            fs::read(home.join("keep.txt")).unwrap(),
            b"existing user data"
        );
        assert!(!home.join("cas.db").exists());
    }

    #[cfg(windows)]
    #[test]
    fn discovers_packaged_database_for_same_identifier_only() {
        let fixture = Fixture::new();
        let local = fixture.0.join("AppData/Local");
        let legacy = local.join(IDENTIFIER);
        let package = local.join("Packages/Example.Codex/LocalCache/Local");
        let correct = package.join(IDENTIFIER).join("cas.db");
        let unrelated = package.join("com.codexagentswitch.other").join("cas.db");
        ProviderService::initialize_native(&correct).unwrap();
        ProviderService::initialize_native(&unrelated).unwrap();
        add_agent(&correct);
        let expected = records(&correct);
        let home = fixture.prepare(&[legacy]).unwrap();
        assert_eq!(records(&home.join("cas.db")), expected);
    }
}
