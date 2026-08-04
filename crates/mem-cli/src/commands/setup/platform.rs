use std::path::{Path, PathBuf};

use mem_core::config::expand_home;

pub(crate) const SHARED_SKILLS_DIR: &str = ".agents/skills";

/// One supported coding-agent platform. Paths are relative to the user's
/// home directory (overridable per call). `skills_dir: None` means the
/// platform has no known skill directory and relies on the policy block;
/// `claude_settings: None` means there is no session-start hook mechanism
/// and the policy block's contract-first, read-only prime instruction is the fallback.
pub(crate) struct PlatformSpec {
    pub(crate) name: &'static str,
    pub(crate) instructions: &'static str,
    pub(crate) skills_dir: Option<&'static str>,
    pub(crate) claude_settings: Option<&'static str>,
}

pub(crate) const PLATFORMS: &[PlatformSpec] = &[
    PlatformSpec {
        name: "claude-code",
        instructions: ".claude/CLAUDE.md",
        skills_dir: Some(".claude/skills"),
        claude_settings: Some(".claude/settings.json"),
    },
    PlatformSpec {
        name: "codex",
        instructions: ".codex/AGENTS.md",
        skills_dir: Some(".codex/skills"),
        claude_settings: None,
    },
    PlatformSpec {
        name: "pi",
        instructions: ".pi/agent/AGENTS.md",
        skills_dir: Some(SHARED_SKILLS_DIR),
        claude_settings: None,
    },
    PlatformSpec {
        name: "gemini-cli",
        instructions: ".gemini/GEMINI.md",
        skills_dir: None,
        claude_settings: None,
    },
    PlatformSpec {
        name: "opencode",
        instructions: ".config/opencode/AGENTS.md",
        skills_dir: None,
        claude_settings: None,
    },
];

pub(crate) fn platform_by_name(name: &str) -> Option<&'static PlatformSpec> {
    PLATFORMS.iter().find(|platform| platform.name == name)
}

pub(crate) fn base_dir(explicit: Option<&Path>) -> PathBuf {
    explicit
        .map(Path::to_path_buf)
        .unwrap_or_else(|| expand_home("~"))
}
