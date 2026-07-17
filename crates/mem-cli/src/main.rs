use std::process::ExitCode;

use anyhow::Result;
use clap::{error::ErrorKind, Parser};
use serde_json::json;

mod args;
mod commands;

use args::*;
use commands::*;
use mem_core::app::{with_lock, with_shared_lock, App};
use mem_core::index as memory_index;
use mem_core::util::strip_secrets;

fn main() -> ExitCode {
    let wants_json_errors = std::env::args_os().any(|arg| arg == "--json-errors");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            let exit_code = u8::try_from(error.exit_code()).unwrap_or(2);
            if wants_json_errors {
                eprintln!(
                    "{}",
                    json!({
                        "status": "error",
                        "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                        "code": "cli_parse_error",
                        "message": safe_error_message(error.to_string()),
                        "exit_code": exit_code
                    })
                );
            } else {
                let _ = error.print();
            }
            return ExitCode::from(exit_code);
        }
    };
    let json_errors = cli.json_errors;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = format!("{error:#}");
            if json_errors {
                eprintln!(
                    "{}",
                    json!({
                        "status": "error",
                        "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                        "code": "command_failed",
                        "message": safe_error_message(message),
                        "exit_code": 1
                    })
                );
            } else {
                eprintln!("Error: {message}");
            }
            ExitCode::FAILURE
        }
    }
}

fn safe_error_message(message: String) -> String {
    strip_secrets(&message).unwrap_or_else(|_| "error details could not be rendered safely".into())
}

fn run(cli: Cli) -> Result<()> {
    // Contract introspection must remain available even when the user or store
    // configuration is missing or malformed.
    if matches!(&cli.command, Command::Contract) {
        return cmd_contract();
    }

    // Every store-facing command uses explicit/runtime discovery. A source
    // checkout is never selected implicitly; development stores must use
    // --home or MNEMARK_HOME.
    let app = App::discover_runtime_with_home(cli.home.as_deref())?;

    match cli.command {
        Command::Init => with_lock(&app, || {
            app.init()?;
            print_json(&json!({"status": "initialized", "root": app.root}))?;
            Ok(())
        })?,
        Command::Migrate(args) if args.dry_run => cmd_migrate(&app, args)?,
        Command::Migrate(args) => with_lock(&app, || cmd_migrate(&app, args))?,
        Command::Save(args) => with_lock(&app, || cmd_save(&app, args))?,
        Command::Query(args) if args.touch || args.repair_index => {
            with_lock(&app, || cmd_query(&app, args))?
        }
        Command::Query(args) => cmd_query(&app, args)?,
        Command::Prime(args) if args.focus.is_some() && app.db_path.exists() => {
            with_lock(&app, || cmd_prime(&app, args))?
        }
        Command::Prime(args) => cmd_prime(&app, args)?,
        Command::Doctor(args) => cmd_doctor(&app, args)?,
        Command::Sync(args) if args.dry_run => cmd_sync(&app, args)?,
        Command::Sync(args) => with_lock(&app, || cmd_sync(&app, args))?,
        Command::Update(args) => with_lock(&app, || cmd_update(&app, args))?,
        Command::Supersede(args) => with_lock(&app, || cmd_supersede(&app, args))?,
        Command::Delete(args) => with_lock(&app, || cmd_delete(&app, args))?,
        Command::Reindex => with_lock(&app, || {
            app.require_schema()?;
            memory_index::reindex_or_mark_stale(&app, "rebuild index")?;
            print_json(&json!({"status": "reindexed"}))?;
            Ok(())
        })?,
        Command::Context(args) => cmd_context(args)?,
        Command::Config { command } => cmd_config(&app, command)?,
        Command::Contract => unreachable!("contract handled before store discovery"),
        Command::Setup { command } => cmd_setup(command)?,
        Command::History(args) => cmd_history(&app, args)?,
        Command::Stats(args) => cmd_stats(&app, args)?,
        Command::Audit(args) if args.fix => with_lock(&app, || cmd_audit(&app, args))?,
        Command::Audit(args) => cmd_audit(&app, args)?,
        // Read-only: verifies claims against the filesystem, never writes.
        Command::Reconcile(args) => cmd_reconcile(&app, args)?,
        Command::Gc(args) => with_lock(&app, || cmd_gc(&app, args))?,
        Command::Export(args) => cmd_export(&app, args)?,
        Command::Import(args) => with_lock(&app, || cmd_import(&app, args))?,
        Command::Merge(args) => with_lock(&app, || cmd_merge(&app, args))?,
        Command::Bundle { command } => match command {
            BundleCommand::Inspect(args) => cmd_bundle(&app, BundleCommand::Inspect(args))?,
            BundleCommand::Export(args) => {
                with_shared_lock(&app, || cmd_bundle(&app, BundleCommand::Export(args)))?
            }
            other => with_lock(&app, || cmd_bundle(&app, other))?,
        },
        Command::Retro { command } => cmd_retro(&app, command)?,
        Command::Workflow { command } => match command {
            WorkflowCommand::Record(args) => {
                with_lock(&app, || cmd_workflow(&app, WorkflowCommand::Record(args)))?
            }
            WorkflowCommand::Show(args) if args.with_graph_context => {
                with_lock(&app, || cmd_workflow(&app, WorkflowCommand::Show(args)))?
            }
            other => cmd_workflow(&app, other)?,
        },
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
        Command::Ambiguity {
            command: AmbiguityCommand::List(args),
        } => cmd_ambiguity(&app, AmbiguityCommand::List(args))?,
        Command::Ambiguity { command } => with_lock(&app, || cmd_ambiguity(&app, command))?,
        Command::Graph {
            command: GraphCommand::Stats,
        } => cmd_graph(&app, GraphCommand::Stats)?,
        Command::Graph {
            command: GraphCommand::Review(args),
        } => cmd_graph(&app, GraphCommand::Review(args))?,
        Command::Graph {
            command: GraphCommand::Candidates(args),
        } if !args.unlinked => cmd_graph(&app, GraphCommand::Candidates(args))?,
        Command::Graph { command } => with_lock(&app, || cmd_graph(&app, command))?,
    }

    Ok(())
}
