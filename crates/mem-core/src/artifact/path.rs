use std::fs;
use std::path::{Component, Path};

use anyhow::{Context, Result};

use crate::error;

use super::manifest::ArtifactKind;

const ALLOWED_ARTIFACT_DIRS: &[&str] = &["scripts", "templates", "snippets", "references"];

pub fn validate_artifact_path(path: &str) -> std::result::Result<(), String> {
    if path.len() > 4_096 || path.chars().any(char::is_control) {
        return Err("path exceeds 4096 bytes or contains control characters".to_string());
    }
    if path.is_empty() {
        return Err("path is empty".to_string());
    }
    if path
        .split(['/', '\\'])
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err("path must be normalized and must not escape the store".to_string());
    }
    let path = Path::new(path);
    if path.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    let components = path.components().collect::<Vec<_>>();
    if components.iter().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir | Component::CurDir
        )
    }) {
        return Err("path must be normalized and must not escape the store".to_string());
    }
    let mut iter = components.iter();
    if !matches!(iter.next(), Some(Component::Normal(value)) if value.to_string_lossy() == "artifacts")
    {
        return Err("path must start with artifacts/".to_string());
    }
    let Some(Component::Normal(kind)) = iter.next() else {
        return Err("path must include an artifact kind directory".to_string());
    };
    let kind = kind.to_string_lossy();
    if !ALLOWED_ARTIFACT_DIRS.contains(&kind.as_ref()) {
        return Err(format!("unsupported artifact directory: {kind}"));
    }
    if iter.next().is_none() {
        return Err("path must include a file name".to_string());
    }
    Ok(())
}

/// Resolve an artifact path without following symlinks in any store-relative
/// component. The returned path is guaranteed to name an existing regular
/// file at validation time.
pub fn validate_artifact_file(root: &Path, relative: &str) -> Result<std::path::PathBuf> {
    validate_artifact_path(relative).map_err(error::safety_violation)?;
    let components = Path::new(relative).components().collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(error::safety_violation(format!(
                "refusing unsafe artifact path: {relative}"
            )));
        };
        current.push(component);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(error::not_found(format!(
                    "artifact path not found: {}",
                    current.display()
                )));
            }
            Err(source) => {
                return Err(source)
                    .with_context(|| format!("inspect artifact path {}", current.display()));
            }
        };
        if metadata.file_type().is_symlink() {
            return Err(error::safety_violation(format!(
                "refusing artifact symlink: {}",
                current.display()
            )));
        }
        let is_last = index + 1 == components.len();
        if is_last && !metadata.is_file() {
            return Err(error::safety_violation(format!(
                "refusing non-regular artifact file: {}",
                current.display()
            )));
        }
        if !is_last && !metadata.is_dir() {
            return Err(error::safety_violation(format!(
                "refusing non-directory artifact path: {}",
                current.display()
            )));
        }
    }
    Ok(current)
}

pub(super) fn path_to_manifest_string(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| error::usage("artifact path is not valid UTF-8"))?;
    Ok(value.trim_start_matches("./").to_string())
}

pub(super) fn artifact_group(path: &str) -> Result<String> {
    let mut components = Path::new(path).components();
    let _artifact = components.next();
    let Some(Component::Normal(group)) = components.next() else {
        return Err(error::usage("artifact path missing group"));
    };
    Ok(group.to_string_lossy().to_string())
}

pub(super) fn artifact_name(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| error::usage("artifact path must include a file name"))?;
    if stem.is_empty() {
        return Err(error::usage("artifact name is empty"));
    }
    Ok(stem.to_string())
}

pub(super) fn validate_artifact_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(error::usage("artifact name is empty"));
    }
    if name.len() > 256 || name.chars().any(char::is_control) {
        return Err(error::usage(
            "artifact name exceeds 256 bytes or contains control characters",
        ));
    }
    if name.contains('.') || name.contains('/') || name.contains('\\') {
        return Err(error::usage(
            "artifact name must not contain '.', '/', or '\\'",
        ));
    }
    Ok(name.to_string())
}

pub(super) fn kind_group(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Script => "scripts",
        ArtifactKind::Template => "templates",
        ArtifactKind::Snippet => "snippets",
        ArtifactKind::Reference => "references",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_artifact_paths() {
        assert!(validate_artifact_path("artifacts/scripts/ci.sh").is_ok());
        assert!(validate_artifact_path("/tmp/ci.sh").is_err());
        assert!(validate_artifact_path("../ci.sh").is_err());
        assert!(validate_artifact_path("artifacts/../../ci.sh").is_err());
        assert!(validate_artifact_path("artifacts/scripts/./ci.sh").is_err());
        assert!(validate_artifact_path("artifacts//scripts/ci.sh").is_err());
        assert!(validate_artifact_path("artifacts/scripts/ci.sh/").is_err());
        assert!(validate_artifact_path("artifacts/bin/ci.sh").is_err());
    }
}
