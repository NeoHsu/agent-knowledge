use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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

fn parse_memory_type(value: &str) -> Result<String, String> {
    parse_allowed(value, MEMORY_TYPES, "memory type")
}

fn parse_memory_source(value: &str) -> Result<String, String> {
    parse_allowed(value, MEMORY_SOURCES, "memory source")
}

fn parse_confidence(value: &str) -> Result<String, String> {
    parse_allowed(value, CONFIDENCE_LEVELS, "confidence")
}

fn parse_allowed(value: &str, allowed: &[&str], label: &str) -> Result<String, String> {
    if allowed.contains(&value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "invalid {label} '{value}'; expected one of: {}",
            allowed.join(", ")
        ))
    }
}

#[derive(Parser)]
#[command(name = "mem", version, about = "Portable agent memory CLI")]
pub(crate) struct Cli {
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help = "Override the active knowledge store root"
    )]
    pub(crate) home: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    #[command(about = "Initialize the active memory store")]
    Init,
    #[command(about = "Save a durable memory")]
    Save(SaveArgs),
    #[command(about = "Search or list memories")]
    Query(QueryArgs),
    #[command(about = "Update an existing memory")]
    Update(UpdateArgs),
    #[command(about = "Replace a memory while keeping history")]
    Supersede(SupersedeArgs),
    #[command(about = "Soft-delete or hard-delete a memory")]
    Delete(DeleteArgs),
    #[command(about = "Rebuild the search index from SQLite")]
    Reindex,
    #[command(about = "Inspect repository context such as auto-detected scope")]
    Context(ContextArgs),
    #[command(about = "Inspect CLI configuration and effective defaults")]
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    #[command(about = "Show memory changelog entries")]
    History(HistoryArgs),
    #[command(about = "Show memory store statistics")]
    Stats(StatsArgs),
    #[command(about = "Audit memory health and optional fixes")]
    Audit(AuditArgs),
    #[command(about = "Garbage-collect old soft-deleted memories")]
    Gc(GcArgs),
    #[command(about = "Export memories as JSON or Markdown")]
    Export(ExportArgs),
    #[command(about = "Import memories from JSON or a document")]
    Import(ImportArgs),
    #[command(about = "Merge another memory database into this store")]
    Merge(MergeArgs),
    #[command(about = "Generate daily or weekly retrospective bundles")]
    Retro {
        #[command(subcommand)]
        command: RetroCommand,
    },
    #[command(about = "List, find, show, and validate workflow memories")]
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    #[command(about = "Inspect knowledge-store artifact manifest entries")]
    Artifact {
        #[command(subcommand)]
        command: ArtifactCommand,
    },
    #[command(about = "Record and resolve ambiguous or conflicting memories")]
    Ambiguity {
        #[command(subcommand)]
        command: AmbiguityCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    #[command(about = "Show active store, config paths, environment, and effective defaults")]
    Show,
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
    #[arg(long)]
    pub(crate) expires_at: Option<String>,
    #[arg(long)]
    pub(crate) why: Option<String>,
    #[arg(long)]
    pub(crate) force: bool,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
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
    #[arg(long, hide = true)]
    pub(crate) semantic: bool,
    #[arg(long, help = "Do not update access counters")]
    pub(crate) no_touch: bool,
    #[arg(long, help = "Treat query as Tantivy query syntax")]
    pub(crate) raw_query: bool,
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
    #[arg(long)]
    pub(crate) content: Option<String>,
    #[arg(long)]
    pub(crate) content_file: Option<PathBuf>,
    #[arg(long)]
    pub(crate) description: Option<String>,
    #[arg(long)]
    pub(crate) add_tags: Option<String>,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(long)]
    pub(crate) expected_version: Option<i64>,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
}

#[derive(Args)]
pub(crate) struct SupersedeArgs {
    pub(crate) old_name: String,
    pub(crate) new_name: String,
    #[arg(long)]
    pub(crate) content: Option<String>,
    #[arg(long)]
    pub(crate) content_file: Option<PathBuf>,
    #[arg(long)]
    pub(crate) description: Option<String>,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(long)]
    pub(crate) expected_version: Option<i64>,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
}

#[derive(Args)]
pub(crate) struct DeleteArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) hard: bool,
    #[arg(long)]
    pub(crate) force: bool,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(long)]
    pub(crate) expected_version: Option<i64>,
}

#[derive(Args)]
pub(crate) struct ContextArgs {
    #[arg(long)]
    pub(crate) detect: bool,
}

#[derive(Args)]
pub(crate) struct HistoryArgs {
    pub(crate) name: Option<String>,
    #[arg(long)]
    pub(crate) action: Option<String>,
    #[arg(long, default_value_t = 20)]
    pub(crate) limit: usize,
    #[arg(long, value_enum, default_value_t = OutputFormat::Json, help = "Output format")]
    pub(crate) format: OutputFormat,
}

#[derive(Args)]
pub(crate) struct StatsArgs {
    #[arg(long, value_enum, default_value_t = OutputFormat::Json, help = "Output format")]
    pub(crate) format: OutputFormat,
}

#[derive(Args)]
pub(crate) struct AuditArgs {
    #[arg(long)]
    pub(crate) fix: bool,
}

#[derive(Args)]
pub(crate) struct GcArgs {
    #[arg(long, default_value_t = 90)]
    pub(crate) days: i64,
}

#[derive(Args)]
pub(crate) struct ExportArgs {
    #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
    pub(crate) format: ExportFormat,
    #[arg(long)]
    pub(crate) include_superseded: bool,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum ExportFormat {
    Json,
    Markdown,
}

#[derive(Args)]
pub(crate) struct ImportArgs {
    pub(crate) file: PathBuf,
    #[arg(long, value_parser = parse_memory_type)]
    pub(crate) r#type: Option<String>,
    #[arg(long, default_value = "manual", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
}

#[derive(Args)]
pub(crate) struct MergeArgs {
    pub(crate) db: PathBuf,
    #[arg(long)]
    pub(crate) prefer_trusted: bool,
}

#[derive(Subcommand)]
pub(crate) enum RetroCommand {
    Daily(RetroArgs),
    Weekly(RetroArgs),
}

#[derive(Subcommand)]
pub(crate) enum WorkflowCommand {
    #[command(about = "List workflow memories")]
    List(WorkflowListArgs),
    #[command(about = "Show a workflow by name or id")]
    Show(WorkflowShowArgs),
    #[command(about = "Find workflows matching an intent")]
    Find(WorkflowFindArgs),
    #[command(about = "Validate workflow content and tags")]
    Validate(WorkflowValidateArgs),
}

#[derive(Args)]
pub(crate) struct WorkflowListArgs {
    #[arg(long, help = "Filter scope; use auto for global plus detected project")]
    pub(crate) scope: Option<String>,
    #[arg(long, help = "Maximum rows; defaults to config or 20")]
    pub(crate) limit: Option<usize>,
    #[arg(long, help = "Include superseded workflows")]
    pub(crate) include_superseded: bool,
}

#[derive(Args)]
pub(crate) struct WorkflowShowArgs {
    pub(crate) reference: String,
}

#[derive(Args)]
pub(crate) struct WorkflowFindArgs {
    #[arg(help = "Intent text such as release, deploy, or fix ci")]
    pub(crate) intent: String,
    #[arg(long, help = "Filter scope; use auto for global plus detected project")]
    pub(crate) scope: Option<String>,
    #[arg(long, help = "Maximum rows; defaults to config or 20")]
    pub(crate) limit: Option<usize>,
}

#[derive(Args)]
pub(crate) struct WorkflowValidateArgs {
    pub(crate) reference: String,
}

#[derive(Subcommand)]
pub(crate) enum ArtifactCommand {
    #[command(about = "List artifact manifest entries")]
    List,
    #[command(about = "Check artifact paths, checksums, and executable bits")]
    Check,
    #[command(about = "Show one artifact manifest entry by name")]
    Show(ArtifactShowArgs),
    #[command(about = "Add or replace artifact metadata in manifest.toml")]
    Add(ArtifactAddArgs),
    #[command(about = "Update artifact metadata")]
    Update(ArtifactUpdateArgs),
    #[command(about = "Remove artifact metadata")]
    Remove(ArtifactRemoveArgs),
}

#[derive(Args)]
pub(crate) struct ArtifactShowArgs {
    pub(crate) name: String,
}

#[derive(Args)]
pub(crate) struct ArtifactAddArgs {
    pub(crate) path: PathBuf,
    #[arg(long)]
    pub(crate) name: Option<String>,
    #[arg(long, value_enum)]
    pub(crate) kind: ArtifactKindArg,
    #[arg(long, default_value = "global")]
    pub(crate) scope: String,
    #[arg(long)]
    pub(crate) description: Option<String>,
    #[arg(long)]
    pub(crate) executable: bool,
    #[arg(long)]
    pub(crate) tags: Option<String>,
    #[arg(long)]
    pub(crate) force: bool,
}

#[derive(Args)]
pub(crate) struct ArtifactUpdateArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) checksum: bool,
}

#[derive(Args)]
pub(crate) struct ArtifactRemoveArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) delete_file: bool,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum ArtifactKindArg {
    Script,
    Template,
    Snippet,
    Reference,
}

#[derive(Args)]
pub(crate) struct RetroArgs {
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: usize,
}

#[derive(Subcommand)]
pub(crate) enum AmbiguityCommand {
    Add(AmbiguityAddArgs),
    List(AmbiguityListArgs),
    Resolve(AmbiguityResolveArgs),
}

#[derive(Args)]
pub(crate) struct AmbiguityAddArgs {
    #[arg(long)]
    pub(crate) query: String,
    #[arg(long)]
    pub(crate) memory_ids: String,
    #[arg(long)]
    pub(crate) context: Option<String>,
}

#[derive(Args)]
pub(crate) struct AmbiguityListArgs {
    #[arg(long)]
    pub(crate) pending: bool,
}

#[derive(Args)]
pub(crate) struct AmbiguityResolveArgs {
    pub(crate) id: i64,
    #[arg(long)]
    pub(crate) note: Option<String>,
    #[arg(long)]
    pub(crate) keep: Option<String>,
    #[arg(long)]
    pub(crate) soft_delete_others: bool,
}
