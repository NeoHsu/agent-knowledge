use super::super::*;

/// Skill files embedded at build time so `mem setup <platform>` installs a
/// skill version that always matches the binary.
pub(crate) const SKILL_FILES: &[(&str, &str)] = &[
    (
        "SKILL.md",
        include_str!("../../../../../skills/mnemark/SKILL.md"),
    ),
    (
        "compatibility.json",
        include_str!("../../../../../skills/mnemark/compatibility.json"),
    ),
    (
        "references/cli-guide.md",
        include_str!("../../../../../skills/mnemark/references/cli-guide.md"),
    ),
    (
        "references/tag-rules.md",
        include_str!("../../../../../skills/mnemark/references/tag-rules.md"),
    ),
    (
        "references/workflow-rules.md",
        include_str!("../../../../../skills/mnemark/references/workflow-rules.md"),
    ),
    (
        "references/graph-rules.md",
        include_str!("../../../../../skills/mnemark/references/graph-rules.md"),
    ),
    (
        "references/memory-quality.md",
        include_str!("../../../../../skills/mnemark/references/memory-quality.md"),
    ),
    (
        "references/daily-retro.md",
        include_str!("../../../../../skills/mnemark/references/daily-retro.md"),
    ),
    (
        "references/weekly-retro.md",
        include_str!("../../../../../skills/mnemark/references/weekly-retro.md"),
    ),
];

fn install_skill_files(skill_root: &Path, dry_run: bool) -> Result<Value> {
    let mut written = Vec::new();
    let mut unchanged = Vec::new();
    for (rel, content) in SKILL_FILES {
        let path = skill_root.join(rel);
        let current = fs::read_to_string(&path).ok();
        if current.as_deref() == Some(*content) {
            unchanged.push(*rel);
            continue;
        }
        written.push(*rel);
        if dry_run {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create skill directory {}", parent.display()))?;
        }
        fs::write(&path, content).with_context(|| format!("write {}", path.display()))?;
    }
    Ok(json!({
        "status": if written.is_empty() { "up_to_date" } else if dry_run { "dry_run" } else { "installed" },
        "root": skill_root.display().to_string(),
        "written": written,
        "unchanged": unchanged
    }))
}

pub(crate) fn skill_files_current(skill_root: &Path) -> bool {
    SKILL_FILES.iter().all(|(rel, expected)| {
        fs::read_to_string(skill_root.join(rel)).ok().as_deref() == Some(*expected)
    })
}

pub(super) fn install_shared_skill(
    shared_root: &Path,
    platform_root: Option<&Path>,
    dry_run: bool,
) -> Result<Value> {
    let canonical = install_skill_files(shared_root, dry_run)?;
    let platform = match platform_root {
        Some(root) if root == shared_root => json!({
            "status": "shared",
            "root": root.display().to_string()
        }),
        Some(root) => ensure_skill_link(root, shared_root, dry_run)?,
        None => json!({
            "status": "unsupported",
            "detail": "platform has no known skill directory; the policy block carries the protocol"
        }),
    };
    let canonical_status = canonical.get("status").and_then(Value::as_str);
    let platform_status = platform.get("status").and_then(Value::as_str);
    let status = if platform_status == Some("conflict") {
        "conflict"
    } else if platform_status == Some("unsupported") {
        "unsupported"
    } else if canonical_status == Some("dry_run") || platform_status == Some("dry_run") {
        "dry_run"
    } else if canonical_status == Some("installed")
        || matches!(platform_status, Some("linked" | "migrated"))
    {
        "installed"
    } else {
        "up_to_date"
    };
    Ok(json!({
        "status": status,
        "root": shared_root.display().to_string(),
        "canonical": canonical,
        "platform": platform
    }))
}

fn ensure_skill_link(link_root: &Path, shared_root: &Path, dry_run: bool) -> Result<Value> {
    match fs::symlink_metadata(link_root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            if skill_link_points_to(link_root, shared_root) {
                Ok(json!({
                    "status": "up_to_date",
                    "root": link_root.display().to_string(),
                    "target": shared_root.display().to_string()
                }))
            } else {
                Ok(json!({
                    "status": "conflict",
                    "root": link_root.display().to_string(),
                    "detail": "existing skill symlink points somewhere else",
                    "target": shared_root.display().to_string()
                }))
            }
        }
        Ok(metadata) if metadata.is_dir() => {
            if !directory_contains_only_managed_skill_files(link_root, link_root)? {
                return Ok(json!({
                    "status": "conflict",
                    "root": link_root.display().to_string(),
                    "detail": "existing skill directory contains unmanaged files; move them before linking",
                    "target": shared_root.display().to_string()
                }));
            }
            if dry_run {
                return Ok(json!({
                    "status": "dry_run",
                    "action": "migrate_copy_to_symlink",
                    "root": link_root.display().to_string(),
                    "target": shared_root.display().to_string()
                }));
            }
            fs::remove_dir_all(link_root)
                .with_context(|| format!("remove managed skill copy {}", link_root.display()))?;
            create_skill_symlink(shared_root, link_root)?;
            Ok(json!({
                "status": "migrated",
                "root": link_root.display().to_string(),
                "target": shared_root.display().to_string()
            }))
        }
        Ok(_) => Ok(json!({
            "status": "conflict",
            "root": link_root.display().to_string(),
            "detail": "existing skill path is neither a directory nor a symlink",
            "target": shared_root.display().to_string()
        })),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if dry_run {
                return Ok(json!({
                    "status": "dry_run",
                    "action": "create_symlink",
                    "root": link_root.display().to_string(),
                    "target": shared_root.display().to_string()
                }));
            }
            if let Some(parent) = link_root.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("create skill link directory {}", parent.display()))?;
            }
            create_skill_symlink(shared_root, link_root)?;
            Ok(json!({
                "status": "linked",
                "root": link_root.display().to_string(),
                "target": shared_root.display().to_string()
            }))
        }
        Err(error) => Err(error).with_context(|| format!("inspect {}", link_root.display())),
    }
}

fn directory_contains_only_managed_skill_files(root: &Path, dir: &Path) -> Result<bool> {
    for entry in fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("inspect skill path {}", path.display()))?;
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let managed_directory = SKILL_FILES
                .iter()
                .any(|(managed, _)| Path::new(managed).starts_with(relative));
            if !managed_directory || !directory_contains_only_managed_skill_files(root, &path)? {
                return Ok(false);
            }
        } else if file_type.is_symlink()
            || !SKILL_FILES
                .iter()
                .any(|(managed, _)| Path::new(managed) == relative)
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn paths_equivalent(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .and_then(|left| fs::canonicalize(right).map(|right| left == right))
            .unwrap_or(false)
}

pub(crate) fn skill_link_points_to(link_root: &Path, shared_root: &Path) -> bool {
    let Ok(target) = fs::read_link(link_root) else {
        return false;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        link_root
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(target)
    };
    paths_equivalent(&resolved, shared_root)
}

#[cfg(unix)]
fn create_skill_symlink(target: &Path, link: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, link)
        .with_context(|| format!("link {} -> {}", link.display(), target.display()))
}

#[cfg(windows)]
fn create_skill_symlink(target: &Path, link: &Path) -> Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
        .with_context(|| format!("link {} -> {}", link.display(), target.display()))
}
