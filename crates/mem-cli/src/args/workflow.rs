use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::memory::{parse_allowed, parse_memory_source};

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
