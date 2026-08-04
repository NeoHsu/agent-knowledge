use mem_core::memory_domain::{
    SAVE_REQUEST_SCHEMA_VERSION, SaveRequest, SaveRequestV1, existing_save_will_write, persist_save,
};

use super::lint::lint_memory;
use super::similarity::similar_candidates;
use super::*;

pub(crate) fn cmd_save(app: &App, args: SaveArgs) -> Result<()> {
    let result = save_memory(app, args)?;
    let is_similar = result
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "similar_found");
    if is_similar {
        print_write_json_pretty(app, result)?;
    } else {
        print_write_json(app, result)?;
    }
    Ok(())
}

fn request_from_args(mut args: SaveArgs) -> Result<SaveRequestV1> {
    let content = required_content(args.content.take(), args.content_file.as_deref())?;
    Ok(SaveRequestV1 {
        schema_version: SAVE_REQUEST_SCHEMA_VERSION,
        memory_type: args.r#type,
        name: args.name,
        description: args.description,
        content,
        tags: args.tags,
        scope: args.scope,
        source: args.source,
        confidence: args.confidence,
        expires_at: args.expires_at,
        why: args.why,
        force: args.force,
        user_confirmed: args.user_confirmed,
        redact_secrets: args.redact_secrets,
        no_validate_workflow: args.no_validate_workflow,
        origin: args.origin,
        origin_ref: args.origin_ref,
    })
}

pub(crate) fn save_memory(app: &App, args: SaveArgs) -> Result<Value> {
    app.require_schema()?;
    let request = request_from_args(args)?.validate_and_normalize()?;
    save_normalized_memory(app, request)
}

fn save_normalized_memory(app: &App, request: SaveRequest) -> Result<Value> {
    let conn = app.conn()?;

    if let Some(existing) = memory_by_name_in_scope(&conn, &request.name, &request.scope)? {
        if !existing_save_will_write(&request, &existing) {
            return persist_save(&conn, &request, Some(&existing))?.to_json();
        }
        let outcome = with_transaction(&conn, |conn| {
            let outcome = persist_save(conn, &request, Some(&existing))?;
            if outcome.changed_id().is_some() {
                mem_core::graph::set_graph_dirty(conn, true)?;
            }
            Ok(outcome)
        })?;
        if let Some(id) = outcome.changed_id() {
            finish_committed_index_write(
                memory_index::upsert_or_mark_stale(app, &conn, id),
                "memory update",
                json!({"memory_id": id, "version": outcome.version()}),
            )?;
        }
        return outcome.to_json();
    }

    if !request.force {
        memory_index::repair_stale(app)?;
        let candidates = similar_candidates(app, &conn, &request.content, &request.scope, 5)?;
        if !candidates.is_empty() {
            return Ok(json!({
                "status": "similar_found",
                "match_type": "bm25_lindera",
                "candidates": candidates,
                "new_content": request.content
            }));
        }
    }

    let outcome = with_transaction(&conn, |conn| {
        let outcome = persist_save(conn, &request, None)?;
        mem_core::graph::set_graph_dirty(conn, true)?;
        Ok(outcome)
    })?;
    let id = outcome
        .changed_id()
        .ok_or_else(|| anyhow!("new save did not return a changed memory id"))?;
    finish_committed_index_write(
        memory_index::upsert_or_mark_stale(app, &conn, id),
        "memory save",
        json!({"memory_id": id, "version": outcome.version()}),
    )?;

    let mut result = outcome.to_json()?;
    let warnings = lint_memory(
        &request.memory_type,
        &request.name,
        &request.content,
        &request.tags,
    );
    if !warnings.is_empty() {
        result["warnings"] = json!(warnings);
    }
    Ok(result)
}

/// Save one imported memory inside a caller-owned transaction or savepoint,
/// skipping similarity and index operations. The caller batches index updates
/// and marks graph materialization dirty once.
pub(crate) fn save_request_no_index_in_connection(
    conn: &Connection,
    request: SaveRequestV1,
) -> Result<(Value, Option<String>)> {
    let request = request.validate_and_normalize()?;
    let existing = memory_by_name_in_scope(conn, &request.name, &request.scope)?;
    let outcome = persist_save(conn, &request, existing.as_ref())?;
    let changed_id = outcome.changed_id().map(str::to_string);
    Ok((outcome.to_json()?, changed_id))
}
