use std::collections::BTreeSet;
use std::process::Command;

mod support;

use support::mem_bin;

const CLI_GUIDE: &str = include_str!("../../../skills/mnemark/references/cli-guide.md");

fn clap_subcommands() -> BTreeSet<String> {
    let output = Command::new(mem_bin())
        .arg("--help")
        .output()
        .expect("run mem --help");
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("utf8 help");

    let mut commands = BTreeSet::new();
    let mut in_commands = false;
    for line in help.lines() {
        if line.starts_with("Commands:") {
            in_commands = true;
            continue;
        }
        if in_commands {
            if line.starts_with("Options:") || line.trim().is_empty() {
                break;
            }
            if let Some(name) = line.trim_start().split_whitespace().next() {
                commands.insert(name.to_string());
            }
        }
    }
    assert!(
        commands.len() > 10,
        "failed to parse subcommands from help: {help}"
    );
    commands
}

fn guide_code_block_commands() -> BTreeSet<String> {
    let mut commands = BTreeSet::new();
    let mut in_fence = false;
    for line in CLI_GUIDE.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            continue;
        }
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("mem ") else {
            continue;
        };
        let Some(first) = rest.split_whitespace().next() else {
            continue;
        };
        if first.starts_with('-') {
            continue;
        }
        commands.insert(first.to_string());
    }
    commands
}

/// Every clap subcommand must appear in the skill CLI guide, and every
/// `mem <subcommand>` example in the guide must be a real subcommand. This
/// keeps the guide and the binary from drifting apart.
#[test]
fn cli_guide_matches_clap_subcommands() {
    let clap = clap_subcommands();
    let guide = guide_code_block_commands();

    let undocumented: Vec<&String> = clap
        .iter()
        .filter(|command| *command != "help")
        .filter(|command| !guide.contains(*command))
        .collect();
    assert!(
        undocumented.is_empty(),
        "subcommands missing from skills/mnemark/references/cli-guide.md: {undocumented:?}"
    );

    let unknown: Vec<&String> = guide
        .iter()
        .filter(|command| !clap.contains(*command))
        .collect();
    assert!(
        unknown.is_empty(),
        "cli-guide.md documents commands clap does not provide: {unknown:?}"
    );
}
