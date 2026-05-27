use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

const INDEX_STALE_MARKER: &str = ".stale";

pub fn marker_path(index_path: &Path) -> PathBuf {
    index_path.join(INDEX_STALE_MARKER)
}

pub fn is_stale(index_path: &Path) -> bool {
    marker_path(index_path).exists()
}

pub fn mark_stale(index_path: &Path, reason: &str, marked_at: &str) -> Result<()> {
    fs::create_dir_all(index_path)?;
    let marker = json!({
        "status": "stale",
        "reason": reason,
        "marked_at": marked_at
    });
    fs::write(marker_path(index_path), serde_json::to_vec_pretty(&marker)?)?;
    Ok(())
}

pub fn clear_stale(index_path: &Path) -> Result<()> {
    match fs::remove_file(marker_path(index_path)) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).context("clear stale index marker"),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::Value;

    use super::*;

    fn temp_index(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("agent-knowledge-index-state-{name}-{stamp}"))
    }

    #[test]
    fn mark_stale_writes_structured_marker() {
        let index = temp_index("mark");

        mark_stale(&index, "index writer failed", "2026-05-27T00:00:00Z").expect("mark stale");

        assert!(is_stale(&index));
        let marker: Value =
            serde_json::from_slice(&fs::read(marker_path(&index)).expect("read stale marker"))
                .expect("marker json");
        assert_eq!(marker["status"], "stale");
        assert_eq!(marker["reason"], "index writer failed");
        assert_eq!(marker["marked_at"], "2026-05-27T00:00:00Z");

        fs::remove_dir_all(index).ok();
    }

    #[test]
    fn clear_stale_is_idempotent() {
        let index = temp_index("clear");
        mark_stale(&index, "stale", "2026-05-27T00:00:00Z").expect("mark stale");

        clear_stale(&index).expect("clear stale");
        clear_stale(&index).expect("clear missing stale marker");

        assert!(!is_stale(&index));
        fs::remove_dir_all(index).ok();
    }
}
