use super::*;
use crate::commands::setup::{
    base_dir, has_current_policy, has_v4_policy, platform_by_name, skill_files_current,
    skill_link_points_to, PlatformSpec, HOOK_COMMAND, LEGACY_HOOK_COMMAND, PLATFORMS,
    POLICY_MARKER_V2, POLICY_MARKER_V3, POLICY_MARKER_V4, POLICY_MARKER_V5, SHARED_SKILLS_DIR,
};

pub(crate) fn cmd_doctor(app: &App, args: DoctorArgs) -> Result<()> {
    let mut checks = Vec::new();

    checks.push(check(
        "binary",
        "ok",
        format!(
            "mem {} at {}",
            env!("CARGO_PKG_VERSION"),
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        ),
        None,
    ));

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
            match memory_count(&conn) {
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
        check_store_permissions(&mut checks, app);
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

    let base = base_dir(args.base_dir.as_deref());
    let shared_skill_root = base.join(SHARED_SKILLS_DIR).join("mnemark");
    check_shared_skill(&mut checks, &shared_skill_root);
    let platforms: Vec<&PlatformSpec> = match args.platform.as_deref() {
        Some(name) => {
            vec![platform_by_name(name).ok_or_else(|| anyhow!("unknown platform: {name}"))?]
        }
        None => PLATFORMS.iter().collect(),
    };
    for platform in platforms {
        check_platform(&mut checks, platform, &base, &shared_skill_root);
    }

    let has_error = checks
        .iter()
        .any(|entry| entry.get("status").and_then(Value::as_str) == Some("error"));
    let has_warn = checks
        .iter()
        .any(|entry| entry.get("status").and_then(Value::as_str) == Some("warn"));
    print_json_pretty(&json!({
        "status": if has_error { "error" } else if has_warn { "warn" } else { "ok" },
        "checks": checks
    }))
}

fn check_shared_skill(checks: &mut Vec<Value>, shared_root: &Path) {
    if skill_files_current(shared_root) {
        checks.push(check(
            "shared.skill",
            "ok",
            format!("shared mnemark skill at {}", shared_root.display()),
            None,
        ));
    } else if shared_root.exists() {
        checks.push(check(
            "shared.skill",
            "warn",
            format!(
                "shared mnemark skill is incomplete or stale at {}",
                shared_root.display()
            ),
            Some("run `mem setup <platform>` to refresh the shared skill"),
        ));
    } else {
        checks.push(check(
            "shared.skill",
            "missing",
            format!(
                "shared mnemark skill not found at {}",
                shared_root.display()
            ),
            Some("run `mem setup <platform>` to install the shared skill"),
        ));
    }
}

fn check_platform(
    checks: &mut Vec<Value>,
    platform: &PlatformSpec,
    base: &Path,
    shared_skill_root: &Path,
) {
    let prefix = platform.name;
    let instructions = base.join(platform.instructions);
    match fs::read_to_string(&instructions) {
        Ok(content) if has_current_policy(&content) => checks.push(check(
            format!("{prefix}.policy"),
            "ok",
            format!("v5 policy in {}", instructions.display()),
            None,
        )),
        Ok(content) if content.contains(POLICY_MARKER_V5) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("drifted v5 policy in {}", instructions.display()),
            Some(
                "replace the managed block with the policy from `mem setup agent-policy --dry-run`",
            ),
        )),
        Ok(content) if has_v4_policy(&content) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v4 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v5"),
        )),
        Ok(content) if content.contains(POLICY_MARKER_V4) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("drifted v4 policy in {}", instructions.display()),
            Some(
                "replace the managed block with the policy from `mem setup agent-policy --dry-run`",
            ),
        )),
        Ok(content) if content.contains(POLICY_MARKER_V3) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v3 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v5"),
        )),
        Ok(content) if content.contains(POLICY_MARKER_V2) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("v2 policy in {}", instructions.display()),
            Some("run `mem setup <platform>` to upgrade the policy block to v5"),
        )),
        Ok(content) if content.contains("mnemark memory policy") => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("legacy policy block in {}", instructions.display()),
            Some("run `mem setup <platform>` and replace the old block with v4"),
        )),
        Ok(_) => checks.push(check(
            format!("{prefix}.policy"),
            "warn",
            format!("{} exists without a mnemark policy", instructions.display()),
            Some("run `mem setup <platform>`"),
        )),
        Err(_) => checks.push(check(
            format!("{prefix}.policy"),
            "missing",
            format!("{} not found", instructions.display()),
            Some("run `mem setup <platform>` (skip if this agent is not installed)"),
        )),
    }

    if let Some(skills_dir) = platform.skills_dir {
        let platform_skill_root = base.join(skills_dir).join("mnemark");
        if platform_skill_root == shared_skill_root {
            checks.push(check(
                format!("{prefix}.skill"),
                if skill_files_current(shared_skill_root) {
                    "ok"
                } else {
                    "warn"
                },
                format!(
                    "platform uses shared skill at {}",
                    shared_skill_root.display()
                ),
                if skill_files_current(shared_skill_root) {
                    None
                } else {
                    Some("run `mem setup <platform>` to refresh the shared skill")
                },
            ));
        } else {
            match fs::symlink_metadata(&platform_skill_root) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if skill_link_points_to(&platform_skill_root, shared_skill_root) {
                        checks.push(check(
                            format!("{prefix}.skill"),
                            "ok",
                            format!(
                                "skill link {} -> {}",
                                platform_skill_root.display(),
                                shared_skill_root.display()
                            ),
                            None,
                        ));
                    } else {
                        checks.push(check(
                            format!("{prefix}.skill"),
                            "warn",
                            format!(
                                "skill symlink {} points somewhere other than {}",
                                platform_skill_root.display(),
                                shared_skill_root.display()
                            ),
                            Some("remove the conflicting link, then run `mem setup <platform>`"),
                        ));
                    }
                }
                Ok(metadata) if metadata.is_dir() => checks.push(check(
                    format!("{prefix}.skill"),
                    "warn",
                    format!("legacy skill copy at {}", platform_skill_root.display()),
                    Some("run `mem setup <platform>` to migrate the managed copy to a shared link"),
                )),
                Ok(_) => checks.push(check(
                    format!("{prefix}.skill"),
                    "warn",
                    format!("invalid skill path at {}", platform_skill_root.display()),
                    Some("move the conflicting path, then run `mem setup <platform>`"),
                )),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => checks.push(check(
                    format!("{prefix}.skill"),
                    "missing",
                    format!("skill link not found at {}", platform_skill_root.display()),
                    Some("run `mem setup <platform>`"),
                )),
                Err(error) => checks.push(check(
                    format!("{prefix}.skill"),
                    "error",
                    format!("cannot inspect {}: {error}", platform_skill_root.display()),
                    None,
                )),
            }
        }
    }

    if let Some(settings_rel) = platform.claude_settings {
        let settings = base.join(settings_rel);
        let status = fs::read_to_string(&settings)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .map(|root| {
                let mut current = false;
                let mut legacy = false;
                let mut custom = false;
                if let Some(entries) = root
                    .pointer("/hooks/SessionStart")
                    .and_then(Value::as_array)
                {
                    for entry in entries {
                        let Some(hooks) = entry.get("hooks").and_then(Value::as_array) else {
                            continue;
                        };
                        for hook in hooks {
                            let Some(command) = hook.get("command").and_then(Value::as_str) else {
                                continue;
                            };
                            if command == HOOK_COMMAND {
                                current = true;
                            } else if command == LEGACY_HOOK_COMMAND {
                                legacy = true;
                            } else if command.contains("mem prime")
                                || command.contains("session-prime")
                            {
                                custom = true;
                            }
                        }
                    }
                }
                if current {
                    "current"
                } else if legacy {
                    "legacy"
                } else if custom {
                    "custom"
                } else {
                    "absent"
                }
            });
        match status {
            Some("current") => checks.push(check(
                format!("{prefix}.session_hook"),
                "ok",
                format!("SessionStart hook in {}", settings.display()),
                None,
            )),
            Some("legacy") => checks.push(check(
                format!("{prefix}.session_hook"),
                "warn",
                format!(
                    "legacy SessionStart hook hides `mem prime` failures in {}",
                    settings.display()
                ),
                Some("run `mem setup claude-code` to upgrade the hook"),
            )),
            Some("custom") => checks.push(check(
                format!("{prefix}.session_hook"),
                "ok",
                format!("custom mnemark SessionStart hook in {}", settings.display()),
                None,
            )),
            Some(_) => checks.push(check(
                format!("{prefix}.session_hook"),
                "warn",
                format!("no mnemark SessionStart hook in {}", settings.display()),
                Some("run `mem setup claude-code` to add the `mem prime` hook"),
            )),
            None => checks.push(check(
                format!("{prefix}.session_hook"),
                "missing",
                format!("{} missing or not valid JSON", settings.display()),
                Some(&format!(
                    "run `mem setup claude-code`; the hook command is `{HOOK_COMMAND}`"
                )),
            )),
        }
    }
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
fn check_store_permissions(_checks: &mut Vec<Value>, _app: &App) {}

fn check(id: impl Into<String>, status: &str, detail: String, fix: Option<&str>) -> Value {
    let mut entry = json!({
        "id": id.into(),
        "status": status,
        "detail": detail
    });
    if let Some(fix) = fix {
        entry["fix"] = json!(fix);
    }
    entry
}
