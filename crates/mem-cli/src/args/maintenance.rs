use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use super::memory::{parse_memory_type, OutputFormat};

#[derive(Subcommand)]
pub(crate) enum ConfigCommand {
    #[command(about = "Show active store, config paths, environment, and effective defaults")]
    Show,
}

#[derive(Subcommand)]
pub(crate) enum SetupCommand {
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
        help = "Maximum emitted output in characters"
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
        help = "Only check one platform: claude-code, codex, pi, gemini-cli, or opencode"
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
