//! Semantic-edge review listing and lifecycle transitions.

use super::*;

pub fn review_semantic_edges(
    conn: &Connection,
    pending_only: bool,
    ambiguous_only: bool,
) -> Result<GraphReviewReport> {
    let mut clauses = Vec::new();
    if pending_only {
        clauses.push("status = 'pending'");
    }
    if ambiguous_only {
        clauses.push("confidence = 'AMBIGUOUS'");
    }
    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    let sql = format!(
        "SELECT id, source_ref, target_ref, relation, confidence, status, evidence,
                rationale, source_spans, tags, generated_by, source, user_confirmed_at,
                created_at, updated_at, valid_until, ambiguity_id, version
         FROM graph_semantic_edges
         {where_clause}
         ORDER BY updated_at DESC, id ASC"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_semantic_edge)?;
    let mut edges = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    for edge in &mut edges {
        edge.source_label = semantic_ref_label(conn, &edge.source_ref)?;
        edge.target_label = semantic_ref_label(conn, &edge.target_ref)?;
    }
    Ok(GraphReviewReport {
        status: "ok".to_string(),
        edges,
    })
}

pub fn set_semantic_edge_status(
    conn: &Connection,
    edge_id: &str,
    status: &str,
    note: Option<&str>,
    allow_secret_redaction: bool,
) -> Result<GraphSemanticUpdateReport> {
    if !matches!(status, "active" | "pending" | "rejected" | "superseded") {
        bail!("invalid semantic edge status: {status}");
    }
    let id = edge_id.strip_prefix("semantic:").unwrap_or(edge_id);
    if note.is_some_and(|value| value.len() > 10_000) {
        bail!("semantic edge review note exceeds 10000 bytes");
    }
    let note = note
        .map(|value| {
            sanitize_secret_field(value, "semantic edge review note", allow_secret_redaction)
        })
        .transpose()?;
    let existing = semantic_edge_by_id(conn, id)?
        .with_context(|| format!("semantic edge not found: {edge_id}"))?;
    if existing.status == status && note.is_none() {
        return Ok(GraphSemanticUpdateReport {
            status: "unchanged".to_string(),
            id: id.to_string(),
            edge_status: status.to_string(),
            ambiguity_id: existing.ambiguity_id,
        });
    }

    with_transaction(conn, |conn| {
        conn.execute(
            "UPDATE graph_semantic_edges
             SET status = ?1,
                 rationale = COALESCE(?2, rationale),
                 updated_at = CURRENT_TIMESTAMP,
                 version = version + 1
             WHERE id = ?3",
            params![status, note.as_deref(), id],
        )?;
        log_semantic_edge_revision(conn, id, "status")?;
        if status != "pending" {
            if let Some(ambiguity_id) = existing.ambiguity_id {
                resolve_ambiguity_record(
                    conn,
                    ambiguity_id,
                    &json!({
                        "status": "resolved",
                        "graph_edge_id": id,
                        "edge_status": status,
                        "note": note.as_deref(),
                    }),
                )?;
            }
        }
        set_graph_dirty(conn, true)
    })?;

    Ok(GraphSemanticUpdateReport {
        status: "updated".to_string(),
        id: id.to_string(),
        edge_status: status.to_string(),
        ambiguity_id: existing.ambiguity_id,
    })
}
