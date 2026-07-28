use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use mem_core::artifact::artifact_file_checksum;
use serde_json::Value;

use super::archive::validate_bundle_path;
use super::*;

pub(super) fn prepare_bundle_import(root: &Path, redact_secrets: bool) -> Result<()> {
    let database = root.join("memory.db");
    if !database.is_file() {
        bail!("bundle does not contain memory.db");
    }
    let mut conn = Connection::open(&database)?;
    let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let supported = mem_core::db::supported_schema_version();
    if schema_version != supported {
        bail!(
            "bundle database schema v{schema_version} is not supported by this binary (v{supported}); \
             import it into a temporary store, run `mem migrate`, then export a new bundle"
        );
    }
    if mem_core::db::schema_compatibility_required(&conn)? {
        bail!(
            "bundle database schema v{schema_version} needs compatibility repair; \
             import it into a temporary store, run `mem migrate`, then export a new bundle"
        );
    }
    let quick_check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        bail!("bundle database failed SQLite quick_check: {quick_check}");
    }
    mem_core::db::validate_store_schema_objects(&conn)?;
    if redact_secrets {
        mem_core::db::redact_store_secrets(&mut conn)?;
        mem_core::graph::set_graph_dirty(&conn, true)?;
    } else {
        mem_core::db::validate_store_secrets(&conn)?;
    }
    for memory in all_memories_compatible(&conn)? {
        validate_tags(&memory.tags)?;
        scope::validate_scope(&memory.scope)?;
        validate_memory_resource_limits(
            &memory.name,
            memory.description.as_deref(),
            memory.content.as_deref().unwrap_or_default(),
            &memory.tags,
            &memory.scope,
            None,
        )?;
        if memory.source == "manual" && memory.user_confirmed_at.is_none() {
            bail!("bundle contains unattested manual memory: {}", memory.name);
        }
    }
    let unattested_semantic: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_semantic_edges
         WHERE source = 'manual' AND user_confirmed_at IS NULL",
        [],
        |row| row.get(0),
    )?;
    if unattested_semantic > 0 {
        bail!("bundle contains {unattested_semantic} unattested manual semantic edges");
    }
    validate_bundle_side_state_resources(&conn)?;
    drop(conn);

    for relative in ["config.toml", "manifest.toml"] {
        sanitize_bundle_text_file(&root.join(relative), redact_secrets)?;
    }
    sanitize_bundle_artifact_tree(&root.join("artifacts"), redact_secrets)?;
    Ok(())
}

fn validate_bundle_side_state_resources(conn: &Connection) -> Result<()> {
    let oversized: i64 = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM ambiguities
            WHERE length(query) > 10000
               OR length(COALESCE(context, '')) > 4194304
               OR length(resolution) > 4194304
               OR json_array_length(memory_ids) > 1000)
         + (SELECT COUNT(*) FROM workflow_runs WHERE length(COALESCE(note, '')) > 65536)
         + (SELECT COUNT(*) FROM changelog
            WHERE length(COALESCE(old_content, '')) > 1048576
               OR length(COALESCE(new_content, '')) > 1048576)
         + (SELECT COUNT(*) FROM graph_semantic_edges
            WHERE length(evidence) > 20000
               OR length(COALESCE(rationale, '')) > 10000
               OR length(source_spans) > 100000
               OR json_array_length(tags) > 100)
         + (SELECT COUNT(*) FROM graph_semantic_edge_revisions
            WHERE length(snapshot) > 1048576)",
        [],
        |row| row.get(0),
    )?;
    if oversized > 0 {
        bail!("bundle contains {oversized} durable side-state rows over resource limits");
    }
    Ok(())
}

fn sanitize_bundle_artifact_tree(root: &Path, redact_secrets: bool) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            sanitize_bundle_artifact_tree(&path, redact_secrets)?;
        } else if path.is_file() {
            sanitize_bundle_text_file(&path, redact_secrets)?;
        }
    }
    Ok(())
}

fn sanitize_bundle_text_file(path: &Path, redact_secrets: bool) -> Result<()> {
    if path.is_file() {
        sanitize_secret_file(
            path,
            &format!("bundle file {}", path.display()),
            redact_secrets,
        )?;
    }
    Ok(())
}

pub(super) fn bundle_hashes(root: &Path, no_config: bool) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    add_bundle_hash(&mut hashes, &root.join("memory.db"), "memory.db")?;
    if !no_config {
        add_bundle_hash(&mut hashes, &root.join("config.toml"), "config.toml")?;
    }
    add_bundle_hash(&mut hashes, &root.join("manifest.toml"), "manifest.toml")?;
    collect_artifact_hashes(root, &root.join("artifacts"), &mut hashes)?;
    Ok(hashes)
}

fn add_bundle_hash(
    hashes: &mut BTreeMap<String, String>,
    path: &Path,
    bundle_path: &str,
) -> Result<()> {
    if path.is_file() {
        hashes.insert(bundle_path.to_string(), artifact_file_checksum(path)?);
    }
    Ok(())
}

fn collect_artifact_hashes(
    root: &Path,
    current: &Path,
    hashes: &mut BTreeMap<String, String>,
) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_artifact_hashes(root, &path, hashes)?;
        } else if path.is_file() {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("resolve bundle path {}", path.display()))?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            hashes.insert(relative, artifact_file_checksum(&path)?);
        }
    }
    Ok(())
}

pub(super) fn validate_snapshot_bundle_limits(root: &Path) -> Result<u64> {
    let mut paths = BTreeSet::new();
    collect_bundle_file_paths(root, root, &mut paths)?;
    if paths.len() + 1 > MAX_BUNDLE_ENTRIES {
        bail!("bundle exceeds {MAX_BUNDLE_ENTRIES} entries");
    }
    let mut total_bytes = 0_u64;
    for relative in paths {
        if relative.len() > MAX_BUNDLE_PATH_BYTES {
            bail!("bundle entry path exceeds {MAX_BUNDLE_PATH_BYTES} bytes");
        }
        validate_bundle_path(Path::new(&relative), false)?;
        let path = root.join(&relative);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            bail!("bundle contains non-regular snapshot entry: {relative}");
        }
        let bytes = metadata.len();
        if bytes > MAX_BUNDLE_FILE_BYTES {
            bail!("bundle entry exceeds {MAX_BUNDLE_FILE_BYTES} bytes: {relative}");
        }
        total_bytes = total_bytes
            .checked_add(bytes)
            .filter(|total| *total <= MAX_BUNDLE_TOTAL_BYTES)
            .ok_or_else(|| anyhow!("bundle exceeds {MAX_BUNDLE_TOTAL_BYTES} unpacked bytes"))?;
    }
    Ok(total_bytes)
}

pub(super) fn validate_bundle_hashes(root: &Path, bundle: &Value) -> Result<()> {
    let Some(hashes) = bundle.get("hashes") else {
        // Version-1 bundles predate per-file hashes and remain importable.
        if bundle.get("version").and_then(Value::as_i64).unwrap_or(1) <= 1 {
            return Ok(());
        }
        bail!("bundle metadata is missing required hashes");
    };
    let hashes = hashes
        .as_object()
        .ok_or_else(|| anyhow!("bundle hashes must be an object"))?;
    let expected_paths = hashes.keys().cloned().collect::<BTreeSet<_>>();
    let mut actual_paths = BTreeSet::new();
    collect_bundle_file_paths(root, root, &mut actual_paths)?;
    actual_paths.remove("bundle.json");
    if actual_paths != expected_paths {
        let missing_hashes = actual_paths
            .difference(&expected_paths)
            .cloned()
            .collect::<Vec<_>>();
        let missing_files = expected_paths
            .difference(&actual_paths)
            .cloned()
            .collect::<Vec<_>>();
        bail!(
            "bundle file manifest mismatch; files without hashes: {missing_hashes:?}; \
             hashes without files: {missing_files:?}"
        );
    }
    for (relative, expected) in hashes {
        let expected = expected
            .as_str()
            .ok_or_else(|| anyhow!("bundle hash for {relative} must be a string"))?;
        let relative_path = Path::new(relative);
        validate_bundle_path(relative_path, false)?;
        let path = root.join(relative_path);
        if !path.is_file() {
            bail!("bundle hash references missing file: {relative}");
        }
        let actual = artifact_file_checksum(&path)?;
        if actual != expected {
            bail!("bundle checksum mismatch for {relative}: expected {expected}, got {actual}");
        }
    }
    Ok(())
}

fn collect_bundle_file_paths(
    root: &Path,
    current: &Path,
    output: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_bundle_file_paths(root, &path, output)?;
        } else if path.is_file() {
            output.insert(
                path.strip_prefix(root)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}
