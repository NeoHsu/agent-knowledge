//! Semantic-edge ingest validation and trust handling.

use super::*;
use crate::error;

pub fn ingest_semantic_edges(
    conn: &Connection,
    payload: Value,
    options: GraphIngestOptions,
) -> Result<GraphIngestReport> {
    let payload: SemanticEdgePayload = serde_json::from_value(payload)
        .map_err(|source| error::usage(format!("parse semantic edge payload: {source}")))?;
    if payload.schema_version != GRAPH_SCHEMA_VERSION {
        return Err(error::compatibility(format!(
            "unsupported semantic edge schema_version {}; expected {}",
            payload.schema_version, GRAPH_SCHEMA_VERSION
        )));
    }
    if payload.edges.len() > 1_000 {
        return Err(error::usage(
            "semantic edge payload cannot exceed 1000 edges",
        ));
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
    let source_spans = normalized_json_array(&source_spans).map_err(error::usage)?;
    let tags = sanitize_json_secrets(
        &input.tags,
        "semantic edge tags",
        options.allow_secret_redaction,
    )?;
    let tags = normalized_string_array(&tags).map_err(error::usage)?;
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
        Some(id) => validate_semantic_edge_id(id).map_err(error::usage)?,
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
            .map_err(error::usage);
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

pub(super) fn validate_concept_node_id(reference: &str) -> std::result::Result<String, String> {
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
