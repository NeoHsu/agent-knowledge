use super::super::*;

pub(crate) fn cmd_migrate(app: &App, args: MigrateArgs) -> Result<()> {
    let current = app.schema_version()?;
    let target = mem_core::db::supported_schema_version();
    if current > target {
        bail!("database schema v{current} is newer than this binary supports (v{target})");
    }
    if args.dry_run {
        let compatibility_required = if current == target {
            let conn = app.read_conn()?;
            let required = mem_core::db::schema_compatibility_required(&conn)?;
            if !required {
                mem_core::db::validate_store_schema_objects(&conn).context(
                    "store contains unexpected schema objects; migration cannot repair untrusted DDL",
                )?;
            }
            required
        } else {
            false
        };
        let migration_required = current < target || compatibility_required;
        return print_json_pretty(&json!({
            "status": "dry_run",
            "root": app.root.display().to_string(),
            "current_schema": current,
            "target_schema": target,
            "migration_required": migration_required,
            "compatibility_repair_required": compatibility_required,
            "backup_required": migration_required,
        }));
    }
    let backup = app.migrate()?;
    print_json_pretty(&json!({
        "status": if backup.is_some() { "migrated" } else { "up_to_date" },
        "root": app.root.display().to_string(),
        "from_schema": current,
        "to_schema": target,
        "backup": backup.map(|path| path.display().to_string()),
    }))
}
