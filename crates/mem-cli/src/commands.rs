use std::fs;
use std::io::{self as std_io, Result as IoResult, Seek, SeekFrom, Write, stderr, stdout};
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{Duration, Utc};
use rusqlite::{Connection, params};
use serde::Serialize;
use serde_json::{Value, json};
use tempfile::{SpooledTempFile, spooled_tempfile};

use crate::args::*;
use crate::cli_error::committed_index_error;
use mem_core::app::App;
use mem_core::atomic_file::atomic_write;
use mem_core::db::*;
use mem_core::index as memory_index;
use mem_core::scope;
use mem_core::util::*;
use mem_core::workflow as workflow_core;

pub(crate) const CLI_OUTPUT_CONTRACT_VERSION: u64 = 1;
pub(crate) const BENCHMARK_REPORT_CONTRACT_VERSION: u64 = 1;

static MAX_OUTPUT_BYTES: OnceLock<Option<u64>> = OnceLock::new();
const OUTPUT_SPOOL_MEMORY_BYTES: usize = 1024 * 1024;

struct BoundedWriter<W> {
    inner: W,
    limit: Option<u64>,
    written: u64,
}

impl<W> BoundedWriter<W> {
    fn new(inner: W, limit: Option<u64>) -> Self {
        Self {
            inner,
            limit,
            written: 0,
        }
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> IoResult<usize> {
        let requested = u64::try_from(buffer.len()).unwrap_or(u64::MAX);
        let attempted = self.written.saturating_add(requested);
        if let Some(limit) = self.limit
            && attempted > limit
        {
            return Err(std_io::Error::other(format!(
                "rendered output is at least {attempted} bytes, exceeding --max-bytes {limit}"
            )));
        }
        let written = self.inner.write(buffer)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> IoResult<()> {
        self.inner.flush()
    }
}

pub(crate) enum Output {
    Json,
    Text,
}

impl Output {
    pub(crate) fn json<T: Serialize + ?Sized>(self, value: &T) -> Result<()> {
        match self {
            Self::Json => write_stdout_rendered(|writer| {
                serde_json::to_writer(&mut *writer, value)?;
                writer.write_all(b"\n")?;
                Ok(())
            }),
            Self::Text => bail!("cannot render JSON value as text"),
        }
    }

    pub(crate) fn json_pretty<T: Serialize + ?Sized>(self, value: &T) -> Result<()> {
        match self {
            Self::Json => write_stdout_rendered(|writer| {
                serde_json::to_writer_pretty(&mut *writer, value)?;
                writer.write_all(b"\n")?;
                Ok(())
            }),
            Self::Text => bail!("cannot render JSON value as text"),
        }
    }

    pub(crate) fn text(self, value: impl AsRef<str>) -> Result<()> {
        match self {
            Self::Text => write_stdout_rendered(|writer| {
                writer.write_all(value.as_ref().as_bytes())?;
                Ok(())
            }),
            Self::Json => bail!("cannot render text value as JSON"),
        }
    }
}

pub(crate) fn configure_output_limit(max_bytes: Option<u64>) -> Result<()> {
    MAX_OUTPUT_BYTES
        .set(max_bytes)
        .map_err(|_| anyhow!("output limit was configured more than once"))
}

fn render_to_spool(
    limit: Option<u64>,
    render: impl FnOnce(&mut dyn Write) -> Result<()>,
) -> Result<SpooledTempFile> {
    let mut spool = spooled_tempfile(OUTPUT_SPOOL_MEMORY_BYTES);
    {
        let mut writer = BoundedWriter::new(&mut spool, limit);
        render(&mut writer)?;
        writer.flush()?;
    }
    spool.seek(SeekFrom::Start(0))?;
    Ok(spool)
}

fn write_stdout_rendered(render: impl FnOnce(&mut dyn Write) -> Result<()>) -> Result<()> {
    let limit = MAX_OUTPUT_BYTES.get().copied().flatten();
    let mut spool = render_to_spool(limit, render)?;
    let mut stdout = stdout().lock();
    std_io::copy(&mut spool, &mut stdout)?;
    stdout.flush()?;
    Ok(())
}

pub(crate) fn write_stderr_line(value: &str) -> IoResult<()> {
    let mut stderr = stderr().lock();
    writeln!(stderr, "{value}")?;
    stderr.flush()
}

pub(crate) fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    Output::Json.json(value)
}

/// Print a write-command response. Store selection is explicit/runtime-only;
/// use `mem config show` when the target needs to be audited.
pub(crate) fn print_write_json(_app: &App, value: Value) -> Result<()> {
    print_json(&value)
}

/// Pretty variant of `print_write_json` for multi-record responses.
pub(crate) fn print_write_json_pretty(_app: &App, value: Value) -> Result<()> {
    print_json_pretty(&value)
}

pub(crate) fn print_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    Output::Json.json_pretty(value)
}

pub(crate) fn print_text(value: impl AsRef<str>) -> Result<()> {
    Output::Text.text(value)
}

pub(crate) fn finish_committed_index_write<T>(
    result: Result<T>,
    operation: &str,
    details: Value,
) -> Result<T> {
    result.map_err(|error| committed_index_error(operation, details, error))
}

pub(crate) fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths = headers
        .iter()
        .map(|header| header.len())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(value.len());
            }
        }
    }

    let mut output = String::new();
    push_table_row(&mut output, headers, &widths);
    let separator = widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>();
    push_table_row(&mut output, &separator, &widths);
    for row in rows {
        push_table_row(&mut output, row, &widths);
    }
    output
}

pub(crate) fn truncate_text(value: &str, max_chars: usize) -> String {
    let trimmed = value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect::<String>();
    let mut chars = trimmed.chars();
    let mut output = String::new();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return output;
        };
        output.push(ch);
    }
    if chars.next().is_some() {
        output.push_str("...");
    }
    output
}

fn push_table_row<T: AsRef<str>>(output: &mut String, row: &[T], widths: &[usize]) {
    for (index, value) in row.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        let text = value.as_ref();
        let width = widths.get(index).copied().unwrap_or(text.len());
        output.push_str(&format!("{text:<width$}"));
    }
    output.push('\n');
}

mod admin;
mod ambiguity;
mod artifact;
mod bundle;
mod doctor;
mod graph;
mod io;
mod memory;
mod merge;
mod prime;
mod query;
mod reconcile;
mod retro;
mod setup;
mod sync;
mod workflow;

pub(crate) use admin::{
    audit_report, cmd_audit, cmd_config, cmd_context, cmd_contract, cmd_gc, cmd_history,
    cmd_migrate, cmd_operation, cmd_schema, cmd_stats, stats_report,
};
pub(crate) use ambiguity::cmd_ambiguity;
pub(crate) use artifact::cmd_artifact;
pub(crate) use bundle::cmd_bundle;
pub(crate) use doctor::cmd_doctor;
pub(crate) use graph::cmd_graph;
pub(crate) use io::{cmd_export, cmd_import};
pub(crate) use memory::{
    cmd_delete, cmd_save, cmd_supersede, cmd_update, save_memory,
    save_request_no_index_in_connection,
};
pub(crate) use merge::{cmd_merge, merge_database};
pub(crate) use prime::cmd_prime;
pub(crate) use query::cmd_query;
pub(crate) use reconcile::cmd_reconcile;
pub(crate) use retro::cmd_retro;
pub(crate) use setup::cmd_setup;
pub(crate) use sync::cmd_sync;
pub(crate) use workflow::cmd_workflow;

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::time::{SystemTime, UNIX_EPOCH};

    use mem_core::{app::StoreSource, config::Config};

    use super::*;

    fn temp_app(name: &str) -> App {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("mnemark-main-{name}-{stamp}"));
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
            store_source: StoreSource::CliOverride,
        }
    }

    #[test]
    fn output_spool_rejects_oversized_content_before_publish() {
        let error = render_to_spool(Some(4), |writer| {
            writer.write_all(b"12345")?;
            Ok(())
        })
        .expect_err("oversized output must fail");

        assert!(error.to_string().contains("exceeding --max-bytes 4"));
    }

    #[test]
    fn output_spool_keeps_small_content_in_memory_and_large_content_off_heap() {
        let mut small = render_to_spool(Some(5), |writer| {
            writer.write_all(b"1234\n")?;
            Ok(())
        })
        .expect("render small output");
        assert!(!small.is_rolled());
        let mut small_text = String::new();
        small
            .read_to_string(&mut small_text)
            .expect("read small output");
        assert_eq!(small_text, "1234\n");

        let large_bytes = vec![b'x'; OUTPUT_SPOOL_MEMORY_BYTES + 1];
        let large = render_to_spool(None, |writer| {
            writer.write_all(&large_bytes)?;
            Ok(())
        })
        .expect("render large output");
        assert!(large.is_rolled());
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
        fs::write(&app.index_path, "not a directory").expect("unsafe index path");

        let result = memory_index::upsert_or_mark_stale(&app, &conn, "broken_index");

        assert!(result.is_err());
        assert!(memory_index::is_stale(&app));
        assert!(memory_index::dirty_in_db(&app).expect("index dirty state"));
        fs::remove_dir_all(app.root).ok();
    }
}
