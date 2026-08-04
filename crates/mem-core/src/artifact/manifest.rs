use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::atomic_file::atomic_write_private;
use crate::error;

const MANIFEST_FILE: &str = "manifest.toml";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtifactManifest {
    pub version: u64,
    #[serde(default)]
    pub artifacts: BTreeMap<String, BTreeMap<String, ArtifactRecord>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtifactRecord {
    pub path: String,
    pub kind: ArtifactKind,
    pub scope: String,
    pub checksum: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Script,
    Template,
    Snippet,
    Reference,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArtifactEntry {
    pub name: String,
    pub short_name: String,
    pub group: String,
    #[serde(flatten)]
    pub record: ArtifactRecord,
}

impl ArtifactManifest {
    pub fn empty() -> Self {
        Self {
            version: 1,
            artifacts: BTreeMap::new(),
        }
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        let path = root.join(MANIFEST_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(error::safety_violation(format!(
                    "refusing unsafe artifact manifest path: {}",
                    path.display()
                )));
            }
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.len() > 8_388_608 {
            return Err(error::safety_violation(
                "artifact manifest exceeds 8388608 bytes",
            ));
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let manifest: Self = toml::from_str(&content).map_err(|source| {
            error::integrity(format!(
                "parse artifact manifest {}: {source}",
                path.display()
            ))
        })?;
        Ok(Some(manifest))
    }

    pub fn load_or_default(root: &Path) -> Result<Self> {
        Ok(Self::load(root)?.unwrap_or_else(Self::empty))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        fs::create_dir_all(root).with_context(|| format!("create {}", root.display()))?;
        let path = root.join(MANIFEST_FILE);
        let content = toml::to_string_pretty(self).context("serialize artifact manifest")?;
        if content.len() > 8_388_608 {
            return Err(error::safety_violation(
                "artifact manifest exceeds 8388608 bytes",
            ));
        }
        atomic_write_private(&path, content.as_bytes())
            .with_context(|| format!("write {}", path.display()))
    }

    pub fn entries(&self) -> Vec<ArtifactEntry> {
        let mut entries = Vec::new();
        for (group, records) in &self.artifacts {
            for (short_name, record) in records {
                entries.push(ArtifactEntry {
                    name: format!("{group}.{short_name}"),
                    short_name: short_name.clone(),
                    group: group.clone(),
                    record: record.clone(),
                });
            }
        }
        entries
    }

    pub fn find_entry(&self, reference: &str) -> Result<ArtifactEntry> {
        let entries = self.entries();
        if let Some(entry) = entries.iter().find(|entry| entry.name == reference) {
            return Ok(entry.clone());
        }
        let matches = entries
            .into_iter()
            .filter(|entry| entry.short_name == reference)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [entry] => Ok(entry.clone()),
            [] => Err(error::not_found(format!("artifact not found: {reference}"))),
            _ => Err(error::conflict(format!(
                "artifact reference is ambiguous: {reference}"
            ))),
        }
    }

    pub(super) fn find_entry_key(&self, reference: &str) -> Result<(String, String)> {
        let entry = self.find_entry(reference)?;
        Ok((entry.group, entry.short_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest_entries() {
        let manifest: ArtifactManifest = toml::from_str(
            r#"
version = 1

[artifacts.scripts.ci-triage]
path = "artifacts/scripts/ci-triage.sh"
kind = "script"
scope = "global"
checksum = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
executable = true
"#,
        )
        .expect("manifest");

        let entry = manifest
            .find_entry("ci-triage")
            .expect("entry by short name");
        assert_eq!(entry.name, "scripts.ci-triage");
        assert_eq!(entry.record.kind, ArtifactKind::Script);
    }
}
