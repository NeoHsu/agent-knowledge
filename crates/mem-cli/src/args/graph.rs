use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use super::memory::{parse_memory_source, parse_memory_type, parse_rfc3339_timestamp};

fn parse_bounded_usize(value: &str, min: usize, max: usize, label: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{label} must be an integer between {min} and {max}"))?;
    if !(min..=max).contains(&parsed) {
        return Err(format!("{label} must be between {min} and {max}"));
    }
    Ok(parsed)
}

fn parse_graph_explain_depth(value: &str) -> Result<usize, String> {
    parse_bounded_usize(value, 0, 1, "graph explain depth")
}

fn parse_graph_path_depth(value: &str) -> Result<usize, String> {
    parse_bounded_usize(value, 1, 20, "graph path max depth")
}

fn parse_graph_query_depth(value: &str) -> Result<usize, String> {
    parse_bounded_usize(value, 0, 8, "graph query depth")
}

fn parse_graph_limit(value: &str) -> Result<usize, String> {
    parse_bounded_usize(value, 1, 500, "graph limit")
}

#[derive(Subcommand)]
pub(crate) enum GraphCommand {
    #[command(about = "Rebuild the materialized graph index from SQLite memories and manifests")]
    Rebuild,
    #[command(about = "Show graph counts and dirty state without rebuilding")]
    Stats,
    #[command(about = "Show one graph node and its active neighbors")]
    Explain(GraphExplainArgs),
    #[command(about = "Find the shortest relationship path between two graph nodes")]
    Path(GraphPathArgs),
    #[command(about = "Query and expand a graph neighborhood for task context")]
    Query(GraphQueryArgs),
    #[command(about = "Export the materialized graph")]
    Export(GraphExportArgs),
    #[command(about = "Emit memories for skill-mediated semantic edge extraction")]
    Candidates(GraphCandidatesArgs),
    #[command(about = "Validate and store skill-generated semantic graph edges")]
    Ingest(GraphIngestArgs),
    #[command(about = "Review pending or ambiguous semantic graph edges")]
    Review(GraphReviewArgs),
    #[command(about = "Mark a semantic graph edge active")]
    Accept(GraphAcceptArgs),
    #[command(about = "Reject a semantic graph edge")]
    Reject(GraphRejectArgs),
}

#[derive(Args)]
pub(crate) struct GraphExplainArgs {
    pub(crate) reference: String,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope; auto = global plus detected project"
    )]
    pub(crate) scope: String,
    #[arg(
        long,
        default_value_t = 1,
        value_parser = parse_graph_explain_depth,
        help = "Neighborhood depth; supported values are 0 and 1"
    )]
    pub(crate) depth: usize,
}

#[derive(Args)]
pub(crate) struct GraphPathArgs {
    pub(crate) from: String,
    pub(crate) to: String,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope; auto = global plus detected project"
    )]
    pub(crate) scope: String,
    #[arg(long, default_value_t = 5, value_parser = parse_graph_path_depth)]
    pub(crate) max_depth: usize,
    #[arg(long, help = "Include pending AMBIGUOUS edges in traversal")]
    pub(crate) include_ambiguous: bool,
    #[arg(
        long,
        help = "Allow type, scope, and source metadata edges as path bridges"
    )]
    pub(crate) include_metadata: bool,
    #[arg(long, value_enum, default_value_t = GraphConfidenceArg::All)]
    pub(crate) confidence: GraphConfidenceArg,
    #[arg(long, value_enum, default_value_t = GraphDirectionArg::Any)]
    pub(crate) direction: GraphDirectionArg,
    #[arg(long, value_enum, default_value_t = GraphPathFormat::Json)]
    pub(crate) format: GraphPathFormat,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum GraphConfidenceArg {
    Extracted,
    Inferred,
    All,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum GraphDirectionArg {
    Any,
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(crate) enum GraphPathFormat {
    Json,
    Compact,
}

#[derive(Args)]
pub(crate) struct GraphQueryArgs {
    pub(crate) query: String,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope; auto = global plus detected project"
    )]
    pub(crate) scope: String,
    #[arg(long, default_value_t = 2, value_parser = parse_graph_query_depth)]
    pub(crate) depth: usize,
    #[arg(long, default_value_t = 20, value_parser = parse_graph_limit)]
    pub(crate) limit: usize,
    #[arg(long, help = "Include pending AMBIGUOUS edges in expansion")]
    pub(crate) include_ambiguous: bool,
    #[arg(long, help = "Traverse type, scope, and source metadata hubs")]
    pub(crate) include_metadata: bool,
    #[arg(long, value_enum, default_value_t = GraphConfidenceArg::All)]
    pub(crate) confidence: GraphConfidenceArg,
    #[arg(long, value_enum, default_value_t = GraphDirectionArg::Any)]
    pub(crate) direction: GraphDirectionArg,
    #[arg(long, value_enum, default_value_t = GraphPathFormat::Json)]
    pub(crate) format: GraphPathFormat,
}

#[derive(Args)]
pub(crate) struct GraphExportArgs {
    #[arg(long, value_enum, default_value_t = GraphExportFormat::Json)]
    pub(crate) format: GraphExportFormat,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum GraphExportFormat {
    Json,
}

#[derive(Args)]
pub(crate) struct GraphCandidatesArgs {
    #[arg(
        long,
        default_value = "auto",
        help = "Scope; auto = global plus detected project"
    )]
    pub(crate) scope: String,
    #[arg(long, value_parser = parse_memory_type, help = "Only include one memory type")]
    pub(crate) r#type: Option<String>,
    #[arg(
        long,
        value_parser = parse_rfc3339_timestamp,
        help = "Only include memories updated at or after this RFC3339 timestamp"
    )]
    pub(crate) changed_since: Option<String>,
    #[arg(
        long,
        help = "Only include memories without useful graph relationships"
    )]
    pub(crate) unlinked: bool,
    #[arg(long, default_value_t = 50, value_parser = parse_graph_limit)]
    pub(crate) limit: usize,
}

#[derive(Args)]
pub(crate) struct GraphIngestArgs {
    pub(crate) file: PathBuf,
    #[arg(long, help = "Store INFERRED edges as pending instead of active")]
    pub(crate) pending_inferred: bool,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(
        long,
        help = "Attest that source=manual reflects an explicit user confirmation"
    )]
    pub(crate) user_confirmed: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values instead of rejecting ingest"
    )]
    pub(crate) redact_secrets: bool,
}

#[derive(Args)]
pub(crate) struct GraphReviewArgs {
    #[arg(long, help = "Only show pending semantic edges")]
    pub(crate) pending: bool,
    #[arg(long, help = "Only show AMBIGUOUS semantic edges")]
    pub(crate) ambiguous: bool,
}

#[derive(Args)]
pub(crate) struct GraphAcceptArgs {
    pub(crate) edge_id: String,
}

#[derive(Args)]
pub(crate) struct GraphRejectArgs {
    pub(crate) edge_id: String,
    #[arg(long, help = "Reason stored in the edge rationale")]
    pub(crate) note: Option<String>,
    #[arg(long, help = "Explicitly redact secret-like values from the reason")]
    pub(crate) redact_secrets: bool,
}
