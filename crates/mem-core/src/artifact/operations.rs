use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::error;
use crate::util::now;

use super::checksum::{file_sha256, is_executable, valid_sha256_checksum};
use super::manifest::{ArtifactEntry, ArtifactKind, ArtifactManifest, ArtifactRecord};
use super::path::{
    artifact_group, artifact_name, kind_group, path_to_manifest_string, validate_artifact_file,
    validate_artifact_name, validate_artifact_path,
};

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactCheckReport {
    pub status: String,
    pub manifest_found: bool,
    pub checked: usize,
    pub missing: Vec<String>,
    pub checksum_mismatch: Vec<ArtifactChecksumMismatch>,
    pub unsafe_paths: Vec<ArtifactPathIssue>,
    pub invalid_checksum: Vec<String>,
    pub invalid_scope: Vec<String>,
    pub not_executable: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactChecksumMismatch {
    pub name: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactPathIssue {
    pub name: String,
    pub path: String,
    pub reason: String,
}

pub fn check_artifacts(root: &Path) -> Result<ArtifactCheckReport> {
    let Some(manifest) = ArtifactManifest::load(root)? else {
        return Ok(ArtifactCheckReport::empty(false));
    };
    let mut report = ArtifactCheckReport::empty(true);
    for entry in manifest.entries() {
        report.checked += 1;
        if let Err(reason) = validate_artifact_path(&entry.record.path) {
            report.unsafe_paths.push(ArtifactPathIssue {
                name: entry.name,
                path: entry.record.path,
                reason,
            });
            continue;
        }
        if !valid_scope(&entry.record.scope) {
            report.invalid_scope.push(entry.name.clone());
        }
        if !valid_sha256_checksum(&entry.record.checksum) {
            report.invalid_checksum.push(entry.name.clone());
            continue;
        }
        let full_path = root.join(&entry.record.path);
        match fs::symlink_metadata(&full_path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                report.missing.push(entry.name.clone());
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        let full_path = match validate_artifact_file(root, &entry.record.path) {
            Ok(path) => path,
            Err(error) => {
                report.unsafe_paths.push(ArtifactPathIssue {
                    name: entry.name,
                    path: entry.record.path,
                    reason: error.to_string(),
                });
                continue;
            }
        };
        let actual = file_sha256(&full_path)?;
        if entry.record.checksum != actual {
            report.checksum_mismatch.push(ArtifactChecksumMismatch {
                name: entry.name.clone(),
                expected: entry.record.checksum.clone(),
                actual,
            });
        }
        if entry.record.executable == Some(true) && !is_executable(&full_path)? {
            report.not_executable.push(entry.name);
        }
    }
    report.finalize();
    Ok(report)
}

pub struct AddArtifact<'a> {
    pub path: &'a Path,
    pub name: Option<String>,
    pub kind: ArtifactKind,
    pub scope: String,
    pub description: Option<String>,
    pub executable: bool,
    pub tags: Option<Vec<String>>,
    pub force: bool,
}

pub fn add_artifact(root: &Path, args: AddArtifact<'_>) -> Result<ArtifactEntry> {
    let relative_path = path_to_manifest_string(args.path)?;
    validate_artifact_path(&relative_path).map_err(error::safety_violation)?;
    let group = artifact_group(&relative_path)?;
    if group != kind_group(&args.kind) {
        return Err(error::usage(format!(
            "artifact kind {:?} does not match path group {group}",
            args.kind
        )));
    }
    let full_path = validate_artifact_file(root, &relative_path)?;
    let short_name = match args.name {
        Some(name) => validate_artifact_name(&name)?,
        None => artifact_name(args.path)?,
    };
    let mut manifest = ArtifactManifest::load_or_default(root)?;
    let entries = manifest.entries();
    let name = format!("{group}.{short_name}");
    if !args.force
        && entries
            .iter()
            .any(|entry| entry.name == name || entry.record.path == relative_path)
    {
        return Err(error::conflict(
            "artifact name or path already exists; use --force to replace metadata",
        ));
    }
    if args.force {
        remove_matching_entries(&mut manifest, &name, &relative_path);
    }

    let timestamp = now();
    let record = ArtifactRecord {
        path: relative_path,
        kind: args.kind,
        scope: args.scope,
        checksum: file_sha256(&full_path)?,
        description: args.description,
        executable: args.executable.then_some(true),
        tags: args.tags,
        created_at: Some(timestamp.clone()),
        updated_at: Some(timestamp),
    };
    manifest
        .artifacts
        .entry(group.clone())
        .or_default()
        .insert(short_name.clone(), record);
    manifest.save(root)?;
    manifest.find_entry(&format!("{group}.{short_name}"))
}

pub fn update_artifact_checksum(root: &Path, reference: &str) -> Result<ArtifactEntry> {
    let mut manifest = ArtifactManifest::load_or_default(root)?;
    let (group, short_name) = manifest.find_entry_key(reference)?;
    let record = manifest
        .artifacts
        .get_mut(&group)
        .and_then(|group| group.get_mut(&short_name))
        .ok_or_else(|| error::not_found(format!("artifact not found: {reference}")))?;
    validate_artifact_path(&record.path).map_err(error::safety_violation)?;
    let full_path = validate_artifact_file(root, &record.path)?;
    record.checksum = file_sha256(&full_path)?;
    record.updated_at = Some(now());
    manifest.save(root)?;
    manifest.find_entry(&format!("{group}.{short_name}"))
}

pub fn remove_artifact(root: &Path, reference: &str, delete_file: bool) -> Result<ArtifactEntry> {
    let mut manifest = ArtifactManifest::load_or_default(root)?;
    let (group, short_name) = manifest.find_entry_key(reference)?;
    let record = manifest
        .artifacts
        .get_mut(&group)
        .and_then(|records| records.remove(&short_name))
        .ok_or_else(|| error::not_found(format!("artifact not found: {reference}")))?;
    if manifest
        .artifacts
        .get(&group)
        .is_some_and(BTreeMap::is_empty)
    {
        manifest.artifacts.remove(&group);
    }
    if delete_file {
        validate_artifact_path(&record.path).map_err(error::safety_violation)?;
        let full_path = root.join(&record.path);
        if full_path.exists() {
            fs::remove_file(&full_path)
                .with_context(|| format!("delete {}", full_path.display()))?;
        }
    }
    manifest.save(root)?;
    Ok(ArtifactEntry {
        name: format!("{group}.{short_name}"),
        short_name,
        group,
        record,
    })
}

fn remove_matching_entries(manifest: &mut ArtifactManifest, name: &str, path: &str) {
    let mut empty_groups = Vec::new();
    for (group, records) in &mut manifest.artifacts {
        records.retain(|short_name, record| {
            let entry_name = format!("{group}.{short_name}");
            entry_name != name && record.path != path
        });
        if records.is_empty() {
            empty_groups.push(group.clone());
        }
    }
    for group in empty_groups {
        manifest.artifacts.remove(&group);
    }
}

impl ArtifactCheckReport {
    fn empty(manifest_found: bool) -> Self {
        Self {
            status: "ok".to_string(),
            manifest_found,
            checked: 0,
            missing: Vec::new(),
            checksum_mismatch: Vec::new(),
            unsafe_paths: Vec::new(),
            invalid_checksum: Vec::new(),
            invalid_scope: Vec::new(),
            not_executable: Vec::new(),
        }
    }

    fn finalize(&mut self) {
        if self.missing.is_empty()
            && self.checksum_mismatch.is_empty()
            && self.unsafe_paths.is_empty()
            && self.invalid_checksum.is_empty()
            && self.invalid_scope.is_empty()
            && self.not_executable.is_empty()
        {
            self.status = "ok".to_string();
        } else {
            self.status = "error".to_string();
        }
    }
}

fn valid_scope(scope: &str) -> bool {
    scope == "global"
        || scope
            .strip_prefix("project:")
            .is_some_and(|value| !value.is_empty())
}
