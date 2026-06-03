use std::fs::{self, File};
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;
use tar::{Archive, Builder, Header};

use super::*;
use mem_core::artifact::{validate_artifact_path, ArtifactManifest};

pub(crate) fn cmd_bundle(app: &App, command: BundleCommand) -> Result<()> {
    match command {
        BundleCommand::Export(args) => cmd_bundle_export(app, args),
        BundleCommand::Inspect(args) => cmd_bundle_inspect(args),
        BundleCommand::Import(args) => cmd_bundle_import(app, args),
    }
}

fn cmd_bundle_export(app: &App, args: BundleExportArgs) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    drop(conn);

    let file =
        File::create(&args.file).with_context(|| format!("create {}", args.file.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    append_file_if_exists(&mut builder, &app.db_path, "memory.db")?;
    if !args.no_config {
        append_file_if_exists(&mut builder, &app.root.join("config.toml"), "config.toml")?;
    }
    append_file_if_exists(
        &mut builder,
        &app.root.join("manifest.toml"),
        "manifest.toml",
    )?;
    let artifacts = app.root.join("artifacts");
    if artifacts.exists() {
        builder.append_dir_all("artifacts", &artifacts)?;
    }

    let metadata = json!({
        "version": 1,
        "created_at": now(),
        "schema_version": schema_version,
        "contains": {
            "memory_db": app.db_path.exists(),
            "config": !args.no_config && app.root.join("config.toml").exists(),
            "manifest": app.root.join("manifest.toml").exists(),
            "artifacts": artifacts.exists()
        }
    });
    append_json(&mut builder, "bundle.json", &metadata)?;
    builder.finish()?;

    print_json_pretty(&json!({
        "status": "exported",
        "file": args.file.display().to_string(),
        "bundle": metadata
    }))
}

fn cmd_bundle_inspect(args: BundleInspectArgs) -> Result<()> {
    let temp = temp_bundle_dir("inspect")?;
    let entries = unpack_bundle(&args.file, &temp)?;
    let bundle = read_bundle_metadata(&temp)?;
    fs::remove_dir_all(&temp).ok();
    print_json_pretty(&json!({
        "status": "ok",
        "bundle": bundle,
        "entries": entries
    }))
}

fn cmd_bundle_import(app: &App, args: BundleImportArgs) -> Result<()> {
    if args.replace && !args.force {
        bail!("bundle import --replace requires --force");
    }
    if store_has_durable_files(app) && !args.merge && !args.replace {
        bail!("active store is not empty; use --merge or --replace --force");
    }

    let temp = temp_bundle_dir("import")?;
    let entries = unpack_bundle(&args.file, &temp)?;
    let bundle = read_bundle_metadata(&temp)?;

    let result = if args.merge {
        import_bundle_merge(app, &temp, entries, bundle)
    } else {
        if args.replace {
            clear_store_for_replace(app)?;
        }
        import_bundle_clean(app, &temp, entries, bundle)
    };
    fs::remove_dir_all(&temp).ok();
    result
}

fn import_bundle_clean(app: &App, temp: &Path, entries: Vec<String>, bundle: Value) -> Result<()> {
    fs::create_dir_all(&app.root)?;
    copy_if_exists(temp.join("memory.db"), app.root.join("memory.db"))?;
    copy_if_exists(temp.join("config.toml"), app.root.join("config.toml"))?;
    copy_if_exists(temp.join("manifest.toml"), app.root.join("manifest.toml"))?;
    copy_dir_if_exists(temp.join("artifacts"), app.root.join("artifacts"))?;
    app.ensure_schema()?;
    memory_index::reindex_or_mark_stale(app, "bundle import")?;
    print_json_pretty(&json!({
        "status": "imported",
        "mode": "clean",
        "entries": entries,
        "bundle": bundle
    }))
}

fn import_bundle_merge(app: &App, temp: &Path, entries: Vec<String>, bundle: Value) -> Result<()> {
    let merge_result = if temp.join("memory.db").exists() {
        Some(merge_database(app, &temp.join("memory.db"), false)?)
    } else {
        None
    };
    let artifact_result = merge_artifacts(app, temp)?;
    memory_index::reindex_or_mark_stale(app, "bundle merge import")?;
    print_json_pretty(&json!({
        "status": "imported",
        "mode": "merge",
        "entries": entries,
        "bundle": bundle,
        "memory_merge": merge_result,
        "artifacts": artifact_result
    }))
}

fn merge_artifacts(app: &App, temp: &Path) -> Result<Value> {
    let Some(incoming) = ArtifactManifest::load(temp)? else {
        return Ok(json!({"imported": 0, "identical": 0, "conflicts": []}));
    };
    let mut local = ArtifactManifest::load_or_default(&app.root)?;
    let mut imported = 0;
    let mut identical = 0;
    let mut conflicts = Vec::new();

    for entry in incoming.entries() {
        if let Err(reason) = validate_artifact_path(&entry.record.path) {
            conflicts
                .push(json!({"name": entry.name, "path": entry.record.path, "reason": reason}));
            continue;
        }
        let source = temp.join(&entry.record.path);
        if !source.exists() {
            conflicts.push(json!({"name": entry.name, "path": entry.record.path, "reason": "missing bundled file"}));
            continue;
        }
        let target = app.root.join(&entry.record.path);
        if target.exists() && fs::read(&target)? != fs::read(&source)? {
            conflicts.push(
                json!({"name": entry.name, "path": entry.record.path, "reason": "file conflict"}),
            );
            continue;
        }
        if let Ok(existing) = local.find_entry(&entry.name) {
            if existing.record.path != entry.record.path
                || existing.record.checksum != entry.record.checksum
            {
                conflicts.push(json!({"name": entry.name, "path": entry.record.path, "reason": "manifest conflict"}));
                continue;
            }
            identical += 1;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &target)?;
        local
            .artifacts
            .entry(entry.group)
            .or_default()
            .insert(entry.short_name, entry.record);
        imported += 1;
    }
    if imported > 0 {
        local.save(&app.root)?;
    }
    Ok(json!({"imported": imported, "identical": identical, "conflicts": conflicts}))
}

fn unpack_bundle(file: &Path, temp: &Path) -> Result<Vec<String>> {
    fs::create_dir_all(temp)?;
    let file = File::open(file).with_context(|| format!("open {}", file.display()))?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut entries = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        validate_bundle_path(&path, entry.header().entry_type().is_dir())?;
        let entry_name = path.to_string_lossy().to_string();
        entries.push(entry_name);
        entry.unpack(temp.join(path))?;
    }
    entries.sort();
    Ok(entries)
}

fn validate_bundle_path(path: &Path, is_dir: bool) -> Result<()> {
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

fn read_bundle_metadata(root: &Path) -> Result<Value> {
    let path = root.join("bundle.json");
    if !path.exists() {
        return Ok(Value::Null);
    }
    let text = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

fn append_file_if_exists(
    builder: &mut Builder<GzEncoder<File>>,
    source: &Path,
    name: &str,
) -> Result<()> {
    if source.exists() {
        builder.append_path_with_name(source, name)?;
    }
    Ok(())
}

fn append_json(builder: &mut Builder<GzEncoder<File>>, name: &str, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder.append_data(&mut header, name, Cursor::new(bytes))?;
    Ok(())
}

fn store_has_durable_files(app: &App) -> bool {
    app.db_path.exists()
        || app.root.join("config.toml").exists()
        || app.root.join("manifest.toml").exists()
        || app.root.join("artifacts").exists()
}

fn clear_store_for_replace(app: &App) -> Result<()> {
    remove_file_if_exists(app.root.join("memory.db"))?;
    remove_file_if_exists(app.root.join("memory.db-wal"))?;
    remove_file_if_exists(app.root.join("memory.db-shm"))?;
    remove_file_if_exists(app.root.join("config.toml"))?;
    remove_file_if_exists(app.root.join("manifest.toml"))?;
    remove_dir_if_exists(app.root.join("artifacts"))?;
    remove_dir_if_exists(app.root.join("index"))?;
    Ok(())
}

fn remove_file_if_exists(path: PathBuf) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn remove_dir_if_exists(path: PathBuf) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

fn copy_if_exists(source: PathBuf, target: PathBuf) -> Result<()> {
    if source.exists() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, target)?;
    }
    Ok(())
}

fn copy_dir_if_exists(source: PathBuf, target: PathBuf) -> Result<()> {
    if !source.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_if_exists(source_path, target_path)?;
        } else {
            copy_if_exists(source_path, target_path)?;
        }
    }
    Ok(())
}

fn temp_bundle_dir(label: &str) -> Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!("mnemark-bundle-{label}-{stamp}")))
}
