use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::Value;
use tar::{Archive, Builder, Header};

use super::*;
use mem_core::artifact::{artifact_file_checksum, validate_artifact_path, ArtifactManifest};

const MAX_BUNDLE_ENTRIES: usize = 10_000;
const MAX_BUNDLE_FILE_BYTES: u64 = 1_073_741_824;
const MAX_BUNDLE_TOTAL_BYTES: u64 = 4_294_967_296;
const MAX_BUNDLE_PATH_BYTES: usize = 4_096;
const SNAPSHOT_PAGES_PER_STEP: i32 = 256;
const SNAPSHOT_STEP_PAUSE: Duration = Duration::from_millis(5);

pub(crate) fn cmd_bundle(app: &App, command: BundleCommand) -> Result<()> {
    match command {
        BundleCommand::Export(args) => cmd_bundle_export(app, args),
        BundleCommand::Inspect(args) => cmd_bundle_inspect(args),
        BundleCommand::Import(args) => cmd_bundle_import(app, args),
    }
}

fn cmd_bundle_export(app: &App, args: BundleExportArgs) -> Result<()> {
    let total_started = Instant::now();
    app.require_schema()?;
    let snapshot_started = Instant::now();
    let conn = app.read_conn()?;
    let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let snapshot_root = temp_bundle_dir("export-snapshot")?;
    let _snapshot_cleanup = RemoveDirOnDrop(snapshot_root.clone());
    fs::create_dir_all(&snapshot_root)?;
    let snapshot_db = snapshot_root.join("memory.db");
    let mut snapshot = Connection::open(&snapshot_db)?;
    let backup = rusqlite::backup::Backup::new(&conn, &mut snapshot)?;
    backup.run_to_completion(SNAPSHOT_PAGES_PER_STEP, SNAPSHOT_STEP_PAUSE, None)?;
    drop(backup);
    drop(snapshot);
    drop(conn);
    if !args.no_config {
        copy_if_exists(
            app.root.join("config.toml"),
            snapshot_root.join("config.toml"),
        )?;
    }
    copy_if_exists(
        app.root.join("manifest.toml"),
        snapshot_root.join("manifest.toml"),
    )?;
    copy_dir_if_exists(app.root.join("artifacts"), snapshot_root.join("artifacts"))?;
    let snapshot_ms = elapsed_ms(snapshot_started);
    let validation_started = Instant::now();
    let snapshot_bytes = validate_snapshot_bundle_limits(&snapshot_root)?;
    prepare_bundle_import(&snapshot_root, args.redact_secrets)?;
    let validation_ms = elapsed_ms(validation_started);

    let artifacts = snapshot_root.join("artifacts");
    let hash_started = Instant::now();
    let hashes = bundle_hashes(&snapshot_root, args.no_config)?;
    let hash_ms = elapsed_ms(hash_started);
    let metadata = json!({
        "version": 2,
        "created_at": now(),
        "schema_version": schema_version,
        "contains": {
            "memory_db": snapshot_db.exists(),
            "config": !args.no_config && snapshot_root.join("config.toml").exists(),
            "manifest": snapshot_root.join("manifest.toml").exists(),
            "artifacts": artifacts.exists()
        },
        "hashes": hashes
    });
    if serde_json::to_vec(&metadata)?.len() > 1_048_576 {
        bail!("bundle metadata exceeds 1048576 bytes");
    }
    let output_parent = args
        .file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let output_name = args
        .file
        .file_name()
        .ok_or_else(|| anyhow!("bundle output must include a file name"))?
        .to_string_lossy();
    let output_temp = output_parent.join(format!(
        ".{output_name}.tmp-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let archive_started = Instant::now();
    let archive_result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&output_temp)
            .with_context(|| format!("create {}", output_temp.display()))?;
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);
        append_file_if_exists(&mut builder, &snapshot_db, "memory.db")?;
        if !args.no_config {
            append_file_if_exists(
                &mut builder,
                &snapshot_root.join("config.toml"),
                "config.toml",
            )?;
        }
        append_file_if_exists(
            &mut builder,
            &snapshot_root.join("manifest.toml"),
            "manifest.toml",
        )?;
        if artifacts.exists() {
            builder.append_dir_all("artifacts", &artifacts)?;
        }
        append_json(&mut builder, "bundle.json", &metadata)?;
        builder.finish()?;
        Ok(())
    })();
    if let Err(error) = archive_result {
        fs::remove_file(&output_temp).ok();
        return Err(error);
    }
    let archive_ms = elapsed_ms(archive_started);
    let install_started = Instant::now();
    if let Err(error) = harden_bundle_permissions(&output_temp) {
        fs::remove_file(&output_temp).ok();
        return Err(error);
    }
    if let Err(error) = install_bundle_file(&output_temp, &args.file) {
        fs::remove_file(&output_temp).ok();
        return Err(error).with_context(|| format!("install bundle {}", args.file.display()));
    }
    let output_bytes = fs::metadata(&args.file)?.len();
    let install_ms = elapsed_ms(install_started);
    fs::remove_dir_all(&snapshot_root).ok();

    let mut result = json!({
        "status": "exported",
        "file": args.file.display().to_string(),
        "bundle": metadata
    });
    if args.profile {
        result["profile"] = json!({
            "snapshot_ms": snapshot_ms,
            "validation_ms": validation_ms,
            "hash_ms": hash_ms,
            "archive_ms": archive_ms,
            "install_ms": install_ms,
            "total_ms": elapsed_ms(total_started),
            "snapshot_bytes": snapshot_bytes,
            "output_bytes": output_bytes
        });
    }
    print_json_pretty(&result)
}

fn cmd_bundle_inspect(args: BundleInspectArgs) -> Result<()> {
    let temp = temp_bundle_dir("inspect")?;
    let _cleanup = RemoveDirOnDrop(temp.clone());
    let entries = unpack_bundle(&args.file, &temp)?;
    let bundle = read_bundle_metadata(&temp)?;
    validate_bundle_hashes(&temp, &bundle)?;
    prepare_bundle_import(&temp, false)?;
    fs::remove_dir_all(&temp).ok();
    print_json_pretty(&json!({
        "status": "ok",
        "checksums_verified": bundle.get("hashes").is_some(),
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
    let _cleanup = RemoveDirOnDrop(temp.clone());
    let entries = unpack_bundle(&args.file, &temp)?;
    let bundle = read_bundle_metadata(&temp)?;
    if bundle.get("hashes").is_none() && !args.allow_unverified {
        bail!(
            "legacy bundle has no complete hash manifest; inspect it first and pass \
             --allow-unverified only if its provenance is trusted"
        );
    }
    validate_bundle_hashes(&temp, &bundle)?;
    prepare_bundle_import(&temp, args.redact_secrets)?;

    let replacement_backup = if args.replace && store_has_durable_files(app) {
        Some(snapshot_store_for_replace(app)?)
    } else {
        None
    };
    let result = if args.merge {
        import_bundle_merge(app, &temp, entries, bundle, args.redact_secrets)
    } else if args.replace {
        clear_store_for_replace(app).and_then(|()| {
            #[cfg(debug_assertions)]
            if std::env::var_os("MNEMARK_TEST_FAIL_BUNDLE_REPLACE_AFTER_CLEAR").is_some() {
                bail!("injected post-clear bundle replacement failure");
            }
            import_bundle_clean(app, &temp, entries, bundle)
        })
    } else {
        import_bundle_clean(app, &temp, entries, bundle)
    };
    fs::remove_dir_all(&temp).ok();
    match (result, replacement_backup) {
        (Ok(()), Some(backup)) => {
            fs::remove_dir_all(backup).ok();
            Ok(())
        }
        (Ok(()), None) => Ok(()),
        (Err(error), Some(backup)) => {
            clear_store_for_replace(app)?;
            restore_store_after_failed_replace(app, &backup)?;
            fs::remove_dir_all(backup).ok();
            Err(error).context("bundle replace failed; the previous store was restored")
        }
        (Err(error), None) => Err(error),
    }
}

fn import_bundle_clean(app: &App, temp: &Path, entries: Vec<String>, bundle: Value) -> Result<()> {
    fs::create_dir_all(&app.root)?;
    copy_if_exists(temp.join("memory.db"), app.root.join("memory.db"))?;
    copy_if_exists(temp.join("config.toml"), app.root.join("config.toml"))?;
    copy_if_exists(temp.join("manifest.toml"), app.root.join("manifest.toml"))?;
    copy_dir_if_exists(temp.join("artifacts"), app.root.join("artifacts"))?;
    app.require_schema()?;
    app.harden_permissions()?;
    let conn = app.conn()?;
    mem_core::graph::set_graph_dirty(&conn, true)?;
    memory_index::reindex_or_mark_stale(app, "bundle import")?;
    print_json_pretty(&json!({
        "status": "imported",
        "mode": "clean",
        "entries": entries,
        "bundle": bundle
    }))
}

fn import_bundle_merge(
    app: &App,
    temp: &Path,
    entries: Vec<String>,
    bundle: Value,
    allow_secret_redaction: bool,
) -> Result<()> {
    let merge_result = if temp.join("memory.db").exists() {
        Some(merge_database(
            app,
            &temp.join("memory.db"),
            false,
            allow_secret_redaction,
        )?)
    } else {
        None
    };
    let artifact_result = merge_artifacts(app, temp)?;
    let conn = app.conn()?;
    mem_core::graph::set_graph_dirty(&conn, true)?;
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
        match fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                conflicts.push(json!({
                    "name": entry.name,
                    "path": entry.record.path,
                    "reason": "unsafe non-regular target"
                }));
                continue;
            }
            Ok(_) if fs::read(&target)? != fs::read(&source)? => {
                conflicts.push(json!({
                    "name": entry.name,
                    "path": entry.record.path,
                    "reason": "file conflict"
                }));
                continue;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
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
        if !target.exists() {
            copy_regular_file_new(&source, &target)?;
        }
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
    if !(1..=2).contains(&version) {
        bail!("unsupported bundle format version: {version}");
    }
    Ok(metadata)
}

#[cfg(not(windows))]
fn install_bundle_file(temporary: &Path, target: &Path) -> Result<()> {
    fs::rename(temporary, target)?;
    Ok(())
}

#[cfg(windows)]
fn install_bundle_file(temporary: &Path, target: &Path) -> Result<()> {
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
fn harden_bundle_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn harden_bundle_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn prepare_bundle_import(root: &Path, redact_secrets: bool) -> Result<()> {
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

fn bundle_hashes(root: &Path, no_config: bool) -> Result<BTreeMap<String, String>> {
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

fn validate_snapshot_bundle_limits(root: &Path) -> Result<u64> {
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

fn validate_bundle_hashes(root: &Path, bundle: &Value) -> Result<()> {
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

fn snapshot_store_for_replace(app: &App) -> Result<PathBuf> {
    let backup_root = app.root.join(format!(
        ".bundle-replace-backup-{}",
        uuid::Uuid::new_v4().simple()
    ));
    fs::create_dir_all(&backup_root)?;
    if app.db_path.is_file() {
        let source = app.read_conn()?;
        let mut destination = Connection::open(backup_root.join("memory.db"))?;
        let backup = rusqlite::backup::Backup::new(&source, &mut destination)?;
        backup.run_to_completion(SNAPSHOT_PAGES_PER_STEP, SNAPSHOT_STEP_PAUSE, None)?;
        drop(backup);
        drop(destination);
        drop(source);
    }
    copy_if_exists(
        app.root.join("config.toml"),
        backup_root.join("config.toml"),
    )?;
    copy_if_exists(
        app.root.join("manifest.toml"),
        backup_root.join("manifest.toml"),
    )?;
    copy_dir_if_exists(app.root.join("artifacts"), backup_root.join("artifacts"))?;
    Ok(backup_root)
}

fn restore_store_after_failed_replace(app: &App, backup_root: &Path) -> Result<()> {
    copy_if_exists(backup_root.join("memory.db"), app.root.join("memory.db"))?;
    copy_if_exists(
        backup_root.join("config.toml"),
        app.root.join("config.toml"),
    )?;
    copy_if_exists(
        backup_root.join("manifest.toml"),
        app.root.join("manifest.toml"),
    )?;
    copy_dir_if_exists(backup_root.join("artifacts"), app.root.join("artifacts"))?;
    if app.db_path.exists() {
        app.require_schema()?;
        app.harden_permissions()?;
        memory_index::reindex_or_mark_stale(app, "restore index after failed bundle replace")?;
    }
    Ok(())
}

fn clear_store_for_replace(app: &App) -> Result<()> {
    for path in [
        app.root.join("memory.db"),
        app.root.join("memory.db-wal"),
        app.root.join("memory.db-shm"),
        app.root.join("config.toml"),
        app.root.join("manifest.toml"),
        app.root.join("artifacts"),
        app.root.join("index"),
    ] {
        remove_path_if_exists(path)?;
    }
    Ok(())
}

fn remove_path_if_exists(path: PathBuf) -> Result<()> {
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

fn copy_regular_file_new(source: &Path, target: &Path) -> Result<()> {
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

fn copy_if_exists(source: PathBuf, target: PathBuf) -> Result<()> {
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

fn copy_dir_if_exists(source: PathBuf, target: PathBuf) -> Result<()> {
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

struct RemoveDirOnDrop(PathBuf);

impl Drop for RemoveDirOnDrop {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).ok();
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn temp_bundle_dir(label: &str) -> Result<PathBuf> {
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
