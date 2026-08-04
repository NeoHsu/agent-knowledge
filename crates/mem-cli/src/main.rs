use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, error::ErrorKind};
use serde_json::json;

mod args;
mod cli_error;
mod command_effect;
mod commands;

use args::*;
use cli_error::StructuredCommandError;
use command_effect::{CommandEffect, StoreAccess};
use commands::*;
use mem_core::app::{App, with_lock, with_shared_lock};
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
                let response = json!({
                    "status": "error",
                    "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                    "code": "cli_parse_error",
                    "message": safe_error_message(error.to_string()),
                    "exit_code": exit_code,
                    "retryable": false
                });
                let _ = write_stderr_line(&response.to_string());
            } else {
                let _ = write_stderr_line(&safe_error_message(error.to_string()));
            }
            return ExitCode::from(exit_code);
        }
    };
    let json_errors = cli.json_errors;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if is_broken_pipe(&error) => ExitCode::SUCCESS,
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
            let message = safe_error_message(format!("{error:#}"));
            if json_errors {
                let mut response = json!({
                    "status": "error",
                    "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                    "code": code,
                    "message": message,
                    "exit_code": exit_code,
                    "retryable": structured.is_some_and(StructuredCommandError::retryable)
                });
                if let Some(structured) = structured {
                    response["details"] = structured.details().clone();
                }
                let _ = write_stderr_line(&response.to_string());
            } else {
                let _ = write_stderr_line(&format!("Error: {message}"));
            }
            ExitCode::from(exit_code)
        }
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::BrokenPipe)
    })
}

fn safe_error_message(message: String) -> String {
    let redacted = strip_secrets(&message)
        .unwrap_or_else(|_| "error details could not be rendered safely".into());
    terminal_safe(&redacted)
}

fn terminal_safe(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control()
            || matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
        {
            output.extend(character.escape_default());
        } else {
            output.push(character);
        }
    }
    output
}

fn run(cli: Cli) -> Result<()> {
    configure_output_limit(cli.max_bytes)?;
    let read_only = cli.read_only;

    // Contract, schema, and operation introspection remain available even when
    // user/store configuration is missing or malformed.
    let command = match cli.command {
        Command::Contract(args) => return cmd_contract(args),
        Command::Schema { command } => return cmd_schema(command),
        Command::Operation { command } => return cmd_operation(command),
        command => command,
    };

    // Every store-facing command uses explicit/runtime discovery. A source
    // checkout is never selected implicitly; development stores must use
    // --home or MNEMARK_HOME.
    let app = App::discover_runtime_with_home(cli.home.as_deref())?;
    let effect = CommandEffect::classify(&command, app.db_path.exists());
    if read_only && effect.mutates() {
        return Err(mem_core::error::safety_violation(format!(
            "command is blocked by --read-only; classified effects: {}",
            serde_json::to_string(&effect)?
        )));
    }

    match effect.store_access {
        StoreAccess::ExclusiveLock => with_lock(&app, || dispatch(&app, command)),
        StoreAccess::SharedLock => with_shared_lock(&app, || dispatch(&app, command)),
        StoreAccess::None | StoreAccess::ReadOnly => dispatch(&app, command),
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
        Command::Contract(_) | Command::Schema { .. } | Command::Operation { .. } => {
            unreachable!("store-independent discovery handled before store discovery")
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_error_text_escapes_controls_and_bidi_overrides() {
        assert_eq!(
            terminal_safe("safe\n\u{1b}[31m\u{202e}txt"),
            r"safe\n\u{1b}[31m\u{202e}txt"
        );
    }
}
