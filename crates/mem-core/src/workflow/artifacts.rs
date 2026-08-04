use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_yaml::Value as YamlValue;

use super::yaml_get;
use crate::artifact::{
    ArtifactManifest, artifact_file_checksum, artifact_file_is_executable, validate_artifact_file,
    validate_artifact_path,
};
use crate::error;

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowArtifactReport {
    /// Knowledge-store artifacts checked against the manifest and filesystem.
    pub checked: usize,
    /// Repository-owned scripts checked beneath the explicit repository root.
    pub repo_checked: usize,
    pub references: Vec<WorkflowArtifactReference>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowArtifactReference {
    pub path: String,
    pub owner: String,
    pub required: bool,
    pub manifest_entry: Option<String>,
    pub checksum: Option<String>,
}

pub fn validate_artifact_references(
    content: &str,
    store_root: &Path,
    repo_root: Option<&Path>,
) -> Result<WorkflowArtifactReport> {
    let value: YamlValue = serde_yaml::from_str(content)
        .map_err(|source| error::usage(format!("parse workflow YAML/JSON: {source}")))?;
    let mapping = value
        .as_mapping()
        .ok_or_else(|| error::usage("workflow content must be a YAML/JSON object"))?;
    let manifest = ArtifactManifest::load(store_root)?;
    let repo_root = repo_root.map(resolve_repo_root).transpose()?;
    let mut errors = Vec::new();
    let mut references = Vec::new();
    let mut declared_knowledge_store_paths = BTreeSet::new();
    let mut repo_checked = 0;

    if let Some(reusable_scripts) = yaml_get(mapping, "reusable_scripts") {
        let scripts = reusable_scripts
            .as_sequence()
            .ok_or_else(|| error::usage("workflow reusable_scripts must be an array"))?;
        for (index, script) in scripts.iter().enumerate() {
            let Some(script) = script.as_mapping() else {
                errors.push(format!("reusable_scripts[{index}] must be an object"));
                continue;
            };
            let path = yaml_get(script, "path")
                .and_then(YamlValue::as_str)
                .unwrap_or_default();
            let owner = yaml_get(script, "owner")
                .and_then(YamlValue::as_str)
                .unwrap_or_default();
            let required = match yaml_get(script, "required") {
                Some(YamlValue::Bool(value)) => *value,
                Some(_) => {
                    errors.push(format!(
                        "reusable_scripts[{index}] required must be boolean"
                    ));
                    false
                }
                None => false,
            };
            if path.trim().is_empty() {
                errors.push(format!("reusable_scripts[{index}] requires path"));
                continue;
            }
            if !matches!(owner, "repo" | "knowledge_store") {
                errors.push(format!(
                    "reusable_scripts[{index}] owner must be repo or knowledge_store"
                ));
                continue;
            }
            if owner == "repo" {
                if validate_repo_script_reference(
                    index,
                    path,
                    required,
                    repo_root.as_deref(),
                    &mut errors,
                    &mut references,
                )? {
                    repo_checked += 1;
                }
                continue;
            }

            declared_knowledge_store_paths.insert(path.to_string());
            validate_one_artifact_reference(
                index,
                script,
                path,
                owner,
                required,
                store_root,
                manifest.as_ref(),
                &mut errors,
                &mut references,
            )?;
        }
    }

    for (index, path) in step_artifact_runs(mapping)? {
        if !declared_knowledge_store_paths.contains(&path) {
            errors.push(format!(
                "workflow step {index} run references artifact path {path}, but reusable_scripts entry is missing"
            ));
        }
    }

    if !errors.is_empty() {
        return Err(error::safety_violation(format!(
            "workflow artifact validation failed: {}",
            errors.join("; ")
        )));
    }
    Ok(WorkflowArtifactReport {
        checked: references
            .iter()
            .filter(|reference| reference.owner == "knowledge_store")
            .count(),
        repo_checked,
        references,
    })
}

fn resolve_repo_root(path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(path).map_err(|source| {
        error::not_found(format!(
            "repository root not found at {}: {source}",
            path.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(error::safety_violation(format!(
            "repository root is not a directory: {}",
            root.display()
        )));
    }
    Ok(root)
}

fn validate_repo_relative_path(path: &str) -> std::result::Result<(), String> {
    if path.is_empty() {
        return Err("path is empty".to_string());
    }
    if path.len() > 4_096 || path.chars().any(char::is_control) {
        return Err("path exceeds 4096 bytes or contains control characters".to_string());
    }
    if path
        .split(['/', '\\'])
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err("path must be normalized and must not escape the repository".to_string());
    }
    let path = Path::new(path);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_)
                    | Component::RootDir
                    | Component::ParentDir
                    | Component::CurDir
            )
        })
    {
        return Err("path must be relative and must not escape the repository".to_string());
    }
    Ok(())
}

fn validate_repo_script_reference(
    index: usize,
    path: &str,
    required: bool,
    repo_root: Option<&Path>,
    errors: &mut Vec<String>,
    references: &mut Vec<WorkflowArtifactReference>,
) -> Result<bool> {
    let error_count = errors.len();
    let Some(repo_root) = repo_root else {
        errors.push(format!(
            "reusable_scripts[{index}] owner repo requires --repo <DIR> for validation"
        ));
        references.push(WorkflowArtifactReference {
            path: path.to_string(),
            owner: "repo".to_string(),
            required,
            manifest_entry: None,
            checksum: None,
        });
        return Ok(false);
    };
    if let Err(reason) = validate_repo_relative_path(path) {
        errors.push(format!(
            "reusable_scripts[{index}] unsafe repository path: {reason}"
        ));
        return Ok(false);
    }

    let components = Path::new(path).components().collect::<Vec<_>>();
    let mut current = repo_root.to_path_buf();
    for (component_index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            errors.push(format!(
                "reusable_scripts[{index}] unsafe repository path: {path}"
            ));
            return Ok(false);
        };
        current.push(component);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                if required {
                    errors.push(format!(
                        "reusable_scripts[{index}] required repository script is missing: {path}"
                    ));
                }
                references.push(WorkflowArtifactReference {
                    path: path.to_string(),
                    owner: "repo".to_string(),
                    required,
                    manifest_entry: None,
                    checksum: None,
                });
                return Ok(false);
            }
            Err(source) => {
                return Err(source)
                    .with_context(|| format!("inspect repository script {}", current.display()));
            }
        };
        if metadata.file_type().is_symlink() {
            errors.push(format!(
                "reusable_scripts[{index}] repository path contains a symlink: {path}"
            ));
            return Ok(false);
        }
        let is_last = component_index + 1 == components.len();
        if is_last && !metadata.is_file() {
            errors.push(format!(
                "reusable_scripts[{index}] repository script is not a regular file: {path}"
            ));
            return Ok(false);
        }
        if !is_last && !metadata.is_dir() {
            errors.push(format!(
                "reusable_scripts[{index}] repository path component is not a directory: {path}"
            ));
            return Ok(false);
        }
    }
    if !artifact_file_is_executable(&current)? {
        errors.push(format!(
            "reusable_scripts[{index}] repository script is not executable: {path}"
        ));
    }
    references.push(WorkflowArtifactReference {
        path: path.to_string(),
        owner: "repo".to_string(),
        required,
        manifest_entry: None,
        checksum: None,
    });
    Ok(errors.len() == error_count)
}

#[allow(clippy::too_many_arguments)]
fn validate_one_artifact_reference(
    index: usize,
    script: &serde_yaml::Mapping,
    path: &str,
    owner: &str,
    required: bool,
    store_root: &Path,
    manifest: Option<&ArtifactManifest>,
    errors: &mut Vec<String>,
    references: &mut Vec<WorkflowArtifactReference>,
) -> Result<()> {
    if let Err(reason) = validate_artifact_path(path) {
        errors.push(format!(
            "reusable_scripts[{index}] unsafe artifact path: {reason}"
        ));
        return Ok(());
    }
    let Some(manifest) = manifest else {
        errors.push(format!(
            "reusable_scripts[{index}] references {path}, but manifest.toml is missing"
        ));
        return Ok(());
    };
    let Some(entry) = manifest
        .entries()
        .into_iter()
        .find(|entry| entry.record.path == path)
    else {
        errors.push(format!(
            "reusable_scripts[{index}] references {path}, but manifest entry is missing"
        ));
        return Ok(());
    };
    let full_path = store_root.join(path);
    if fs_metadata_missing(&full_path)? {
        if required {
            errors.push(format!(
                "reusable_scripts[{index}] required artifact is missing: {path}"
            ));
        }
        references.push(WorkflowArtifactReference {
            path: path.to_string(),
            owner: owner.to_string(),
            required,
            manifest_entry: Some(entry.name),
            checksum: None,
        });
        return Ok(());
    }
    let full_path = match validate_artifact_file(store_root, path) {
        Ok(path) => path,
        Err(error) => {
            errors.push(format!(
                "reusable_scripts[{index}] unsafe artifact file {path}: {error}"
            ));
            return Ok(());
        }
    };
    let actual_checksum = artifact_file_checksum(&full_path)?;
    if actual_checksum != entry.record.checksum {
        errors.push(format!(
            "reusable_scripts[{index}] checksum mismatch for {path}: expected {}, got {actual_checksum}",
            entry.record.checksum
        ));
    }
    if let Some(expected) = yaml_get(script, "checksum").and_then(YamlValue::as_str)
        && expected != actual_checksum
    {
        errors.push(format!(
                "reusable_scripts[{index}] workflow checksum mismatch for {path}: expected {expected}, got {actual_checksum}"
            ));
    }
    if entry.record.executable == Some(true) && !artifact_file_is_executable(&full_path)? {
        errors.push(format!(
            "reusable_scripts[{index}] artifact is not executable: {path}"
        ));
    }
    references.push(WorkflowArtifactReference {
        path: path.to_string(),
        owner: owner.to_string(),
        required,
        manifest_entry: Some(entry.name),
        checksum: Some(actual_checksum),
    });
    Ok(())
}

fn fs_metadata_missing(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn step_artifact_runs(mapping: &serde_yaml::Mapping) -> Result<Vec<(usize, String)>> {
    let Some(steps) = yaml_get(mapping, "steps").and_then(YamlValue::as_sequence) else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let Some(step) = step.as_mapping() else {
            continue;
        };
        let Some(run) = yaml_get(step, "run").and_then(YamlValue::as_str) else {
            continue;
        };
        let Some(path) = first_artifact_token(run) else {
            continue;
        };
        if let Err(reason) = validate_artifact_path(&path) {
            return Err(error::safety_violation(format!(
                "workflow step {index} run has unsafe artifact path: {reason}"
            )));
        }
        paths.push((index, path));
    }
    Ok(paths)
}

fn first_artifact_token(run: &str) -> Option<String> {
    let token = run.split_whitespace().next()?.trim_matches(['"', '\'']);
    token.starts_with("artifacts/").then(|| token.to_string())
}
