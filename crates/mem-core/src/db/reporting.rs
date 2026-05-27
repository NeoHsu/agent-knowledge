use super::*;

pub fn grouped_count(conn: &Connection, column: &str) -> Result<Value> {
    let sql = format!(
        "SELECT {column}, COUNT(*) FROM memories WHERE valid_until IS NULL GROUP BY {column}"
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
