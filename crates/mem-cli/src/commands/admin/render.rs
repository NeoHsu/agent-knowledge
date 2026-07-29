use super::super::*;

pub(super) fn render_history_table(rows: &[Value]) -> String {
    let table_rows = rows
        .iter()
        .map(|row| {
            vec![
                value_text(row, "id"),
                value_text(row, "action"),
                truncate_text(&value_text(row, "memory_id"), 32),
                value_text(row, "source"),
                truncate_text(&value_text(row, "created_at"), 20),
            ]
        })
        .collect::<Vec<_>>();
    render_table(
        &["id", "action", "memory_id", "source", "created"],
        &table_rows,
    )
}

pub(super) fn render_history_compact(rows: &[Value]) -> String {
    let mut output = String::new();
    for row in rows {
        output.push_str(&format!(
            "{} {} {} source={}",
            value_text(row, "created_at"),
            value_text(row, "action"),
            value_text(row, "memory_id"),
            value_text(row, "source")
        ));
        output.push('\n');
        let old_content = value_text(row, "old_content");
        if !old_content.is_empty() {
            output.push_str(&format!("  old: {}\n", truncate_text(&old_content, 120)));
        }
        let new_content = value_text(row, "new_content");
        if !new_content.is_empty() {
            output.push_str(&format!("  new: {}\n", truncate_text(&new_content, 120)));
        }
    }
    output
}

pub(super) fn render_stats_table(report: &Value) -> String {
    let mut rows = vec![vec![
        "total_active".to_string(),
        value_text(report, "total_active"),
    ]];
    append_count_rows(&mut rows, report, "by_type", "type");
    append_count_rows(&mut rows, report, "by_scope", "scope");
    append_count_rows(&mut rows, report, "by_confidence", "confidence");
    let mut output = render_table(&["metric", "value"], &rows);

    let top_rows = report
        .get("top_accessed")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| {
                    vec![
                        truncate_text(&value_text(row, "name"), 32),
                        value_text(row, "access_count"),
                        value_text(row, "last_accessed_at"),
                    ]
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if !top_rows.is_empty() {
        output.push('\n');
        output.push_str("top_accessed\n");
        output.push_str(&render_table(
            &["name", "access", "last_accessed"],
            &top_rows,
        ));
    }
    output
}

pub(super) fn render_stats_compact(report: &Value) -> String {
    let mut output = String::new();
    output.push_str(&format!(
        "total_active: {}\n",
        value_text(report, "total_active")
    ));
    output.push_str(&format!(
        "by_type: {}",
        count_map_text(report.get("by_type")).unwrap_or_else(|| "-".to_string())
    ));
    output.push('\n');
    output.push_str(&format!(
        "by_scope: {}",
        count_map_text(report.get("by_scope")).unwrap_or_else(|| "-".to_string())
    ));
    output.push('\n');
    output.push_str(&format!(
        "by_confidence: {}",
        count_map_text(report.get("by_confidence")).unwrap_or_else(|| "-".to_string())
    ));
    output.push('\n');
    if let Some(rows) = report.get("top_accessed").and_then(Value::as_array) {
        if !rows.is_empty() {
            output.push_str("top_accessed:\n");
            for row in rows {
                output.push_str(&format!(
                    "  {} access={} last={}",
                    value_text(row, "name"),
                    value_text(row, "access_count"),
                    value_text(row, "last_accessed_at")
                ));
                output.push('\n');
            }
        }
    }
    output
}

fn append_count_rows(rows: &mut Vec<Vec<String>>, report: &Value, key: &str, prefix: &str) {
    let Some(map) = report.get(key).and_then(Value::as_object) else {
        return;
    };
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(b.0));
    for (name, value) in entries {
        rows.push(vec![format!("{prefix}:{name}"), scalar_text(value)]);
    }
}

fn count_map_text(value: Option<&Value>) -> Option<String> {
    let map = value?.as_object()?;
    let mut entries = map
        .iter()
        .map(|(key, value)| format!("{key}={}", scalar_text(value)))
        .collect::<Vec<_>>();
    entries.sort();
    Some(entries.join(", "))
}

fn value_text(row: &Value, key: &str) -> String {
    row.get(key).map(scalar_text).unwrap_or_default()
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}
