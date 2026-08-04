use std::fs;
use std::path::Path;

use mem_core::artifact::{ArtifactManifest, validate_artifact_path};
use serde_json::Value;

use super::archive::{read_bundle_metadata, unpack_bundle};
use super::install::{
    RemoveDirOnDrop, copy_dir_if_exists, copy_if_exists, copy_regular_file_new, temp_bundle_dir,
};
use super::rollback::{
    clear_store_for_replace, restore_store_after_failed_replace, snapshot_store_for_replace,
    store_has_durable_files,
};
use super::validation::{prepare_bundle_import, validate_bundle_hashes};
use super::*;

pub(super) fn cmd_bundle_inspect(args: BundleInspectArgs) -> Result<()> {
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

pub(super) fn cmd_bundle_import(app: &App, args: BundleImportArgs) -> Result<()> {
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
            import_bundle_clean(app, &temp, entries, bundle, false)
        })
    } else {
        import_bundle_clean(app, &temp, entries, bundle, true)
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

fn import_bundle_clean(
    app: &App,
    temp: &Path,
    entries: Vec<String>,
    bundle: Value,
    report_committed_index_failure: bool,
) -> Result<()> {
    fs::create_dir_all(&app.root)?;
    copy_if_exists(temp.join("memory.db"), app.root.join("memory.db"))?;
    copy_if_exists(temp.join("config.toml"), app.root.join("config.toml"))?;
    copy_if_exists(temp.join("manifest.toml"), app.root.join("manifest.toml"))?;
    copy_dir_if_exists(temp.join("artifacts"), app.root.join("artifacts"))?;
    app.require_schema()?;
    app.harden_permissions()?;
    let conn = app.conn()?;
    mem_core::graph::set_graph_dirty(&conn, true)?;
    let index_result = memory_index::reindex_or_mark_stale(app, "bundle import");
    if report_committed_index_failure {
        finish_committed_index_write(index_result, "bundle import", json!({"mode": "clean"}))?;
    } else {
        index_result?;
    }
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
    finish_committed_index_write(
        memory_index::reindex_or_mark_stale(app, "bundle merge import"),
        "bundle merge import",
        json!({"mode": "merge"}),
    )?;
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
