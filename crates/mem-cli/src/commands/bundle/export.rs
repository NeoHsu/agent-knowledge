use std::fs::{self, OpenOptions};
use std::path::Path;
use std::time::Instant;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use flate2::write::GzEncoder;
use flate2::Compression;
use tar::Builder;

use super::archive::{append_file_if_exists, append_json};
use super::install::{
    copy_dir_if_exists, copy_if_exists, harden_bundle_permissions, install_bundle_file,
    temp_bundle_dir, RemoveDirOnDrop,
};
use super::profile::elapsed_ms;
use super::validation::{bundle_hashes, prepare_bundle_import, validate_snapshot_bundle_limits};
use super::*;

pub(super) fn cmd_bundle_export(app: &App, args: BundleExportArgs) -> Result<()> {
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
    let (metadata, hash_ms) = match write_bundle_archive(
        &output_temp,
        &snapshot_root,
        &snapshot_db,
        &artifacts,
        schema_version,
        args.no_config,
    ) {
        Ok(result) => result,
        Err(error) => {
            fs::remove_file(&output_temp).ok();
            return Err(error);
        }
    };
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

fn write_bundle_archive(
    output: &Path,
    snapshot_root: &Path,
    snapshot_db: &Path,
    artifacts: &Path,
    schema_version: i64,
    no_config: bool,
) -> Result<(serde_json::Value, f64)> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let file = options
        .open(output)
        .with_context(|| format!("create {}", output.display()))?;
    let encoder = GzEncoder::new(file, Compression::default());
    let mut builder = Builder::new(encoder);

    let hash_root = snapshot_root.to_path_buf();
    let hash_worker = std::thread::spawn(move || {
        let started = Instant::now();
        let hashes = bundle_hashes(&hash_root, no_config);
        (hashes, elapsed_ms(started))
    });

    let append_result = (|| -> Result<()> {
        append_file_if_exists(&mut builder, snapshot_db, "memory.db")?;
        if !no_config {
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
            builder.append_dir_all("artifacts", artifacts)?;
        }
        Ok(())
    })();
    let hash_result = hash_worker
        .join()
        .map_err(|_| anyhow!("bundle hash worker panicked"));
    append_result?;
    let (hashes, hash_ms) = hash_result?;
    let hashes = hashes?;

    let metadata = json!({
        "version": BUNDLE_FORMAT_VERSION,
        "created_at": now(),
        "schema_version": schema_version,
        "contains": {
            "memory_db": snapshot_db.exists(),
            "config": !no_config && snapshot_root.join("config.toml").exists(),
            "manifest": snapshot_root.join("manifest.toml").exists(),
            "artifacts": artifacts.exists()
        },
        "hashes": hashes
    });
    if serde_json::to_vec(&metadata)?.len() > 1_048_576 {
        bail!("bundle metadata exceeds 1048576 bytes");
    }
    append_json(&mut builder, "bundle.json", &metadata)?;
    builder.finish()?;
    Ok((metadata, hash_ms))
}
