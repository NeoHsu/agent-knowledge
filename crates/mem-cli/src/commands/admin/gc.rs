use super::super::*;

pub(crate) fn cmd_gc(app: &App, args: GcArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.conn()?;
    let cutoff = (Utc::now() - Duration::days(args.days)).to_rfc3339();
    let changed = with_transaction(&conn, |conn| {
        let gc_memories = gc_candidate_memories(conn, &cutoff)?;
        for memory in &gc_memories {
            log_change(
                conn,
                &memory.id,
                "gc",
                memory.content.as_deref(),
                None,
                "gc",
            )?;
        }
        let changed = conn.execute(
            "DELETE FROM memories WHERE valid_until IS NOT NULL AND datetime(valid_until) < datetime(?1)",
            params![cutoff],
        )?;
        Ok(changed)
    })?;
    mem_core::graph::set_graph_dirty(&conn, true)?;
    finish_committed_index_write(
        memory_index::reindex_or_mark_stale(app, "rebuild index after gc"),
        "garbage collection",
        json!({"deleted": changed}),
    )?;
    print_json(&json!({"status": "gc_complete", "deleted": changed}))?;
    Ok(())
}
