use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

mod support;

use support::mem_bin;

const CLI_GUIDE: &str = include_str!("../../../skills/mnemark/references/cli-guide.md");

fn parse_help_subcommands(help: &str) -> BTreeSet<String> {
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
            if let Some(name) = line.split_whitespace().next() {
                commands.insert(name.to_string());
            }
        }
    }
    commands
}

fn help_for(args: &[String]) -> String {
    let help_home = std::env::temp_dir().join("mnemark-doc-drift-help");
    let output = Command::new(mem_bin())
        .arg("--home")
        .arg(help_home)
        .args(args)
        .arg("--help")
        .output()
        .unwrap_or_else(|err| panic!("run mem {} --help: {err}", args.join(" ")));
    assert!(
        output.status.success(),
        "mem {} --help failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 help")
}

fn clap_subcommands() -> BTreeSet<String> {
    let commands = parse_help_subcommands(&help_for(&[]));
    assert!(
        commands.len() > 10,
        "failed to parse subcommands from mem --help"
    );
    commands
}

fn clap_command_paths() -> BTreeSet<Vec<String>> {
    let mut paths = BTreeSet::new();
    for command in clap_subcommands() {
        if command == "help" {
            continue;
        }
        let args = vec![command.clone()];
        let children = parse_help_subcommands(&help_for(&args));
        if children.is_empty() {
            paths.insert(vec![command]);
        } else {
            for child in children {
                if child != "help" {
                    paths.insert(vec![command.clone(), child]);
                }
            }
        }
    }
    paths
}

fn guide_code_block_commands() -> BTreeSet<String> {
    code_block_mem_lines(CLI_GUIDE)
        .into_iter()
        .filter_map(|line| line.split_whitespace().nth(1).map(str::to_string))
        .filter(|command| !command.starts_with('-'))
        .collect()
}

fn code_block_mem_lines(markdown: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut in_fence = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("mem ") {
            lines.push(trimmed.to_string());
        }
    }
    lines
}

fn example_command_path(line: &str, valid_paths: &BTreeSet<Vec<String>>) -> Option<Vec<String>> {
    let mut tokens = line
        .strip_prefix("mem ")?
        .split_whitespace()
        .take_while(|token| !token.starts_with('#'))
        .map(str::to_string);
    let first = tokens.next()?;
    if first.starts_with('-') {
        return None;
    }
    let has_nested = valid_paths
        .iter()
        .any(|path| path.len() > 1 && path.first() == Some(&first));
    if has_nested {
        let Some(second) = tokens.next() else {
            return Some(vec![first]);
        };
        Some(vec![first, second])
    } else {
        Some(vec![first])
    }
}

fn mem_command_paths_from_markdown(
    markdown: &str,
    valid_paths: &BTreeSet<Vec<String>>,
) -> BTreeSet<Vec<String>> {
    code_block_mem_lines(markdown)
        .into_iter()
        .filter_map(|line| example_command_path(&line, valid_paths))
        .collect()
}

fn skill_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/mnemark")
}

fn skill_markdown_files() -> Vec<PathBuf> {
    fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        {
            let entry = entry.expect("read directory entry");
            let path = entry.path();
            if path.is_dir() {
                collect(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    collect(&skill_root(), &mut files);
    files.sort();
    files
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

/// Nested clap commands such as `mem workflow show` and `mem artifact add`
/// must also be represented in the CLI guide. Otherwise references can drift
/// even when every top-level subcommand is mentioned.
#[test]
fn cli_guide_matches_nested_clap_commands() {
    let clap = clap_command_paths();
    let guide = mem_command_paths_from_markdown(CLI_GUIDE, &clap);

    let undocumented: Vec<String> = clap
        .iter()
        .filter(|path| !guide.contains(*path))
        .map(|path| format!("mem {}", path.join(" ")))
        .collect();
    assert!(
        undocumented.is_empty(),
        "nested commands missing from skills/mnemark/references/cli-guide.md: {undocumented:?}"
    );
}

/// All fenced `mem ...` examples across the mnemark skill must name a real
/// clap command path. The test validates command shape through `--help` only;
/// it never executes the documented commands.
#[test]
fn skill_mem_examples_use_real_clap_commands() {
    let valid_paths = clap_command_paths();
    let mut unknown = Vec::new();

    for file in skill_markdown_files() {
        let markdown = fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("read {}: {err}", file.display()));
        for line in code_block_mem_lines(&markdown) {
            let Some(path) = example_command_path(&line, &valid_paths) else {
                continue;
            };
            if !valid_paths.contains(&path) {
                let display = file
                    .strip_prefix(skill_root())
                    .unwrap_or(&file)
                    .display()
                    .to_string();
                unknown.push(format!(
                    "{display}: `{line}` parsed as `mem {}`",
                    path.join(" ")
                ));
            }
        }
    }

    assert!(
        unknown.is_empty(),
        "skill markdown documents commands clap does not provide: {unknown:?}"
    );
}
