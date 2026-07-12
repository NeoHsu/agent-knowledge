use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::util::now;

const MANIFEST_FILE: &str = "manifest.toml";
const SHA256_PREFIX: &str = "sha256:";
const ALLOWED_ARTIFACT_DIRS: &[&str] = &["scripts", "templates", "snippets", "references"];

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
                bail!("refusing unsafe artifact manifest path: {}", path.display())
            }
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.len() > 8_388_608 {
            bail!("artifact manifest exceeds 8388608 bytes");
        }
        let content =
            fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let manifest: Self =
            toml::from_str(&content).with_context(|| format!("parse {}", path.display()))?;
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
            bail!("artifact manifest exceeds 8388608 bytes");
        }
        let temporary = root.join(format!(
            ".manifest.toml.tmp-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let write_result = (|| -> Result<()> {
            let mut options = fs::OpenOptions::new();
            options.create_new(true).write(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options.open(&temporary)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            install_atomic_file(&temporary, &path)?;
            Ok(())
        })();
        if write_result.is_err() {
            fs::remove_file(&temporary).ok();
        }
        write_result.with_context(|| format!("write {}", path.display()))
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
            [] => bail!("artifact not found: {reference}"),
            _ => bail!("artifact reference is ambiguous: {reference}"),
        }
    }

    fn find_entry_key(&self, reference: &str) -> Result<(String, String)> {
        let entry = self.find_entry(reference)?;
        Ok((entry.group, entry.short_name))
    }
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
    validate_artifact_path(&relative_path).map_err(|reason| anyhow::anyhow!(reason))?;
    let group = artifact_group(&relative_path)?;
    if group != kind_group(&args.kind) {
        bail!(
            "artifact kind {:?} does not match path group {group}",
            args.kind
        );
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
        bail!("artifact name or path already exists; use --force to replace metadata");
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
        .ok_or_else(|| anyhow::anyhow!("artifact not found: {reference}"))?;
    validate_artifact_path(&record.path).map_err(|reason| anyhow::anyhow!(reason))?;
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
        .ok_or_else(|| anyhow::anyhow!("artifact not found: {reference}"))?;
    if manifest
        .artifacts
        .get(&group)
        .is_some_and(BTreeMap::is_empty)
    {
        manifest.artifacts.remove(&group);
    }
    if delete_file {
        validate_artifact_path(&record.path).map_err(|reason| anyhow::anyhow!(reason))?;
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

#[cfg(not(windows))]
fn install_atomic_file(temporary: &Path, target: &Path) -> Result<()> {
    fs::rename(temporary, target)?;
    Ok(())
}

#[cfg(windows)]
fn install_atomic_file(temporary: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        fs::rename(temporary, target)?;
        return Ok(());
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
    validate_artifact_path(relative).map_err(anyhow::Error::msg)?;
    let components = Path::new(relative).components().collect::<Vec<_>>();
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            bail!("refusing unsafe artifact path: {relative}");
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .with_context(|| format!("inspect artifact path {}", current.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("refusing artifact symlink: {}", current.display());
        }
        let is_last = index + 1 == components.len();
        if is_last && !metadata.is_file() {
            bail!("refusing non-regular artifact file: {}", current.display());
        }
        if !is_last && !metadata.is_dir() {
            bail!(
                "refusing non-directory artifact path: {}",
                current.display()
            );
        }
    }
    Ok(current)
}

fn path_to_manifest_string(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("artifact path is not valid UTF-8"))?;
    Ok(value.trim_start_matches("./").to_string())
}

fn artifact_group(path: &str) -> Result<String> {
    let mut components = Path::new(path).components();
    let _artifact = components.next();
    let Some(Component::Normal(group)) = components.next() else {
        bail!("artifact path missing group");
    };
    Ok(group.to_string_lossy().to_string())
}

fn artifact_name(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow::anyhow!("artifact path must include a file name"))?;
    if stem.is_empty() {
        bail!("artifact name is empty");
    }
    Ok(stem.to_string())
}

fn validate_artifact_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        bail!("artifact name is empty");
    }
    if name.len() > 256 || name.chars().any(char::is_control) {
        bail!("artifact name exceeds 256 bytes or contains control characters");
    }
    if name.contains('.') || name.contains('/') || name.contains('\\') {
        bail!("artifact name must not contain '.', '/', or '\\'");
    }
    Ok(name.to_string())
}

fn kind_group(kind: &ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Script => "scripts",
        ArtifactKind::Template => "templates",
        ArtifactKind::Snippet => "snippets",
        ArtifactKind::Reference => "references",
    }
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

fn valid_sha256_checksum(value: &str) -> bool {
    let Some(hex) = value.strip_prefix(SHA256_PREFIX) else {
        return false;
    };
    hex.len() == 64 && hex.as_bytes().iter().all(u8::is_ascii_hexdigit)
}

fn file_sha256(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("refusing to hash non-regular file: {}", path.display());
    }
    let mut file = fs::File::open(path).with_context(|| format!("read {}", path.display()))?;
    let mut state = sha256_initial_state();
    let mut remaining = metadata.len();
    let mut block = [0_u8; 64];
    while remaining >= 64 {
        file.read_exact(&mut block)?;
        compress_sha256(&mut state, &block);
        remaining -= 64;
    }
    let mut tail = vec![0_u8; remaining as usize];
    file.read_exact(&mut tail)?;
    let mut extra = [0_u8; 1];
    if file.read(&mut extra)? != 0 {
        bail!("file changed while hashing: {}", path.display());
    }
    finalize_sha256(&mut state, &tail, metadata.len())?;
    Ok(format!("{SHA256_PREFIX}{}", format_sha256(&state)))
}

pub fn artifact_file_checksum(path: &Path) -> Result<String> {
    file_sha256(path)
}

pub fn artifact_file_is_executable(path: &Path) -> Result<bool> {
    is_executable(path)
}

#[cfg(unix)]
fn is_executable(path: &Path) -> Result<bool> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> Result<bool> {
    let metadata = fs::metadata(path).with_context(|| format!("stat {}", path.display()))?;
    Ok(metadata.is_file())
}

#[cfg(test)]
fn sha256_hex(input: &[u8]) -> String {
    let mut state = sha256_initial_state();
    let mut chunks = input.chunks_exact(64);
    for chunk in &mut chunks {
        let block: &[u8; 64] = chunk.try_into().expect("SHA-256 block length");
        compress_sha256(&mut state, block);
    }
    finalize_sha256(&mut state, chunks.remainder(), input.len() as u64)
        .expect("in-memory SHA-256 input length");
    format_sha256(&state)
}

fn sha256_initial_state() -> [u32; 8] {
    [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ]
}

fn finalize_sha256(state: &mut [u32; 8], tail: &[u8], total_bytes: u64) -> Result<()> {
    if tail.len() >= 64 {
        bail!("invalid SHA-256 tail length");
    }
    let bit_len = total_bytes
        .checked_mul(8)
        .ok_or_else(|| anyhow::anyhow!("file is too large to hash with SHA-256"))?;
    let mut padding = Vec::with_capacity(128);
    padding.extend_from_slice(tail);
    padding.push(0x80);
    while padding.len() % 64 != 56 {
        padding.push(0);
    }
    padding.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padding.chunks_exact(64) {
        let block: &[u8; 64] = chunk.try_into().expect("SHA-256 padding block length");
        compress_sha256(state, block);
    }
    Ok(())
}

fn compress_sha256(state: &mut [u32; 8], chunk: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (index, word) in chunk.chunks_exact(4).take(16).enumerate() {
        w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }
    for index in 16..64 {
        let s0 =
            w[index - 15].rotate_right(7) ^ w[index - 15].rotate_right(18) ^ (w[index - 15] >> 3);
        let s1 =
            w[index - 2].rotate_right(17) ^ w[index - 2].rotate_right(19) ^ (w[index - 2] >> 10);
        w[index] = w[index - 16]
            .wrapping_add(s0)
            .wrapping_add(w[index - 7])
            .wrapping_add(s1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for index in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[index])
            .wrapping_add(w[index]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

fn format_sha256(state: &[u32; 8]) -> String {
    state
        .iter()
        .map(|word| format!("{word:08x}"))
        .collect::<String>()
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

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

    #[test]
    fn computes_sha256() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

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
