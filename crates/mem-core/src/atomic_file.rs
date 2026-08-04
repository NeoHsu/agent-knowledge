//! Durable same-directory file replacement helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

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

        replace_staged_file(&temporary, &target)?;
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

/// Install a fully written staged file on Windows with best-effort rollback.
#[cfg(windows)]
pub fn replace_staged_file(temporary: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        fs::rename(temporary, target)?;
        return Ok(());
    }
    let metadata = fs::symlink_metadata(target)?;
    if !metadata.is_file() && !metadata.file_type().is_symlink() {
        bail!("output target is not a regular file: {}", target.display());
    }
    let backup =
        target.with_extension(format!("mnemark-replace-{}", uuid::Uuid::new_v4().simple()));
    fs::rename(target, &backup)?;
    if let Err(error) = fs::rename(temporary, target) {
        let _ = fs::rename(&backup, target);
        return Err(error.into());
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
