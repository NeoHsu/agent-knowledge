use std::collections::HashMap;

use super::*;

#[derive(Debug, Clone, Serialize)]
struct RetrievalScore {
    total: f64,
    lexical: f64,
    source_trust: f64,
    confidence: f64,
    scope_specificity: f64,
    recency: f64,
}

#[derive(Serialize)]
struct ExplainedMemory<'a> {
    #[serde(flatten)]
    memory: &'a Memory,
    retrieval_score: &'a RetrievalScore,
}

pub(crate) fn cmd_query(app: &App, args: QueryArgs) -> Result<()> {
    app.require_schema()?;
    // TODO(F2): --semantic is intentionally hidden until an embedding backend
    // is planned and configured. Keep the guard so hidden/manual use fails
    // explicitly instead of silently falling back to full-text search.
    if args
        .query
        .as_deref()
        .is_some_and(|query| query.chars().count() > 1_000)
    {
        bail!("query cannot exceed 1000 characters");
    }
    if args.semantic {
        anyhow::bail!(
            "--semantic is not yet implemented; no embedding backend is configured. \
             Remove --semantic to use full-text/fuzzy search instead."
        );
    }
    let writes_store = (args.touch && !args.no_touch) || args.repair_index;
    let conn = if writes_store {
        app.conn()?
    } else {
        app.read_conn()?
    };
    let limit = args
        .limit
        .or_else(|| app.config.query_default_limit())
        .unwrap_or(DEFAULT_LIMIT);
    let scope = args
        .scope
        .as_deref()
        .or_else(|| app.config.query_default_scope());
    let detected_scopes = if scope == Some("auto") {
        Some(scope::detect_scope_set()?)
    } else {
        None
    };
    let scope_filter = match scope {
        Some("auto") => detected_scopes
            .as_ref()
            .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>()),
        Some("all") | None => None,
        Some(value) => {
            scope::validate_scope(value)?;
            Some(vec!["global", value])
        }
    };

    let hits = if let Some(query) = args.query.as_deref() {
        if memory_index::is_stale(app) {
            if args.repair_index {
                memory_index::repair_stale(app)?;
            } else {
                bail!(
                    "search index is stale; run `mem reindex` or retry with `mem query --repair-index ...`"
                );
            }
        }
        let search_limit = memory_count(&conn)?.max(limit).max(DEFAULT_LIMIT);
        memory_index::search_hits(
            app,
            query,
            args.fuzzy,
            args.raw_query,
            search_limit,
            args.r#type.as_deref(),
            scope_filter.as_deref(),
            args.repair_index,
        )?
    } else {
        Vec::new()
    };
    let ids = hits.iter().map(|hit| hit.id.clone()).collect::<Vec<_>>();
    let lexical_scores = hits
        .iter()
        .map(|hit| (hit.id.clone(), hit.score))
        .collect::<HashMap<_, _>>();

    // P2: when listing (no query), push filters into SQL to avoid a full table scan.
    let mut memories = if args.query.is_some() {
        let mut by_id = memories_by_ids(&conn, &ids)?;
        let mut rows = ids
            .iter()
            .filter_map(|id| by_id.remove(id))
            .collect::<Vec<_>>();
        // Post-filter search results (IDs come from tantivy which doesn't know about filters).
        rows.retain(|memory| passes_filters(memory, &args, scope_filter.as_deref()));
        rows
    } else {
        // Push all applicable filters into the SQL WHERE clause.
        mem_core::db::list_memories_filtered(
            &conn,
            args.include_superseded,
            args.r#type.as_deref(),
            args.tags.as_deref(),
            scope_filter.as_deref(),
            args.expired,
        )?
    };

    let retrieval_scores = if args.query.is_some() {
        deterministic_retrieval_scores(&memories, &lexical_scores, scope_filter.as_deref())
    } else {
        HashMap::new()
    };
    match args.sort {
        SortMode::Relevance if args.query.is_some() => memories.sort_by(|left, right| {
            let left_score = retrieval_scores
                .get(&left.id)
                .map(|score| score.total)
                .unwrap_or_default();
            let right_score = retrieval_scores
                .get(&right.id)
                .map(|score| score.total)
                .unwrap_or_default();
            right_score
                .total_cmp(&left_score)
                .then_with(|| right.updated_at.cmp(&left.updated_at))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.id.cmp(&right.id))
        }),
        SortMode::Relevance => {}
        SortMode::Time => memories.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        SortMode::AccessCount => {
            memories.sort_by_key(|memory| std::cmp::Reverse(memory.access_count))
        }
    }
    memories.truncate(limit);

    // P4: wrap all access_count updates in a single transaction instead of
    // individual UPDATE statements in a loop.
    if args.touch && !args.no_touch && !memories.is_empty() {
        let now = now();
        let ids_list: Vec<&str> = memories.iter().map(|m| m.id.as_str()).collect();
        let placeholders = placeholders(ids_list.len());
        let sql = format!(
            "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ? WHERE id IN ({placeholders})",
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_params: Vec<&dyn rusqlite::types::ToSql> = vec![&now];
        for id in &ids_list {
            bind_params.push(id);
        }
        stmt.execute(bind_params.as_slice())?;
    }

    if args.explain_score {
        if args.query.is_none() {
            bail!("--explain-score requires a search query");
        }
        if !matches!(args.format, OutputFormat::Json) {
            bail!("--explain-score requires --format json");
        }
        let explained = memories
            .iter()
            .filter_map(|memory| {
                retrieval_scores
                    .get(&memory.id)
                    .map(|score| ExplainedMemory {
                        memory,
                        retrieval_score: score,
                    })
            })
            .collect::<Vec<_>>();
        print_json_pretty(&explained)?;
        return Ok(());
    }

    match args.format {
        OutputFormat::Json => print_json_pretty(&memories)?,
        OutputFormat::Table => print_text(render_memory_table(&memories))?,
        OutputFormat::Compact => print_text(render_memory_compact(&memories))?,
    }
    Ok(())
}

fn deterministic_retrieval_scores(
    memories: &[Memory],
    lexical_scores: &HashMap<String, f64>,
    scope_filter: Option<&[&str]>,
) -> HashMap<String, RetrievalScore> {
    let max_lexical = memories
        .iter()
        .filter_map(|memory| lexical_scores.get(&memory.id).copied())
        .filter(|score| score.is_finite())
        .fold(0.0_f64, f64::max);
    memories
        .iter()
        .map(|memory| {
            let lexical = lexical_scores
                .get(&memory.id)
                .copied()
                .filter(|score| score.is_finite())
                .unwrap_or_default();
            let lexical = if max_lexical > 0.0 {
                (lexical / max_lexical).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let source_trust = f64::from(source_priority(&memory.source)) / 4.0;
            let confidence = match memory.confidence.as_str() {
                "high" => 1.0,
                "medium" => 0.6,
                "low" => 0.2,
                _ => 0.0,
            };
            let scope_specificity = match scope_filter {
                Some(_) if memory.scope == "global" => 0.6,
                Some(scopes) if scopes.contains(&memory.scope.as_str()) => 1.0,
                Some(_) => 0.0,
                None => 0.5,
            };
            let recency = recency_score(&memory.updated_at);
            let total = lexical * 0.72
                + source_trust * 0.12
                + confidence * 0.08
                + scope_specificity * 0.05
                + recency * 0.03;
            (
                memory.id.clone(),
                RetrievalScore {
                    total: round_score(total),
                    lexical: round_score(lexical),
                    source_trust: round_score(source_trust),
                    confidence: round_score(confidence),
                    scope_specificity: round_score(scope_specificity),
                    recency: round_score(recency),
                },
            )
        })
        .collect()
}

fn recency_score(timestamp: &str) -> f64 {
    let parsed = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
                .map(|value| value.and_utc())
        });
    let Ok(updated_at) = parsed else {
        return 0.0;
    };
    let age_days = (Utc::now() - updated_at).num_seconds().max(0) as f64 / 86_400.0;
    1.0 / (1.0 + age_days / 30.0)
}

fn round_score(score: f64) -> f64 {
    (score * 1_000_000.0).round() / 1_000_000.0
}

fn passes_filters(
    memory: &mem_core::db::Memory,
    args: &QueryArgs,
    scope_filter: Option<&[&str]>,
) -> bool {
    if args.expired {
        if !is_expired(memory.expires_at.as_deref()) || memory.valid_until.is_some() {
            return false;
        }
    } else {
        if is_expired(memory.expires_at.as_deref()) {
            return false;
        }
        if !args.include_superseded && memory.valid_until.is_some() {
            return false;
        }
    }
    if let Some(want_type) = &args.r#type {
        if &memory.r#type != want_type {
            return false;
        }
    }
    if let Some(tag) = &args.tags {
        if !memory_has_tag(&memory.tags, tag) {
            return false;
        }
    }
    if let Some(scopes) = scope_filter {
        if !scopes.contains(&memory.scope.as_str()) {
            return false;
        }
    }
    true
}

fn placeholders(count: usize) -> String {
    let mut output = String::with_capacity(count.saturating_mul(3));
    for index in 0..count {
        if index > 0 {
            output.push_str(", ");
        }
        output.push('?');
    }
    output
}

fn render_memory_table(memories: &[mem_core::db::Memory]) -> String {
    let rows = memories
        .iter()
        .map(|memory| {
            vec![
                truncate_text(&memory.id, 28),
                truncate_text(&memory.name, 28),
                memory.r#type.clone(),
                truncate_text(&memory.scope, 32),
                memory.confidence.clone(),
                truncate_text(&tags_text(&memory.tags), 36),
                memory.access_count.to_string(),
                truncate_text(&memory.updated_at, 20),
            ]
        })
        .collect::<Vec<_>>();
    render_table(
        &[
            "id",
            "name",
            "type",
            "scope",
            "confidence",
            "tags",
            "access",
            "updated",
        ],
        &rows,
    )
}

fn render_memory_compact(memories: &[mem_core::db::Memory]) -> String {
    let mut output = String::new();
    for memory in memories {
        let tags = truncate_text(&tags_text(&memory.tags), 160);
        let suffix = if tags.is_empty() {
            String::new()
        } else {
            format!(" tags={tags}")
        };
        output.push_str(&format!(
            "{} [{}] scope={} confidence={}{}",
            memory.name, memory.r#type, memory.scope, memory.confidence, suffix
        ));
        output.push('\n');
        if let Some(description) = memory.description.as_deref() {
            if !description.trim().is_empty() {
                output.push_str(&format!("  {}\n", truncate_text(description, 120)));
            }
        }
        if let Some(content) = memory.content.as_deref() {
            if !content.trim().is_empty() {
                output.push_str(&format!("  {}\n", truncate_text(content, 160)));
            }
        }
    }
    output
}

fn tags_text(tags: &str) -> String {
    parse_string_array(tags)
        .map(|tags| tags.join(","))
        .unwrap_or_else(|_| tags.to_string())
}
