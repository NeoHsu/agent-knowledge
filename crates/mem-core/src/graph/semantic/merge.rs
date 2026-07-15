//! Idempotent semantic-edge merge and conflict handling.

use super::*;

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
