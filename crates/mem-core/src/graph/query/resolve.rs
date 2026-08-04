use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::db::{memory_by_id, memory_by_name, memory_by_name_in_scope};
use crate::error;

use super::super::ids::memory_node_id;
use super::super::model::GraphQueryStart;
use super::super::store::node_by_id;

pub fn resolve_query_start_nodes(
    conn: &Connection,
    query: &str,
    memory_hits: &[(String, f64)],
    scope_filter: Option<&[String]>,
    limit: usize,
) -> Result<Vec<GraphQueryStart>> {
    if query.chars().count() > 1_000 {
        return Err(error::usage("graph query cannot exceed 1000 characters"));
    }
    let mut scores = HashMap::<String, f64>::new();
    if let Ok(node_id) = resolve_node_id(conn, query, scope_filter) {
        scores.insert(node_id, 1.25);
    }
    for (memory_id, score) in memory_hits {
        let node_id = memory_node_id(memory_id);
        if node_by_id(conn, &node_id)?.is_some() {
            scores
                .entry(node_id)
                .and_modify(|current| *current = current.max(*score))
                .or_insert(*score);
        }
    }

    let pattern = format!("%{}%", escape_like_pattern(query.trim()));
    let mut stmt = conn.prepare(
        "SELECT id, scope FROM graph_nodes
         WHERE id LIKE ?1 ESCAPE '\\' OR label LIKE ?1 ESCAPE '\\'
         ORDER BY CASE kind
             WHEN 'memory' THEN 0
             WHEN 'concept' THEN 1
             WHEN 'tag' THEN 2
             ELSE 3
         END, id
         LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![pattern, (limit * 3).max(limit) as i64], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (id, scope) = row?;
        if scope_allowed(scope.as_deref(), scope_filter) {
            scores.entry(id).or_insert(0.55);
        }
    }
    let mut starts = scores
        .into_iter()
        .map(|(node_id, score)| GraphQueryStart { node_id, score })
        .collect::<Vec<_>>();
    starts.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    starts.truncate(limit.max(1));
    Ok(starts)
}

pub(super) fn resolve_node_id(
    conn: &Connection,
    reference: &str,
    scope_filter: Option<&[String]>,
) -> Result<String> {
    let reference = reference.trim();
    if node_by_id(conn, reference)?.is_some() {
        return Ok(reference.to_string());
    }
    if let Some(memory_ref) = reference.strip_prefix("memory:") {
        let node_id = memory_node_id(memory_ref);
        if node_by_id(conn, &node_id)?.is_some() {
            return Ok(node_id);
        }
    }
    if let Some(id) = reference.strip_prefix("id:") {
        if let Some(memory) = memory_by_id(conn, id)? {
            let node_id = memory_node_id(&memory.id);
            if node_by_id(conn, &node_id)?.is_some() {
                return Ok(node_id);
            }
        }
    }
    if scope_filter.is_none() {
        if let Some(memory) = memory_by_id(conn, reference)? {
            let node_id = memory_node_id(&memory.id);
            if node_by_id(conn, &node_id)?.is_some() {
                return Ok(node_id);
            }
        }
    }
    let named_memory = match scope_filter {
        Some(scopes) => {
            let mut project_matches = Vec::new();
            let mut global_match = None;
            for scope in scopes {
                if let Some(memory) = memory_by_name_in_scope(conn, reference, scope)? {
                    if scope == "global" {
                        global_match = Some(memory);
                    } else {
                        project_matches.push(memory);
                    }
                }
            }
            match project_matches.as_slice() {
                [memory] => Some(memory.clone()),
                [] => global_match,
                _ => {
                    return Err(error::conflict(format!(
                        "graph memory reference is ambiguous across scopes: {reference}"
                    )));
                }
            }
        }
        None => memory_by_name(conn, reference)?,
    };
    if let Some(memory) = named_memory {
        let node_id = memory_node_id(&memory.id);
        if node_by_id(conn, &node_id)?.is_some() {
            return Ok(node_id);
        }
    }
    if let Some(memory) = memory_by_id(conn, reference)? {
        let node_id = memory_node_id(&memory.id);
        if node_by_id(conn, &node_id)?.is_some() {
            return Ok(node_id);
        }
    }

    let mut stmt = conn.prepare("SELECT id FROM graph_nodes WHERE label = ?1 ORDER BY id")?;
    let rows = stmt.query_map(params![reference], |row| row.get::<_, String>(0))?;
    let matches = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    match matches.as_slice() {
        [node_id] => Ok(node_id.clone()),
        [] => Err(error::not_found(format!(
            "graph node not found: {reference}"
        ))),
        _ => Err(error::conflict(format!(
            "graph node reference is ambiguous: {reference}"
        ))),
    }
}

pub(super) fn scope_allowed(scope: Option<&str>, scope_filter: Option<&[String]>) -> bool {
    let Some(filter) = scope_filter else {
        return true;
    };
    match scope {
        Some(scope) => filter.iter().any(|candidate| candidate == scope),
        None => true,
    }
}

fn escape_like_pattern(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}
