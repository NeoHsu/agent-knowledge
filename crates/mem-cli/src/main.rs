use anyhow::Result;
use clap::Parser;
use serde_json::json;

mod args;
mod commands;

use args::*;
use commands::*;
use mem_core::app::{with_lock, App};
use mem_core::index as memory_index;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let app = App::discover_with_home(cli.home.as_deref())?;

    match cli.command {
        Command::Init => {
            app.init()?;
            print_json(&json!({"status": "initialized", "root": app.root}))?;
        }
        Command::Save(args) => with_lock(&app, || cmd_save(&app, args))?,
        Command::Query(args) if args.no_touch => cmd_query(&app, args)?,
        Command::Query(args) => with_lock(&app, || cmd_query(&app, args))?,
        Command::Update(args) => with_lock(&app, || cmd_update(&app, args))?,
        Command::Supersede(args) => with_lock(&app, || cmd_supersede(&app, args))?,
        Command::Delete(args) => with_lock(&app, || cmd_delete(&app, args))?,
        Command::Reindex => with_lock(&app, || {
            app.ensure_schema()?;
            memory_index::reindex_or_mark_stale(&app, "rebuild index")?;
            print_json(&json!({"status": "reindexed"}))?;
            Ok(())
        })?,
        Command::Context(args) => cmd_context(args)?,
        Command::Config { command } => cmd_config(&app, command)?,
        Command::Setup { command } => cmd_setup(command)?,
        Command::History(args) => cmd_history(&app, args)?,
        Command::Stats(args) => cmd_stats(&app, args)?,
        Command::Audit(args) => with_lock(&app, || cmd_audit(&app, args))?,
        Command::Gc(args) => with_lock(&app, || cmd_gc(&app, args))?,
        Command::Export(args) => cmd_export(&app, args)?,
        Command::Import(args) => with_lock(&app, || cmd_import(&app, args))?,
        Command::Merge(args) => with_lock(&app, || cmd_merge(&app, args))?,
        Command::Bundle { command } => match command {
            BundleCommand::Inspect(args) => cmd_bundle(&app, BundleCommand::Inspect(args))?,
            other => with_lock(&app, || cmd_bundle(&app, other))?,
        },
        Command::Retro { command } => cmd_retro(&app, command)?,
        Command::Workflow { command } => cmd_workflow(&app, command)?,
        Command::Artifact { command } => {
            let writes_manifest = matches!(
                command,
                ArtifactCommand::Add(_) | ArtifactCommand::Update(_) | ArtifactCommand::Remove(_)
            );
            if writes_manifest {
                with_lock(&app, || cmd_artifact(&app, command))?;
            } else {
                cmd_artifact(&app, command)?;
            }
        }
        Command::Ambiguity { command } => with_lock(&app, || cmd_ambiguity(&app, command))?,
    }

    Ok(())
}
