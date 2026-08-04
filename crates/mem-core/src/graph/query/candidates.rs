use anyhow::Result;
use rusqlite::{Connection, params};

use crate::util::parse_string_array;

use super::super::ids::memory_node_id;
use super::super::model::{GraphCandidateMemory, GraphCandidates};
use super::super::{GRAPH_SCHEMA_VERSION, SEMANTIC_RELATIONS};

pub fn candidates(
    conn: &Connection,
    scope_filter: Option<&[&str]>,
    memory_type: Option<&str>,
    changed_since: Option<&str>,
    unlinked: bool,
    limit: usize,
) -> Result<GraphCandidates> {
    let mut memories =
        crate::db::list_memories_filtered(conn, false, memory_type, None, scope_filter, false)?;
    if let Some(changed_since) = changed_since {
        memories.retain(|memory| memory.updated_at.as_str() >= changed_since);
    }
    if unlinked {
        let mut unlinked_memories = Vec::new();
        for memory in memories {
            if !memory_has_useful_graph_edge(conn, &memory.id)? {
                unlinked_memories.push(memory);
            }
        }
        memories = unlinked_memories;
    }
    memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    memories.truncate(limit.clamp(1, 500));
    let memories = memories
        .into_iter()
        .map(|memory| GraphCandidateMemory {
            id: memory.id,
            name: memory.name,
            r#type: memory.r#type,
            scope: memory.scope,
            tags: parse_string_array(&memory.tags).unwrap_or_default(),
            content: memory.content,
        })
        .collect();
    Ok(GraphCandidates {
        status: "ok".to_string(),
        schema_version: GRAPH_SCHEMA_VERSION,
        instructions: vec![
            "Generate semantic edges only when useful.".to_string(),
            "Use EXTRACTED when explicitly stated.".to_string(),
            "Use INFERRED for reasonable relations.".to_string(),
            "Use AMBIGUOUS for uncertain relations requiring review.".to_string(),
            "Every edge needs evidence; do not include secrets.".to_string(),
        ],
        allowed_relations: SEMANTIC_RELATIONS
            .iter()
            .map(|relation| relation.to_string())
            .collect(),
        memories,
    })
}

fn memory_has_useful_graph_edge(conn: &Connection, memory_id: &str) -> Result<bool> {
    let node_id = memory_node_id(memory_id);
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_edges
         WHERE status = 'active'
           AND (source_node_id = ?1 OR target_node_id = ?1)
           AND relation NOT IN ('has_type', 'in_scope', 'from_source')",
        params![node_id],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}
