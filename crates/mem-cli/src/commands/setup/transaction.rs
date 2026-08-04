use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tempfile::TempDir;

struct Snapshot {
    path: PathBuf,
    backup: Option<PathBuf>,
}

/// Snapshot the setup-owned paths so a later policy, skill, or hook failure can
/// restore the complete pre-command state.
pub(super) struct SetupTransaction {
    _backup_root: TempDir,
    snapshots: Vec<Snapshot>,
}

impl SetupTransaction {
    pub(super) fn begin(paths: impl IntoIterator<Item = PathBuf>) -> Result<Self> {
        let backup_root = tempfile::Builder::new()
            .prefix("mnemark-setup-")
            .tempdir()
            .context("create setup rollback directory")?;
        let mut snapshots = Vec::new();
        for (index, path) in paths
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .enumerate()
        {
            let backup = match fs::symlink_metadata(&path) {
                Ok(_) => {
                    let backup = backup_root.path().join(index.to_string());
                    copy_node(&path, &backup)
                        .with_context(|| format!("snapshot setup target {}", path.display()))?;
                    Some(backup)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("inspect setup target {}", path.display()));
                }
            };
            snapshots.push(Snapshot { path, backup });
        }
        Ok(Self {
            _backup_root: backup_root,
            snapshots,
        })
    }

    pub(super) fn rollback(self) -> Result<()> {
        for snapshot in self.snapshots.iter().rev() {
            remove_node_if_present(&snapshot.path).with_context(|| {
                format!("remove changed setup target {}", snapshot.path.display())
            })?;
            if let Some(backup) = &snapshot.backup {
                copy_node(backup, &snapshot.path)
                    .with_context(|| format!("restore setup target {}", snapshot.path.display()))?;
            }
        }
        Ok(())
    }
}

fn copy_node(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        create_symlink(source, &target, destination)?;
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_node(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
        return Ok(());
    }
    if !metadata.is_file() {
        bail!("unsupported setup target type: {}", source.display());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination)?;
    fs::set_permissions(destination, metadata.permissions())?;
    Ok(())
}

fn remove_node_if_present(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
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

#[cfg(unix)]
fn create_symlink(_source: &Path, target: &Path, destination: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, destination)?;
    Ok(())
}

#[cfg(windows)]
fn create_symlink(source: &Path, target: &Path, destination: &Path) -> Result<()> {
    if fs::metadata(source)?.is_dir() {
        std::os::windows::fs::symlink_dir(target, destination)?;
    } else {
        std::os::windows::fs::symlink_file(target, destination)?;
    }
    Ok(())
}
