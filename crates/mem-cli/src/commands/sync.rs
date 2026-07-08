use super::*;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

const GITIGNORE_LINES: &[&str] = &["index/", ".mem.lock", "memory.db-wal", "memory.db-shm"];

/// Sync model: git moves bytes, `mem merge` resolves meaning. A concurrent
/// remote change to memory.db always conflicts in git (binary file); the
/// conflict is resolved by keeping the local db and semantically merging the
/// remote copy through the same merge logic as `mem merge`, so same-name
/// conflicts end up as ambiguity records instead of clobbered rows.
pub(crate) fn cmd_sync(app: &App, args: SyncArgs) -> Result<()> {
    app.ensure_schema()?;
    // Fold any WAL content into memory.db before git sees it, so the
    // committed file is complete on its own.
    app.conn()?
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .context("checkpoint WAL before sync")?;
    let root = &app.root;

    let toplevel = git_capture(root, &["rev-parse", "--show-toplevel"]).map_err(|_| {
        anyhow!(
            "store {} is not a git repository; run `git init` there, add a .gitignore for \
             index/ and lock/WAL files, then retry",
            root.display()
        )
    })?;
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.clone());
    let canonical_top =
        fs::canonicalize(toplevel.trim()).unwrap_or_else(|_| PathBuf::from(toplevel.trim()));
    if canonical_top != canonical_root {
        bail!(
            "store {} is inside the git repository {}, not its own repository; \
             `mem sync` refuses to commit unrelated files",
            root.display(),
            canonical_top.display()
        );
    }

    // symbolic-ref works even on a freshly initialized repo with no commits.
    let branch = git_capture(root, &["symbolic-ref", "--short", "HEAD"])?
        .trim()
        .to_string();
    let remote_url = git_capture(root, &["remote", "get-url", &args.remote]).ok();

    if args.dry_run {
        let dirty = git_capture(root, &["status", "--porcelain"])?;
        let dirty_files = dirty.lines().count();
        return print_json_pretty(&json!({
            "status": "dry_run",
            "root": root.display().to_string(),
            "branch": branch,
            "remote": remote_url.as_deref().map(str::trim),
            "dirty_files": dirty_files
        }));
    }

    ensure_gitignore(root)?;

    git_run(root, &["add", "-A"])?;
    let staged = !git_ok(root, &["diff", "--cached", "--quiet"])?;
    let mut committed = false;
    if staged {
        let message = args
            .message
            .clone()
            .unwrap_or_else(|| format!("mem sync: {}", now()));
        git_run(root, &["commit", "-m", &message])?;
        committed = true;
    }

    let Some(_) = remote_url else {
        return print_json_pretty(&json!({
            "status": "local_only",
            "committed": committed,
            "detail": format!("no `{}` remote; add one to enable pull/push", args.remote)
        }));
    };

    git_run(root, &["fetch", &args.remote])?;
    let remote_ref = format!("{}/{}", args.remote, branch);
    let remote_exists = git_ok(root, &["rev-parse", "--verify", "--quiet", &remote_ref])?;

    let mut merge_report = Value::Null;
    let mut pulled = false;
    let head_exists = git_ok(root, &["rev-parse", "--verify", "--quiet", "HEAD"])?;
    if remote_exists && !head_exists {
        // Empty local history: adopt the remote branch as-is.
        git_run(root, &["pull", &args.remote, &branch])?;
        pulled = true;
    } else if remote_exists {
        let behind: usize = git_capture(
            root,
            &["rev-list", "--count", &format!("HEAD..{remote_ref}")],
        )?
        .trim()
        .parse()
        .unwrap_or(0);
        if behind > 0 {
            pulled = true;
            if !git_ok(root, &["merge", "--no-edit", &remote_ref])? {
                merge_report = resolve_db_conflict(app, root, &remote_ref)?;
            }
        }
    }

    let mut pushed = false;
    if !args.no_push {
        let push_args: Vec<&str> = if remote_exists {
            vec!["push", &args.remote, &branch]
        } else {
            vec!["push", "-u", &args.remote, &branch]
        };
        git_run(root, &push_args)?;
        pushed = true;
    }

    print_json_pretty(&json!({
        "status": "synced",
        "branch": branch,
        "committed": committed,
        "pulled": pulled,
        "merge": merge_report,
        "pushed": pushed
    }))
}

fn resolve_db_conflict(app: &App, root: &Path, remote_ref: &str) -> Result<Value> {
    let conflicted = git_capture(root, &["diff", "--name-only", "--diff-filter=U"])?;
    let files: Vec<&str> = conflicted.lines().filter(|line| !line.is_empty()).collect();
    if files.iter().any(|file| *file != "memory.db") {
        git_run(root, &["merge", "--abort"])?;
        bail!(
            "merge conflicts beyond memory.db ({}); resolve them manually in {} and rerun",
            files.join(", "),
            root.display()
        );
    }
    if files.is_empty() {
        git_run(root, &["merge", "--abort"])?;
        bail!(
            "merge failed without conflicted files; resolve manually in {}",
            root.display()
        );
    }

    let theirs_path = root.join(".mem-sync-theirs.db");
    let theirs_bytes = git_capture_bytes(root, &["show", &format!("{remote_ref}:memory.db")])?;
    fs::write(&theirs_path, theirs_bytes).context("write remote memory.db snapshot")?;
    git_run(root, &["checkout", "--ours", "memory.db"])?;
    git_run(root, &["add", "memory.db"])?;

    let report = merge_database(app, &theirs_path, false);
    fs::remove_file(&theirs_path).ok();
    let report = report?;

    git_run(root, &["add", "-A"])?;
    git_run(root, &["commit", "--no-edit"])?;
    Ok(report)
}

fn ensure_gitignore(root: &Path) -> Result<()> {
    let path = root.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut updated = existing.clone();
    for line in GITIGNORE_LINES {
        if !existing.lines().any(|current| current.trim() == *line) {
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push_str(line);
            updated.push('\n');
        }
    }
    if updated != existing {
        fs::write(&path, updated).context("update store .gitignore")?;
    }
    Ok(())
}

fn git_command(root: &Path, args: &[&str]) -> ProcessCommand {
    let mut command = ProcessCommand::new("git");
    command.arg("-C").arg(root).args(args);
    command
}

fn git_run(root: &Path, args: &[&str]) -> Result<()> {
    let output = git_command(root, args).output().context("run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn git_ok(root: &Path, args: &[&str]) -> Result<bool> {
    let output = git_command(root, args).output().context("run git")?;
    Ok(output.status.success())
}

fn git_capture(root: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(root, args).output().context("run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn git_capture_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = git_command(root, args).output().context("run git")?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}
