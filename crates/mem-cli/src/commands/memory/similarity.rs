use super::*;

pub(super) fn similar_candidates(
    app: &App,
    conn: &Connection,
    content: &str,
    scope: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    let ids = memory_index::search_ids(
        app,
        content,
        false,
        false,
        25,
        memory_index::SearchFilters::default(),
        true,
    )?;
    let mut candidates = Vec::new();
    for id in ids {
        let Some(memory) = memory_by_id(conn, &id)? else {
            continue;
        };
        if !memory_is_active(&memory) || memory.scope != scope {
            continue;
        }
        let score = content_similarity(content, memory.content.as_deref().unwrap_or_default());
        if score >= 0.55 {
            candidates.push(json!({
                "id": memory.id,
                "name": memory.name,
                "content": memory.content,
                "score": score
            }));
        }
    }
    candidates.sort_by(|a, b| {
        let a_score = a.get("score").and_then(Value::as_f64).unwrap_or_default();
        let b_score = b.get("score").and_then(Value::as_f64).unwrap_or_default();
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(limit);
    Ok(candidates)
}
