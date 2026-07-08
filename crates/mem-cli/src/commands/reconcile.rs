use std::path::{Path, PathBuf};

use super::*;
use mem_core::config::expand_home;
use mem_core::util::{Claim, ClaimKind};

pub(crate) fn cmd_reconcile(app: &App, args: ReconcileArgs) -> Result<()> {
    app.ensure_schema()?;
    let repo_root = match args.repo {
        Some(dir) => dir,
        None => std::env::current_dir().context("resolve current directory")?,
    };
    if !repo_root.is_dir() {
        bail!("repo root {} is not a directory", repo_root.display());
    }

    let scopes = if args.scope == "auto" {
        scope::detect_scope_set()?
    } else {
        vec![args.scope.clone()]
    };
    let scope_refs = scopes.iter().map(String::as_str).collect::<Vec<_>>();

    let conn = app.conn()?;
    let mut memories = list_memories_filtered(
        &conn,
        false,
        args.r#type.as_deref(),
        None,
        Some(&scope_refs),
        false,
    )?;
    if args.r#type.is_none() {
        // Workflow runbooks have their own checker (`mem workflow validate`).
        memories.retain(|memory| memory.r#type != "workflow");
    }

    let checked = memories.len();
    let mut flagged_count = 0;
    let mut results = Vec::new();
    for memory in &memories {
        let content = memory.content.as_deref().unwrap_or_default();
        let extracted = extract_claims(content);
        if extracted.claims.is_empty() && extracted.unverifiable.is_empty() {
            continue;
        }
        let mut flagged = false;
        let claims = extracted
            .claims
            .iter()
            .map(|claim| {
                let status = verify_claim(claim, &repo_root);
                flagged |= status == "missing";
                json!({
                    "claim": claim.text,
                    "kind": claim.kind.as_str(),
                    "backticked": claim.backticked,
                    "status": status
                })
            })
            .collect::<Vec<_>>();
        if flagged {
            flagged_count += 1;
        }
        results.push(json!({
            "name": memory.name,
            "type": memory.r#type,
            "scope": memory.scope,
            "flagged": flagged,
            "claims": claims,
            "unverifiable": extracted.unverifiable
        }));
    }

    print_json_pretty(&json!({
        "status": "reconciled",
        "repo_root": repo_root,
        "scopes": scopes,
        "memories_checked": checked,
        "memories_flagged": flagged_count,
        "results": results,
        "instructions": [
            "reconcile is read-only: it verifies claims but never edits memories; you decide each fix.",
            "For each flagged memory judge the missing claim: fact still true but path/command moved (mem update), fact replaced (mem supersede), fact obsolete (mem delete), or the claim describes another machine/repo (leave it).",
            "unverifiable entries could not be checked mechanically; review them only when the memory is already suspect.",
            "Run mem sync after fixing flagged memories."
        ]
    }))
}

fn verify_claim(claim: &Claim, repo_root: &Path) -> &'static str {
    let exists = match claim.kind {
        ClaimKind::Command => command_on_path(&claim.text),
        ClaimKind::Path => path_claim_exists(&claim.text, repo_root),
    };
    if exists {
        "ok"
    } else {
        "missing"
    }
}

fn path_claim_exists(text: &str, repo_root: &Path) -> bool {
    let trimmed = text.trim_end_matches('/');
    let trimmed = if trimmed.is_empty() { "/" } else { trimmed };
    let expanded = expand_home(trimmed);
    if !trimmed.contains('<') {
        let full = if expanded.is_absolute() {
            expanded
        } else {
            repo_root.join(expanded)
        };
        return full.exists();
    }
    let (start, relative) = if expanded.is_absolute() {
        let relative = expanded
            .strip_prefix("/")
            .unwrap_or(&expanded)
            .to_path_buf();
        (PathBuf::from("/"), relative)
    } else {
        (repo_root.to_path_buf(), expanded)
    };
    let segments = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    placeholder_path_exists(&start, &segments)
}

/// Match a path whose `<placeholder>` segments stand for any directory entry,
/// e.g. `.claude/commands/experts/<domain>/plan.md`.
fn placeholder_path_exists(dir: &Path, segments: &[String]) -> bool {
    let Some((first, rest)) = segments.split_first() else {
        return dir.exists();
    };
    if first.contains('<') {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            if placeholder_path_exists(&entry.path(), rest) {
                return true;
            }
        }
        false
    } else {
        let next = dir.join(first);
        if rest.is_empty() {
            next.exists()
        } else {
            placeholder_path_exists(&next, rest)
        }
    }
}

fn command_on_path(name: &str) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path_var).any(|dir| {
        if dir.join(name).is_file() {
            return true;
        }
        cfg!(windows)
            && ["exe", "cmd", "bat"]
                .iter()
                .any(|extension| dir.join(format!("{name}.{extension}")).is_file())
    })
}
