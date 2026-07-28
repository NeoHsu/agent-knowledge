use std::path::PathBuf;

use clap::{Args, ValueEnum};

pub(crate) const DEFAULT_LIMIT: usize = 20;
pub(crate) const MEMORY_TYPES: &[&str] = &[
    "user",
    "feedback",
    "project",
    "reference",
    "preference",
    "workflow",
];
pub(crate) const MEMORY_SOURCES: &[&str] = &["manual", "agent", "daily_retro", "weekly_retro"];
pub(crate) const CONFIDENCE_LEVELS: &[&str] = &["high", "medium", "low"];

pub(crate) fn parse_memory_type(value: &str) -> Result<String, String> {
    parse_allowed(value, MEMORY_TYPES, "memory type")
}

pub(crate) fn parse_memory_source(value: &str) -> Result<String, String> {
    parse_allowed(value, MEMORY_SOURCES, "memory source")
}

pub(crate) fn parse_confidence(value: &str) -> Result<String, String> {
    parse_allowed(value, CONFIDENCE_LEVELS, "confidence")
}

pub(crate) fn parse_rfc3339_timestamp(value: &str) -> Result<String, String> {
    mem_core::util::normalize_rfc3339(value).map_err(|error| error.to_string())
}

pub(crate) fn parse_allowed(value: &str, allowed: &[&str], label: &str) -> Result<String, String> {
    if allowed.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "invalid {label} '{value}'; expected one of: {}",
            allowed.join(", ")
        ))
    }
}

#[derive(Args)]
pub(crate) struct SaveArgs {
    #[arg(long, default_value = "reference", value_parser = parse_memory_type)]
    pub(crate) r#type: String,
    #[arg(long)]
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) description: Option<String>,
    #[arg(long)]
    pub(crate) content: Option<String>,
    #[arg(long)]
    pub(crate) content_file: Option<PathBuf>,
    #[arg(long, default_value = "[]")]
    pub(crate) tags: String,
    #[arg(long, default_value = "global")]
    pub(crate) scope: String,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(long, value_parser = parse_confidence)]
    pub(crate) confidence: Option<String>,
    #[arg(long, value_parser = parse_rfc3339_timestamp)]
    pub(crate) expires_at: Option<String>,
    #[arg(long)]
    pub(crate) why: Option<String>,
    #[arg(long)]
    pub(crate) force: bool,
    #[arg(
        long,
        help = "Attest that source=manual reflects an explicit user confirmation"
    )]
    pub(crate) user_confirmed: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values instead of rejecting the write"
    )]
    pub(crate) redact_secrets: bool,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
    #[arg(skip)]
    pub(crate) origin: Option<String>,
    #[arg(skip)]
    pub(crate) origin_ref: Option<String>,
}

#[derive(Args)]
pub(crate) struct QueryArgs {
    #[arg(help = "Text to search; omitted lists memories with filters")]
    pub(crate) query: Option<String>,
    #[arg(long, value_parser = parse_memory_type, help = "Filter by memory type")]
    pub(crate) r#type: Option<String>,
    #[arg(long, help = "Filter by exact tag value")]
    pub(crate) tags: Option<String>,
    #[arg(long, help = "Filter scope; use auto for global plus detected project")]
    pub(crate) scope: Option<String>,
    #[arg(long, help = "Only include expired memories")]
    pub(crate) expired: bool,
    #[arg(long, help = "Include superseded or soft-deleted memories")]
    pub(crate) include_superseded: bool,
    #[arg(long, help = "Maximum rows; defaults to config or 20")]
    pub(crate) limit: Option<usize>,
    #[arg(long, value_enum, default_value_t = SortMode::Relevance, help = "Sort mode")]
    pub(crate) sort: SortMode,
    #[arg(long, help = "Use fuzzy matching across indexed fields")]
    pub(crate) fuzzy: bool,
    #[arg(
        long,
        conflicts_with = "no_touch",
        help = "Explicitly update access counters for returned memories"
    )]
    pub(crate) touch: bool,
    #[arg(
        long,
        hide = true,
        help = "Deprecated compatibility alias; reads are now no-touch by default"
    )]
    pub(crate) no_touch: bool,
    #[arg(long, help = "Explicitly repair a stale search index before querying")]
    pub(crate) repair_index: bool,
    #[arg(long, help = "Treat query as Tantivy query syntax")]
    pub(crate) raw_query: bool,
    #[arg(
        long,
        help = "Include deterministic retrieval score components in JSON output"
    )]
    pub(crate) explain_score: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json, help = "Output format")]
    pub(crate) format: OutputFormat,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum SortMode {
    Relevance,
    Time,
    AccessCount,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum OutputFormat {
    Json,
    Table,
    Compact,
}

#[derive(Args)]
pub(crate) struct UpdateArgs {
    pub(crate) name: String,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve a name; use id:<memory-id> for an explicit id"
    )]
    pub(crate) scope: String,
    #[arg(long, help = "Move the memory to a validated scope")]
    pub(crate) set_scope: Option<String>,
    #[arg(long, value_parser = parse_memory_type)]
    pub(crate) r#type: Option<String>,
    #[arg(long)]
    pub(crate) content: Option<String>,
    #[arg(long)]
    pub(crate) content_file: Option<PathBuf>,
    #[arg(long)]
    pub(crate) description: Option<String>,
    #[arg(long, conflicts_with = "description")]
    pub(crate) clear_description: bool,
    #[arg(long)]
    pub(crate) set_tags: Option<String>,
    #[arg(long)]
    pub(crate) add_tags: Option<String>,
    #[arg(long)]
    pub(crate) remove_tags: Option<String>,
    #[arg(long, value_parser = parse_memory_source)]
    pub(crate) source: Option<String>,
    #[arg(long, value_parser = parse_confidence)]
    pub(crate) confidence: Option<String>,
    #[arg(long, value_parser = parse_rfc3339_timestamp, conflicts_with = "clear_expires_at")]
    pub(crate) expires_at: Option<String>,
    #[arg(long)]
    pub(crate) clear_expires_at: bool,
    #[arg(
        long,
        help = "Attest that source=manual reflects an explicit user confirmation"
    )]
    pub(crate) user_confirmed: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values instead of rejecting the write"
    )]
    pub(crate) redact_secrets: bool,
    #[arg(long)]
    pub(crate) expected_version: Option<i64>,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
}

#[derive(Args)]
pub(crate) struct SupersedeArgs {
    pub(crate) old_name: String,
    pub(crate) new_name: String,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve the old name"
    )]
    pub(crate) scope: String,
    #[arg(long, help = "Scope for the replacement; defaults to the old scope")]
    pub(crate) new_scope: Option<String>,
    #[arg(long)]
    pub(crate) content: Option<String>,
    #[arg(long)]
    pub(crate) content_file: Option<PathBuf>,
    #[arg(long)]
    pub(crate) description: Option<String>,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(
        long,
        help = "Attest that source=manual reflects an explicit user confirmation"
    )]
    pub(crate) user_confirmed: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values instead of rejecting the write"
    )]
    pub(crate) redact_secrets: bool,
    #[arg(long)]
    pub(crate) expected_version: Option<i64>,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
}

#[derive(Args)]
pub(crate) struct DeleteArgs {
    pub(crate) name: String,
    #[arg(long, default_value = "auto", help = "Scope used to resolve a name")]
    pub(crate) scope: String,
    #[arg(long)]
    pub(crate) hard: bool,
    #[arg(long)]
    pub(crate) force: bool,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(
        long,
        help = "Attest that source=manual reflects an explicit user confirmation"
    )]
    pub(crate) user_confirmed: bool,
    #[arg(long)]
    pub(crate) expected_version: Option<i64>,
}
