use super::super::*;
use crate::cli_error::usage_error;
use crate::command_effect::CommandEffect;
use clap::{CommandFactory, Parser};

fn collect_leaf_operations(command: &clap::Command, path: &str, operations: &mut Vec<Value>) {
    let children = command
        .get_subcommands()
        .filter(|child| child.get_name() != "help" && !child.is_hide_set())
        .collect::<Vec<_>>();
    if children.is_empty() {
        let operation = path.strip_prefix("mem ").unwrap_or(path).replace(' ', ".");
        let invocation = path.strip_prefix("mem ").unwrap_or(path);
        operations.push(json!({
            "id": operation,
            "command": path,
            "inspect": format!("mem operation inspect -- {invocation} <ARGS>")
        }));
        return;
    }
    for child in children {
        collect_leaf_operations(child, &format!("{path} {}", child.get_name()), operations);
    }
}

fn matching_subcommand<'a>(command: &'a clap::Command, token: &str) -> Option<&'a clap::Command> {
    command.get_subcommands().find(|subcommand| {
        subcommand.get_name() == token || subcommand.get_all_aliases().any(|alias| alias == token)
    })
}

fn canonical_command_path(arguments: &[String]) -> Result<Vec<String>> {
    let mut root = Cli::command();
    root.build();
    let mut command = &root;
    let mut path = Vec::new();
    let mut index = 0usize;

    while command.has_subcommands() {
        let mut found = None;
        while index < arguments.len() {
            let token = &arguments[index];
            if token == "--home" {
                index = index.saturating_add(2);
                continue;
            }
            if token.starts_with("--home=") || token == "--json-errors" || token == "--" {
                index += 1;
                continue;
            }
            if let Some(subcommand) = matching_subcommand(command, token) {
                found = Some(subcommand);
                index += 1;
                break;
            }
            index += 1;
        }
        let Some(subcommand) = found else {
            break;
        };
        path.push(subcommand.get_name().to_string());
        command = subcommand;
    }

    if path.is_empty() {
        return Err(usage_error(
            "operation inspect could not resolve a mem subcommand",
        ));
    }
    Ok(path)
}

pub(crate) fn cmd_operation(command: OperationCommand) -> Result<()> {
    match command {
        OperationCommand::List => {
            let mut root = Cli::command();
            root.build();
            let mut operations = Vec::new();
            collect_leaf_operations(&root, "mem", &mut operations);
            print_json_pretty(&json!({
                "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                "effect_source": "Parse the exact invocation with mem operation inspect; effects can depend on flags and store existence.",
                "operations": operations
            }))
        }
        OperationCommand::Inspect(args) => {
            let argv = std::iter::once("mem".to_string())
                .chain(args.command.iter().cloned())
                .collect::<Vec<_>>();
            let parsed = Cli::try_parse_from(&argv)
                .map_err(|error| usage_error(format!("invalid inspected command: {error}")))?;
            if matches!(parsed.command, Command::Operation { .. }) {
                return Err(usage_error(
                    "operation inspect cannot recursively inspect operation",
                ));
            }
            let path = canonical_command_path(&args.command)?;
            let effect = CommandEffect::classify(&parsed.command, args.store_exists);
            print_json_pretty(&json!({
                "contract_version": CLI_OUTPUT_CONTRACT_VERSION,
                "operation": path.join("."),
                "store_exists": args.store_exists,
                "allowed_in_read_only": !effect.mutates(),
                "effect": effect
            }))
        }
    }
}
