use super::*;
use std::fmt::Write as _;

pub(crate) fn cmd_export(app: &App, args: ExportArgs) -> Result<()> {
    app.require_schema()?;
    let conn = app.read_conn()?;
    let estimated_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(
             length(CAST(id AS BLOB)) + length(CAST(type AS BLOB))
             + length(CAST(name AS BLOB))
             + length(CAST(COALESCE(description, '') AS BLOB))
             + length(CAST(COALESCE(content, '') AS BLOB))
             + length(CAST(tags AS BLOB)) + length(CAST(scope AS BLOB))
             + length(CAST(source AS BLOB))
         ), 0)
         FROM memories",
        [],
        |row| row.get(0),
    )?;
    if estimated_bytes > 268_435_456 {
        bail!(
            "memory export exceeds the 268435456-byte in-memory limit; use `mem bundle export` \
             for a complete large-store snapshot or `mem query` for bounded results"
        );
    }
    let mut memories = all_memories(&conn)?;
    if args.include_superseded {
        memories.retain(|memory| !is_expired(memory.expires_at.as_deref()));
    } else {
        memories.retain(memory_is_active);
    }

    match args.format {
        ExportFormat::Json => print_json_pretty(&memories)?,
        ExportFormat::Markdown => print_text(render_export_markdown(&memories))?,
    }
    Ok(())
}

fn render_export_markdown(memories: &[mem_core::db::Memory]) -> String {
    let mut output = String::new();
    for memory in memories {
        writeln!(output, "## {}\n", memory.name).expect("write markdown heading");
        writeln!(output, "- id: {}", memory.id).expect("write markdown id");
        writeln!(output, "- type: {}", memory.r#type).expect("write markdown type");
        writeln!(output, "- scope: {}", memory.scope).expect("write markdown scope");
        writeln!(output, "- confidence: {}", memory.confidence).expect("write markdown confidence");
        writeln!(output, "- tags: {}\n", memory.tags).expect("write markdown tags");
        if let Some(description) = memory.description.as_deref() {
            output.push_str(description);
            output.push_str("\n\n");
        }
        if let Some(content) = memory.content.as_deref() {
            output.push_str(content);
            output.push_str("\n\n");
        }
    }
    output
}

fn save_args_from_import_value(
    value: Value,
    source: &str,
    user_confirmed: bool,
    redact_secrets: bool,
    origin_ref: &str,
    no_validate_workflow: bool,
) -> Result<SaveArgs> {
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("import item missing name"))?;
    let content = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Ok(SaveArgs {
        r#type: value
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("reference")
            .to_string(),
        name: name.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
        content: Some(content.to_string()),
        content_file: None,
        tags: import_tags(&value)?,
        scope: value
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or("global")
            .to_string(),
        source: source.to_string(),
        confidence: None,
        expires_at: value
            .get("expires_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        why: None,
        force: false,
        user_confirmed,
        redact_secrets,
        no_validate_workflow,
        origin: Some("import".to_string()),
        origin_ref: Some(origin_ref.to_string()),
    })
}

fn result_status(result: &Value) -> String {
    result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn import_tags(value: &Value) -> Result<String> {
    match value.get("tags") {
        Some(Value::String(tags)) => {
            validate_tags(tags)?;
            Ok(tags.clone())
        }
        Some(tags) => {
            let tags = tags.to_string();
            validate_tags(&tags)?;
            Ok(tags)
        }
        None => Ok("[]".to_string()),
    }
}

fn increment_count(counts: &mut serde_json::Map<String, Value>, status: &str) {
    let current = counts.get(status).and_then(Value::as_u64).unwrap_or(0);
    counts.insert(status.to_string(), json!(current + 1));
}

pub(crate) fn cmd_import(app: &App, args: ImportArgs) -> Result<()> {
    app.require_schema()?;
    let import_bytes = fs::metadata(&args.file)
        .with_context(|| format!("inspect {}", args.file.display()))?
        .len();
    if import_bytes > 268_435_456 {
        bail!("import file exceeds 268435456 bytes");
    }
    let text =
        fs::read_to_string(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    let mut results = Vec::new();
    let mut counts = serde_json::Map::new();

    if args.file.extension().and_then(|s| s.to_str()) == Some("json") {
        let values: Vec<Value> = serde_json::from_str(&text).context("parse json import")?;
        let conn = app.conn()?;
        let mut saved_ids: Vec<String> = Vec::new();
        for (index, value) in values.into_iter().enumerate() {
            let import_result = save_args_from_import_value(
                value,
                &args.source,
                args.user_confirmed,
                args.redact_secrets,
                &args.file.display().to_string(),
                args.no_validate_workflow,
            )
            .and_then(|save_args| save_memory_no_index(app, save_args));
            match import_result {
                Ok((result, maybe_id)) => {
                    let status = result_status(&result);
                    if let Some(id) = maybe_id {
                        saved_ids.push(id);
                    }
                    increment_count(&mut counts, &status);
                    results.push(json!({
                        "index": index,
                        "status": status,
                        "result": result
                    }));
                }
                Err(err) => {
                    increment_count(&mut counts, "failed");
                    results.push(json!({
                        "index": index,
                        "status": "failed",
                        "error": err.to_string()
                    }));
                }
            }
        }
        memory_index::upsert_batch_or_mark_stale(app, &conn, &saved_ids)?;
    } else {
        let name = args
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("cannot infer name from file"))?
            .to_string();
        let result = save_memory(
            app,
            SaveArgs {
                r#type: args.r#type.unwrap_or_else(|| "reference".to_string()),
                name,
                description: None,
                content: Some(text),
                content_file: None,
                tags: "[]".to_string(),
                scope: "global".to_string(),
                source: args.source,
                confidence: None,
                expires_at: None,
                why: None,
                force: false,
                user_confirmed: args.user_confirmed,
                redact_secrets: args.redact_secrets,
                no_validate_workflow: args.no_validate_workflow,
                origin: Some("import".to_string()),
                origin_ref: Some(args.file.display().to_string()),
            },
        )?;
        let status = result_status(&result);
        increment_count(&mut counts, &status);
        results.push(json!({
            "index": 0,
            "status": status,
            "result": result
        }));
    }
    print_write_json_pretty(
        app,
        json!({
            "status": "import_complete",
            "total": results.len(),
            "counts": Value::Object(counts),
            "results": results
        }),
    )?;
    Ok(())
}
