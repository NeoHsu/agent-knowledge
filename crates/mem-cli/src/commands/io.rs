use super::*;

pub(crate) fn cmd_export(app: &App, args: ExportArgs) -> Result<()> {
    app.ensure_schema()?;
    let conn = app.conn()?;
    let mut memories = all_memories(&conn)?;
    if !args.include_superseded {
        memories.retain(|m| m.valid_until.is_none());
    }

    match args.format {
        ExportFormat::Json => print_json_pretty(&memories)?,
        ExportFormat::Markdown => {
            for memory in memories {
                println!("## {}", memory.name);
                println!();
                println!("- id: {}", memory.id);
                println!("- type: {}", memory.r#type);
                println!("- scope: {}", memory.scope);
                println!("- confidence: {}", memory.confidence);
                println!("- tags: {}", memory.tags);
                println!();
                if let Some(description) = memory.description {
                    println!("{}", description);
                    println!();
                }
                if let Some(content) = memory.content {
                    println!("{}", content);
                    println!();
                }
            }
        }
    }
    Ok(())
}

fn save_args_from_import_value(
    value: Value,
    source: &str,
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
        no_validate_workflow,
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
    app.ensure_schema()?;
    let text =
        fs::read_to_string(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    let mut results = Vec::new();
    let mut counts = serde_json::Map::new();

    if args.file.extension().and_then(|s| s.to_str()) == Some("json") {
        let values: Vec<Value> = serde_json::from_str(&text).context("parse json import")?;
        for (index, value) in values.into_iter().enumerate() {
            let import_result =
                save_args_from_import_value(value, &args.source, args.no_validate_workflow)
                    .and_then(|save_args| save_memory(app, save_args));
            match import_result {
                Ok(result) => {
                    let status = result_status(&result);
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
                no_validate_workflow: args.no_validate_workflow,
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
    print_json_pretty(&json!({
        "status": "import_complete",
        "total": results.len(),
        "counts": Value::Object(counts),
        "results": results
    }))?;
    Ok(())
}
