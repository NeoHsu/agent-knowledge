use std::collections::BTreeSet;

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::error;
use crate::util::sanitize_secret_field;

use super::introspection::table_exists;

const VALIDATED_TEXT_COLUMNS: &[(&str, &[&str])] = &[
    (
        "memories",
        &[
            "id",
            "type",
            "name",
            "description",
            "content",
            "tags",
            "scope",
            "source",
            "confidence",
            "why",
            "origin",
            "origin_ref",
            "superseded_by",
        ],
    ),
    (
        "ambiguities",
        &["uid", "query", "memory_ids", "context", "resolution"],
    ),
    (
        "changelog",
        &[
            "uid",
            "memory_id",
            "action",
            "old_content",
            "new_content",
            "source",
        ],
    ),
    (
        "workflow_runs",
        &["uid", "memory_id", "result", "note", "source"],
    ),
    (
        "graph_semantic_edges",
        &[
            "id",
            "source_ref",
            "target_ref",
            "relation",
            "evidence",
            "rationale",
            "source_spans",
            "tags",
            "generated_by",
            "source",
        ],
    ),
    (
        "graph_semantic_edge_revisions",
        &["uid", "edge_id", "action", "snapshot", "source"],
    ),
    ("metadata", &["key", "value"]),
    (
        "graph_nodes",
        &[
            "id",
            "kind",
            "label",
            "ref_table",
            "ref_id",
            "scope",
            "metadata",
            "origin",
        ],
    ),
    (
        "graph_edges",
        &[
            "id",
            "source_node_id",
            "target_node_id",
            "relation",
            "confidence",
            "status",
            "evidence",
            "source_ref",
            "scope",
            "origin",
            "metadata",
        ],
    ),
];

const REDACTABLE_TEXT_COLUMNS: &[(&str, &[&str])] = &[
    (
        "memories",
        &[
            "name",
            "description",
            "content",
            "tags",
            "scope",
            "why",
            "origin_ref",
        ],
    ),
    ("ambiguities", &["query", "context", "resolution"]),
    ("changelog", &["old_content", "new_content"]),
    ("workflow_runs", &["note"]),
    (
        "graph_semantic_edges",
        &["evidence", "rationale", "source_spans", "tags"],
    ),
    ("graph_semantic_edge_revisions", &["snapshot"]),
    ("metadata", &["value"]),
];

/// Reject a store containing secret-like values in any durable text field.
///
/// This is deliberately read-only and is suitable for validating an incoming
/// merge or bundle before any destination mutation occurs.
pub fn validate_store_schema_objects(conn: &Connection) -> Result<()> {
    const TABLES: &[&str] = &[
        "memories",
        "ambiguities",
        "changelog",
        "workflow_runs",
        "metadata",
        "graph_semantic_edges",
        "graph_semantic_edge_revisions",
        "graph_nodes",
        "graph_edges",
    ];
    const REQUIRED_TABLES: &[&str] = &[
        "memories",
        "ambiguities",
        "changelog",
        "workflow_runs",
        "metadata",
        "graph_semantic_edges",
        "graph_semantic_edge_revisions",
    ];
    const UID_TRIGGERS: &[(&str, &str, &str)] = &[
        ("enforce_ambiguities_uid_insert", "ambiguities", "INSERT"),
        (
            "enforce_ambiguities_uid_update",
            "ambiguities",
            "UPDATE OF uid",
        ),
        ("enforce_changelog_uid_insert", "changelog", "INSERT"),
        ("enforce_changelog_uid_update", "changelog", "UPDATE OF uid"),
        (
            "enforce_workflow_runs_uid_insert",
            "workflow_runs",
            "INSERT",
        ),
        (
            "enforce_workflow_runs_uid_update",
            "workflow_runs",
            "UPDATE OF uid",
        ),
        (
            "enforce_graph_semantic_edge_revisions_uid_insert",
            "graph_semantic_edge_revisions",
            "INSERT",
        ),
        (
            "enforce_graph_semantic_edge_revisions_uid_update",
            "graph_semantic_edge_revisions",
            "UPDATE OF uid",
        ),
    ];

    let mut stmt = conn.prepare(
        "SELECT type, name, sql FROM sqlite_master
         WHERE type IN ('table', 'view', 'trigger') AND name NOT LIKE 'sqlite_%'
         ORDER BY type, name",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut found_tables = BTreeSet::new();
    let mut found_triggers = BTreeSet::new();
    for row in rows {
        let (kind, name, sql) = row?;
        match kind.as_str() {
            "table" if TABLES.contains(&name.as_str()) => {
                found_tables.insert(name);
            }
            "trigger" => {
                let Some((_, table, action)) = UID_TRIGGERS
                    .iter()
                    .find(|(expected, _, _)| *expected == name)
                else {
                    return Err(error::integrity(format!(
                        "store contains unexpected trigger: {name}"
                    )));
                };
                let expected = format!(
                    "CREATE TRIGGER {name} BEFORE {action} ON {table} \
                     WHEN NEW.uid IS NULL OR NEW.uid = '' \
                     BEGIN SELECT RAISE(ABORT, 'durable event uid is required'); END"
                );
                let actual = sql
                    .ok_or_else(|| error::integrity(format!("store trigger {name} has no SQL")))?;
                if normalize_schema_sql(&actual) != normalize_schema_sql(&expected) {
                    return Err(error::integrity(format!(
                        "store trigger definition is not trusted: {name}"
                    )));
                }
                found_triggers.insert(name);
            }
            _ => {
                return Err(error::integrity(format!(
                    "store contains unexpected {kind}: {name}"
                )));
            }
        }
    }
    if REQUIRED_TABLES
        .iter()
        .any(|name| !found_tables.contains(*name))
    {
        return Err(error::integrity(
            "store is missing durable schema-v5 tables",
        ));
    }
    let expected_triggers = UID_TRIGGERS
        .iter()
        .map(|(name, _, _)| (*name).to_string())
        .collect();
    if found_triggers != expected_triggers {
        return Err(error::integrity(
            "store trigger set does not match schema v5",
        ));
    }
    Ok(())
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

pub fn validate_store_secrets(conn: &Connection) -> Result<()> {
    for (table, columns) in VALIDATED_TEXT_COLUMNS {
        for_each_text_value(conn, table, columns, |value| {
            sanitize_secret_field(&value.value, &value.field, false).map(|_| ())
        })?;
    }
    Ok(())
}

/// Redact secret-like values across durable text fields in one transaction.
/// Returns the number of individual fields changed.
pub fn redact_store_secrets(conn: &mut Connection) -> Result<usize> {
    // Identity/reference fields cannot be replaced safely without a full ID
    // remap. Reject those even in explicit redaction mode. Rebuildable graph
    // materialization is cleared below instead.
    for (table, columns) in VALIDATED_TEXT_COLUMNS {
        if matches!(*table, "graph_nodes" | "graph_edges") {
            continue;
        }
        for_each_text_value(conn, table, columns, |value| {
            if !is_redactable(table, &value.column) {
                sanitize_secret_field(&value.value, &value.field, false)?;
            }
            Ok(())
        })?;
    }

    let tx = conn.transaction()?;
    // Materialized graph rows are rebuildable and may carry secret-like node
    // identities that cannot be safely renamed in place.
    if table_exists(&tx, "graph_edges")? {
        tx.execute("DELETE FROM graph_edges", [])?;
    }
    if table_exists(&tx, "graph_nodes")? {
        tx.execute("DELETE FROM graph_nodes", [])?;
    }
    let mut changes = 0;
    for (table, columns) in REDACTABLE_TEXT_COLUMNS {
        for_each_text_value(&tx, table, columns, |value| {
            let redacted = sanitize_secret_field(&value.value, &value.field, true)?;
            if redacted != value.value {
                let sql = format!(
                    "UPDATE {} SET {} = ?1 WHERE rowid = ?2",
                    quote_identifier(table),
                    quote_identifier(&value.column)
                );
                tx.execute(&sql, params![redacted, value.rowid])
                    .with_context(|| format!("redact {}.{}", table, value.column))?;
                changes += 1;
            }
            Ok(())
        })?;
    }
    tx.commit()?;
    Ok(changes)
}

#[derive(Debug)]
struct TextValue {
    rowid: i64,
    column: String,
    field: String,
    value: String,
}

fn is_redactable(table: &str, column: &str) -> bool {
    REDACTABLE_TEXT_COLUMNS
        .iter()
        .any(|(candidate, columns)| *candidate == table && columns.contains(&column))
}

fn for_each_text_value<F>(
    conn: &Connection,
    table: &str,
    columns: &[&str],
    mut visit: F,
) -> Result<()>
where
    F: FnMut(TextValue) -> Result<()>,
{
    if !table_exists(conn, table)? {
        return Ok(());
    }
    let available = table_columns(conn, table)?;
    let columns = columns
        .iter()
        .copied()
        .filter(|column| available.iter().any(|candidate| candidate == column))
        .collect::<Vec<_>>();
    if columns.is_empty() {
        return Ok(());
    }
    let selected = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "SELECT rowid, {selected} FROM {} ORDER BY rowid",
        quote_identifier(table)
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let rowid = row.get::<_, i64>(0)?;
        let values = columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                row.get::<_, Option<String>>(index + 1)
                    .map(|value| (column.to_string(), value))
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok((rowid, values))
    })?;
    for row in rows {
        let (rowid, values) = row?;
        for (column, value) in values {
            if let Some(value) = value {
                visit(TextValue {
                    rowid,
                    field: format!("{table}.{column}"),
                    column,
                    value,
                })?;
            }
        }
    }
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
