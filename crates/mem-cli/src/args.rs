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

fn parse_rfc3339_timestamp(value: &str) -> Result<String, String> {
    mem_core::util::normalize_rfc3339(value).map_err(|error| error.to_string())
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
    #[command(about = "Initialize a new active memory store")]
    Init,
    #[command(about = "Explicitly migrate an existing store after creating a backup")]
    Migrate(MigrateArgs),
    #[command(about = "Save a durable memory")]
    Save(SaveArgs),
    #[command(about = "Search or list memories")]
    Query(QueryArgs),
    #[command(
        about = "Emit a compact session-priming context block for agents (runtime store only)"
    )]
    Prime(PrimeArgs),
    #[command(about = "Check mnemark wiring: store, index, platform policy, skill, hooks")]
    Doctor(DoctorArgs),
    #[command(about = "Commit, merge, and push the runtime store through its git repository")]
    Sync(SyncArgs),
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
    #[command(about = "Install mnemark setup helpers such as coding-agent memory policy")]
    Setup {
        #[command(subcommand)]
        command: SetupCommand,
    },
    #[command(about = "Show memory changelog entries")]
    History(HistoryArgs),
    #[command(about = "Show memory store statistics")]
    Stats(StatsArgs),
    #[command(about = "Audit memory health and optional fixes")]
    Audit(AuditArgs),
    #[command(
        about = "Verify path and command claims in memories against the filesystem (read-only)"
    )]
    Reconcile(ReconcileArgs),
    #[command(about = "Garbage-collect old soft-deleted memories")]
    Gc(GcArgs),
    #[command(about = "Export memories as JSON or Markdown")]
    Export(ExportArgs),
    #[command(about = "Import memories from JSON or a document")]
    Import(ImportArgs),
    #[command(about = "Merge another memory database into this store")]
    Merge(MergeArgs),
    #[command(about = "Export, inspect, or import portable store bundles")]
    Bundle {
        #[command(subcommand)]
        command: BundleCommand,
    },
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
    #[command(about = "Build, inspect, traverse, and export the memory graph")]
    Graph {
        #[command(subcommand)]
        command: GraphCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    #[command(about = "Show active store, config paths, environment, and effective defaults")]
    Show,
}

#[derive(Subcommand)]
pub(crate) enum SetupCommand {
    #[command(about = "Prepend the mnemark memory policy to CLAUDE.md or AGENTS.md")]
    AgentPolicy(SetupAgentPolicyArgs),
    #[command(about = "List supported coding-agent platforms and their default wiring paths")]
    List,
    #[command(about = "Wire mnemark into Claude Code: policy, skill files, SessionStart hook")]
    ClaudeCode(SetupPlatformArgs),
    #[command(about = "Wire mnemark into OpenAI Codex CLI: policy and shared skill link")]
    Codex(SetupPlatformArgs),
    #[command(about = "Wire mnemark into pi: policy and shared Agent Skill")]
    Pi(SetupPlatformArgs),
    #[command(about = "Wire mnemark into Gemini CLI: policy block")]
    GeminiCli(SetupPlatformArgs),
    #[command(about = "Wire mnemark into opencode: policy block")]
    Opencode(SetupPlatformArgs),
}

#[derive(Args)]
pub(crate) struct SetupPlatformArgs {
    #[arg(
        long,
        value_name = "DIR",
        help = "Base directory treated as the user home; defaults to ~"
    )]
    pub(crate) base_dir: Option<PathBuf>,
    #[arg(
        long,
        value_name = "FILE",
        help = "Override the instructions file path"
    )]
    pub(crate) instructions: Option<PathBuf>,
    #[arg(
        long,
        value_name = "DIR",
        help = "Override the platform skills parent directory used for the mnemark link"
    )]
    pub(crate) skills_dir: Option<PathBuf>,
    #[arg(
        long,
        value_name = "DIR",
        help = "Override the shared Agent Skills parent; defaults to <base>/.agents/skills"
    )]
    pub(crate) shared_skills_dir: Option<PathBuf>,
    #[arg(long, help = "Skip installing bundled skill files")]
    pub(crate) no_skill: bool,
    #[arg(long, help = "Skip session-start hook wiring")]
    pub(crate) no_hook: bool,
    #[arg(long, help = "Report planned changes without writing")]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct MigrateArgs {
    #[arg(long, help = "Report the migration plan without writing")]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct PrimeArgs {
    #[arg(
        long,
        default_value = "auto",
        help = "Scope; auto = global plus detected project"
    )]
    pub(crate) scope: String,
    #[arg(
        long,
        default_value_t = 4000,
        help = "Approximate output budget in characters"
    )]
    pub(crate) budget: usize,
    #[arg(long, default_value_t = 8, help = "Maximum entries per section")]
    pub(crate) per_section: usize,
    #[arg(long, help = "Add graph context for a task focus")]
    pub(crate) focus: Option<String>,
    #[arg(
        long,
        value_enum,
        default_value_t = PrimeFormat::Text,
        help = "Output format"
    )]
    pub(crate) format: PrimeFormat,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum PrimeFormat {
    Text,
    Json,
}

#[derive(Args)]
pub(crate) struct DoctorArgs {
    #[arg(
        long,
        help = "Only check one platform: claude-code, codex, gemini-cli, or opencode"
    )]
    pub(crate) platform: Option<String>,
    #[arg(
        long,
        value_name = "DIR",
        help = "Base directory treated as the user home; defaults to ~"
    )]
    pub(crate) base_dir: Option<PathBuf>,
}

#[derive(Args)]
pub(crate) struct SyncArgs {
    #[arg(long, default_value = "origin", help = "Git remote name")]
    pub(crate) remote: String,
    #[arg(long, help = "Commit message; defaults to a timestamped message")]
    pub(crate) message: Option<String>,
    #[arg(long, help = "Explicitly push after committing and merging")]
    pub(crate) push: bool,
    #[arg(long, hide = true, conflicts_with = "push")]
    pub(crate) no_push: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secrets during database conflict merge"
    )]
    pub(crate) redact_secrets: bool,
    #[arg(long, help = "Report pending changes and divergence without writing")]
    pub(crate) dry_run: bool,
}

#[derive(Args)]
pub(crate) struct SetupAgentPolicyArgs {
    #[arg(
        long,
        value_name = "FILE",
        help = "Entry file to update; defaults to CLAUDE.md if present, otherwise AGENTS.md"
    )]
    pub(crate) target: Option<PathBuf>,
    #[arg(long, help = "Print the selected target and policy without writing")]
    pub(crate) dry_run: bool,
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
    #[arg(long, hide = true)]
    pub(crate) semantic: bool,
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

#[derive(Args)]
pub(crate) struct ContextArgs {
    #[arg(long)]
    pub(crate) detect: bool,
}

#[derive(Args)]
pub(crate) struct HistoryArgs {
    pub(crate) name: Option<String>,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve a memory name"
    )]
    pub(crate) scope: String,
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
pub(crate) struct ReconcileArgs {
    #[arg(
        long,
        default_value = "auto",
        help = "Scope to check; auto = global plus detected project, otherwise exactly the named scope"
    )]
    pub(crate) scope: String,
    #[arg(
        long,
        value_name = "DIR",
        help = "Repository root for resolving relative path claims; defaults to the current directory"
    )]
    pub(crate) repo: Option<PathBuf>,
    #[arg(long, value_parser = parse_memory_type, help = "Only check one memory type; workflow memories are skipped otherwise")]
    pub(crate) r#type: Option<String>,
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
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(
        long,
        help = "Attest that source=manual reflects an explicit user confirmation"
    )]
    pub(crate) user_confirmed: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values instead of rejecting the import"
    )]
    pub(crate) redact_secrets: bool,
    #[arg(long)]
    pub(crate) no_validate_workflow: bool,
}

#[derive(Args)]
pub(crate) struct MergeArgs {
    pub(crate) db: PathBuf,
    #[arg(long)]
    pub(crate) prefer_trusted: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values from incoming durable data"
    )]
    pub(crate) redact_secrets: bool,
}

#[derive(Subcommand)]
pub(crate) enum BundleCommand {
    #[command(about = "Export memory.db, config, manifest, and artifacts as a .tgz bundle")]
    Export(BundleExportArgs),
    #[command(about = "Inspect bundle metadata and entries")]
    Inspect(BundleInspectArgs),
    #[command(about = "Import a bundle into the active store")]
    Import(BundleImportArgs),
}

#[derive(Args)]
pub(crate) struct BundleExportArgs {
    pub(crate) file: PathBuf,
    #[arg(long)]
    pub(crate) no_config: bool,
    #[arg(
        long,
        help = "Explicitly redact secret-like values in the exported copy"
    )]
    pub(crate) redact_secrets: bool,
}

#[derive(Args)]
pub(crate) struct BundleInspectArgs {
    pub(crate) file: PathBuf,
}

#[derive(Args)]
pub(crate) struct BundleImportArgs {
    pub(crate) file: PathBuf,
    #[arg(long, conflicts_with = "replace")]
    pub(crate) merge: bool,
    #[arg(long, conflicts_with = "merge")]
    pub(crate) replace: bool,
    #[arg(long)]
    pub(crate) force: bool,
    #[arg(
        long,
        help = "Explicitly redact detected secret-like values in the imported copy"
    )]
    pub(crate) redact_secrets: bool,
    #[arg(long, help = "Import a legacy bundle without a complete hash manifest")]
    pub(crate) allow_unverified: bool,
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
    #[command(about = "Scaffold a new workflow YAML file from the baseline template")]
    New(WorkflowNewArgs),
    #[command(about = "Record one workflow execution result for retro quality loops")]
    Record(WorkflowRecordArgs),
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
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve a workflow name"
    )]
    pub(crate) scope: String,
    #[arg(
        long,
        help = "Render the runbook as an ordered execution checklist instead of JSON"
    )]
    pub(crate) checklist: bool,
    #[arg(long, help = "Include graph context for this workflow")]
    pub(crate) with_graph_context: bool,
}

#[derive(Args)]
pub(crate) struct WorkflowNewArgs {
    pub(crate) name: String,
    #[arg(
        long,
        value_name = "FILE",
        help = "Output path; defaults to <name>.yaml"
    )]
    pub(crate) output: Option<PathBuf>,
    #[arg(long, help = "Overwrite an existing file")]
    pub(crate) force: bool,
}

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

fn parse_run_result(value: &str) -> Result<String, String> {
    parse_allowed(value, &["success", "failure"], "run result")
}

#[derive(Args)]
pub(crate) struct WorkflowRecordArgs {
    pub(crate) reference: String,
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve a workflow name"
    )]
    pub(crate) scope: String,
    #[arg(long, value_parser = parse_run_result, help = "success or failure")]
    pub(crate) result: String,
    #[arg(long, help = "One-line note about what happened")]
    pub(crate) note: Option<String>,
    #[arg(long, default_value = "agent", value_parser = parse_memory_source)]
    pub(crate) source: String,
    #[arg(long, help = "Attest that a human explicitly supplied a manual source")]
    pub(crate) user_confirmed: bool,
    #[arg(long, help = "Explicitly redact secret-like values from the run note")]
    pub(crate) redact_secrets: bool,
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
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve a workflow name"
    )]
    pub(crate) scope: String,
    #[arg(long)]
    pub(crate) check_artifacts: bool,
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
    #[arg(
        long,
        help = "Explicitly redact secret-like values in the artifact and metadata"
    )]
    pub(crate) redact_secrets: bool,
}

#[derive(Args)]
pub(crate) struct ArtifactUpdateArgs {
    pub(crate) name: String,
    #[arg(long)]
    pub(crate) checksum: bool,
    #[arg(
        long,
        help = "Explicitly redact secret-like values before updating the checksum"
    )]
    pub(crate) redact_secrets: bool,
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
    #[arg(long, help = "Explicitly redact secret-like values before persistence")]
    pub(crate) redact_secrets: bool,
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
    #[arg(
        long,
        default_value = "auto",
        help = "Scope used to resolve --keep when it is a name"
    )]
    pub(crate) scope: String,
    #[arg(long)]
    pub(crate) soft_delete_others: bool,
    #[arg(
        long,
        help = "Explicitly redact secret-like values from the resolution note"
    )]
    pub(crate) redact_secrets: bool,
}
