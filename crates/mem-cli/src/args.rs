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
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Subcommand)]
pub(crate) enum Command {
    Init,
    Save(SaveArgs),
    Query(QueryArgs),
    Update(UpdateArgs),
    Supersede(SupersedeArgs),
    Delete(DeleteArgs),
    Reindex,
    Context(ContextArgs),
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    History(HistoryArgs),
    Stats,
    Audit(AuditArgs),
    Gc(GcArgs),
    Export(ExportArgs),
    Import(ImportArgs),
    Merge(MergeArgs),
    Retro {
        #[command(subcommand)]
        command: RetroCommand,
    },
    Workflow {
        #[command(subcommand)]
        command: WorkflowCommand,
    },
    Ambiguity {
        #[command(subcommand)]
        command: AmbiguityCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
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
    pub(crate) query: Option<String>,
    #[arg(long, value_parser = parse_memory_type)]
    pub(crate) r#type: Option<String>,
    #[arg(long)]
    pub(crate) tags: Option<String>,
    #[arg(long)]
    pub(crate) scope: Option<String>,
    #[arg(long)]
    pub(crate) expired: bool,
    #[arg(long)]
    pub(crate) include_superseded: bool,
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    #[arg(long, value_enum, default_value_t = SortMode::Relevance)]
    pub(crate) sort: SortMode,
    #[arg(long)]
    pub(crate) fuzzy: bool,
    #[arg(long)]
    pub(crate) semantic: bool,
    #[arg(long)]
    pub(crate) no_touch: bool,
    #[arg(long)]
    pub(crate) raw_query: bool,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum SortMode {
    Relevance,
    Time,
    AccessCount,
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
    List(WorkflowListArgs),
    Show(WorkflowShowArgs),
    Find(WorkflowFindArgs),
    Validate(WorkflowValidateArgs),
}

#[derive(Args)]
pub(crate) struct WorkflowListArgs {
    #[arg(long)]
    pub(crate) scope: Option<String>,
    #[arg(long)]
    pub(crate) limit: Option<usize>,
    #[arg(long)]
    pub(crate) include_superseded: bool,
}

#[derive(Args)]
pub(crate) struct WorkflowShowArgs {
    pub(crate) reference: String,
}

#[derive(Args)]
pub(crate) struct WorkflowFindArgs {
    pub(crate) intent: String,
    #[arg(long)]
    pub(crate) scope: Option<String>,
    #[arg(long)]
    pub(crate) limit: Option<usize>,
}

#[derive(Args)]
pub(crate) struct WorkflowValidateArgs {
    pub(crate) reference: String,
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
