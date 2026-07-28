use clap::{Parser, Subcommand};

mod graph;
mod maintenance;
mod memory;
mod portability;
mod workflow;

pub(crate) use graph::*;
pub(crate) use maintenance::*;
pub(crate) use memory::*;
pub(crate) use portability::*;
pub(crate) use workflow::*;

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
    #[arg(
        long,
        global = true,
        help = "Emit machine-readable JSON errors on stderr"
    )]
    pub(crate) json_errors: bool,
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
    #[command(about = "Describe machine-readable output and persisted format contracts")]
    Contract,
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
