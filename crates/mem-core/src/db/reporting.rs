use super::*;

/// Whitelisted columns allowed in `grouped_count` to prevent SQL injection.
enum GroupColumn {
    Type,
    Scope,
    Source,
    Confidence,
}

impl GroupColumn {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "type" => Some(Self::Type),
            "scope" => Some(Self::Scope),
            "source" => Some(Self::Source),
            "confidence" => Some(Self::Confidence),
            _ => None,
        }
    }

    fn as_sql_column(&self) -> &'static str {
        match self {
            Self::Type => "type",
            Self::Scope => "scope",
            Self::Source => "source",
            Self::Confidence => "confidence",
        }
    }
}

pub fn grouped_count(conn: &Connection, column: &str) -> Result<Value> {
    let col = GroupColumn::from_str(column)
        .ok_or_else(|| anyhow::anyhow!("grouped_count: unsupported column '{column}'; expected one of: type, scope, source, confidence"))?;
    let sql = format!(
        "SELECT {}, COUNT(*) FROM memories WHERE valid_until IS NULL GROUP BY {}",
        col.as_sql_column(),
        col.as_sql_column()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut map = serde_json::Map::new();
    for row in rows {
        let (key, count) = row?;
        map.insert(key, json!(count));
    }
    Ok(Value::Object(map))
}

pub fn query_json_rows(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(sql)?;
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt.query_map([], |row| {
        let mut map = serde_json::Map::new();
        for (idx, name) in column_names.iter().enumerate() {
            let value: rusqlite::types::Value = row.get(idx)?;
            map.insert(name.clone(), sqlite_value_to_json(value));
        }
        Ok(Value::Object(map))
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

pub(super) fn parse_json_string_field(row: &mut Value, field: &str) {
    let Some(map) = row.as_object_mut() else {
        return;
    };
    let Some(raw) = map.get(field).and_then(Value::as_str) else {
        return;
    };
    if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
        map.insert(field.to_string(), parsed);
    }
}

fn sqlite_value_to_json(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(v) => json!(v),
        rusqlite::types::Value::Real(v) => json!(v),
        rusqlite::types::Value::Text(v) => json!(v),
        rusqlite::types::Value::Blob(v) => json!(v),
    }
}
