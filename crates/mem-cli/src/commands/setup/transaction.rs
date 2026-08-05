use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::fs::FileTypeExt;

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
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        #[cfg(windows)]
        if file_type.is_symlink_dir() {
            fs::remove_dir(path)?;
            return Ok(());
        }
        fs::remove_file(path)?;
    } else if metadata.is_dir() {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_restores_files_directories_and_removes_new_targets() {
        let root = tempfile::tempdir().expect("test root");
        let file = root.path().join("policy.md");
        let directory = root.path().join("skill");
        let new_target = root.path().join("new-hook.json");
        fs::write(&file, "before").expect("seed policy");
        fs::create_dir(&directory).expect("seed skill directory");
        fs::write(directory.join("SKILL.md"), "before skill").expect("seed skill");

        let transaction = SetupTransaction::begin([
            file.clone(),
            directory.clone(),
            new_target.clone(),
            file.clone(),
        ])
        .expect("begin transaction");
        fs::write(&file, "after").expect("change policy");
        fs::remove_dir_all(&directory).expect("remove skill directory");
        fs::write(&directory, "replacement file").expect("replace skill with file");
        fs::write(&new_target, "new hook").expect("create hook");

        transaction.rollback().expect("rollback transaction");

        assert_eq!(fs::read_to_string(&file).expect("read policy"), "before");
        assert_eq!(
            fs::read_to_string(directory.join("SKILL.md")).expect("read skill"),
            "before skill"
        );
        assert!(!new_target.exists());
    }

    #[cfg(windows)]
    #[test]
    fn rollback_removes_new_directory_link_before_restoring_earlier_snapshot() {
        use std::os::windows::fs::symlink_dir;

        let root = tempfile::tempdir().expect("test root");
        let policy = root.path().join("a-policy.md");
        let target = root.path().join("shared-skill");
        let link = root.path().join("z-platform-skill");
        fs::write(&policy, "before").expect("seed policy");
        fs::create_dir(&target).expect("create shared skill");

        let transaction =
            SetupTransaction::begin([policy.clone(), link.clone()]).expect("begin transaction");
        fs::write(&policy, "after").expect("change policy");
        symlink_dir(&target, &link).expect("create directory link");

        transaction.rollback().expect("rollback transaction");

        assert_eq!(fs::read_to_string(&policy).expect("read policy"), "before");
        assert!(matches!(
            fs::symlink_metadata(&link),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rollback_preserves_permissions_and_symlink_identity() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let root = tempfile::tempdir().expect("test root");
        let file = root.path().join("policy.md");
        let target = root.path().join("shared-skill");
        let link = root.path().join("platform-skill");
        fs::write(&file, "before").expect("seed policy");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o640)).expect("set policy mode");
        fs::create_dir(&target).expect("create shared skill");
        symlink(&target, &link).expect("create skill link");

        let transaction =
            SetupTransaction::begin([file.clone(), link.clone()]).expect("begin transaction");
        fs::write(&file, "after").expect("change policy");
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).expect("change policy mode");
        fs::remove_file(&link).expect("remove skill link");
        fs::write(&link, "replacement").expect("replace link with file");

        transaction.rollback().expect("rollback transaction");

        assert_eq!(fs::read_to_string(&file).expect("read policy"), "before");
        assert_eq!(
            fs::metadata(&file)
                .expect("policy metadata")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert!(
            fs::symlink_metadata(&link)
                .expect("link metadata")
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link).expect("link target"), target);
    }
}
