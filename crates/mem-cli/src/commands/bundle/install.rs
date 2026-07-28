use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

use super::*;

#[cfg(not(windows))]
pub(super) fn install_bundle_file(temporary: &Path, target: &Path) -> Result<()> {
    fs::rename(temporary, target)?;
    Ok(())
}

#[cfg(windows)]
pub(super) fn install_bundle_file(temporary: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        fs::rename(temporary, target)?;
        return Ok(());
    }
    let metadata = fs::symlink_metadata(target)?;
    if !metadata.is_file() && !metadata.file_type().is_symlink() {
        bail!(
            "bundle output target is not a regular file: {}",
            target.display()
        );
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
pub(super) fn harden_bundle_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn harden_bundle_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

pub(super) fn copy_regular_file_new(source: &Path, target: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "refusing to copy non-regular bundle file: {}",
            source.display()
        );
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let copy_result = (|| -> Result<()> {
        let mut input = File::open(source)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        #[cfg(unix)]
        {
            let source_mode = metadata.permissions().mode();
            let safe_mode = 0o600 | (source_mode & 0o100);
            fs::set_permissions(target, fs::Permissions::from_mode(safe_mode))?;
        }
        Ok(())
    })();
    if copy_result.is_err() {
        fs::remove_file(target).ok();
    }
    copy_result
}

pub(super) fn copy_if_exists(source: PathBuf, target: PathBuf) -> Result<()> {
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "refusing to copy non-regular bundle file: {}",
            source.display()
        );
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, target)?;
    Ok(())
}

pub(super) fn copy_dir_if_exists(source: PathBuf, target: PathBuf) -> Result<()> {
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "refusing to copy non-directory bundle tree: {}",
            source.display()
        );
    }
    for entry in fs::read_dir(&source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            bail!("refusing to copy bundle symlink: {}", source_path.display());
        }
        if metadata.is_dir() {
            copy_dir_if_exists(source_path, target_path)?;
        } else if metadata.is_file() {
            copy_if_exists(source_path, target_path)?;
        } else {
            bail!(
                "refusing to copy non-regular bundle entry: {}",
                source_path.display()
            );
        }
    }
    Ok(())
}

pub(super) struct RemoveDirOnDrop(pub(super) PathBuf);

impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

pub(super) fn temp_bundle_dir(label: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!(
        "mnemark-bundle-{label}-{}",
        uuid::Uuid::new_v4().simple()
    ));
    #[cfg(unix)]
    let builder = {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
    };
    #[cfg(not(unix))]
    let builder = fs::DirBuilder::new();
    builder
        .create(&path)
        .with_context(|| format!("create secure temporary directory {}", path.display()))?;
    Ok(path)
}
