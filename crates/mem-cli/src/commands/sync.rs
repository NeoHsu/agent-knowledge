use super::*;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;

const GITIGNORE_LINES: &[&str] = &[
    "index/",
    ".mem.lock",
    "memory.db-wal",
    "memory.db-shm",
    "memory.db.backup-*",
    ".bundle-replace-backup-*",
];

/// Sync model: git moves bytes, `mem merge` resolves meaning. A concurrent
/// remote change to memory.db always conflicts in git (binary file); the
/// conflict is resolved by keeping the local db and semantically merging the
/// remote copy through the same merge logic as `mem merge`, so same-name
/// conflicts end up as ambiguity records instead of clobbered rows.
pub(crate) fn cmd_sync(app: &App, args: SyncArgs) -> Result<()> {
    validate_git_remote_name(&args.remote)?;
    if args
        .message
        .as_deref()
        .is_some_and(|message| message.len() > 10_000)
    {
        bail!("sync commit message exceeds 10000 bytes");
    }
    app.require_schema()?;
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
    validate_sync_secrets(app)?;

    if args.dry_run {
        let dirty = git_capture(root, &["status", "--porcelain"])?;
        let dirty_files = dirty.lines().count();
        return print_json_pretty(&json!({
            "status": "dry_run",
            "root": root.display().to_string(),
            "branch": branch,
            "remote": args.remote,
            "remote_configured": remote_url.is_some(),
            "dirty_files": dirty_files
        }));
    }

    // Mutating sync begins only after the dry-run return above. Fold any WAL
    // content into memory.db before git sees it so the committed file is whole.
    checkpoint_database(app, "before sync")?;
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

    let pre_pull_head = git_capture(root, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|head| head.trim().to_string());
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
                merge_report = resolve_db_conflict(app, root, &remote_ref, args.redact_secrets)?;
            }
        }
    }

    if pulled {
        if let Err(error) = validate_pulled_store(app) {
            if let Some(head) = pre_pull_head.as_deref() {
                let _ = git_run(root, &["reset", "--hard", head]);
                fs::remove_file(root.join("memory.db-wal")).ok();
                fs::remove_file(root.join("memory.db-shm")).ok();
                fs::remove_dir_all(&app.index_path).ok();
                let _ =
                    memory_index::reindex_or_mark_stale(app, "restore index after rejected pull");
            }
            return Err(error).context("reject pulled store state; restored the pre-pull checkout");
        }
        checkpoint_database(app, "after rebuilding the pulled search index")?;
        git_run(root, &["add", "memory.db"])?;
        if !git_ok(root, &["diff", "--cached", "--quiet"])? {
            git_run(
                root,
                &["commit", "-m", "mem sync: refresh pulled index state"],
            )?;
            committed = true;
        }
    }

    let mut pushed = false;
    if args.push && !args.no_push {
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

fn resolve_db_conflict(
    app: &App,
    root: &Path,
    remote_ref: &str,
    allow_secret_redaction: bool,
) -> Result<Value> {
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
    atomic_write(&theirs_path, &theirs_bytes).context("write remote memory.db snapshot")?;
    git_run(root, &["checkout", "--ours", "memory.db"])?;
    git_run(root, &["add", "memory.db"])?;

    let report = merge_database(app, &theirs_path, false, allow_secret_redaction);
    remove_temporary_database(&theirs_path);
    let report = match report {
        Ok(report) => report,
        Err(error) => {
            if let Err(abort_error) = git_run(root, &["merge", "--abort"]) {
                return Err(error).context(format!(
                    "semantic database merge failed and git merge abort also failed: {abort_error:#}"
                ));
            }
            return Err(error).context("semantic database merge failed; git merge was aborted");
        }
    };
    checkpoint_database(app, "after semantic conflict merge")?;

    git_run(root, &["add", "-A"])?;
    git_run(root, &["commit", "--no-edit"])?;
    Ok(report)
}

fn remove_temporary_database(path: &Path) {
    fs::remove_file(path).ok();
    for suffix in ["-wal", "-shm"] {
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(suffix);
        fs::remove_file(PathBuf::from(sidecar)).ok();
    }
}

fn validate_git_remote_name(remote: &str) -> Result<()> {
    if remote.is_empty()
        || remote.len() > 128
        || remote.starts_with('-')
        || !remote
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        bail!("invalid git remote name: {remote:?}");
    }
    Ok(())
}

fn validate_pulled_store(app: &App) -> Result<()> {
    app.require_schema()?;
    let conn = app.read_conn()?;
    let quick_check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if quick_check != "ok" {
        bail!("pulled memory.db failed SQLite quick_check: {quick_check}");
    }
    mem_core::db::validate_store_schema_objects(&conn)?;
    drop(conn);
    validate_sync_secrets(app)?;
    memory_index::reindex_or_mark_stale(app, "rebuild index after sync pull")
}

fn validate_sync_secrets(app: &App) -> Result<()> {
    let conn = app.read_conn()?;
    mem_core::db::validate_store_secrets(&conn)?;
    drop(conn);
    validate_sync_worktree(&app.root, &app.root)
}

fn validate_sync_worktree(root: &Path, current: &Path) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let path = entry?.path();
        let relative = path.strip_prefix(root)?;
        let first = relative.components().next();
        if current == root {
            if first.is_some_and(|component| {
                component
                    .as_os_str()
                    .to_string_lossy()
                    .starts_with(".bundle-replace-backup-")
            }) {
                bail!(
                    "stale bundle replacement backup found at {}; inspect and remove it only after confirming the active store is healthy",
                    path.display()
                );
            }
            if first.is_some_and(|component| {
                let name = component.as_os_str().to_string_lossy();
                name == ".git"
                    || name == "index"
                    || name == ".mem.lock"
                    || name == "memory.db"
                    || name.starts_with("memory.db-")
                    || name.starts_with("memory.db.backup-")
            }) {
                continue;
            }
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            bail!("refusing to sync worktree symlink: {}", path.display());
        }
        if metadata.is_dir() {
            validate_sync_worktree(root, &path)?;
        } else if metadata.is_file() {
            validate_sync_file(&path)?;
        } else {
            bail!(
                "refusing to sync non-regular worktree entry: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_sync_file(path: &Path) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!("refusing to sync non-regular file: {}", path.display());
    }
    sanitize_secret_file(path, &format!("sync file {}", path.display()), false)?;
    let bytes = fs::read(path)?;
    std::str::from_utf8(&bytes).with_context(|| {
        format!(
            "refusing to sync non-UTF-8 file {}; binary files cannot be secret-scanned safely",
            path.display()
        )
    })?;
    Ok(())
}

fn checkpoint_database(app: &App, context: &str) -> Result<()> {
    let checkpoint = app.conn()?;
    let (busy, wal_frames, checkpointed_frames): (i64, i64, i64) =
        checkpoint.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
    if busy != 0 || wal_frames != checkpointed_frames {
        bail!(
            "cannot checkpoint memory.db safely {context}: busy={busy}, \
             wal_frames={wal_frames}, checkpointed_frames={checkpointed_frames}; retry after \
             other writers exit"
        );
    }
    Ok(())
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
        atomic_write(&path, updated.as_bytes()).context("update store .gitignore")?;
    }
    Ok(())
}

fn git_command(root: &Path, args: &[&str]) -> ProcessCommand {
    let mut hooks_path = OsString::from("core.hooksPath=");
    hooks_path.push(root.join(".git/mnemark-disabled-hooks"));
    let mut command = ProcessCommand::new("git");
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .arg("-c")
        .arg(hooks_path)
        .arg("-c")
        .arg("commit.gpgSign=false")
        .arg("-c")
        .arg("tag.gpgSign=false")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(root)
        .args(args);
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
