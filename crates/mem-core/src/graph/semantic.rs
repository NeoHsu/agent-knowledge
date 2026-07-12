use super::*;

pub fn ingest_semantic_edges(
    conn: &Connection,
    payload: Value,
    options: GraphIngestOptions,
) -> Result<GraphIngestReport> {
    let payload: SemanticEdgePayload =
        serde_json::from_value(payload).context("parse semantic edge payload")?;
    if payload.schema_version != GRAPH_SCHEMA_VERSION {
        bail!(
            "unsupported semantic edge schema_version {}; expected {}",
            payload.schema_version,
            GRAPH_SCHEMA_VERSION
        );
    }
    if payload.edges.len() > 1_000 {
        bail!("semantic edge payload cannot exceed 1000 edges");
    }
    let inputs = payload.edges;

    with_transaction(conn, |conn| {
        let mut report = GraphIngestReport {
            status: "ingested".to_string(),
            total: inputs.len(),
            inserted: 0,
            updated: 0,
            unchanged: 0,
            rejected: 0,
            pending: 0,
            results: Vec::new(),
        };

        for (index, input) in inputs.iter().enumerate() {
            let result = ingest_one_semantic_edge(conn, index, input, &options)?;
            match result.status.as_str() {
                "inserted" => report.inserted += 1,
                "updated" => report.updated += 1,
                "unchanged" => report.unchanged += 1,
                "rejected" => report.rejected += 1,
                _ => {}
            }
            if result.edge_status.as_deref() == Some("pending") {
                report.pending += 1;
            }
            report.results.push(result);
        }
        set_graph_dirty(conn, true)?;
        Ok(report)
    })
}

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

pub fn merge_semantic_edges(
    conn: &Connection,
    incoming: &Connection,
    memory_id_map: &HashMap<String, String>,
    ambiguity_id_map: &HashMap<i64, i64>,
    review_memory_ids: &HashSet<String>,
    prefer_trusted: bool,
    allow_secret_redaction: bool,
) -> Result<GraphSemanticMergeReport> {
    if !table_exists(incoming, "graph_semantic_edges")? {
        return Ok(GraphSemanticMergeReport::default());
    }

    let incoming_edges = load_semantic_edges(incoming)?;
    let mut report = GraphSemanticMergeReport::default();
    for mut edge in incoming_edges {
        if edge.source == "manual" && edge.user_confirmed_at.is_none() {
            edge.source = "agent".to_string();
            edge.generated_by = "import".to_string();
            report.unattested_manual_downgraded += 1;
        }
        let evidence = sanitize_secret_field(
            &edge.evidence,
            "semantic edge evidence",
            allow_secret_redaction,
        )?;
        let rationale = edge
            .rationale
            .as_deref()
            .map(|value| {
                sanitize_secret_field(value, "semantic edge rationale", allow_secret_redaction)
            })
            .transpose()?;
        let source_spans = sanitize_json_secrets(
            &edge.source_spans,
            "semantic edge source_spans",
            allow_secret_redaction,
        )?;
        let tags = sanitize_json_secrets(&edge.tags, "semantic edge tags", allow_secret_redaction)?;
        let tags = normalized_string_array(&tags).map_err(anyhow::Error::msg)?;
        if evidence.chars().count() > 20_000 {
            bail!("merged semantic edge evidence exceeds 20000 characters");
        }
        if rationale
            .as_deref()
            .is_some_and(|value| value.chars().count() > 10_000)
        {
            bail!("merged semantic edge rationale exceeds 10000 characters");
        }
        if source_spans.to_string().len() > 100_000 {
            bail!("merged semantic edge source_spans exceeds 100000 bytes");
        }
        if tags.as_array().is_some_and(|values| values.len() > 100) {
            bail!("merged semantic edge tags cannot exceed 100 entries");
        }
        validate_semantic_edge_id(&edge.id).map_err(anyhow::Error::msg)?;
        let (source_ref, source_resolved) =
            remap_merge_endpoint(conn, &edge.source_ref, memory_id_map)?;
        let (target_ref, target_resolved) =
            remap_merge_endpoint(conn, &edge.target_ref, memory_id_map)?;
        let unresolved = !source_resolved || !target_resolved;
        if unresolved {
            report.unresolved_endpoints += 1;
        }
        let cross_scope = semantic_edge_crosses_project_scopes(conn, &source_ref, &target_ref)?;
        let memory_conflict =
            endpoint_references_review_memory(&edge.source_ref, review_memory_ids)
                || endpoint_references_review_memory(&edge.target_ref, review_memory_ids);
        let mut status = edge.status.as_str();
        if edge.confidence == "AMBIGUOUS" && status == "active" {
            // Preserve reviewed active ambiguous edges from the incoming store.
        } else if edge.confidence == "AMBIGUOUS" || unresolved {
            status = "pending";
        }
        if cross_scope && edge.source != "manual" {
            status = "pending";
        }
        if memory_conflict {
            status = "pending";
        }

        let by_id = semantic_edge_by_id(conn, &edge.id)?;
        let by_logical_key = if by_id.is_none() {
            semantic_edge_by_logical_key(conn, &source_ref, &target_ref, &edge.relation)?
        } else {
            None
        };
        let incumbent = by_id.as_ref().or(by_logical_key.as_ref());
        if let Some(existing) = incumbent {
            if semantic_edge_matches(
                conn,
                &existing.id,
                &source_ref,
                &target_ref,
                &edge.relation,
                &edge.confidence,
                status,
                &evidence,
                rationale.as_deref(),
                &source_spans,
                &tags,
                &edge.source,
                edge.valid_until.as_deref(),
            )? {
                report
                    .edge_id_map
                    .insert(edge.id.clone(), existing.id.clone());
                report.identical += 1;
                continue;
            }
            let incoming_priority = source_priority(&edge.source);
            let existing_priority = source_priority(&existing.source);
            if incoming_priority < existing_priority {
                report.rejected_lower_trust += 1;
                continue;
            }
            if prefer_trusted && incoming_priority > existing_priority {
                let input = SemanticEdgeInput {
                    id: Some(existing.id.clone()),
                    source: source_ref.clone(),
                    target: target_ref.clone(),
                    relation: edge.relation.clone(),
                    confidence: edge.confidence.clone(),
                    evidence: evidence.clone(),
                    rationale: rationale.clone(),
                    source_spans: source_spans.clone(),
                    tags: tags.clone(),
                    valid_until: edge.valid_until.clone(),
                };
                let ambiguity_id = if status == "pending" {
                    report.pending += 1;
                    Some(ensure_pending_edge_ambiguity(
                        conn,
                        &input,
                        &source_ref,
                        &target_ref,
                        existing.ambiguity_id,
                        None,
                        cross_scope,
                    )?)
                } else {
                    if let Some(ambiguity_id) = existing.ambiguity_id {
                        resolve_ambiguity_record(
                            conn,
                            ambiguity_id,
                            &json!({
                                "status": "resolved",
                                "graph_edge_id": existing.id,
                                "edge_status": status,
                                "reason": "higher-trust semantic edge merge",
                            }),
                        )?;
                    }
                    existing.ambiguity_id
                };
                update_semantic_edge_from_merge(
                    conn,
                    &existing.id,
                    &source_ref,
                    &target_ref,
                    &edge,
                    status,
                    &evidence,
                    rationale.as_deref(),
                    &source_spans,
                    &tags,
                    ambiguity_id,
                )?;
                report
                    .edge_id_map
                    .insert(edge.id.clone(), existing.id.clone());
                report.trusted_updates += 1;
                continue;
            }
            status = "pending";
            report.conflicts += 1;
        }

        let id = unique_semantic_edge_id(conn, &edge.id)?;
        let input = SemanticEdgeInput {
            id: Some(id.clone()),
            source: source_ref.clone(),
            target: target_ref.clone(),
            relation: edge.relation.clone(),
            confidence: edge.confidence.clone(),
            evidence: evidence.clone(),
            rationale: rationale.clone(),
            source_spans: source_spans.clone(),
            tags: tags.clone(),
            valid_until: edge.valid_until.clone(),
        };
        let ambiguity_id = if status == "pending" {
            report.pending += 1;
            let mapped = edge
                .ambiguity_id
                .and_then(|incoming_id| ambiguity_id_map.get(&incoming_id).copied());
            if incumbent.is_none() && !unresolved && !cross_scope && !memory_conflict {
                mapped
            } else {
                Some(ensure_pending_edge_ambiguity(
                    conn,
                    &input,
                    &source_ref,
                    &target_ref,
                    mapped,
                    incumbent.map(|existing| existing.id.as_str()),
                    cross_scope,
                )?)
            }
        } else {
            None
        };
        conn.execute(
            "INSERT INTO graph_semantic_edges
             (id, source_ref, target_ref, relation, confidence, status, evidence,
              rationale, source_spans, tags, generated_by, source, user_confirmed_at,
              created_at, updated_at, valid_until, ambiguity_id, version)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                     ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                id,
                source_ref,
                target_ref,
                edge.relation,
                edge.confidence,
                status,
                evidence,
                rationale,
                source_spans.to_string(),
                tags.to_string(),
                edge.generated_by,
                edge.source,
                edge.user_confirmed_at,
                edge.created_at,
                edge.updated_at,
                edge.valid_until,
                ambiguity_id,
                edge.version.max(1),
            ],
        )?;
        log_semantic_edge_revision(conn, &id, "merge")?;
        report.edge_id_map.insert(edge.id.clone(), id.clone());
        report.imported += 1;
    }
    if report.changed() {
        set_graph_dirty(conn, true)?;
    }
    Ok(report)
}

pub(super) fn materialize_semantic_edges(conn: &Connection) -> Result<usize> {
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

fn ingest_one_semantic_edge(
    conn: &Connection,
    index: usize,
    input: &SemanticEdgeInput,
    options: &GraphIngestOptions,
) -> Result<GraphIngestResult> {
    let mut input = input.clone();
    input.id = input
        .id
        .as_deref()
        .map(|value| {
            sanitize_secret_field(value, "semantic edge id", options.allow_secret_redaction)
        })
        .transpose()?;
    input.source = sanitize_secret_field(
        &input.source,
        "semantic edge source",
        options.allow_secret_redaction,
    )?;
    input.target = sanitize_secret_field(
        &input.target,
        "semantic edge target",
        options.allow_secret_redaction,
    )?;
    input.relation = sanitize_secret_field(
        &input.relation,
        "semantic edge relation",
        options.allow_secret_redaction,
    )?;
    input.confidence = sanitize_secret_field(
        &input.confidence,
        "semantic edge confidence",
        options.allow_secret_redaction,
    )?;
    let rejected = |reason: String| GraphIngestResult {
        index,
        status: "rejected".to_string(),
        id: input.id.clone(),
        reason: Some(reason),
        source: Some(input.source.clone()),
        target: Some(input.target.clone()),
        relation: Some(input.relation.clone()),
        confidence: Some(input.confidence.clone()),
        edge_status: None,
    };

    if !SEMANTIC_RELATIONS.contains(&input.relation.as_str()) {
        return Ok(rejected(format!(
            "relation is not allowlisted: {}",
            input.relation
        )));
    }
    if !matches!(
        input.confidence.as_str(),
        "EXTRACTED" | "INFERRED" | "AMBIGUOUS"
    ) {
        return Ok(rejected(format!(
            "invalid confidence: {}",
            input.confidence
        )));
    }
    if options.source == "manual" && !options.user_confirmed {
        return Ok(rejected(
            "source=manual requires explicit user confirmation".to_string(),
        ));
    }
    let evidence = sanitize_secret_field(
        input.evidence.trim(),
        "semantic edge evidence",
        options.allow_secret_redaction,
    )?;
    if evidence.trim().is_empty() {
        return Ok(rejected("evidence is required".to_string()));
    }
    if evidence.chars().count() > 20_000 {
        return Ok(rejected("evidence exceeds 20000 characters".to_string()));
    }
    let rationale = input
        .rationale
        .as_deref()
        .map(|value| {
            sanitize_secret_field(
                value,
                "semantic edge rationale",
                options.allow_secret_redaction,
            )
        })
        .transpose()?;
    if rationale
        .as_deref()
        .is_some_and(|value| value.chars().count() > 10_000)
    {
        return Ok(rejected("rationale exceeds 10000 characters".to_string()));
    }
    let source_spans = sanitize_json_secrets(
        &input.source_spans,
        "semantic edge source_spans",
        options.allow_secret_redaction,
    )?;
    let source_spans = normalized_json_array(&source_spans).map_err(|err| anyhow::anyhow!(err))?;
    let tags = sanitize_json_secrets(
        &input.tags,
        "semantic edge tags",
        options.allow_secret_redaction,
    )?;
    let tags = normalized_string_array(&tags).map_err(|err| anyhow::anyhow!(err))?;
    let valid_until = input
        .valid_until
        .as_deref()
        .map(normalize_rfc3339)
        .transpose()?;
    if source_spans.to_string().len() > 100_000 {
        return Ok(rejected("source_spans exceeds 100000 bytes".to_string()));
    }
    if tags.as_array().is_some_and(|values| values.len() > 100) {
        return Ok(rejected("tags cannot exceed 100 entries".to_string()));
    }
    let Some(source_ref) = normalize_endpoint_for_ingest(conn, &input.source)? else {
        return Ok(rejected(format!(
            "unknown source endpoint: {}",
            input.source
        )));
    };
    let Some(target_ref) = normalize_endpoint_for_ingest(conn, &input.target)? else {
        return Ok(rejected(format!(
            "unknown target endpoint: {}",
            input.target
        )));
    };

    let cross_scope = semantic_edge_crosses_project_scopes(conn, &source_ref, &target_ref)?;
    let mut edge_status = match input.confidence.as_str() {
        "AMBIGUOUS" => "pending",
        "INFERRED" if options.pending_inferred => "pending",
        _ => "active",
    };
    if cross_scope && options.source != "manual" {
        edge_status = "pending";
    }

    let id = match input.id.as_deref().filter(|id| !id.trim().is_empty()) {
        Some(id) => validate_semantic_edge_id(id).map_err(|err| anyhow::anyhow!(err))?,
        None => format!(
            "sem_{}",
            stable_hash_hex(&format!(
                "{}\0{}\0{}\0{}",
                source_ref, target_ref, input.relation, evidence
            ))
        ),
    };

    let existing_by_id = semantic_edge_by_id(conn, &id)?;
    let user_confirmed_at = if options.source == "manual" {
        existing_by_id
            .as_ref()
            .and_then(|existing| existing.user_confirmed_at.clone())
            .or_else(|| Some(now()))
    } else {
        None
    };
    if let Some(existing) = existing_by_id.as_ref() {
        if source_priority(&options.source) < source_priority(&existing.source) {
            return Ok(GraphIngestResult {
                index,
                status: "rejected".to_string(),
                id: Some(id),
                reason: Some("lower_trust_source_cannot_overwrite".to_string()),
                source: Some(source_ref),
                target: Some(target_ref),
                relation: Some(input.relation.clone()),
                confidence: Some(input.confidence.clone()),
                edge_status: Some(edge_status.to_string()),
            });
        }
    }

    if existing_by_id.is_none() {
        if let Some(existing_id) = identical_semantic_edge_id(
            conn,
            &source_ref,
            &target_ref,
            &input.relation,
            &input.confidence,
            edge_status,
            &evidence,
            valid_until.as_deref(),
        )? {
            return Ok(GraphIngestResult {
                index,
                status: "unchanged".to_string(),
                id: Some(existing_id),
                reason: None,
                source: Some(source_ref),
                target: Some(target_ref),
                relation: Some(input.relation.clone()),
                confidence: Some(input.confidence.clone()),
                edge_status: Some(edge_status.to_string()),
            });
        }
    }

    let logical_conflict = if existing_by_id.is_none() {
        semantic_edge_by_logical_key(conn, &source_ref, &target_ref, &input.relation)?
    } else {
        None
    };
    if let Some(existing) = logical_conflict.as_ref() {
        if source_priority(&options.source) < source_priority(&existing.source) {
            return Ok(GraphIngestResult {
                index,
                status: "rejected".to_string(),
                id: Some(id),
                reason: Some("lower_trust_source_cannot_override_logical_edge".to_string()),
                source: Some(source_ref),
                target: Some(target_ref),
                relation: Some(input.relation.clone()),
                confidence: Some(input.confidence.clone()),
                edge_status: Some("pending".to_string()),
            });
        }
        edge_status = "pending";
    }

    let previous_ambiguity_id = existing_by_id
        .as_ref()
        .and_then(|existing| existing.ambiguity_id);
    let ambiguity_id = if edge_status == "pending" {
        Some(ensure_pending_edge_ambiguity(
            conn,
            &input,
            &source_ref,
            &target_ref,
            previous_ambiguity_id,
            logical_conflict.as_ref().map(|edge| edge.id.as_str()),
            cross_scope,
        )?)
    } else {
        if let Some(ambiguity_id) = previous_ambiguity_id {
            resolve_ambiguity_record(
                conn,
                ambiguity_id,
                &json!({
                    "status": "resolved",
                    "graph_edge_id": id,
                    "edge_status": edge_status,
                    "reason": "semantic edge was reclassified during ingest",
                }),
            )?;
        }
        previous_ambiguity_id
    };

    if existing_by_id.is_some() {
        let changed = conn.execute(
            "UPDATE graph_semantic_edges
             SET source_ref = ?1, target_ref = ?2, relation = ?3, confidence = ?4,
                 status = ?5, evidence = ?6, rationale = ?7, source_spans = ?8,
                 tags = ?9, generated_by = ?10, source = ?11, user_confirmed_at = ?12,
                 valid_until = ?13, ambiguity_id = ?14, updated_at = CURRENT_TIMESTAMP,
                 version = version + 1
             WHERE id = ?15
               AND NOT (source_ref = ?1 AND target_ref = ?2 AND relation = ?3
                        AND confidence = ?4 AND status = ?5 AND evidence = ?6
                        AND COALESCE(rationale, '') = COALESCE(?7, '')
                        AND source_spans = ?8 AND tags = ?9
                        AND generated_by = ?10 AND source = ?11
                        AND COALESCE(user_confirmed_at, '') = COALESCE(?12, '')
                        AND COALESCE(valid_until, '') = COALESCE(?13, '')
                        AND COALESCE(ambiguity_id, -1) = COALESCE(?14, -1))",
            params![
                source_ref,
                target_ref,
                input.relation,
                input.confidence,
                edge_status,
                evidence,
                rationale,
                source_spans.to_string(),
                tags.to_string(),
                generated_by_for_source(&options.source),
                options.source,
                user_confirmed_at,
                valid_until,
                ambiguity_id,
                id,
            ],
        )?;
        if changed > 0 {
            log_semantic_edge_revision(conn, &id, "update")?;
        }
        return Ok(GraphIngestResult {
            index,
            status: if changed == 0 { "unchanged" } else { "updated" }.to_string(),
            id: Some(id),
            reason: logical_conflict
                .as_ref()
                .map(|_| "logical_edge_conflict_pending_review".to_string()),
            source: Some(source_ref),
            target: Some(target_ref),
            relation: Some(input.relation.clone()),
            confidence: Some(input.confidence.clone()),
            edge_status: Some(edge_status.to_string()),
        });
    }

    conn.execute(
        "INSERT INTO graph_semantic_edges
         (id, source_ref, target_ref, relation, confidence, status, evidence,
          rationale, source_spans, tags, generated_by, source, user_confirmed_at,
          valid_until, ambiguity_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            id,
            source_ref,
            target_ref,
            input.relation,
            input.confidence,
            edge_status,
            evidence,
            rationale,
            source_spans.to_string(),
            tags.to_string(),
            generated_by_for_source(&options.source),
            options.source,
            user_confirmed_at,
            valid_until,
            ambiguity_id,
        ],
    )?;
    log_semantic_edge_revision(conn, &id, "ingest")?;
    Ok(GraphIngestResult {
        index,
        status: "inserted".to_string(),
        id: Some(id),
        reason: logical_conflict
            .as_ref()
            .map(|_| "logical_edge_conflict_pending_review".to_string()),
        source: Some(source_ref),
        target: Some(target_ref),
        relation: Some(input.relation.clone()),
        confidence: Some(input.confidence.clone()),
        edge_status: Some(edge_status.to_string()),
    })
}

fn normalize_endpoint_for_ingest(conn: &Connection, reference: &str) -> Result<Option<String>> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Ok(None);
    }
    if reference.starts_with("concept:") {
        return validate_concept_node_id(reference)
            .map(Some)
            .map_err(|err| anyhow::anyhow!(err));
    }
    if node_by_id(conn, reference)?.is_some() {
        return Ok(Some(reference.to_string()));
    }
    if reference.starts_with("artifacts/") {
        let artifact_id = artifact_node_id(reference);
        return Ok(node_by_id(conn, &artifact_id)?.map(|_| artifact_id));
    }
    if let Some(memory_ref) = reference.strip_prefix("memory:") {
        if let Some(memory) = memory_by_id(conn, memory_ref)? {
            return Ok(Some(memory_node_id(&memory.id)));
        }
        if let Some(memory) = memory_by_name(conn, memory_ref)? {
            return Ok(Some(memory_node_id(&memory.id)));
        }
        return Ok(None);
    }
    if let Some(memory) = memory_by_id(conn, reference)? {
        return Ok(Some(memory_node_id(&memory.id)));
    }
    if let Some(memory) = memory_by_name(conn, reference)? {
        return Ok(Some(memory_node_id(&memory.id)));
    }
    Ok(None)
}

fn validate_concept_node_id(reference: &str) -> std::result::Result<String, String> {
    let Some(label) = reference.strip_prefix("concept:") else {
        return Err("concept endpoint must start with concept:".to_string());
    };
    if label.is_empty() {
        return Err("concept endpoint requires a label".to_string());
    }
    if label.len() > 128 {
        return Err("concept endpoint cannot exceed 128 bytes".to_string());
    }
    if label
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | ':'))
    {
        Ok(reference.to_string())
    } else {
        Err(format!(
            "unsafe concept endpoint {reference}; use lowercase snake_case"
        ))
    }
}

fn validate_semantic_edge_id(id: &str) -> std::result::Result<String, String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err("semantic edge id cannot be empty".to_string());
    }
    if trimmed.len() > 256 {
        return Err("semantic edge id cannot exceed 256 bytes".to_string());
    }
    if trimmed
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(format!(
            "semantic edge id contains whitespace/control chars: {id}"
        ));
    }
    Ok(trimmed.to_string())
}

fn normalized_json_array(value: &Value) -> std::result::Result<Value, String> {
    if value.is_null() {
        return Ok(json!([]));
    }
    if !value.is_array() {
        return Err("source_spans must be an array".to_string());
    }
    Ok(strip_json_secrets(value))
}

fn normalized_string_array(value: &Value) -> std::result::Result<Value, String> {
    if value.is_null() {
        return Ok(json!([]));
    }
    let Some(array) = value.as_array() else {
        return Err("tags must be an array".to_string());
    };
    let mut tags = Vec::with_capacity(array.len());
    for item in array {
        let Some(tag) = item.as_str() else {
            return Err("tags must be an array of strings".to_string());
        };
        tags.push(strip_secrets(tag).map_err(|err| err.to_string())?);
    }
    Ok(json!(tags))
}

fn sanitize_json_secrets(value: &Value, field: &str, allow_redaction: bool) -> Result<Value> {
    let redacted = strip_json_secrets(value);
    if redacted != *value && !allow_redaction {
        bail!(
            "secret-like value detected in {field}; merge rejected. \
             Remove the secret or pass --redact-secrets explicitly"
        );
    }
    Ok(redacted)
}

fn strip_json_secrets(value: &Value) -> Value {
    match value {
        Value::String(value) => strip_secrets(value)
            .map(Value::String)
            .unwrap_or_else(|_| Value::String("[REDACTED]".to_string())),
        Value::Array(values) => Value::Array(values.iter().map(strip_json_secrets).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), strip_json_secrets(value)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn semantic_edge_by_id(conn: &Connection, id: &str) -> Result<Option<ExistingSemanticEdge>> {
    conn.query_row(
        "SELECT id, source, status, ambiguity_id, user_confirmed_at
         FROM graph_semantic_edges WHERE id = ?1",
        params![id],
        |row| {
            Ok(ExistingSemanticEdge {
                id: row.get(0)?,
                source: row.get(1)?,
                status: row.get(2)?,
                ambiguity_id: row.get(3)?,
                user_confirmed_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn semantic_edge_by_logical_key(
    conn: &Connection,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
) -> Result<Option<ExistingSemanticEdge>> {
    conn.query_row(
        "SELECT id, source, status, ambiguity_id, user_confirmed_at
         FROM graph_semantic_edges
         WHERE source_ref = ?1 AND target_ref = ?2 AND relation = ?3
           AND status IN ('active', 'pending')
         ORDER BY updated_at DESC, id
         LIMIT 1",
        params![source_ref, target_ref, relation],
        |row| {
            Ok(ExistingSemanticEdge {
                id: row.get(0)?,
                source: row.get(1)?,
                status: row.get(2)?,
                ambiguity_id: row.get(3)?,
                user_confirmed_at: row.get(4)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn identical_semantic_edge_id(
    conn: &Connection,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
    confidence: &str,
    status: &str,
    evidence: &str,
    valid_until: Option<&str>,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM graph_semantic_edges
         WHERE source_ref = ?1 AND target_ref = ?2 AND relation = ?3
           AND confidence = ?4 AND status = ?5 AND evidence = ?6
           AND COALESCE(valid_until, '') = COALESCE(?7, '')
         ORDER BY id LIMIT 1",
        params![
            source_ref,
            target_ref,
            relation,
            confidence,
            status,
            evidence,
            valid_until
        ],
        |row| row.get(0),
    )
    .optional()
    .map_err(Into::into)
}

fn log_semantic_edge_revision(conn: &Connection, edge_id: &str, action: &str) -> Result<()> {
    let uid = new_event_uid(conn, "semantic-revision")?;
    conn.execute(
        "INSERT INTO graph_semantic_edge_revisions
         (uid, edge_id, version, action, snapshot, source)
         SELECT ?3, id, version, ?2,
                json_object(
                    'source_ref', source_ref,
                    'target_ref', target_ref,
                    'relation', relation,
                    'confidence', confidence,
                    'status', status,
                    'evidence', evidence,
                    'rationale', rationale,
                    'source_spans', json(source_spans),
                    'tags', json(tags),
                    'generated_by', generated_by,
                    'source', source,
                    'valid_until', valid_until,
                    'ambiguity_id', ambiguity_id
                ),
                source
         FROM graph_semantic_edges
         WHERE id = ?1",
        params![edge_id, action, uid],
    )?;
    Ok(())
}

fn generated_by_for_source(source: &str) -> &'static str {
    if source == "manual" {
        "manual"
    } else {
        "agent"
    }
}

fn ensure_pending_edge_ambiguity(
    conn: &Connection,
    input: &SemanticEdgeInput,
    source_ref: &str,
    target_ref: &str,
    existing_ambiguity_id: Option<i64>,
    conflicts_with: Option<&str>,
    cross_scope: bool,
) -> Result<i64> {
    if let Some(id) = existing_ambiguity_id {
        return Ok(id);
    }
    let memory_ids = [source_ref, target_ref]
        .iter()
        .filter_map(|reference| reference.strip_prefix("memory:"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    let context = json!({
        "source": source_ref,
        "target": target_ref,
        "relation": input.relation,
        "confidence": input.confidence,
        "evidence": input.evidence,
        "conflicts_with": conflicts_with,
        "cross_scope": cross_scope,
    })
    .to_string();
    add_ambiguity_record(
        conn,
        &format!("graph:{}", input.relation),
        &memory_ids,
        Some(&context),
    )
}

fn semantic_edge_crosses_project_scopes(
    conn: &Connection,
    source_ref: &str,
    target_ref: &str,
) -> Result<bool> {
    let source_scope = semantic_endpoint_scope(conn, source_ref)?;
    let target_scope = semantic_endpoint_scope(conn, target_ref)?;
    Ok(matches!(
        (source_scope.as_deref(), target_scope.as_deref()),
        (Some(source), Some(target))
            if source.starts_with("project:")
                && target.starts_with("project:")
                && source != target
    ))
}

fn semantic_endpoint_scope(conn: &Connection, reference: &str) -> Result<Option<String>> {
    if let Some(memory_ref) = reference.strip_prefix("memory:") {
        return Ok(memory_by_id(conn, memory_ref)?.map(|memory| memory.scope));
    }
    Ok(node_by_id(conn, reference)?.and_then(|node| node.scope))
}

fn semantic_ref_label(conn: &Connection, reference: &str) -> Result<Option<String>> {
    if let Some(memory_id) = reference.strip_prefix("memory:") {
        return Ok(memory_by_id(conn, memory_id)?.map(|memory| memory.name));
    }
    if let Some(node) = node_by_id(conn, reference)? {
        return Ok(Some(node.label));
    }
    Ok(reference
        .strip_prefix("concept:")
        .map(|label| label.to_string()))
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', "\"\""));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        if row? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn load_semantic_edges(conn: &Connection) -> Result<Vec<GraphSemanticEdgeRow>> {
    let confirmed_expr = if column_exists(conn, "graph_semantic_edges", "user_confirmed_at")? {
        "user_confirmed_at"
    } else {
        "NULL AS user_confirmed_at"
    };
    let sql = format!(
        "SELECT id, source_ref, target_ref, relation, confidence, status, evidence,
                rationale, source_spans, tags, generated_by, source, {confirmed_expr}, created_at,
                updated_at, valid_until, ambiguity_id, version
         FROM graph_semantic_edges
         ORDER BY created_at, id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], row_to_semantic_edge)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn endpoint_references_review_memory(reference: &str, review_memory_ids: &HashSet<String>) -> bool {
    let id = reference.strip_prefix("memory:").unwrap_or(reference);
    review_memory_ids.contains(id)
}

fn remap_merge_endpoint(
    conn: &Connection,
    reference: &str,
    memory_id_map: &HashMap<String, String>,
) -> Result<(String, bool)> {
    if let Some(memory_id) = reference.strip_prefix("memory:") {
        return Ok(match memory_id_map.get(memory_id) {
            Some(local_id) => (memory_node_id(local_id), true),
            None => (reference.to_string(), false),
        });
    }
    if let Some(step_ref) = reference.strip_prefix("workflow_step:") {
        if let Some((memory_id, step_id)) = step_ref.split_once(':') {
            return Ok(match memory_id_map.get(memory_id) {
                Some(local_id) => (workflow_step_node_id(local_id, step_id), true),
                None => (reference.to_string(), false),
            });
        }
        return Ok((reference.to_string(), false));
    }
    if reference.starts_with("concept:") {
        return validate_concept_node_id(reference)
            .map(|value| (value, true))
            .map_err(anyhow::Error::msg);
    }
    let recognized = [
        "artifact:",
        "tag:",
        "scope:",
        "type:",
        "source:",
        "claim:path:",
        "claim:command:",
    ]
    .iter()
    .any(|prefix| reference.starts_with(prefix));
    if recognized || node_by_id(conn, reference)?.is_some() {
        return Ok((reference.to_string(), true));
    }
    Ok((reference.to_string(), false))
}

#[allow(clippy::too_many_arguments)]
fn semantic_edge_matches(
    conn: &Connection,
    id: &str,
    source_ref: &str,
    target_ref: &str,
    relation: &str,
    confidence: &str,
    status: &str,
    evidence: &str,
    rationale: Option<&str>,
    source_spans: &Value,
    tags: &Value,
    source: &str,
    valid_until: Option<&str>,
) -> Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM graph_semantic_edges
         WHERE id = ?1 AND source_ref = ?2 AND target_ref = ?3 AND relation = ?4
           AND confidence = ?5 AND status = ?6 AND evidence = ?7
           AND COALESCE(rationale, '') = COALESCE(?8, '')
           AND source_spans = ?9 AND tags = ?10 AND source = ?11
           AND COALESCE(valid_until, '') = COALESCE(?12, '')",
        params![
            id,
            source_ref,
            target_ref,
            relation,
            confidence,
            status,
            evidence,
            rationale,
            source_spans.to_string(),
            tags.to_string(),
            source,
            valid_until,
        ],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

#[allow(clippy::too_many_arguments)]
fn update_semantic_edge_from_merge(
    conn: &Connection,
    id: &str,
    source_ref: &str,
    target_ref: &str,
    incoming: &GraphSemanticEdgeRow,
    status: &str,
    evidence: &str,
    rationale: Option<&str>,
    source_spans: &Value,
    tags: &Value,
    ambiguity_id: Option<i64>,
) -> Result<()> {
    conn.execute(
        "UPDATE graph_semantic_edges
         SET source_ref = ?1, target_ref = ?2, relation = ?3, confidence = ?4,
             status = ?5, evidence = ?6, rationale = ?7, source_spans = ?8,
             tags = ?9, generated_by = ?10, source = ?11, user_confirmed_at = ?12,
             valid_until = ?13, ambiguity_id = ?14, updated_at = CURRENT_TIMESTAMP,
             version = version + 1
         WHERE id = ?15",
        params![
            source_ref,
            target_ref,
            incoming.relation,
            incoming.confidence,
            status,
            evidence,
            rationale,
            source_spans.to_string(),
            tags.to_string(),
            incoming.generated_by,
            incoming.source,
            incoming.user_confirmed_at,
            incoming.valid_until,
            ambiguity_id,
            id,
        ],
    )?;
    log_semantic_edge_revision(conn, id, "merge")?;
    Ok(())
}

fn unique_semantic_edge_id(conn: &Connection, preferred: &str) -> Result<String> {
    if semantic_edge_by_id(conn, preferred)?.is_none() {
        return Ok(preferred.to_string());
    }
    for suffix in 2..=10_000 {
        let candidate = format!("{preferred}_{suffix}");
        if semantic_edge_by_id(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    bail!("could not allocate semantic edge id for {preferred}")
}

fn row_to_semantic_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<GraphSemanticEdgeRow> {
    let source_spans: String = row.get(8)?;
    let tags: String = row.get(9)?;
    Ok(GraphSemanticEdgeRow {
        id: row.get(0)?,
        source_ref: row.get(1)?,
        source_label: None,
        target_ref: row.get(2)?,
        target_label: None,
        relation: row.get(3)?,
        confidence: row.get(4)?,
        status: row.get(5)?,
        evidence: row.get(6)?,
        rationale: row.get(7)?,
        source_spans: parse_json_value(&source_spans),
        tags: parse_json_value(&tags),
        generated_by: row.get(10)?,
        source: row.get(11)?,
        user_confirmed_at: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        valid_until: row.get(15)?,
        ambiguity_id: row.get(16)?,
        version: row.get(17)?,
    })
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
