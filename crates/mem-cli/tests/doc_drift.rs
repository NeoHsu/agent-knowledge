use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{error::ErrorKind, Parser};

#[allow(dead_code)]
#[path = "../src/args/mod.rs"]
mod cli_args;
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
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("decode mem help as UTF-8: {error}"))
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
            let entry = entry.unwrap_or_else(|error| panic!("read directory entry: {error}"));
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExampleKind {
    ShellFence,
    Inline,
}

struct DocExample {
    line: usize,
    command: String,
    kind: ExampleKind,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn versioned_docs_match_package_version() {
    let root = repository_root();
    let expected = format!("source version `{}`", env!("CARGO_PKG_VERSION"));
    for relative in [
        "README.md",
        "docs/getting-started.md",
        "docs/performance.md",
        "skills/mnemark/references/cli-guide.md",
    ] {
        let path = root.join(relative);
        let markdown = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            markdown.contains(&expected),
            "{relative} must identify the current {expected}"
        );
    }
}

#[test]
fn agent_reference_tracks_split_module_paths() {
    let root = repository_root();
    let path = root.join("docs/agent-reference.md");
    let reference = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

    for relative in [
        "crates/mem-cli/src/args/mod.rs",
        "crates/mem-cli/src/commands/memory/",
        "crates/mem-cli/src/commands/merge/",
        "crates/mem-cli/src/commands/bundle/",
        "crates/mem-cli/src/commands/workflow/mod.rs",
        "crates/mem-cli/src/commands/setup/mod.rs",
        "crates/mem-cli/src/commands/doctor/mod.rs",
        "crates/mem-core/src/graph/query/mod.rs",
        "crates/mem-core/src/graph/materialize/mod.rs",
        "crates/mem-core/src/artifact/mod.rs",
    ] {
        assert!(
            root.join(relative).exists(),
            "expected split module path to exist: {relative}"
        );
        assert!(
            reference.contains(relative),
            "agent reference must route readers to split module path: {relative}"
        );
    }

    for removed in [
        "crates/mem-cli/src/args.rs",
        "crates/mem-cli/src/commands/memory.rs",
        "crates/mem-cli/src/commands/merge.rs",
        "crates/mem-cli/src/commands/bundle.rs",
        "crates/mem-cli/src/commands/workflow.rs",
        "crates/mem-cli/src/commands/setup.rs",
        "crates/mem-cli/src/commands/doctor.rs",
        "crates/mem-core/src/graph/query.rs",
        "crates/mem-core/src/graph/materialize.rs",
        "crates/mem-core/src/artifact.rs",
    ] {
        assert!(
            !reference.contains(removed),
            "agent reference must not point to removed module: {removed}"
        );
    }
}

#[test]
fn agent_entrypoints_describe_runtime_only_store_discovery() {
    let root = repository_root();
    for relative in ["AGENTS.md", "CLAUDE.md"] {
        let path = root.join(relative);
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        assert!(
            content.contains("docs/agent-reference.md"),
            "{relative} must route repository work through the canonical agent reference"
        );
    }

    let claude = fs::read_to_string(root.join("CLAUDE.md")).expect("read CLAUDE.md");
    assert!(
        claude.contains("source checkouts are never selected implicitly"),
        "CLAUDE.md must describe runtime-only store discovery"
    );
    assert!(
        !claude.contains("store discovery would treat the repo itself as a runtime store"),
        "CLAUDE.md must not retain the removed source-checkout discovery behavior"
    );
}

#[test]
fn mnemark_skill_keeps_safety_and_routing_boundaries() {
    let root = repository_root();
    let skill_path = root.join("skills/mnemark/SKILL.md");
    let guide_path = root.join("skills/mnemark/references/cli-guide.md");
    let skill = fs::read_to_string(&skill_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", skill_path.display()));
    let guide = fs::read_to_string(&guide_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", guide_path.display()));
    let combined = format!("{skill}\n{guide}");
    let normalized = combined.split_whitespace().collect::<Vec<_>>().join(" ");

    for required in [
        "store_source",
        "remember this",
        "記住",
        "匯入記憶庫",
        "把這個流程存成 runbook",
        "mem sync --dry-run",
        "fetch/merge",
        "generic Git sync",
        "ordinary data import/export",
        "CI workflows",
        "sprint retrospectives",
        "auditing/developing a skill",
        "Stores and bundles are plaintext",
        "do not authenticate the bundle publisher",
        "does not copy an external file",
    ] {
        assert!(
            normalized.contains(required),
            "mnemark skill docs must retain safety/routing contract: {required}"
        );
    }
    assert!(
        !skill.contains("The default creates only a local checkpoint"),
        "sync docs must not hide fetch/merge side effects"
    );
}

fn repository_markdown_files() -> Vec<PathBuf> {
    fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
        {
            let entry = entry.unwrap_or_else(|error| panic!("read directory entry: {error}"));
            let path = entry.path();
            if path.is_dir() {
                collect(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                files.push(path);
            }
        }
    }

    let root = repository_root();
    let mut files = fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("read repository root: {error}"))
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.is_file() && path.extension().is_some_and(|extension| extension == "md")
        })
        .collect::<Vec<_>>();
    for directory in [root.join("docs"), root.join("skills")] {
        collect(&directory, &mut files);
    }
    files.sort();
    files
}

fn markdown_mem_examples(markdown: &str) -> Vec<DocExample> {
    let mut examples = Vec::new();
    let mut in_fence = false;
    let mut shell_fence = false;
    let mut pending = String::new();
    let mut pending_line = 0;

    for (index, line) in markdown.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim_start();
        if let Some(info) = trimmed.strip_prefix("```") {
            if in_fence {
                if shell_fence && !pending.is_empty() {
                    push_shell_example(&mut examples, pending_line, &pending);
                    pending.clear();
                }
                in_fence = false;
                shell_fence = false;
            } else {
                in_fence = true;
                shell_fence = matches!(
                    info.trim().to_ascii_lowercase().as_str(),
                    "bash" | "sh" | "shell" | "zsh" | "console"
                );
            }
            continue;
        }

        if in_fence {
            if !shell_fence {
                continue;
            }
            let command_line = trimmed.strip_prefix("$ ").unwrap_or(trimmed).trim_end();
            if pending.is_empty() {
                pending_line = line_number;
            }
            if let Some(prefix) = command_line.strip_suffix('\\') {
                pending.push_str(prefix.trim_end());
                pending.push(' ');
                continue;
            }
            pending.push_str(command_line);
            push_shell_example(&mut examples, pending_line, &pending);
            pending.clear();
            continue;
        }

        for (span_index, span) in line.split('`').enumerate() {
            if span_index % 2 == 1 && (span.trim() == "mem" || span.trim().starts_with("mem ")) {
                examples.push(DocExample {
                    line: line_number,
                    command: span.trim().to_string(),
                    kind: ExampleKind::Inline,
                });
            }
        }
    }
    examples
}

fn push_shell_example(examples: &mut Vec<DocExample>, line: usize, command: &str) {
    let command = command.trim();
    if command.starts_with("mem ") || command == "mem" {
        examples.push(DocExample {
            line,
            command: command.to_string(),
            kind: ExampleKind::ShellFence,
        });
    }
}

fn placeholder_value(previous: Option<&str>, command: &[String], index: usize) -> &'static str {
    match previous {
        Some("--scope" | "--new-scope" | "--set-scope") => "global",
        Some("--source") => "agent",
        Some("--type") => "reference",
        Some("--confidence") => "medium",
        Some("--expires-at" | "--changed-since") => "2026-01-01T00:00:00Z",
        Some("--tags" | "--set-tags" | "--add-tags" | "--remove-tags") => "[]",
        Some("--memory-ids") => "[]",
        Some("--result") => "success",
        Some("--format") => "json",
        Some("--expected-version" | "--limit" | "--depth" | "--max-depth" | "--days") => "1",
        Some("--budget") => "4000",
        _ if index == 1
            && command
                .get(index)
                .is_some_and(|token| token == "<subcommand>") =>
        {
            "help"
        }
        _ if index == 2 && command.get(1).is_some_and(|token| token == "setup") => "pi",
        _ => "dummy",
    }
}

fn replace_placeholders(token: &str, replacement: &str) -> String {
    let mut output = String::new();
    let mut rest = token;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let after_start = &rest[start + 1..];
        let Some(end) = after_start.find('>') else {
            output.push_str(&rest[start..]);
            return output;
        };
        output.push_str(replacement);
        rest = &after_start[end + 1..];
    }
    output.push_str(rest);
    if output == "..." || output == "…" {
        replacement.to_string()
    } else {
        output
    }
}

fn token_alternatives(token: &str) -> Vec<String> {
    let separator = if token.contains('|') {
        '|'
    } else if token.contains('/')
        && token
            .chars()
            .all(|character| character.is_ascii_lowercase() || matches!(character, '-' | '/'))
    {
        '/'
    } else {
        return vec![token.to_string()];
    };
    if token.contains(['[', '{', '"', '\'', '=']) || token.split(separator).any(str::is_empty) {
        return vec![token.to_string()];
    }
    let alternatives = token
        .split(separator)
        .map(str::to_string)
        .collect::<Vec<_>>();
    if alternatives.len() > 8 {
        vec![token.to_string()]
    } else {
        alternatives
    }
}

fn normalized_invocations(command: &str) -> Result<Vec<Vec<String>>, String> {
    let mut tokens = shell_words::split(command).map_err(|error| error.to_string())?;
    if tokens.first().is_some_and(|token| token == "$") {
        tokens.remove(0);
    }
    let stop = tokens
        .iter()
        .position(|token| matches!(token.as_str(), "#" | ";" | "&&" | "||" | "|" | ">" | ">>"))
        .unwrap_or(tokens.len());
    tokens.truncate(stop);
    if tokens.first().is_none_or(|token| token != "mem") {
        return Err("example does not start with mem".to_string());
    }

    let original = tokens.clone();
    for (index, token) in tokens.iter_mut().enumerate() {
        let previous = index
            .checked_sub(1)
            .and_then(|previous| original.get(previous));
        let replacement = placeholder_value(previous.map(String::as_str), &original, index);
        *token = replace_placeholders(token, replacement);
    }

    let mut invocations = vec![Vec::new()];
    for token in tokens {
        let alternatives = token_alternatives(&token);
        let mut expanded = Vec::new();
        for invocation in &invocations {
            for alternative in &alternatives {
                let mut next = invocation.clone();
                next.push(alternative.clone());
                expanded.push(next);
            }
        }
        if expanded.len() > 64 {
            return Err("example expands to more than 64 command variants".to_string());
        }
        invocations = expanded;
    }

    Ok(invocations)
}

/// Every fenced shell command and inline `mem ...` span in repository-facing
/// Markdown is parsed by the real Clap definition. Inline command-name
/// references may omit required operands, but unknown commands, flags, and
/// invalid values always fail.
#[test]
fn repository_mem_examples_parse_with_real_clap() {
    let root = repository_root();
    let mut invalid = Vec::new();
    for file in repository_markdown_files() {
        let markdown = fs::read_to_string(&file)
            .unwrap_or_else(|error| panic!("read {}: {error}", file.display()));
        for example in markdown_mem_examples(&markdown) {
            let invocations = match normalized_invocations(&example.command) {
                Ok(invocations) => invocations,
                Err(error) => {
                    invalid.push(format!(
                        "{}:{} `{}`: {error}",
                        file.strip_prefix(&root).unwrap_or(&file).display(),
                        example.line,
                        example.command
                    ));
                    continue;
                }
            };
            for invocation in invocations {
                if let Err(error) = cli_args::Cli::try_parse_from(&invocation) {
                    let partial_inline = example.kind == ExampleKind::Inline
                        && matches!(
                            error.kind(),
                            ErrorKind::MissingRequiredArgument
                                | ErrorKind::MissingSubcommand
                                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                        );
                    if partial_inline
                        || matches!(
                            error.kind(),
                            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
                        )
                    {
                        continue;
                    }

                    if example.kind == ExampleKind::Inline {
                        if let Some(option) = invocation
                            .last()
                            .filter(|token| token.starts_with("--"))
                            .cloned()
                        {
                            let mut completed = invocation.clone();
                            completed.push(
                                placeholder_value(Some(&option), &completed, completed.len())
                                    .into(),
                            );
                            match cli_args::Cli::try_parse_from(&completed) {
                                Ok(_) => continue,
                                Err(completed_error)
                                    if matches!(
                                        completed_error.kind(),
                                        ErrorKind::MissingRequiredArgument
                                            | ErrorKind::MissingSubcommand
                                            | ErrorKind::DisplayHelp
                                            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                                            | ErrorKind::DisplayVersion
                                    ) =>
                                {
                                    continue;
                                }
                                Err(_) => {}
                            }
                        }
                    }

                    invalid.push(format!(
                        "{}:{} `{}` as `{}`: {}",
                        file.strip_prefix(&root).unwrap_or(&file).display(),
                        example.line,
                        example.command,
                        invocation.join(" "),
                        error.to_string().replace('\n', " ")
                    ));
                }
            }
        }
    }
    assert!(
        invalid.is_empty(),
        "Markdown mem examples rejected by Clap:\n{}",
        invalid.join("\n")
    );
}
