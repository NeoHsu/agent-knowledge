//! Projection of durable semantic assertions into traversable graph edges.

use super::*;

pub(crate) fn materialize_semantic_edges(conn: &Connection) -> Result<usize> {
    let mut stmt = conn.prepare(
        "SELECT id, source_ref, target_ref, relation, confidence, status, evidence, rationale,
                source_spans, tags, source, user_confirmed_at
         FROM graph_semantic_edges
         WHERE status IN ('active', 'pending')
           AND (valid_until IS NULL OR datetime(valid_until) >= datetime('now'))",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, String>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, Option<String>>(11)?,
        ))
    })?;
    let mut materialized = 0usize;
    for row in rows {
        let (
            id,
            source_ref,
            target_ref,
            relation,
            confidence,
            status,
            evidence,
            rationale,
            source_spans,
            tags,
            source,
            user_confirmed_at,
        ) = row?;
        let Some(source_id) = normalize_semantic_endpoint(conn, &source_ref)? else {
            continue;
        };
        let Some(target_id) = normalize_semantic_endpoint(conn, &target_ref)? else {
            continue;
        };
        let weight = relation_weight(&relation);
        insert_edge(
            conn,
            &GraphEdge {
                id: format!("semantic:{}", id),
                source_node_id: source_id,
                target_node_id: target_id,
                relation,
                confidence,
                status,
                evidence: Some(evidence),
                source_ref: Some(id),
                scope: None,
                weight,
                origin: SEMANTIC.to_string(),
                metadata: json!({
                    "rationale": rationale,
                    "source_spans": parse_json_value(&source_spans),
                    "tags": parse_json_value(&tags),
                    "source": source,
                    "user_confirmed_at": user_confirmed_at,
                }),
            },
        )?;
        materialized += 1;
    }
    Ok(materialized)
}
fn normalize_semantic_endpoint(conn: &Connection, reference: &str) -> Result<Option<String>> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Ok(None);
    }
    if reference.starts_with("concept:") {
        let label = reference.trim_start_matches("concept:");
        insert_simple_node(conn, reference, "concept", label, None, SEMANTIC, json!({}))?;
        return Ok(Some(reference.to_string()));
    }
    if node_by_id(conn, reference)?.is_some() {
        return Ok(Some(reference.to_string()));
    }
    if let Some(memory_ref) = reference.strip_prefix("memory:") {
        let node = memory_node_id(memory_ref);
        return Ok(node_by_id(conn, &node)?.map(|_| node));
    }
    if let Some(memory) = memory_by_id(conn, reference)? {
        let node = memory_node_id(&memory.id);
        return Ok(node_by_id(conn, &node)?.map(|_| node));
    }
    if let Some(memory) = memory_by_name(conn, reference)? {
        let node = memory_node_id(&memory.id);
        return Ok(node_by_id(conn, &node)?.map(|_| node));
    }
    Ok(None)
}
