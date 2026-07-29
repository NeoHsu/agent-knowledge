use super::super::*;
use super::report::check;

pub(super) fn check_store(checks: &mut Vec<Value>, app: &App) -> Result<()> {
    if app.db_path.exists() {
        let conn = app.read_conn()?;
        let actual_schema: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let supported_schema = supported_schema_version();
        let quick_check: String = conn
            .query_row("PRAGMA quick_check", [], |row| row.get(0))
            .unwrap_or_else(|error| format!("error: {error}"));
        checks.push(check(
            "store_integrity",
            if quick_check == "ok" { "ok" } else { "error" },
            format!("SQLite quick_check: {quick_check}"),
            (quick_check != "ok")
                .then_some("stop using the store; restore the latest verified backup or bundle"),
        ));
        if actual_schema == supported_schema {
            checks.push(check(
                "store_schema",
                "ok",
                format!("database schema v{actual_schema} matches this binary"),
                None,
            ));
            let store_id = conn
                .query_row(
                    "SELECT value FROM metadata WHERE key = 'store_id'",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .ok();
            checks.push(check(
                "store_identity",
                if store_id.as_deref().is_some_and(|id| !id.is_empty()) {
                    "ok"
                } else {
                    "error"
                },
                store_id
                    .map(|id| format!("store_id {id}"))
                    .unwrap_or_else(|| "store_id metadata is missing".to_string()),
                Some("run `mem migrate` explicitly to repair store identity metadata"),
            ));
            match mem_core::db::schema_compatibility_required(&conn) {
                Ok(false) => match mem_core::db::validate_store_schema_objects(&conn) {
                    Ok(()) => checks.push(check(
                        "store_compatibility",
                        "ok",
                        "schema compatibility invariants are current".to_string(),
                        None,
                    )),
                    Err(error) => checks.push(check(
                        "store_compatibility",
                        "error",
                        format!("store contains unexpected schema objects: {error}"),
                        Some("restore a trusted backup; do not use or sync this store"),
                    )),
                },
                Ok(true) => checks.push(check(
                    "store_compatibility",
                    "error",
                    "schema version matches, but compatibility repairs are required".to_string(),
                    Some("run `mem migrate --dry-run`, then `mem migrate` explicitly"),
                )),
                Err(error) => checks.push(check(
                    "store_compatibility",
                    "error",
                    format!("could not verify schema compatibility: {error}"),
                    Some("stop using the store until `mem doctor` and `mem migrate --dry-run` succeed"),
                )),
            }
            match mem_core::graph::stats(&conn) {
                Ok(graph_stats) => checks.push(check(
                    "graph",
                    if graph_stats.dirty { "warn" } else { "ok" },
                    format!(
                        "schema v{}, nodes {}, edges {}, dirty {}",
                        graph_stats.schema_version,
                        graph_stats.nodes,
                        graph_stats.edges,
                        graph_stats.dirty
                    ),
                    graph_stats
                        .dirty
                        .then_some("run `mem graph rebuild` after verifying the store target"),
                )),
                Err(error) => checks.push(check(
                    "graph",
                    "error",
                    format!("cannot inspect graph materialization: {error:#}"),
                    Some("run `mem graph rebuild` to recreate rebuildable graph tables"),
                )),
            }
        } else if actual_schema > supported_schema {
            checks.push(check(
                "store_schema",
                "error",
                format!(
                    "database schema v{actual_schema} is newer than this binary supports (v{supported_schema})"
                ),
                Some("install a mem binary that supports this database schema"),
            ));
        } else {
            checks.push(check(
                "store_schema",
                "warn",
                format!(
                    "database schema v{actual_schema} requires migration to v{supported_schema}"
                ),
                Some("run `mem migrate --dry-run`, verify the backup plan, then run `mem migrate`"),
            ));
        }
        if actual_schema > supported_schema {
            checks.push(check(
                "store",
                "warn",
                format!(
                    "root {} ({}) is accessible; memory count skipped because the schema is incompatible",
                    app.root.display(),
                    app.store_source.as_str()
                ),
                None,
            ));
        } else {
            match active_memory_count(&conn) {
                Ok(active) => checks.push(check(
                    "store",
                    "ok",
                    format!(
                        "root {} ({}), {} active memories",
                        app.root.display(),
                        app.store_source.as_str(),
                        active
                    ),
                    None,
                )),
                Err(error) => checks.push(check(
                    "store",
                    "error",
                    format!(
                        "cannot read memories from {}: {error:#}",
                        app.root.display()
                    ),
                    Some("back up the store and inspect its schema before attempting repairs"),
                )),
            }
        }
        let index_stale = memory_index::is_stale(app);
        match memory_index::validate_physical_index(app) {
            Ok(()) if !index_stale => checks.push(check(
                "index",
                "ok",
                "search index present and current".to_string(),
                None,
            )),
            Ok(()) => checks.push(check(
                "index",
                "warn",
                "search index is marked stale".to_string(),
                Some("run `mem reindex`"),
            )),
            Err(error) => {
                let compatibility = memory_index::is_compatibility_error(&error);
                checks.push(check(
                    "index",
                    if compatibility { "warn" } else { "error" },
                    format!("search index is unavailable: {error:#}"),
                    Some("inspect the index path, then run `mem reindex`"),
                ));
            }
        }
        check_store_permissions(checks, app);
        let git_dir = app.root.join(".git");
        if git_dir.exists() {
            checks.push(check(
                "store_git",
                "ok",
                "store is version-controlled".to_string(),
                None,
            ));
        } else {
            checks.push(check(
                "store_git",
                "warn",
                "store is not a git repository; history and `mem sync` are unavailable".to_string(),
                Some("git init the store root, ignore index/ and lock/WAL files, commit"),
            ));
        }
    } else {
        checks.push(check(
            "store",
            "error",
            format!("no memory store at {}", app.root.display()),
            Some("run `mem init`"),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn check_store_permissions(checks: &mut Vec<Value>, app: &App) {
    use std::os::unix::fs::PermissionsExt;

    let root_mode = fs::metadata(&app.root)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .ok();
    let db_mode = fs::metadata(&app.db_path)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .ok();
    let secure = root_mode.is_some_and(|mode| mode & 0o077 == 0)
        && db_mode.is_some_and(|mode| mode & 0o077 == 0);
    checks.push(check(
        "store_permissions",
        if secure { "ok" } else { "warn" },
        format!(
            "root mode {}, memory.db mode {}",
            root_mode
                .map(|mode| format!("{mode:03o}"))
                .unwrap_or_else(|| "unknown".to_string()),
            db_mode
                .map(|mode| format!("{mode:03o}"))
                .unwrap_or_else(|| "unknown".to_string())
        ),
        (!secure).then_some("restrict the store directory to 0700 and memory.db to 0600"),
    ));
}

#[cfg(not(unix))]
fn check_store_permissions(checks: &mut Vec<Value>, app: &App) {
    checks.push(check(
        "store_permissions",
        "warn",
        format!(
            "automatic ACL verification is unavailable on this platform for {}",
            app.root.display()
        ),
        Some(
            "restrict the store directory and memory.db to the current user with platform ACL tools",
        ),
    ));
}
