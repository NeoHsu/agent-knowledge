use super::*;
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

const IMPORT_TRANSACTION_CHUNK: usize = 500;

type ImportedMemoryResult = std::result::Result<(Value, Option<String>), String>;
type IndexedImportResult = (usize, ImportedMemoryResult);

struct JsonArrayValidationVisitor;

impl<'de> Visitor<'de> for JsonArrayValidationVisitor {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array of memory objects")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<IgnoredAny>()?.is_some() {}
        Ok(())
    }
}

struct JsonArrayChunkVisitor<'a, F> {
    chunk_size: usize,
    process_chunk: &'a mut F,
    processing_error: &'a mut Option<anyhow::Error>,
}

impl<'de, F> Visitor<'de> for JsonArrayChunkVisitor<'_, F>
where
    F: FnMut(Vec<Value>) -> Result<()>,
{
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON array of memory objects")
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<(), A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut chunk = Vec::with_capacity(self.chunk_size);
        while let Some(value) = sequence.next_element::<Value>()? {
            chunk.push(value);
            if chunk.len() == self.chunk_size {
                let full_chunk = std::mem::replace(&mut chunk, Vec::with_capacity(self.chunk_size));
                if let Err(error) = (self.process_chunk)(full_chunk) {
                    *self.processing_error = Some(error);
                    return Err(<A::Error as serde::de::Error>::custom(
                        "import chunk processing failed",
                    ));
                }
            }
        }
        if !chunk.is_empty() {
            if let Err(error) = (self.process_chunk)(chunk) {
                *self.processing_error = Some(error);
                return Err(<A::Error as serde::de::Error>::custom(
                    "import chunk processing failed",
                ));
            }
        }
        Ok(())
    }
}

fn validate_json_array(reader: impl Read) -> Result<()> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    serde::de::Deserializer::deserialize_seq(&mut deserializer, JsonArrayValidationVisitor)
        .context("parse json import")?;
    deserializer
        .end()
        .context("parse trailing json import data")
}

fn process_json_array_chunks(
    reader: impl Read,
    chunk_size: usize,
    mut process_chunk: impl FnMut(Vec<Value>) -> Result<()>,
) -> Result<()> {
    let mut processing_error = None;
    let visitor = JsonArrayChunkVisitor {
        chunk_size,
        process_chunk: &mut process_chunk,
        processing_error: &mut processing_error,
    };
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    let parsed = serde::de::Deserializer::deserialize_seq(&mut deserializer, visitor);
    if let Some(error) = processing_error {
        return Err(error);
    }
    parsed.context("parse json import")?;
    deserializer
        .end()
        .context("parse trailing json import data")
}

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
        ExportFormat::Markdown => print_text(render_export_markdown(&memories)?)?,
    }
    Ok(())
}

fn render_export_markdown(memories: &[mem_core::db::Memory]) -> Result<String> {
    let mut output = String::new();
    for memory in memories {
        writeln!(output, "## {}\n", memory.name)?;
        writeln!(output, "- id: {}", memory.id)?;
        writeln!(output, "- type: {}", memory.r#type)?;
        writeln!(output, "- scope: {}", memory.scope)?;
        writeln!(output, "- confidence: {}", memory.confidence)?;
        writeln!(output, "- tags: {}\n", memory.tags)?;
        if let Some(description) = memory.description.as_deref() {
            output.push_str(description);
            output.push_str("\n\n");
        }
        if let Some(content) = memory.content.as_deref() {
            output.push_str(content);
            output.push_str("\n\n");
        }
    }
    Ok(output)
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
    let summary_only = args.summary_only;
    let mut total = 0_usize;
    let mut results = Vec::new();
    let mut counts = serde_json::Map::new();

    if args.file.extension().and_then(|s| s.to_str()) == Some("json") {
        let mut file = File::open(&args.file)
            .with_context(|| format!("open {} for import", args.file.display()))?;
        validate_json_array(BufReader::new(&mut file))?;
        file.seek(SeekFrom::Start(0))
            .with_context(|| format!("rewind {} after validation", args.file.display()))?;

        let rebuild_index =
            memory_index::is_stale(app) || memory_index::validate_physical_index(app).is_err();
        let conn = app.conn()?;
        let mut saved_ids = BTreeSet::new();
        let origin_ref = args.file.display().to_string();
        let mut first_index = 0;
        process_json_array_chunks(BufReader::new(file), IMPORT_TRANSACTION_CHUNK, |chunk| {
            let chunk_len = chunk.len();
            let chunk_results = with_transaction(&conn, |conn| {
                let mut chunk_results: Vec<IndexedImportResult> = Vec::with_capacity(chunk_len);
                let mut changed = false;
                for (offset, value) in chunk.into_iter().enumerate() {
                    conn.execute_batch("SAVEPOINT import_item")?;
                    let import_result = save_args_from_import_value(
                        value,
                        &args.source,
                        args.user_confirmed,
                        args.redact_secrets,
                        &origin_ref,
                        args.no_validate_workflow,
                    )
                    .and_then(|save_args| save_memory_no_index_in_connection(conn, save_args));
                    match import_result {
                        Ok((result, maybe_id)) => {
                            conn.execute_batch("RELEASE import_item")?;
                            changed |= maybe_id.is_some();
                            chunk_results.push((first_index + offset, Ok((result, maybe_id))));
                        }
                        Err(error) => {
                            conn.execute_batch("ROLLBACK TO import_item; RELEASE import_item;")?;
                            chunk_results.push((first_index + offset, Err(error.to_string())));
                        }
                    }
                }
                if changed {
                    mem_core::graph::set_graph_dirty(conn, true)?;
                    mem_core::db::set_index_dirty(conn, true)?;
                }
                Ok(chunk_results)
            })?;

            for (index, import_result) in chunk_results {
                total += 1;
                match import_result {
                    Ok((result, maybe_id)) => {
                        let status = result_status(&result);
                        if let Some(id) = maybe_id {
                            saved_ids.insert(id);
                        }
                        increment_count(&mut counts, &status);
                        if !summary_only {
                            results.push(json!({
                                "index": index,
                                "status": status,
                                "result": result
                            }));
                        }
                    }
                    Err(error) => {
                        increment_count(&mut counts, "failed");
                        if !summary_only {
                            results.push(json!({
                                "index": index,
                                "status": "failed",
                                "error": error
                            }));
                        }
                    }
                }
            }
            first_index += chunk_len;
            Ok(())
        })?;
        let saved_ids = saved_ids.into_iter().collect::<Vec<_>>();
        memory_index::complete_bulk_write(app, &conn, &saved_ids, rebuild_index)?;
    } else {
        let text = fs::read_to_string(&args.file)
            .with_context(|| format!("read {}", args.file.display()))?;
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
        total = 1;
        if !summary_only {
            results.push(json!({
                "index": 0,
                "status": status,
                "result": result
            }));
        }
    }
    let mut response = json!({
        "status": "import_complete",
        "total": total,
        "counts": Value::Object(counts),
    });
    if !summary_only {
        response["results"] = json!(results);
    }
    print_write_json_pretty(app, response)?;
    Ok(())
}
