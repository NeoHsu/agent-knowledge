//! Durable same-directory file replacement helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(any(windows, test))]
use anyhow::anyhow;
use anyhow::{Context, Result, bail};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AtomicWriteStage {
    BeforeReplace,
    AfterReplace,
}

/// Atomically replace a regular file while preserving existing permissions.
/// Existing symlinks are followed so managed dotfile links keep their identity.
pub fn atomic_write(path: &Path, contents: &[u8]) -> Result<()> {
    atomic_write_inner(path, contents, None, true)
}

/// Atomically replace a private file with user-only permissions on Unix.
/// The path itself is replaced instead of following a symlink.
pub fn atomic_write_private(path: &Path, contents: &[u8]) -> Result<()> {
    atomic_write_inner(path, contents, Some(0o600), false)
}

fn atomic_write_inner(
    path: &Path,
    contents: &[u8],
    unix_mode: Option<u32>,
    follow_symlink: bool,
) -> Result<()> {
    atomic_write_inner_with_hook(path, contents, unix_mode, follow_symlink, |_| Ok(()))
}

fn atomic_write_inner_with_hook(
    path: &Path,
    contents: &[u8],
    unix_mode: Option<u32>,
    follow_symlink: bool,
    mut hook: impl FnMut(AtomicWriteStage) -> Result<()>,
) -> Result<()> {
    let target = resolved_target(path, follow_symlink)?;
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("create parent directory {}", parent.display()))?;

    let existing_permissions = fs::metadata(&target)
        .ok()
        .map(|metadata| metadata.permissions());
    let temporary = parent.join(format!(
        ".mnemark-write-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        if let Some(mode) = unix_mode {
            options.mode(mode);
        }
        let mut file = options
            .open(&temporary)
            .with_context(|| format!("create staged file {}", temporary.display()))?;
        file.write_all(contents)
            .with_context(|| format!("write staged file {}", temporary.display()))?;
        file.flush()?;
        file.sync_all()?;

        #[cfg(unix)]
        if let Some(mode) = unix_mode {
            fs::set_permissions(&temporary, fs::Permissions::from_mode(mode))?;
        } else if let Some(permissions) = existing_permissions {
            fs::set_permissions(&temporary, permissions)?;
        }
        #[cfg(not(unix))]
        if let Some(permissions) = existing_permissions {
            fs::set_permissions(&temporary, permissions)?;
        }

        hook(AtomicWriteStage::BeforeReplace)?;
        replace_staged_file(&temporary, &target)?;
        hook(AtomicWriteStage::AfterReplace)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        fs::remove_file(&temporary).ok();
    }
    result.with_context(|| format!("atomically write {}", path.display()))
}

fn resolved_target(path: &Path, follow_symlink: bool) -> Result<PathBuf> {
    if !follow_symlink {
        return Ok(path.to_path_buf());
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .with_context(|| format!("resolve symlinked output {}", path.display())),
        Ok(metadata) if metadata.is_file() => Ok(path.to_path_buf()),
        Ok(_) => bail!("output target is not a regular file: {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path.to_path_buf()),
        Err(error) => Err(error).with_context(|| format!("inspect output {}", path.display())),
    }
}

/// Install a fully written staged file at its destination, rolling back the
/// previous destination on platforms where rename cannot replace a file.
#[cfg(not(windows))]
pub fn replace_staged_file(temporary: &Path, target: &Path) -> Result<()> {
    fs::rename(temporary, target)?;
    Ok(())
}

/// Install a fully written staged file on Windows with rollback.
#[cfg(windows)]
pub fn replace_staged_file(temporary: &Path, target: &Path) -> Result<()> {
    replace_staged_file_with_rollback(temporary, target, |source, destination| {
        fs::rename(source, destination)
    })
}

#[cfg(any(windows, test))]
fn replace_staged_file_with_rollback(
    temporary: &Path,
    target: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    if !target.exists() {
        rename(temporary, target)?;
        return Ok(());
    }
    let metadata = fs::symlink_metadata(target)?;
    if !metadata.is_file() && !metadata.file_type().is_symlink() {
        bail!("output target is not a regular file: {}", target.display());
    }
    let backup =
        target.with_extension(format!("mnemark-replace-{}", uuid::Uuid::new_v4().simple()));
    rename(target, &backup)?;
    if let Err(replace_error) = rename(temporary, target) {
        if let Err(rollback_error) = rename(&backup, target) {
            return Err(anyhow!(
                "replace {} failed: {replace_error}; rollback from {} also failed: {rollback_error}",
                target.display(),
                backup.display()
            ));
        }
        return Err(replace_error.into());
    }
    fs::remove_file(backup).ok();
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn atomically_replaces_existing_contents() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("output.txt");
        fs::write(&path, "before").expect("seed output");

        atomic_write(&path, b"after").expect("atomic write");

        assert_eq!(fs::read_to_string(path).expect("read output"), "after");
    }

    proptest! {
        #[test]
        fn atomic_writes_round_trip_arbitrary_bytes(contents in proptest::collection::vec(any::<u8>(), 0..8192)) {
            let directory = tempfile::tempdir().expect("temp directory");
            let path = directory.path().join("arbitrary.bin");

            atomic_write(&path, &contents).expect("atomic write");

            prop_assert_eq!(fs::read(path).expect("read output"), contents);
        }
    }

    #[test]
    fn failure_before_replace_preserves_target_and_removes_staging_file() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("output.txt");
        fs::write(&path, "before").expect("seed output");

        let error =
            atomic_write_inner_with_hook(&path, b"after", None, true, |stage| match stage {
                AtomicWriteStage::BeforeReplace => Err(anyhow!("injected pre-replace failure")),
                AtomicWriteStage::AfterReplace => Ok(()),
            })
            .expect_err("pre-replace failure");

        assert!(error.to_string().contains("atomically write"));
        assert_eq!(fs::read_to_string(&path).expect("read target"), "before");
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("read directory")
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mnemark-write-"))
                .count(),
            0
        );
    }

    #[test]
    fn failure_after_replace_reports_committed_contents_without_staging_file() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("output.txt");
        fs::write(&path, "before").expect("seed output");

        let error =
            atomic_write_inner_with_hook(&path, b"after", None, true, |stage| match stage {
                AtomicWriteStage::BeforeReplace => Ok(()),
                AtomicWriteStage::AfterReplace => Err(anyhow!("injected post-replace failure")),
            })
            .expect_err("post-replace failure");

        assert!(error.to_string().contains("atomically write"));
        assert_eq!(fs::read_to_string(&path).expect("read target"), "after");
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("read directory")
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".mnemark-write-"))
                .count(),
            0
        );
    }

    #[test]
    fn rollback_replacement_helper_installs_new_and_existing_targets() {
        let directory = tempfile::tempdir().expect("temp directory");
        let new_target = directory.path().join("new.txt");
        let new_temporary = directory.path().join("new.tmp");
        fs::write(&new_temporary, "new").expect("seed new temporary");
        replace_staged_file_with_rollback(&new_temporary, &new_target, |source, target| {
            fs::rename(source, target)
        })
        .expect("install new target");
        assert_eq!(fs::read_to_string(&new_target).expect("new target"), "new");

        let existing_target = directory.path().join("existing.txt");
        let replacement = directory.path().join("existing.tmp");
        fs::write(&existing_target, "before").expect("seed existing target");
        fs::write(&replacement, "after").expect("seed replacement");
        replace_staged_file_with_rollback(&replacement, &existing_target, |source, target| {
            fs::rename(source, target)
        })
        .expect("replace existing target");
        assert_eq!(
            fs::read_to_string(&existing_target).expect("existing target"),
            "after"
        );
    }

    #[test]
    fn rollback_replacement_restores_previous_target() {
        let directory = tempfile::tempdir().expect("temp directory");
        let target = directory.path().join("target.txt");
        let temporary = directory.path().join("temporary.txt");
        fs::write(&target, "before").expect("seed target");
        fs::write(&temporary, "after").expect("seed temporary");
        let mut calls = 0;

        let error =
            replace_staged_file_with_rollback(&temporary, &target, |source, destination| {
                calls += 1;
                if calls == 2 {
                    Err(std::io::Error::other("injected replacement failure"))
                } else {
                    fs::rename(source, destination)
                }
            })
            .expect_err("replacement must fail");

        assert!(error.to_string().contains("injected replacement failure"));
        assert_eq!(calls, 3, "target move, failed replace, rollback");
        assert_eq!(fs::read_to_string(&target).expect("read target"), "before");
        assert_eq!(
            fs::read_to_string(&temporary).expect("read staged file"),
            "after"
        );
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("read directory")
                .filter_map(std::result::Result::ok)
                .count(),
            2,
            "rollback must not leave a backup"
        );
    }

    #[test]
    fn rollback_failure_reports_both_replacement_errors() {
        let directory = tempfile::tempdir().expect("temp directory");
        let target = directory.path().join("target.txt");
        let temporary = directory.path().join("temporary.txt");
        fs::write(&target, "before").expect("seed target");
        fs::write(&temporary, "after").expect("seed temporary");
        let mut calls = 0;

        let error =
            replace_staged_file_with_rollback(&temporary, &target, |source, destination| {
                calls += 1;
                if calls >= 2 {
                    Err(std::io::Error::other(format!("injected failure {calls}")))
                } else {
                    fs::rename(source, destination)
                }
            })
            .expect_err("replacement and rollback must fail");

        let message = error.to_string();
        assert!(message.contains("injected failure 2"));
        assert!(message.contains("rollback"));
        assert!(message.contains("injected failure 3"));
    }

    #[cfg(unix)]
    #[test]
    fn private_writes_use_user_only_permissions() {
        let directory = tempfile::tempdir().expect("temp directory");
        let path = directory.path().join("private.txt");

        atomic_write_private(&path, b"private").expect("private write");

        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_writes_replace_symlinks_without_changing_their_targets() {
        let directory = tempfile::tempdir().expect("temp directory");
        let target = directory.path().join("target.txt");
        let link = directory.path().join("private.txt");
        fs::write(&target, "target").expect("seed target");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");

        atomic_write_private(&link, b"private").expect("replace symlink");

        assert_eq!(fs::read_to_string(&target).expect("read target"), "target");
        assert_eq!(
            fs::read_to_string(&link).expect("read private file"),
            "private"
        );
        assert!(fs::symlink_metadata(&link).expect("metadata").is_file());
        assert_eq!(
            fs::metadata(&link).expect("metadata").permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn managed_writes_follow_existing_symlinks() {
        let directory = tempfile::tempdir().expect("temp directory");
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        fs::write(&target, "before").expect("seed target");
        std::os::unix::fs::symlink(&target, &link).expect("create symlink");

        atomic_write(&link, b"after").expect("atomic write through symlink");

        assert_eq!(fs::read_to_string(&target).expect("read target"), "after");
        assert!(
            fs::symlink_metadata(link)
                .expect("link metadata")
                .file_type()
                .is_symlink()
        );
    }
}
