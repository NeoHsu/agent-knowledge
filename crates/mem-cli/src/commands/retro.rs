use super::*;

pub(crate) fn cmd_retro(app: &App, command: RetroCommand) -> Result<()> {
    app.require_schema()?;
    let conn = app.read_conn()?;
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
    let workflow_runs = workflow_run_stats(&conn, limit)?;
    let active_memories = query_json_rows(
        &conn,
        &format!(
            "SELECT id, type, name, tags, scope, source, confidence, version, access_count, updated_at
             FROM memories
             WHERE valid_until IS NULL
               AND (expires_at IS NULL OR datetime(expires_at) >= datetime('now'))
             ORDER BY updated_at DESC
             LIMIT {limit}"
        ),
    )?;
    let pending_graph_edges = if kind == "weekly" {
        Some(mem_core::graph::review_semantic_edges(&conn, true, false)?)
    } else {
        None
    };
    let instructions = match kind {
        "daily" => vec![
            "Use platform-provided conversation context.",
            "Compare today's conversation facts against active_memories.",
            "Persist durable new facts with source=daily_retro.",
            "Detect repeated manual procedures and suggest type=workflow memories.",
            "Use update/supersede/delete with --expected-version when changing existing memory.",
            "Record unresolved conflicts with mem ambiguity add.",
        ],
        _ => vec![
            "Review memory quality from changelog, audit, and pending ambiguities.",
            "Merge duplicates, resolve ambiguities, and identify workflow/profile/skill candidates.",
            "Use workflow_runs stats to detect stale workflow steps and workflows with repeated failures; propose runbook updates for any workflow whose failures exceed successes.",
            "Prefer repeated project procedures as workflow memory; reserve skills for stable cross-project execution policy.",
            "Calibrate low-confidence high-access memories after review.",
            "Review pending_graph_edges: accept evidence-backed relationships, reject noise, and resolve linked graph ambiguities.",
            "Use audit.graph to curate orphan memories, old pending edges, unsafe high-risk workflows, high-degree nodes, and artifact blast radius.",
            "Promote stable recurring concepts into typed tags; propose recurring graph clusters as workflows or cross-project skills.",
            "Curate every audit.over_budget_scopes entry down to per_scope_max: merge duplicates, supersede stale facts, delete obsolete low-access memories. Raise budget.per_scope_max in config.toml only when the scope genuinely needs more.",
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
        "pending_graph_edges": pending_graph_edges,
        "workflow_runs": workflow_runs,
        "recent_history": recent_history,
        "active_memories": active_memories
    }))?;
    Ok(())
}
