use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Component, Path};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use mem_core::artifact::validate_artifact_path;
use serde_json::Value;
use tar::{Archive, Builder, Header};

use super::*;

pub(super) fn unpack_bundle(file: &Path, temp: &Path) -> Result<Vec<String>> {
    fs::create_dir_all(temp)?;
    let file = File::open(file).with_context(|| format!("open {}", file.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    let mut total_bytes = 0_u64;
    for entry in archive.entries()? {
        if entries.len() >= MAX_BUNDLE_ENTRIES {
            bail!("bundle exceeds {MAX_BUNDLE_ENTRIES} entries");
        }
        let mut entry = entry?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            bail!("bundle contains unsupported non-regular archive entry");
        }
        let entry_bytes = entry.header().size()?;
        if entry_bytes > MAX_BUNDLE_FILE_BYTES {
            bail!("bundle entry exceeds {MAX_BUNDLE_FILE_BYTES} bytes");
        }
        total_bytes = total_bytes
            .checked_add(entry_bytes)
            .filter(|total| *total <= MAX_BUNDLE_TOTAL_BYTES)
            .ok_or_else(|| anyhow!("bundle exceeds {MAX_BUNDLE_TOTAL_BYTES} unpacked bytes"))?;
        let path = entry.path()?.into_owned();
        if path.as_os_str().len() > MAX_BUNDLE_PATH_BYTES {
            bail!("bundle entry path exceeds {MAX_BUNDLE_PATH_BYTES} bytes");
        }
        validate_bundle_path(&path, entry_type.is_dir())?;
        let entry_name = path.to_string_lossy().to_string();
        if !seen.insert(entry_name.clone()) {
            bail!("bundle contains duplicate entry: {entry_name}");
        }
        entries.push(entry_name);
        entry.unpack(temp.join(path))?;
    }
    entries.sort();
    Ok(entries)
}

pub(super) fn validate_bundle_path(path: &Path, is_dir: bool) -> Result<()> {
    if path.is_absolute() {
        bail!("bundle entry uses absolute path: {}", path.display());
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::Prefix(_) | Component::RootDir | Component::ParentDir
        )
    }) {
        bail!("bundle entry escapes target: {}", path.display());
    }
    let path_text = path.to_string_lossy();
    if matches!(
        path_text.as_ref(),
        "bundle.json" | "memory.db" | "config.toml" | "manifest.toml"
    ) {
        return Ok(());
    }
    if path_text == "artifacts" || path_text.starts_with("artifacts/") {
        if is_dir {
            return Ok(());
        }
        validate_artifact_path(&path_text).map_err(|reason| anyhow::anyhow!(reason))?;
        return Ok(());
    }
    bail!("unsupported bundle entry: {}", path.display());
}

pub(super) fn read_bundle_metadata(root: &Path) -> Result<Value> {
    let path = root.join("bundle.json");
    let bytes = fs::metadata(&path)
        .with_context(|| format!("bundle metadata missing: {}", path.display()))?
        .len();
    if bytes > 1_048_576 {
        bail!("bundle metadata exceeds 1048576 bytes");
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("read bundle metadata: {}", path.display()))?;
    let metadata: Value = serde_json::from_str(&text)?;
    if !metadata.is_object() {
        bail!("bundle metadata must be a JSON object");
    }
    let version = metadata.get("version").and_then(Value::as_i64).unwrap_or(1);
    if !(1..=BUNDLE_FORMAT_VERSION).contains(&version) {
        bail!("unsupported bundle format version: {version}");
    }
    Ok(metadata)
}

pub(super) fn append_file_if_exists(
    builder: &mut Builder<GzEncoder<File>>,
    source: &Path,
    name: &str,
) -> Result<()> {
    if source.exists() {
        builder.append_path_with_name(source, name)?;
    }
    Ok(())
}

pub(super) fn append_json(
    builder: &mut Builder<GzEncoder<File>>,
    name: &str,
    value: &Value,
) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, name, Cursor::new(bytes))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_path_allowlist_rejects_escape_and_unknown_entries() {
        for (path, is_dir) in [
            ("../memory.db", false),
            ("artifacts/../../memory.db", false),
            ("unknown.txt", false),
            ("index/meta.json", false),
            ("artifacts", false),
            ("artifacts/unknown/file.txt", false),
            ("artifacts/scripts", false),
        ] {
            assert!(
                validate_bundle_path(Path::new(path), is_dir).is_err(),
                "unsafe bundle path was accepted: {path}"
            );
        }
    }

    #[test]
    fn bundle_path_allowlist_accepts_only_durable_layout() {
        for (path, is_dir) in [
            ("bundle.json", false),
            ("memory.db", false),
            ("config.toml", false),
            ("manifest.toml", false),
            ("artifacts", true),
            ("artifacts/scripts", true),
            ("artifacts/scripts/check.sh", false),
            ("artifacts/templates/release.md", false),
        ] {
            validate_bundle_path(Path::new(path), is_dir)
                .unwrap_or_else(|error| panic!("valid bundle path {path} was rejected: {error}"));
        }
    }
}
