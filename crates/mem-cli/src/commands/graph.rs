use anyhow::bail;
use mem_core::graph::{
    self, GraphConfidenceFilter, GraphDirection, GraphIngestOptions, GraphPathOptions,
    GraphQueryOptions,
};

use super::*;

pub(crate) fn cmd_graph(app: &App, command: GraphCommand) -> Result<()> {
    app.require_schema()?;
    let strict_read = matches!(&command, GraphCommand::Stats | GraphCommand::Review(_))
        || matches!(&command, GraphCommand::Candidates(args) if !args.unlinked);
    let conn = if strict_read {
        app.read_conn()?
    } else {
        app.conn()?
    };
    match command {
        GraphCommand::Rebuild => {
            let report = graph::rebuild(&conn, &app.root)?;
            print_write_json_pretty(app, serde_json::to_value(report)?)?;
        }
        GraphCommand::Stats => {
            let report = graph::stats(&conn)?;
            print_json_pretty(&report)?;
        }
        GraphCommand::Explain(args) => {
            graph::ensure_fresh(&conn, &app.root)?;
            let scope_filter = graph_scope_filter(&args.scope)?;
            let report =
                graph::explain(&conn, &args.reference, args.depth, scope_filter.as_deref())?;
            print_json_pretty(&report)?;
        }
        GraphCommand::Path(args) => {
            graph::ensure_fresh(&conn, &app.root)?;
            let scope_filter = graph_scope_filter(&args.scope)?;
            let report = graph::shortest_path(
                &conn,
                &args.from,
                &args.to,
                GraphPathOptions {
                    max_depth: args.max_depth,
                    include_ambiguous: args.include_ambiguous,
                    include_metadata: args.include_metadata,
                    confidence: graph_confidence(args.confidence),
                    direction: graph_direction(args.direction),
                    scope_filter,
                },
            )?;
            match args.format {
                GraphPathFormat::Json => print_json_pretty(&report)?,
                GraphPathFormat::Compact => print_text(render_path_compact(&report))?,
            }
        }
        GraphCommand::Query(args) => {
            graph::ensure_fresh(&conn, &app.root)?;
            let scope_filter = graph_scope_filter(&args.scope)?;
            let borrowed = scope_filter
                .as_ref()
                .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
            memory_index::repair_stale(app)?;
            let memory_hits = memory_index::search_hits(
                app,
                &args.query,
                false,
                false,
                (args.limit * 3).max(DEFAULT_LIMIT),
                memory_index::SearchFilters {
                    scopes: borrowed.as_deref(),
                    ..Default::default()
                },
                true,
            )?;
            let scored_memory_ids = memory_hits
                .into_iter()
                .map(|hit| (hit.id, hit.score))
                .collect::<Vec<_>>();
            let start_nodes = graph::resolve_query_start_nodes(
                &conn,
                &args.query,
                &scored_memory_ids,
                scope_filter.as_deref(),
                args.limit,
            )?;
            let report = graph::query_neighborhood(
                &conn,
                &args.query,
                &start_nodes,
                GraphQueryOptions {
                    depth: args.depth,
                    limit: args.limit,
                    include_ambiguous: args.include_ambiguous,
                    include_metadata: args.include_metadata,
                    confidence: graph_confidence(args.confidence),
                    direction: graph_direction(args.direction),
                    scope_filter,
                },
            )?;
            match args.format {
                GraphPathFormat::Json => print_json_pretty(&report)?,
                GraphPathFormat::Compact => print_text(render_query_compact(&report))?,
            }
        }
        GraphCommand::Export(args) => {
            graph::ensure_fresh(&conn, &app.root)?;
            match args.format {
                GraphExportFormat::Json => {
                    let export = graph::export_json(&conn)?;
                    print_json_pretty(&export)?;
                }
            }
        }
        GraphCommand::Candidates(args) => {
            if args.unlinked {
                graph::ensure_fresh(&conn, &app.root)?;
            }
            let scope_filter = graph_scope_filter(&args.scope)?;
            let borrowed = scope_filter
                .as_ref()
                .map(|scopes| scopes.iter().map(String::as_str).collect::<Vec<_>>());
            let candidates = graph::candidates(
                &conn,
                borrowed.as_deref(),
                args.r#type.as_deref(),
                args.changed_since.as_deref(),
                args.unlinked,
                args.limit,
            )?;
            print_json_pretty(&candidates)?;
        }
        GraphCommand::Ingest(args) => {
            graph::ensure_fresh(&conn, &app.root)?;
            let payload_bytes = fs::metadata(&args.file)
                .with_context(|| format!("inspect {}", args.file.display()))?
                .len();
            if payload_bytes > 134_217_728 {
                bail!("semantic edge payload exceeds 134217728 bytes");
            }
            let content = fs::read_to_string(&args.file)
                .with_context(|| format!("read {}", args.file.display()))?;
            let payload: Value = serde_json::from_str(&content)
                .with_context(|| format!("parse {}", args.file.display()))?;
            let report = graph::ingest_semantic_edges(
                &conn,
                payload,
                GraphIngestOptions {
                    pending_inferred: args.pending_inferred,
                    source: args.source,
                    user_confirmed: args.user_confirmed,
                    allow_secret_redaction: args.redact_secrets,
                },
            )?;
            graph::ensure_fresh(&conn, &app.root)?;
            print_write_json_pretty(app, serde_json::to_value(report)?)?;
        }
        GraphCommand::Review(args) => {
            let report = graph::review_semantic_edges(&conn, args.pending, args.ambiguous)?;
            print_json_pretty(&report)?;
        }
        GraphCommand::Accept(args) => {
            let report =
                graph::set_semantic_edge_status(&conn, &args.edge_id, "active", None, false)?;
            graph::ensure_fresh(&conn, &app.root)?;
            print_write_json(app, serde_json::to_value(report)?)?;
        }
        GraphCommand::Reject(args) => {
            let report = graph::set_semantic_edge_status(
                &conn,
                &args.edge_id,
                "rejected",
                args.note.as_deref(),
                args.redact_secrets,
            )?;
            graph::ensure_fresh(&conn, &app.root)?;
            print_write_json(app, serde_json::to_value(report)?)?;
        }
    }
    Ok(())
}

fn graph_confidence(value: GraphConfidenceArg) -> GraphConfidenceFilter {
    match value {
        GraphConfidenceArg::Extracted => GraphConfidenceFilter::Extracted,
        GraphConfidenceArg::Inferred => GraphConfidenceFilter::Inferred,
        GraphConfidenceArg::All => GraphConfidenceFilter::All,
    }
}

fn graph_direction(value: GraphDirectionArg) -> GraphDirection {
    match value {
        GraphDirectionArg::Any => GraphDirection::Any,
        GraphDirectionArg::Outgoing => GraphDirection::Outgoing,
        GraphDirectionArg::Incoming => GraphDirection::Incoming,
    }
}

fn graph_scope_filter(scope_value: &str) -> Result<Option<Vec<String>>> {
    match scope_value {
        "auto" => Ok(Some(scope::detect_scope_set()?)),
        "all" => Ok(None),
        "" => bail!("scope cannot be empty"),
        value => {
            scope::validate_scope(value)?;
            Ok(Some(vec!["global".to_string(), value.to_string()]))
        }
    }
}

fn render_query_compact(report: &graph::GraphQueryReport) -> String {
    if report.status != "ok" {
        return format!("no graph context found for {}\n", report.query);
    }
    let mut output = format!("Graph context for {}:\n", report.query);
    for node in &report.nodes {
        output.push_str(&format!(
            "- {} [{}] score={:.3}\n",
            node.node.id, node.node.kind, node.score
        ));
    }
    if !report.edges.is_empty() {
        output.push_str("edges:\n");
        for edge in &report.edges {
            output.push_str(&format!(
                "- {} --{} [{}]--> {}\n",
                edge.source, edge.relation, edge.confidence, edge.target
            ));
        }
    }
    output
}

fn render_path_compact(report: &graph::GraphPathReport) -> String {
    if report.status != "ok" {
        return format!("no path found: {} -> {}\n", report.from.id, report.to.id);
    }
    if report.hops == 0 {
        return format!("{}\n", report.from.id);
    }
    let mut output = format!(
        "Minimum-hop path, {} hops, score {:.6}:\n",
        report.hops, report.path_score
    );
    for (index, node) in report.nodes.iter().enumerate() {
        if index == 0 {
            output.push_str(&format!("  {}\n", node.id));
            continue;
        }
        if let Some(edge) = report.edges.get(index - 1) {
            let arrow = if edge.source == edge.traversed_from && edge.target == edge.traversed_to {
                "-->"
            } else {
                "<--"
            };
            output.push_str(&format!(
                "    --{} [{}] {}\n  {}\n",
                edge.relation, edge.confidence, arrow, node.id
            ));
        }
    }
    output
}
