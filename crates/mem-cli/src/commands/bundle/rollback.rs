use std::fs;
use std::path::{Path, PathBuf};

use super::install::{copy_dir_if_exists, copy_if_exists};
use super::*;

pub(super) fn store_has_durable_files(app: &App) -> bool {
    app.db_path.exists()
        || app.root.join("config.toml").exists()
        || app.root.join("manifest.toml").exists()
        || app.root.join("artifacts").exists()
}

pub(super) fn snapshot_store_for_replace(app: &App) -> Result<PathBuf> {
    let backup_root = app.root.join(format!(
        ".bundle-replace-backup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&backup_root)?;
    if app.db_path.is_file() {
        let source = app.read_conn()?;
        let mut destination = Connection::open(backup_root.join("memory.db"))?;
        let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
        backup.run_to_completion(SNAPSHOT_PAGES_PER_STEP, SNAPSHOT_STEP_PAUSE, None)?;
        drop(backup);
        drop(destination);
        drop(source);
    }
    copy_if_exists(
        app.root.join("config.toml"),
        backup_root.join("config.toml"),
    )?;
    copy_if_exists(
        app.root.join("manifest.toml"),
        backup_root.join("manifest.toml"),
    )?;
    copy_dir_if_exists(app.root.join("artifacts"), backup_root.join("artifacts"))?;
    Ok(backup_root)
}

pub(super) fn restore_store_after_failed_replace(app: &App, backup_root: &Path) -> Result<()> {
    copy_if_exists(backup_root.join("memory.db"), app.root.join("memory.db"))?;
    copy_if_exists(
        backup_root.join("config.toml"),
        app.root.join("config.toml"),
    )?;
    copy_if_exists(
        backup_root.join("manifest.toml"),
        app.root.join("manifest.toml"),
    )?;
    copy_dir_if_exists(backup_root.join("artifacts"), app.root.join("artifacts"))?;
    if app.db_path.exists() {
        app.require_schema()?;
        app.harden_permissions()?;
        memory_index::reindex_or_mark_stale(app, "restore index after failed bundle replace")?;
    }
    Ok(())
}

pub(super) fn clear_store_for_replace(app: &App) -> Result<()> {
    for path in [
        app.root.join("memory.db"),
        app.root.join("memory.db-wal"),
        app.root.join("memory.db-shm"),
        app.root.join("config.toml"),
        app.root.join("manifest.toml"),
        app.root.join("artifacts"),
        app.root.join("index"),
    ] {
        remove_path_if_exists(path)?;
    }
    Ok(())
}

fn remove_path_if_exists(path: PathBuf) -> Result<()> {
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}
