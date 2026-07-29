use std::process::ExitCode;

use anyhow::Result;
use clap::{error::ErrorKind, Parser};
use serde_json::json;

mod args;
mod cli_error;
mod command_effect;
mod commands;

use args::*;
use cli_error::StructuredCommandError;
use command_effect::{CommandEffect, StoreAccess};
use commands::*;
use mem_core::app::{with_lock, with_shared_lock, App};
use mem_core::error::MnemarkError;
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
                        "exit_code": exit_code,
                        "retryable": false
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
            let structured = error.downcast_ref::<StructuredCommandError>();
            let core = error.downcast_ref::<MnemarkError>();
            let exit_code = structured.map_or_else(
                || core.map_or(1, |error| error.code().exit_code()),
                StructuredCommandError::exit_code,
            );
            let code = structured.map_or_else(
                || core.map_or("command_failed", |error| error.code().as_str()),
                StructuredCommandError::code,
            );
            let message = format!("{error:#}");
            if json_errors {
                let mut response = json!({
                    "status": "error",
                    "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                    "code": code,
                    "message": safe_error_message(message),
                    "exit_code": exit_code,
                    "retryable": structured.is_some_and(StructuredCommandError::retryable)
                });
                if let Some(structured) = structured {
                    response["details"] = structured.details().clone();
                }
                eprintln!("{response}");
            } else {
                eprintln!("Error: {message}");
            }
            ExitCode::from(exit_code)
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
    let effect = CommandEffect::classify(&cli.command, app.db_path.exists());

    match effect.store_access {
        StoreAccess::ExclusiveLock => with_lock(&app, || dispatch(&app, cli.command)),
        StoreAccess::SharedLock => with_shared_lock(&app, || dispatch(&app, cli.command)),
        StoreAccess::None | StoreAccess::ReadOnly => dispatch(&app, cli.command),
    }
}

fn dispatch(app: &App, command: Command) -> Result<()> {
    match command {
        Command::Init => {
            app.init()?;
            print_json(&json!({"status": "initialized", "root": app.root}))?;
        }
        Command::Migrate(args) => cmd_migrate(app, args)?,
        Command::Save(args) => cmd_save(app, args)?,
        Command::Query(args) => cmd_query(app, args)?,
        Command::Prime(args) => cmd_prime(app, args)?,
        Command::Doctor(args) => cmd_doctor(app, args)?,
        Command::Sync(args) => cmd_sync(app, args)?,
        Command::Update(args) => cmd_update(app, args)?,
        Command::Supersede(args) => cmd_supersede(app, args)?,
        Command::Delete(args) => cmd_delete(app, args)?,
        Command::Reindex => {
            app.require_schema()?;
            memory_index::reindex_or_mark_stale(app, "rebuild index")?;
            print_json(&json!({"status": "reindexed"}))?;
        }
        Command::Context(args) => cmd_context(args)?,
        Command::Config { command } => cmd_config(app, command)?,
        Command::Contract => unreachable!("contract handled before store discovery"),
        Command::Setup { command } => cmd_setup(command)?,
        Command::History(args) => cmd_history(app, args)?,
        Command::Stats(args) => cmd_stats(app, args)?,
        Command::Audit(args) => cmd_audit(app, args)?,
        Command::Reconcile(args) => cmd_reconcile(app, args)?,
        Command::Gc(args) => cmd_gc(app, args)?,
        Command::Export(args) => cmd_export(app, args)?,
        Command::Import(args) => cmd_import(app, args)?,
        Command::Merge(args) => cmd_merge(app, args)?,
        Command::Bundle { command } => cmd_bundle(app, command)?,
        Command::Retro { command } => cmd_retro(app, command)?,
        Command::Workflow { command } => cmd_workflow(app, command)?,
        Command::Artifact { command } => cmd_artifact(app, command)?,
        Command::Ambiguity { command } => cmd_ambiguity(app, command)?,
        Command::Graph { command } => cmd_graph(app, command)?,
    }

    Ok(())
}
