// index_state.rs — stale-tracking via SQLite metadata only.
//
// The previous implementation also wrote a `.stale` file to the filesystem
// (dual tracking). That file-based side effect has been removed; staleness is
// now authoritative from the database `index_dirty` key (see db/metadata.rs).
//
// The `mark_stale` / `clear_stale` / `is_stale` functions in `index.rs` call
// `db::set_index_dirty` / `db::index_dirty` directly. This module is retained
// only so that existing call-sites in `index.rs` compile without further edits.

use std::path::PathBuf;

use anyhow::Result;

/// Retained for tests that previously tested the file-based marker.
/// These are kept as no-ops so compilation is not broken.
#[allow(dead_code)]
pub fn marker_path(_index_path: &std::path::Path) -> PathBuf {
    // No longer used — staleness is tracked in SQLite.
    PathBuf::new()
}

/// Returns `false` always; staleness is now read from the DB via `index.rs`.
#[allow(dead_code)]
pub fn is_stale(_index_path: &std::path::Path) -> bool {
    false
}

/// No-op: staleness is now written to SQLite via `index.rs`.
#[allow(dead_code)]
pub fn mark_stale(_index_path: &std::path::Path, _reason: &str, _marked_at: &str) -> Result<()> {
    Ok(())
}

/// No-op: staleness is now cleared in SQLite via `index.rs`.
#[allow(dead_code)]
pub fn clear_stale(_index_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mark_stale_is_noop() {
        let index = std::env::temp_dir().join("mnemark-index-state-noop");
        mark_stale(&index, "index writer failed", "2026-05-27T00:00:00Z").expect("mark stale");
        // File-based marker is no longer written; is_stale always returns false.
        assert!(!is_stale(&index));
    }

    #[test]
    fn clear_stale_is_idempotent() {
        let index = std::env::temp_dir().join("mnemark-index-state-noop2");
        mark_stale(&index, "stale", "2026-05-27T00:00:00Z").expect("mark stale");
        clear_stale(&index).expect("clear stale");
        clear_stale(&index).expect("clear missing stale marker");
        assert!(!is_stale(&index));
    }
}
