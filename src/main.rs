use std::collections::HashSet;
use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use fs2::FileExt;
use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, BooleanQuery, FuzzyTermQuery, Occur, QueryParser};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value as TantivyValue,
    STORED, STRING,
};
use tantivy::{doc, Index, IndexWriter, TantivyDocument, Term};

const DEFAULT_LIMIT: usize = 20;

#[derive(Parser)]
#[command(name = "mem", version, about = "Portable agent memory CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Init,
    Save(SaveArgs),
    Query(QueryArgs),
    Update(UpdateArgs),
    Supersede(SupersedeArgs),
    Delete(DeleteArgs),
    Reindex,
    Context(ContextArgs),
    History(HistoryArgs),
    Stats,
    Audit(AuditArgs),
    Gc(GcArgs),
    Export(ExportArgs),
    Import(ImportArgs),
    Merge(MergeArgs),
    Ambiguity {
        #[command(subcommand)]
        command: AmbiguityCommand,
    },
}

#[derive(Args)]
struct SaveArgs {
    #[arg(long, default_value = "reference")]
    r#type: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    content: String,
    #[arg(long, default_value = "[]")]
    tags: String,
    #[arg(long, default_value = "global")]
    scope: String,
    #[arg(long, default_value = "agent")]
    source: String,
    #[arg(long)]
    confidence: Option<String>,
    #[arg(long)]
    expires_at: Option<String>,
    #[arg(long)]
    why: Option<String>,
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
struct QueryArgs {
    query: Option<String>,
    #[arg(long)]
    r#type: Option<String>,
    #[arg(long)]
    tags: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long)]
    expired: bool,
    #[arg(long)]
    include_superseded: bool,
    #[arg(long, default_value_t = DEFAULT_LIMIT)]
    limit: usize,
    #[arg(long, value_enum, default_value_t = SortMode::Relevance)]
    sort: SortMode,
    #[arg(long)]
    fuzzy: bool,
    #[arg(long)]
    semantic: bool,
}

#[derive(Clone, ValueEnum)]
enum SortMode {
    Relevance,
    Time,
    AccessCount,
}

#[derive(Args)]
struct UpdateArgs {
    name: String,
    #[arg(long)]
    content: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    add_tags: Option<String>,
    #[arg(long, default_value = "agent")]
    source: String,
    #[arg(long)]
    expected_version: Option<i64>,
}

#[derive(Args)]
struct SupersedeArgs {
    old_name: String,
    new_name: String,
    #[arg(long)]
    content: String,
    #[arg(long)]
    description: Option<String>,
    #[arg(long, default_value = "agent")]
    source: String,
    #[arg(long)]
    expected_version: Option<i64>,
}

#[derive(Args)]
struct DeleteArgs {
    name: String,
    #[arg(long)]
    hard: bool,
    #[arg(long)]
    force: bool,
    #[arg(long, default_value = "agent")]
    source: String,
    #[arg(long)]
    expected_version: Option<i64>,
}

#[derive(Args)]
struct ContextArgs {
    #[arg(long)]
    detect: bool,
}

#[derive(Args)]
struct HistoryArgs {
    name: Option<String>,
    #[arg(long)]
    recent: bool,
    #[arg(long)]
    action: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Args)]
struct AuditArgs {
    #[arg(long)]
    fix: bool,
}

#[derive(Args)]
struct GcArgs {
    #[arg(long, default_value_t = 90)]
    days: i64,
}

#[derive(Args)]
struct ExportArgs {
    #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
    format: ExportFormat,
    #[arg(long)]
    include_superseded: bool,
}

#[derive(Clone, ValueEnum)]
enum ExportFormat {
    Json,
    Markdown,
}

#[derive(Args)]
struct ImportArgs {
    file: PathBuf,
    #[arg(long)]
    r#type: Option<String>,
    #[arg(long, default_value = "manual")]
    source: String,
}

#[derive(Args)]
struct MergeArgs {
    db: PathBuf,
    #[arg(long)]
    prefer_trusted: bool,
}

#[derive(Subcommand)]
enum AmbiguityCommand {
    Add(AmbiguityAddArgs),
    List(AmbiguityListArgs),
    Resolve(AmbiguityResolveArgs),
}

#[derive(Args)]
struct AmbiguityAddArgs {
    #[arg(long)]
    query: String,
    #[arg(long)]
    memory_ids: String,
    #[arg(long)]
    context: Option<String>,
}

#[derive(Args)]
struct AmbiguityListArgs {
    #[arg(long)]
    pending: bool,
}

#[derive(Args)]
struct AmbiguityResolveArgs {
    id: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Memory {
    id: String,
    r#type: String,
    name: String,
    description: Option<String>,
    content: Option<String>,
    tags: String,
    scope: String,
    source: String,
    confidence: String,
    protected: bool,
    created_at: String,
    updated_at: String,
    expires_at: Option<String>,
    valid_until: Option<String>,
    superseded_by: Option<String>,
    version: i64,
    access_count: i64,
    last_accessed_at: Option<String>,
}

#[derive(Debug)]
struct App {
    root: PathBuf,
    db_path: PathBuf,
    index_path: PathBuf,
    schema_path: PathBuf,
}

#[derive(Clone)]
struct IndexFields {
    id: Field,
    name: Field,
    description: Field,
    content: Field,
    tags: Field,
    scope: Field,
    r#type: Field,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let app = App::discover()?;

    match cli.command {
        Command::Init => {
            app.init()?;
            println!("{}", json!({"status": "initialized", "root": app.root}));
        }
        Command::Save(args) => with_lock(&app, || cmd_save(&app, args))?,
        Command::Query(args) => cmd_query(&app, args)?,
        Command::Update(args) => with_lock(&app, || cmd_update(&app, args))?,
        Command::Supersede(args) => with_lock(&app, || cmd_supersede(&app, args))?,
        Command::Delete(args) => with_lock(&app, || cmd_delete(&app, args))?,
        Command::Reindex => with_lock(&app, || {
            app.init()?;
            reindex(&app)?;
            println!("{}", json!({"status": "reindexed"}));
            Ok(())
        })?,
        Command::Context(args) => cmd_context(args)?,
        Command::History(args) => cmd_history(&app, args)?,
        Command::Stats => cmd_stats(&app)?,
        Command::Audit(args) => with_lock(&app, || cmd_audit(&app, args))?,
        Command::Gc(args) => with_lock(&app, || cmd_gc(&app, args))?,
        Command::Export(args) => cmd_export(&app, args)?,
        Command::Import(args) => with_lock(&app, || cmd_import(&app, args))?,
        Command::Merge(args) => with_lock(&app, || cmd_merge(&app, args))?,
        Command::Ambiguity { command } => with_lock(&app, || cmd_ambiguity(&app, command))?,
    }

    Ok(())
}

impl App {
    fn discover() -> Result<Self> {
        let exe_dir = env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf));
        let cwd = env::current_dir().context("read current directory")?;
        let root = if cwd.join("schema/memory-schema.sql").exists() {
            cwd
        } else if let Some(dir) = exe_dir {
            find_root(&dir).unwrap_or_else(|| {
                env::var("AGENT_KNOWLEDGE_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| home_dir().join(".agent-knowledge"))
            })
        } else {
            env::var("AGENT_KNOWLEDGE_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home_dir().join(".agent-knowledge"))
        };

        Ok(Self {
            db_path: root.join("memory.db"),
            index_path: root.join("index"),
            schema_path: root.join("schema/memory-schema.sql"),
            root,
        })
    }

    fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.index_path).context("create index directory")?;
        let conn = self.conn()?;
        let schema =
            fs::read_to_string(&self.schema_path).context("read schema/memory-schema.sql")?;
        conn.execute_batch(&schema)
            .context("apply database schema")?;
        ensure_index(&self.index_path)?;
        Ok(())
    }

    fn conn(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)
            .with_context(|| format!("open {}", self.db_path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Ok(conn)
    }
}

fn find_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start);
    while let Some(path) = current {
        if path.join("schema/memory-schema.sql").exists() {
            return Some(path.to_path_buf());
        }
        current = path.parent();
    }
    None
}

fn home_dir() -> PathBuf {
    env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn with_lock<F>(app: &App, f: F) -> Result<()>
where
    F: FnOnce() -> Result<()>,
{
    fs::create_dir_all(&app.root)?;
    let lock_path = app.root.join(".mem.lock");
    let lock = File::create(lock_path)?;
    lock.lock_exclusive()?;
    let result = f();
    lock.unlock()?;
    result
}

fn cmd_save(app: &App, args: SaveArgs) -> Result<()> {
    app.init()?;
    validate_tags(&args.tags)?;

    let conn = app.conn()?;
    if let Some(existing) = memory_by_name(&conn, &args.name)? {
        let content = strip_secrets(&args.content)?;
        if args.force {
            if source_priority(&args.source) < source_priority(&existing.source) {
                println!(
                    "{}",
                    json!({
                        "status": "rejected",
                        "reason": "lower_trust_source_cannot_overwrite",
                        "existing": existing,
                        "new_source": args.source
                    })
                );
                return Ok(());
            }
            let now = now();
            let description = args
                .description
                .or(args.why)
                .or(existing.description.clone());
            let confidence = args
                .confidence
                .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
            conn.execute(
                "UPDATE memories
                 SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
                     source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
                     expires_at = ?10, version = version + 1
                 WHERE id = ?11",
                params![
                    args.r#type,
                    description,
                    content,
                    args.tags,
                    args.scope,
                    args.source,
                    confidence,
                    args.source == "manual",
                    now,
                    args.expires_at,
                    existing.id
                ],
            )?;
            log_change(
                &conn,
                &existing.id,
                "update",
                existing.content.as_deref(),
                Some(&content),
                &args.source,
            )?;
            upsert_index(app, &conn, &existing.id)?;
            let updated = memory_by_id(&conn, &existing.id)?.expect("updated memory exists");
            println!(
                "{}",
                json!({
                    "status": "updated",
                    "match_type": "exact_name_force",
                    "id": updated.id,
                    "version": updated.version
                })
            );
            return Ok(());
        }
        println!(
            "{}",
            json!({
                "status": "duplicate_found",
                "match_type": "exact_name",
                "existing": existing,
                "new_content": content
            })
        );
        return Ok(());
    }

    let content = strip_secrets(&args.content)?;
    if !args.force {
        let candidates = similar_candidates(app, &conn, &content, 5)?;
        if !candidates.is_empty() {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "status": "similar_found",
                    "match_type": "bm25_lindera",
                    "candidates": candidates,
                    "new_content": content
                }))?
            );
            return Ok(());
        }
    }

    let id = slugify(&args.name);
    let now = now();
    let confidence = args
        .confidence
        .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
    let protected = args.source == "manual";
    let description = args.description.or(args.why);

    conn.execute(
        "INSERT INTO memories
        (id, type, name, description, content, tags, scope, source, confidence, protected, created_at, updated_at, expires_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11, ?12)",
        params![
            id,
            args.r#type,
            args.name,
            description,
            content,
            args.tags,
            args.scope,
            args.source,
            confidence,
            protected,
            now,
            args.expires_at
        ],
    )
    .context("insert memory")?;
    log_change(&conn, &id, "save", None, Some(&content), &args.source)?;
    upsert_index(app, &conn, &id)?;

    println!("{}", json!({"status": "saved", "id": id, "version": 1}));
    Ok(())
}

fn cmd_query(app: &App, args: QueryArgs) -> Result<()> {
    app.init()?;
    if args.semantic {
        println!(
            "{}",
            json!({
                "status": "unsupported",
                "feature": "semantic_query",
                "message": "semantic query interface is reserved; no embedding backend is configured"
            })
        );
        return Ok(());
    }
    let conn = app.conn()?;
    let scope_filter = match args.scope.as_deref() {
        Some("auto") => Some(detect_scope_set()?),
        Some(scope) => Some(vec!["global".to_string(), scope.to_string()]),
        None => None,
    };

    let mut ids = if let Some(query) = args.query.as_deref() {
        search_index(app, query, args.fuzzy, args.limit.max(DEFAULT_LIMIT))?
    } else {
        Vec::new()
    };

    let mut memories = if args.query.is_some() {
        let mut rows = Vec::new();
        for id in ids.drain(..) {
            if let Some(memory) = memory_by_id(&conn, &id)? {
                rows.push(memory);
            }
        }
        rows
    } else {
        all_memories(&conn)?
    };

    memories.retain(|memory| {
        if !args.include_superseded && memory.valid_until.is_some() {
            return false;
        }
        if let Some(want_type) = &args.r#type {
            if &memory.r#type != want_type {
                return false;
            }
        }
        if let Some(tag) = &args.tags {
            if !memory.tags.contains(tag) {
                return false;
            }
        }
        if let Some(scopes) = &scope_filter {
            if !scopes.contains(&memory.scope) {
                return false;
            }
        }
        if args.expired {
            return is_expired(memory.expires_at.as_deref());
        }
        true
    });

    match args.sort {
        SortMode::Relevance => {}
        SortMode::Time => memories.sort_by(|a, b| b.created_at.cmp(&a.created_at)),
        SortMode::AccessCount => memories.sort_by(|a, b| b.access_count.cmp(&a.access_count)),
    }
    memories.truncate(args.limit);

    let now = now();
    for memory in &memories {
        conn.execute(
            "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
            params![now, memory.id],
        )?;
    }

    println!("{}", serde_json::to_string_pretty(&memories)?);
    Ok(())
}

fn cmd_update(app: &App, args: UpdateArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let old = memory_by_name(&conn, &args.name)?
        .ok_or_else(|| anyhow!("memory not found: {}", args.name))?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            println!("{}", conflict);
            return Ok(());
        }
    }
    if source_priority(&args.source) < source_priority(&old.source) {
        println!(
            "{}",
            json!({
                "status": "rejected",
                "reason": "lower_trust_source_cannot_update",
                "existing_source": old.source,
                "new_source": args.source,
                "id": old.id
            })
        );
        return Ok(());
    }
    let new_content = match args.content {
        Some(content) => Some(strip_secrets(&content)?),
        None => old.content.clone(),
    };
    let description = args.description.or(old.description.clone());
    let tags = match args.add_tags {
        Some(add) => merge_tags(&old.tags, &add)?,
        None => old.tags.clone(),
    };
    let now = now();

    conn.execute(
        "UPDATE memories
        SET description = ?1, content = ?2, tags = ?3, updated_at = ?4, version = version + 1
        WHERE id = ?5",
        params![description, new_content, tags, now, old.id],
    )?;
    log_change(
        &conn,
        &old.id,
        "update",
        old.content.as_deref(),
        new_content.as_deref(),
        &args.source,
    )?;
    upsert_index(app, &conn, &old.id)?;

    let updated = memory_by_id(&conn, &old.id)?.expect("updated memory exists");
    println!(
        "{}",
        json!({"status": "updated", "id": updated.id, "version": updated.version})
    );
    Ok(())
}

fn cmd_supersede(app: &App, args: SupersedeArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let old = memory_by_name(&conn, &args.old_name)?
        .ok_or_else(|| anyhow!("memory not found: {}", args.old_name))?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            println!("{}", conflict);
            return Ok(());
        }
    }
    if source_priority(&args.source) < source_priority(&old.source) {
        println!(
            "{}",
            json!({
                "status": "rejected",
                "reason": "lower_trust_source_cannot_supersede",
                "existing_source": old.source,
                "new_source": args.source,
                "id": old.id
            })
        );
        return Ok(());
    }
    let new_id = slugify(&args.new_name);
    let now = now();
    let content = strip_secrets(&args.content)?;
    let confidence = confidence_for_source(&args.source);
    let protected = args.source == "manual";

    conn.execute(
        "INSERT INTO memories
        (id, type, name, description, content, tags, scope, source, confidence, protected, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
        params![
            new_id,
            old.r#type,
            args.new_name,
            args.description.or(old.description),
            content,
            old.tags,
            old.scope,
            args.source,
            confidence,
            protected,
            now
        ],
    )?;
    conn.execute(
        "UPDATE memories SET valid_until = ?1, superseded_by = ?2, updated_at = ?1 WHERE id = ?3",
        params![now, new_id, old.id],
    )?;
    log_change(
        &conn,
        &old.id,
        "supersede",
        old.content.as_deref(),
        Some(&content),
        &args.source,
    )?;
    upsert_index(app, &conn, &new_id)?;
    reindex(app)?;

    println!(
        "{}",
        json!({"status": "superseded", "old_id": old.id, "new_id": new_id})
    );
    Ok(())
}

fn cmd_delete(app: &App, args: DeleteArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let old = memory_by_name(&conn, &args.name)?
        .ok_or_else(|| anyhow!("memory not found: {}", args.name))?;
    if let Some(expected) = args.expected_version {
        if let Some(conflict) = version_conflict(&old, expected) {
            println!("{}", conflict);
            return Ok(());
        }
    }
    if old.protected && !args.force {
        println!(
            "{}",
            json!({"status": "rejected", "reason": "protected_memory_requires_force", "id": old.id})
        );
        return Ok(());
    }

    if args.hard {
        conn.execute("DELETE FROM memories WHERE id = ?1", params![old.id])?;
        log_change(
            &conn,
            &old.id,
            "delete",
            old.content.as_deref(),
            None,
            &args.source,
        )?;
        reindex(app)?;
        println!(
            "{}",
            json!({"status": "deleted", "mode": "hard", "id": old.id})
        );
    } else {
        let now = now();
        conn.execute(
            "UPDATE memories SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, old.id],
        )?;
        log_change(
            &conn,
            &old.id,
            "delete",
            old.content.as_deref(),
            None,
            &args.source,
        )?;
        reindex(app)?;
        println!(
            "{}",
            json!({"status": "deleted", "mode": "soft", "id": old.id})
        );
    }
    Ok(())
}

fn cmd_context(args: ContextArgs) -> Result<()> {
    if !args.detect {
        bail!("use --detect");
    }
    println!("{}", json!({"scope": detect_scope()?}));
    Ok(())
}

fn cmd_history(app: &App, args: HistoryArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let mut sql = String::from(
        "SELECT changelog.id, memory_id, action, old_content, new_content, source, changelog.created_at
         FROM changelog",
    );
    let mut clauses = Vec::new();
    let mut bind_values = Vec::new();
    if let Some(name) = args.name {
        let memory_id = memory_by_name(&conn, &name)?
            .map(|m| m.id)
            .ok_or_else(|| anyhow!("memory not found: {name}"))?;
        clauses.push("memory_id = ?");
        bind_values.push(rusqlite::types::Value::Text(memory_id));
    }
    if let Some(action) = args.action {
        clauses.push("action = ?");
        bind_values.push(rusqlite::types::Value::Text(action));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY changelog.created_at DESC LIMIT ?");
    bind_values.push(rusqlite::types::Value::Integer(args.limit as i64));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(bind_values), |row| {
        Ok(json!({
            "id": row.get::<_, i64>(0)?,
            "memory_id": row.get::<_, String>(1)?,
            "action": row.get::<_, String>(2)?,
            "old_content": row.get::<_, Option<String>>(3)?,
            "new_content": row.get::<_, Option<String>>(4)?,
            "source": row.get::<_, Option<String>>(5)?,
            "created_at": row.get::<_, String>(6)?,
        }))
    })?;

    let values: Result<Vec<_>, _> = rows.collect();
    println!("{}", serde_json::to_string_pretty(&values?)?);
    Ok(())
}

fn cmd_stats(app: &App) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE valid_until IS NULL",
        [],
        |r| r.get(0),
    )?;
    let by_type = grouped_count(&conn, "type")?;
    let by_scope = grouped_count(&conn, "scope")?;
    let by_confidence = grouped_count(&conn, "confidence")?;
    let top_accessed = query_json_rows(
        &conn,
        "SELECT name, access_count, last_accessed_at FROM memories WHERE valid_until IS NULL ORDER BY access_count DESC LIMIT 10",
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "total_active": total,
            "by_type": by_type,
            "by_scope": by_scope,
            "by_confidence": by_confidence,
            "top_accessed": top_accessed
        }))?
    );
    Ok(())
}

fn cmd_audit(app: &App, args: AuditArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let broken = query_json_rows(
        &conn,
        "SELECT name, superseded_by FROM memories
         WHERE superseded_by IS NOT NULL
         AND superseded_by NOT IN (SELECT id FROM memories)",
    )?;
    let expired = query_json_rows(
        &conn,
        "SELECT name, expires_at FROM memories
         WHERE expires_at IS NOT NULL AND datetime(expires_at) < datetime('now') AND valid_until IS NULL",
    )?;
    let stale_low_access = query_json_rows(
        &conn,
        "SELECT name, created_at, access_count FROM memories
         WHERE access_count = 0 AND datetime(created_at) < datetime('now', '-30 day') AND valid_until IS NULL",
    )?;
    let low_confidence_high_access = query_json_rows(
        &conn,
        "SELECT name, confidence, access_count, last_accessed_at FROM memories
         WHERE confidence = 'low' AND access_count >= 3 AND valid_until IS NULL
         ORDER BY access_count DESC",
    )?;
    let cleanup_candidates = query_json_rows(
        &conn,
        "SELECT name, confidence, created_at, access_count FROM memories
         WHERE access_count = 0
         AND confidence IN ('low', 'medium')
         AND datetime(created_at) < datetime('now', '-60 day')
         AND valid_until IS NULL
         ORDER BY created_at ASC",
    )?;

    let mut fixed_expired = 0;
    let mut fixed_broken_links = 0;
    if args.fix {
        let now = now();
        let expired_memories = active_expired_memories(&conn)?;
        for memory in &expired_memories {
            conn.execute(
                "UPDATE memories SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                params![now, memory.id],
            )?;
            log_change(
                &conn,
                &memory.id,
                "delete",
                memory.content.as_deref(),
                None,
                "audit",
            )?;
        }
        fixed_expired = expired_memories.len();
        fixed_broken_links = conn.execute(
            "UPDATE memories
             SET superseded_by = NULL
             WHERE superseded_by IS NOT NULL
             AND superseded_by NOT IN (SELECT id FROM memories)",
            [],
        )?;
        reindex(app)?;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "broken_superseded_links": broken,
            "expired_active_memories": expired,
            "stale_low_access": stale_low_access,
            "low_confidence_high_access": low_confidence_high_access,
            "cleanup_candidates": cleanup_candidates,
            "fixed": args.fix,
            "fixed_expired": fixed_expired,
            "fixed_broken_links": fixed_broken_links
        }))?
    );
    Ok(())
}

fn cmd_gc(app: &App, args: GcArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let cutoff = (Utc::now() - Duration::days(args.days)).to_rfc3339();
    let changed = conn.execute(
        "DELETE FROM memories WHERE valid_until IS NOT NULL AND datetime(valid_until) < datetime(?1)",
        params![cutoff],
    )?;
    reindex(app)?;
    println!("{}", json!({"status": "gc_complete", "deleted": changed}));
    Ok(())
}

fn cmd_export(app: &App, args: ExportArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let mut memories = all_memories(&conn)?;
    if !args.include_superseded {
        memories.retain(|m| m.valid_until.is_none());
    }

    match args.format {
        ExportFormat::Json => println!("{}", serde_json::to_string_pretty(&memories)?),
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

fn cmd_import(app: &App, args: ImportArgs) -> Result<()> {
    app.init()?;
    let text =
        fs::read_to_string(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    if args.file.extension().and_then(|s| s.to_str()) == Some("json") {
        let values: Vec<Value> = serde_json::from_str(&text).context("parse json import")?;
        for value in values {
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("import item missing name"))?;
            let content = value
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();
            cmd_save(
                app,
                SaveArgs {
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
                    content: content.to_string(),
                    tags: value
                        .get("tags")
                        .map(Value::to_string)
                        .unwrap_or_else(|| "[]".to_string()),
                    scope: value
                        .get("scope")
                        .and_then(Value::as_str)
                        .unwrap_or("global")
                        .to_string(),
                    source: args.source.clone(),
                    confidence: None,
                    expires_at: value
                        .get("expires_at")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    why: None,
                    force: false,
                },
            )?;
        }
    } else {
        let name = args
            .file
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("cannot infer name from file"))?
            .to_string();
        cmd_save(
            app,
            SaveArgs {
                r#type: args.r#type.unwrap_or_else(|| "reference".to_string()),
                name,
                description: None,
                content: text,
                tags: "[]".to_string(),
                scope: "global".to_string(),
                source: args.source,
                confidence: None,
                expires_at: None,
                why: None,
                force: false,
            },
        )?;
    }
    Ok(())
}

fn cmd_merge(app: &App, args: MergeArgs) -> Result<()> {
    app.init()?;
    if !args.db.exists() {
        bail!("merge database not found: {}", args.db.display());
    }

    let conn = app.conn()?;
    let theirs = Connection::open(&args.db)
        .with_context(|| format!("open merge database {}", args.db.display()))?;
    let incoming = all_memories(&theirs)?;
    let mut imported = 0;
    let mut identical = 0;
    let mut conflicts = 0;
    let mut trusted_updates = 0;
    let mut rejected_lower_trust = 0;
    let mut regenerated_ids = 0;

    for mut memory in incoming {
        if let Some(existing) = memory_by_name(&conn, &memory.name)? {
            if normalized_text(existing.content.as_deref().unwrap_or_default())
                == normalized_text(memory.content.as_deref().unwrap_or_default())
            {
                identical += 1;
                continue;
            }

            let incoming_priority = source_priority(&memory.source);
            let existing_priority = source_priority(&existing.source);
            if incoming_priority < existing_priority {
                rejected_lower_trust += 1;
                continue;
            }
            if args.prefer_trusted && incoming_priority > existing_priority {
                update_memory_from_merge(&conn, app, &existing, &memory)?;
                trusted_updates += 1;
                continue;
            }

            let context = format!(
                "merge conflict from {}: local source={} priority={}, incoming source={} priority={}",
                args.db.display(),
                existing.source,
                existing_priority,
                memory.source,
                incoming_priority
            );
            add_ambiguity_record(
                &conn,
                &format!("merge:{}", memory.name),
                &[existing.id.clone(), memory.id.clone()],
                Some(&context),
            )?;
            conflicts += 1;
            continue;
        }

        let original_id = memory.id.clone();
        memory.id = unique_memory_id(&conn, &memory.id)?;
        if memory.id != original_id {
            regenerated_ids += 1;
        }
        insert_memory_record(&conn, &memory)?;
        log_change(
            &conn,
            &memory.id,
            "merge",
            None,
            memory.content.as_deref(),
            "merge",
        )?;
        upsert_index(app, &conn, &memory.id)?;
        imported += 1;
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "merged",
            "imported": imported,
            "identical": identical,
            "conflicts": conflicts,
            "trusted_updates": trusted_updates,
            "rejected_lower_trust": rejected_lower_trust,
            "regenerated_ids": regenerated_ids
        }))?
    );
    Ok(())
}

fn cmd_ambiguity(app: &App, command: AmbiguityCommand) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    match command {
        AmbiguityCommand::Add(args) => {
            validate_tags(&args.memory_ids)?;
            let memory_ids = parse_string_array(&args.memory_ids)?;
            add_ambiguity_record(&conn, &args.query, &memory_ids, args.context.as_deref())?;
            println!(
                "{}",
                json!({"status": "ambiguity_added", "id": conn.last_insert_rowid()})
            );
        }
        AmbiguityCommand::List(args) => {
            let sql = if args.pending {
                "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at FROM ambiguities WHERE resolution = 'pending' ORDER BY created_at DESC"
            } else {
                "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at FROM ambiguities ORDER BY created_at DESC"
            };
            let rows = query_json_rows(&conn, sql)?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        AmbiguityCommand::Resolve(args) => {
            let now = now();
            conn.execute(
                "UPDATE ambiguities SET resolution = 'resolved', resolved_at = ?1 WHERE id = ?2",
                params![now, args.id],
            )?;
            println!("{}", json!({"status": "resolved", "id": args.id}));
        }
    }
    Ok(())
}

fn confidence_for_source(source: &str) -> &'static str {
    match source {
        "manual" => "high",
        "agent" => "medium",
        "daily_retro" | "weekly_retro" => "low",
        _ => "medium",
    }
}

fn source_priority(source: &str) -> u8 {
    match source {
        "manual" => 4,
        "agent" => 3,
        "daily_retro" => 2,
        "weekly_retro" => 1,
        _ => 2,
    }
}

fn version_conflict(memory: &Memory, expected_version: i64) -> Option<Value> {
    (memory.version != expected_version).then(|| {
        json!({
            "status": "version_conflict",
            "id": memory.id,
            "name": memory.name,
            "expected_version": expected_version,
            "actual_version": memory.version
        })
    })
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn strip_secrets(input: &str) -> Result<String> {
    let patterns = [
        r"sk-[A-Za-z0-9_\-]{16,}",
        r"ghp_[A-Za-z0-9_]{16,}",
        r"xoxb-[A-Za-z0-9\-]{16,}",
        r"AKIA[0-9A-Z]{16}",
        r"(?i)bearer\s+[A-Za-z0-9._\-]{16,}",
        r"(?i)(password|secret)\s*=\s*[^ \n\r]+",
    ];
    let mut output = input.to_string();
    for pattern in patterns {
        let re = Regex::new(pattern)?;
        output = re.replace_all(&output, "[REDACTED]").to_string();
    }
    Ok(output)
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if ch == '_' || ch == '-' || ch.is_whitespace() || ch == '/' {
            if !slug.ends_with('_') {
                slug.push('_');
            }
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        format!("memory_{}", uuid::Uuid::new_v4())
    } else {
        slug
    }
}

fn validate_tags(tags: &str) -> Result<()> {
    parse_string_array(tags)?;
    Ok(())
}

fn parse_string_array(raw: &str) -> Result<Vec<String>> {
    let parsed: Value =
        serde_json::from_str(raw).context("tags/memory_ids must be a JSON array")?;
    let Value::Array(values) = parsed else {
        bail!("expected JSON array");
    };
    values
        .into_iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("array items must be strings"))
        })
        .collect()
}

fn merge_tags(existing: &str, add: &str) -> Result<String> {
    let mut set = HashSet::new();
    for raw in [existing, add] {
        let Value::Array(values) = serde_json::from_str(raw)? else {
            bail!("tags must be JSON arrays");
        };
        for value in values {
            if let Some(tag) = value.as_str() {
                set.insert(tag.to_string());
            }
        }
    }
    let mut tags: Vec<_> = set.into_iter().collect();
    tags.sort();
    Ok(serde_json::to_string(&tags)?)
}

fn is_expired(expires_at: Option<&str>) -> bool {
    expires_at
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|expires| expires.with_timezone(&Utc) < Utc::now())
        .unwrap_or(false)
}

fn memory_by_name(conn: &Connection, name: &str) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE name = ?1")?;
    stmt.query_row(params![name], row_to_memory)
        .optional()
        .map_err(Into::into)
}

fn memory_by_id(conn: &Connection, id: &str) -> Result<Option<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories WHERE id = ?1")?;
    stmt.query_row(params![id], row_to_memory)
        .optional()
        .map_err(Into::into)
}

fn all_memories(conn: &Connection) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare("SELECT * FROM memories ORDER BY created_at DESC")?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn active_expired_memories(conn: &Connection) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM memories
         WHERE expires_at IS NOT NULL
         AND datetime(expires_at) < datetime('now')
         AND valid_until IS NULL",
    )?;
    let rows = stmt.query_map([], row_to_memory)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get("id")?,
        r#type: row.get("type")?,
        name: row.get("name")?,
        description: row.get("description")?,
        content: row.get("content")?,
        tags: row.get("tags")?,
        scope: row.get("scope")?,
        source: row.get("source")?,
        confidence: row.get("confidence")?,
        protected: row.get("protected")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        expires_at: row.get("expires_at")?,
        valid_until: row.get("valid_until")?,
        superseded_by: row.get("superseded_by")?,
        version: row.get("version")?,
        access_count: row.get("access_count")?,
        last_accessed_at: row.get("last_accessed_at")?,
    })
}

fn insert_memory_record(conn: &Connection, memory: &Memory) -> Result<()> {
    conn.execute(
        "INSERT INTO memories
        (id, type, name, description, content, tags, scope, source, confidence, protected,
         created_at, updated_at, expires_at, valid_until, superseded_by, version, access_count, last_accessed_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            memory.id,
            memory.r#type,
            memory.name,
            memory.description,
            memory.content,
            memory.tags,
            memory.scope,
            memory.source,
            memory.confidence,
            memory.protected,
            memory.created_at,
            memory.updated_at,
            memory.expires_at,
            memory.valid_until,
            memory.superseded_by,
            memory.version,
            memory.access_count,
            memory.last_accessed_at,
        ],
    )?;
    Ok(())
}

fn update_memory_from_merge(
    conn: &Connection,
    app: &App,
    existing: &Memory,
    incoming: &Memory,
) -> Result<()> {
    let now = now();
    conn.execute(
        "UPDATE memories
         SET type = ?1, description = ?2, content = ?3, tags = ?4, scope = ?5,
             source = ?6, confidence = ?7, protected = ?8, updated_at = ?9,
             expires_at = ?10, valid_until = ?11, superseded_by = ?12,
             version = version + 1
         WHERE id = ?13",
        params![
            &incoming.r#type,
            &incoming.description,
            &incoming.content,
            &incoming.tags,
            &incoming.scope,
            &incoming.source,
            &incoming.confidence,
            incoming.protected,
            now,
            &incoming.expires_at,
            &incoming.valid_until,
            &incoming.superseded_by,
            &existing.id,
        ],
    )?;
    log_change(
        conn,
        &existing.id,
        "merge",
        existing.content.as_deref(),
        incoming.content.as_deref(),
        "merge",
    )?;
    upsert_index(app, conn, &existing.id)?;
    Ok(())
}

fn unique_memory_id(conn: &Connection, preferred: &str) -> Result<String> {
    let base = if preferred.trim().is_empty() {
        format!("memory_{}", uuid::Uuid::new_v4())
    } else {
        preferred.to_string()
    };
    if memory_by_id(conn, &base)?.is_none() {
        return Ok(base);
    }
    for suffix in 2.. {
        let candidate = format!("{base}_{suffix}");
        if memory_by_id(conn, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    unreachable!()
}

fn log_change(
    conn: &Connection,
    memory_id: &str,
    action: &str,
    old_content: Option<&str>,
    new_content: Option<&str>,
    source: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO changelog (memory_id, action, old_content, new_content, source)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![memory_id, action, old_content, new_content, source],
    )?;
    Ok(())
}

fn add_ambiguity_record(
    conn: &Connection,
    query: &str,
    memory_ids: &[String],
    context: Option<&str>,
) -> Result<()> {
    let memory_ids = serde_json::to_string(memory_ids)?;
    conn.execute(
        "INSERT INTO ambiguities (query, memory_ids, context, resolution)
         VALUES (?1, ?2, ?3, 'pending')",
        params![query, memory_ids, context],
    )?;
    Ok(())
}

fn similar_candidates(
    app: &App,
    conn: &Connection,
    content: &str,
    limit: usize,
) -> Result<Vec<Value>> {
    let ids = search_index(app, content, false, 25)?;
    let mut candidates = Vec::new();
    for id in ids {
        let Some(memory) = memory_by_id(conn, &id)? else {
            continue;
        };
        if memory.valid_until.is_some() {
            continue;
        }
        let score = content_similarity(content, memory.content.as_deref().unwrap_or_default());
        if score >= 0.55 {
            candidates.push(json!({
                "id": memory.id,
                "name": memory.name,
                "content": memory.content,
                "score": score
            }));
        }
    }
    candidates.sort_by(|a, b| {
        let a_score = a.get("score").and_then(Value::as_f64).unwrap_or_default();
        let b_score = b.get("score").and_then(Value::as_f64).unwrap_or_default();
        b_score
            .partial_cmp(&a_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(limit);
    Ok(candidates)
}

fn content_similarity(a: &str, b: &str) -> f64 {
    let a_set = shingles(&normalized_text(a));
    let b_set = shingles(&normalized_text(b));
    if a_set.is_empty() || b_set.is_empty() {
        return 0.0;
    }
    let intersection = a_set.intersection(&b_set).count() as f64;
    let union = a_set.union(&b_set).count() as f64;
    let smaller = a_set.len().min(b_set.len()) as f64;
    (intersection / union).max(intersection / smaller)
}

fn normalized_text(input: &str) -> String {
    input
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| !ch.is_whitespace())
        .collect()
}

fn shingles(input: &str) -> HashSet<String> {
    let chars: Vec<char> = input.chars().collect();
    if chars.is_empty() {
        return HashSet::new();
    }
    if chars.len() <= 2 {
        return HashSet::from([input.to_string()]);
    }
    chars
        .windows(2)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

fn grouped_count(conn: &Connection, column: &str) -> Result<Value> {
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

fn query_json_rows(conn: &Connection, sql: &str) -> Result<Vec<Value>> {
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

fn sqlite_value_to_json(value: rusqlite::types::Value) -> Value {
    match value {
        rusqlite::types::Value::Null => Value::Null,
        rusqlite::types::Value::Integer(v) => json!(v),
        rusqlite::types::Value::Real(v) => json!(v),
        rusqlite::types::Value::Text(v) => json!(v),
        rusqlite::types::Value::Blob(v) => json!(v),
    }
}

fn build_schema() -> (Schema, IndexFields) {
    let mut builder = Schema::builder();
    let id = builder.add_text_field("id", STRING | STORED);
    let text_options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("multilingual")
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let name = builder.add_text_field("name", text_options.clone());
    let description = builder.add_text_field("description", text_options.clone());
    let content = builder.add_text_field("content", text_options.clone());
    let tags = builder.add_text_field("tags", text_options);
    let scope = builder.add_text_field("scope", STRING);
    let r#type = builder.add_text_field("type", STRING);
    let schema = builder.build();
    let fields = IndexFields {
        id,
        name,
        description,
        content,
        tags,
        scope,
        r#type,
    };
    (schema, fields)
}

fn ensure_index(path: &Path) -> Result<Index> {
    fs::create_dir_all(path)?;
    let index = match Index::open_in_dir(path) {
        Ok(index) => Ok(index),
        Err(_) => {
            let (schema, _) = build_schema();
            Index::create_in_dir(path, schema).context("create Tantivy index")
        }
    }?;
    register_tokenizers(&index)?;
    Ok(index)
}

fn register_tokenizers(index: &Index) -> Result<()> {
    let dictionary = load_dictionary("embedded://cc-cedict").context("load embedded CC-CEDICT")?;
    let segmenter = Segmenter::new(Mode::Normal, dictionary, None);
    let tokenizer = LinderaTokenizer::from_segmenter(segmenter);
    index.tokenizers().register("multilingual", tokenizer);
    Ok(())
}

fn reindex(app: &App) -> Result<()> {
    if app.index_path.exists() {
        fs::remove_dir_all(&app.index_path)?;
    }
    fs::create_dir_all(&app.index_path)?;
    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(&app.index_path, schema)?;
    register_tokenizers(&index)?;
    let mut writer = index.writer(50_000_000)?;
    let conn = app.conn()?;
    for memory in all_memories(&conn)? {
        add_memory_doc(&mut writer, &fields, &memory)?;
    }
    writer.commit()?;
    Ok(())
}

fn upsert_index(app: &App, conn: &Connection, id: &str) -> Result<()> {
    let index = ensure_index(&app.index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let memory =
        memory_by_id(conn, id)?.ok_or_else(|| anyhow!("memory not found for index: {id}"))?;
    let mut writer = index.writer(50_000_000)?;
    writer.delete_term(Term::from_field_text(fields.id, id));
    add_memory_doc(&mut writer, &fields, &memory)?;
    writer.commit()?;
    Ok(())
}

fn add_memory_doc(writer: &mut IndexWriter, fields: &IndexFields, memory: &Memory) -> Result<()> {
    writer.add_document(doc!(
        fields.id => memory.id.clone(),
        fields.name => memory.name.clone(),
        fields.description => memory.description.clone().unwrap_or_default(),
        fields.content => memory.content.clone().unwrap_or_default(),
        fields.tags => memory.tags.clone(),
        fields.scope => memory.scope.clone(),
        fields.r#type => memory.r#type.clone(),
    ))?;
    Ok(())
}

fn fields_from_schema(schema: Schema) -> Result<IndexFields> {
    Ok(IndexFields {
        id: schema.get_field("id").context("index missing id field")?,
        name: schema
            .get_field("name")
            .context("index missing name field")?,
        description: schema
            .get_field("description")
            .context("index missing description field")?,
        content: schema
            .get_field("content")
            .context("index missing content field")?,
        tags: schema
            .get_field("tags")
            .context("index missing tags field")?,
        scope: schema
            .get_field("scope")
            .context("index missing scope field")?,
        r#type: schema
            .get_field("type")
            .context("index missing type field")?,
    })
}

fn search_index(app: &App, query: &str, fuzzy: bool, limit: usize) -> Result<Vec<String>> {
    let index = ensure_index(&app.index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let boxed_query: Box<dyn tantivy::query::Query> = if query.trim().is_empty() {
        Box::new(AllQuery)
    } else if fuzzy {
        let terms = [fields.name, fields.description, fields.content, fields.tags]
            .into_iter()
            .map(|field| {
                (
                    Occur::Should,
                    Box::new(FuzzyTermQuery::new(
                        Term::from_field_text(field, query),
                        1,
                        true,
                    )) as Box<dyn tantivy::query::Query>,
                )
            })
            .collect();
        Box::new(BooleanQuery::new(terms))
    } else {
        let parser = QueryParser::for_index(
            &index,
            vec![fields.name, fields.description, fields.content, fields.tags],
        );
        Box::new(parser.parse_query(query)?)
    };
    let docs = searcher.search(&boxed_query, &TopDocs::with_limit(limit))?;
    let mut ids = Vec::new();
    for (_score, address) in docs {
        let retrieved = searcher.doc::<TantivyDocument>(address)?;
        if let Some(value) = retrieved
            .get_first(fields.id)
            .and_then(|value| value.as_str())
        {
            ids.push(value.to_string());
        }
    }
    Ok(ids)
}

fn detect_scope_set() -> Result<Vec<String>> {
    Ok(vec!["global".to_string(), detect_scope()?])
}

fn detect_scope() -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output();
    let Ok(output) = output else {
        return Ok("global".to_string());
    };
    if !output.status.success() {
        return Ok("global".to_string());
    }
    let remote = String::from_utf8_lossy(&output.stdout);
    Ok(remote_to_scope(remote.trim()))
}

fn remote_to_scope(remote: &str) -> String {
    let cleaned = remote
        .trim_end_matches(".git")
        .replace("git@github.com:", "")
        .replace("https://github.com/", "");
    if cleaned.contains('/') {
        format!("project:{cleaned}")
    } else {
        "global".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_with_version(version: i64) -> Memory {
        Memory {
            id: "id".to_string(),
            r#type: "feedback".to_string(),
            name: "name".to_string(),
            description: None,
            content: Some("content".to_string()),
            tags: "[]".to_string(),
            scope: "global".to_string(),
            source: "manual".to_string(),
            confidence: "high".to_string(),
            protected: true,
            created_at: now(),
            updated_at: now(),
            expires_at: None,
            valid_until: None,
            superseded_by: None,
            version,
            access_count: 0,
            last_accessed_at: None,
        }
    }

    #[test]
    fn source_priority_orders_trust() {
        assert!(source_priority("manual") > source_priority("agent"));
        assert!(source_priority("agent") > source_priority("daily_retro"));
        assert!(source_priority("daily_retro") > source_priority("weekly_retro"));
    }

    #[test]
    fn version_conflict_reports_mismatch() {
        let memory = memory_with_version(3);
        assert!(version_conflict(&memory, 3).is_none());
        let conflict = version_conflict(&memory, 2).expect("conflict");
        assert_eq!(conflict["status"], "version_conflict");
        assert_eq!(conflict["actual_version"], 3);
    }

    #[test]
    fn strips_common_secret_patterns() {
        let stripped = strip_secrets("token=Bearer abcdefghijklmnop password=hunter2").unwrap();
        assert!(stripped.contains("[REDACTED]"));
        assert!(!stripped.contains("hunter2"));
    }

    #[test]
    fn parses_string_arrays_only() {
        assert_eq!(parse_string_array(r#"["a","b"]"#).unwrap(), vec!["a", "b"]);
        assert!(parse_string_array(r#"["a",1]"#).is_err());
        assert!(parse_string_array(r#"{"a":1}"#).is_err());
    }

    #[test]
    fn content_similarity_handles_cjk_overlap() {
        let score = content_similarity("不要使用 emoji", "不要在回覆中使用 emoji");
        assert!(score > 0.8);
    }

    #[test]
    fn remote_scope_supports_ssh_and_https() {
        assert_eq!(
            remote_to_scope("git@github.com:NeoHsu/agent-knowledge.git"),
            "project:NeoHsu/agent-knowledge"
        );
        assert_eq!(
            remote_to_scope("https://github.com/NeoHsu/agent-knowledge.git"),
            "project:NeoHsu/agent-knowledge"
        );
    }
}
