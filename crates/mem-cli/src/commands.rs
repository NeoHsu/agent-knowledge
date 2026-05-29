use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;
use serde_json::{json, Value};

use crate::args::*;
use mem_core::app::App;
use mem_core::db::*;
use mem_core::index as memory_index;
use mem_core::scope;
use mem_core::util::*;
use mem_core::workflow as workflow_core;

pub(crate) fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

pub(crate) fn print_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

mod admin;
mod ambiguity;
mod io;
mod memory;
mod merge;
mod query;
mod retro;
mod workflow;

pub(crate) use admin::{
    audit_report, cmd_audit, cmd_config, cmd_context, cmd_gc, cmd_history, cmd_stats, stats_report,
};
pub(crate) use ambiguity::cmd_ambiguity;
pub(crate) use io::{cmd_export, cmd_import};
pub(crate) use memory::{
    cmd_delete, cmd_save, cmd_supersede, cmd_update, save_memory, save_memory_no_index,
};
pub(crate) use merge::cmd_merge;
pub(crate) use query::cmd_query;
pub(crate) use retro::cmd_retro;
pub(crate) use workflow::cmd_workflow;

#[cfg(test)]
mod tests {
    use super::*;
    use mem_core::{app::StoreSource, config::Config};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_app(name: &str) -> App {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("agent-knowledge-main-{name}-{stamp}"));
        fs::create_dir_all(root.join("schema")).expect("schema dir");
        fs::write(
            root.join("schema/memory-schema.sql"),
            include_str!("../../../schema/memory-schema.sql"),
        )
        .expect("schema");
        App {
            db_path: root.join("memory.db"),
            index_path: root.join("index"),
            root,
            config: Config::default(),
            store_source: StoreSource::CurrentDirectory,
        }
    }

    #[test]
    fn upsert_index_failure_marks_stale() {
        let app = temp_app("upsert-stale");
        app.init().expect("init app");
        let conn = app.conn().expect("open db");
        conn.execute(
            "INSERT INTO memories
            (id, type, name, content, tags, scope, source, confidence, protected, created_at, updated_at)
            VALUES ('broken_index', 'reference', 'broken_index', 'content', '[]', 'global', 'manual', 'high', 1, '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
            [],
        )
        .expect("insert memory");

        fs::remove_dir_all(&app.index_path).expect("remove index");
        fs::create_dir_all(&app.index_path).expect("index dir");
        fs::write(
            app.index_path.join("meta.json"),
            "not valid tantivy metadata",
        )
        .expect("corrupt index");

        let result = memory_index::upsert_or_mark_stale(&app, &conn, "broken_index");

        assert!(result.is_err());
        assert!(memory_index::is_stale(&app));
        assert!(memory_index::dirty_in_db(&app).expect("index dirty state"));
        fs::remove_dir_all(app.root).ok();
    }
}
