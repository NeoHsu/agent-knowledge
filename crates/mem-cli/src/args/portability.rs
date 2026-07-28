use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

use super::memory::{parse_memory_source, parse_memory_type};

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
    #[arg(long, help = "Omit per-item results and emit only total/counts")]
    pub(crate) summary_only: bool,
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
    #[arg(long, help = "Include per-stage timing and byte metrics in the result")]
    pub(crate) profile: bool,
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

#[derive(Args)]
pub(crate) struct RetroArgs {
    #[arg(long, default_value_t = 50)]
    pub(crate) limit: usize,
}
