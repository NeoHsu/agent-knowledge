use super::*;

pub(crate) fn cmd_retro(app: &App, command: RetroCommand) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    let (kind, limit) = match command {
        RetroCommand::Daily(args) => ("daily", args.limit),
        RetroCommand::Weekly(args) => ("weekly", args.limit),
    };
    let limit = limit.clamp(1, 500);
    let recent_history = query_json_rows(
        &conn,
        &format!(
            "SELECT id, memory_id, action, old_content, new_content, source, created_at
             FROM changelog
             ORDER BY created_at DESC
             LIMIT {limit}"
        ),
    )?;
    let pending_ambiguities = ambiguity_rows(&conn, true)?;
    let active_memories = query_json_rows(
        &conn,
        &format!(
            "SELECT id, type, name, tags, scope, source, confidence, version, access_count, updated_at
             FROM memories
             WHERE valid_until IS NULL
             ORDER BY updated_at DESC
             LIMIT {limit}"
        ),
    )?;
    let instructions = match kind {
        "daily" => vec![
            "Use platform-provided conversation context; repo readers are optional adapters.",
            "Compare today's conversation facts against active_memories.",
            "Persist durable new facts with source=daily_retro.",
            "Detect repeated manual procedures and suggest type=workflow memories.",
            "Use update/supersede/delete with --expected-version when changing existing memory.",
            "Record unresolved conflicts with mem ambiguity add.",
        ],
        _ => vec![
            "Review memory quality from changelog, audit, and pending ambiguities.",
            "Merge duplicates, resolve ambiguities, and identify workflow/profile/skill candidates.",
            "Detect stale workflow steps and workflows with repeated failures.",
            "Prefer repeated project procedures as workflow memory; reserve skills for stable cross-project execution policy.",
            "Calibrate low-confidence high-access memories after review.",
            "Use audit --fix only for deterministic repairs.",
        ],
    };

    print_json_pretty(&json!({
        "status": "retro_bundle",
        "kind": kind,
        "generated_at": now(),
        "instructions": instructions,
        "stats": stats_report(&conn)?,
        "audit": audit_report(&conn, app, false)?,
        "pending_ambiguities": pending_ambiguities,
        "recent_history": recent_history,
        "active_memories": active_memories
    }))?;
    Ok(())
}
