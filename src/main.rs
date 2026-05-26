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
const SCHEMA_VERSION: i64 = 1;

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
    Retro {
        #[command(subcommand)]
        command: RetroCommand,
    },
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
    #[arg(long)]
    no_touch: bool,
    #[arg(long)]
    raw_query: bool,
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
enum RetroCommand {
    Daily(RetroArgs),
    Weekly(RetroArgs),
}

#[derive(Args)]
struct RetroArgs {
    #[arg(long, default_value_t = 50)]
    limit: usize,
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
    #[arg(long)]
    note: Option<String>,
    #[arg(long)]
    keep: Option<String>,
    #[arg(long)]
    soft_delete_others: bool,
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
        Command::Retro { command } => cmd_retro(&app, command)?,
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
        migrate_schema(&conn)?;
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
    FileExt::unlock(&lock)?;
    result
}

fn with_db_transaction<T, F>(conn: &Connection, f: F) -> Result<T>
where
    F: FnOnce(&Connection) -> Result<T>,
{
    conn.execute_batch("BEGIN IMMEDIATE TRANSACTION;")?;
    let result = f(conn);
    match result {
        Ok(value) => {
            if let Err(err) = conn.execute_batch("COMMIT;") {
                let _ = conn.execute_batch("ROLLBACK;");
                Err(err.into())
            } else {
                Ok(value)
            }
        }
        Err(err) => {
            let _ = conn.execute_batch("ROLLBACK;");
            Err(err)
        }
    }
}

fn migrate_schema(conn: &Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        bail!("database schema version {version} is newer than supported version {SCHEMA_VERSION}");
    }
    if version == 0 {
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    Ok(())
}

fn cmd_save(app: &App, args: SaveArgs) -> Result<()> {
    let result = save_memory(app, args)?;
    let is_similar = result
        .get("status")
        .and_then(Value::as_str)
        .map(|status| status == "similar_found")
        .unwrap_or(false);
    if is_similar {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", result);
    }
    Ok(())
}

fn save_memory(app: &App, args: SaveArgs) -> Result<Value> {
    app.init()?;
    validate_tags(&args.tags)?;

    let conn = app.conn()?;
    if let Some(existing) = memory_by_name(&conn, &args.name)? {
        let content = strip_secrets(&args.content)?;
        if args.force {
            if source_priority(&args.source) < source_priority(&existing.source) {
                return Ok(json!({
                    "status": "rejected",
                    "reason": "lower_trust_source_cannot_overwrite",
                    "existing": existing,
                    "new_source": args.source
                }));
            }
            let now = now();
            let description = args
                .description
                .or(args.why)
                .or(existing.description.clone());
            let confidence = args
                .confidence
                .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
            with_db_transaction(&conn, |conn| {
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
                    conn,
                    &existing.id,
                    "update",
                    existing.content.as_deref(),
                    Some(&content),
                    &args.source,
                )?;
                Ok(())
            })?;
            upsert_index(app, &conn, &existing.id).with_context(|| {
                format!(
                    "update index for {}; run `mem reindex` if needed",
                    existing.id
                )
            })?;
            let updated = memory_by_id(&conn, &existing.id)?.expect("updated memory exists");
            return Ok(json!({
                "status": "updated",
                "match_type": "exact_name_force",
                "id": updated.id,
                "version": updated.version
            }));
        }
        return Ok(json!({
            "status": "duplicate_found",
            "match_type": "exact_name",
            "existing": existing,
            "new_content": content
        }));
    }

    let content = strip_secrets(&args.content)?;
    if !args.force {
        let candidates = similar_candidates(app, &conn, &content, 5)?;
        if !candidates.is_empty() {
            return Ok(json!({
                "status": "similar_found",
                "match_type": "bm25_lindera",
                "candidates": candidates,
                "new_content": content
            }));
        }
    }

    let id = unique_memory_id(&conn, &slugify(&args.name))?;
    let now = now();
    let confidence = args
        .confidence
        .unwrap_or_else(|| confidence_for_source(&args.source).to_string());
    let protected = args.source == "manual";
    let description = args.description.or(args.why);

    with_db_transaction(&conn, |conn| {
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
        log_change(conn, &id, "save", None, Some(&content), &args.source)?;
        Ok(())
    })?;
    upsert_index(app, &conn, &id)
        .with_context(|| format!("update index for {id}; run `mem reindex` if needed"))?;

    Ok(json!({"status": "saved", "id": id, "version": 1}))
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
        search_index(
            app,
            query,
            args.fuzzy,
            args.raw_query,
            args.limit.max(DEFAULT_LIMIT),
        )?
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
            if !memory_has_tag(&memory.tags, tag) {
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
        SortMode::AccessCount => {
            memories.sort_by_key(|memory| std::cmp::Reverse(memory.access_count))
        }
    }
    memories.truncate(args.limit);

    if !args.no_touch {
        let now = now();
        for memory in &memories {
            conn.execute(
                "UPDATE memories SET access_count = access_count + 1, last_accessed_at = ?1 WHERE id = ?2",
                params![now, memory.id],
            )?;
        }
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

    with_db_transaction(&conn, |conn| {
        conn.execute(
            "UPDATE memories
            SET description = ?1, content = ?2, tags = ?3, updated_at = ?4, version = version + 1
            WHERE id = ?5",
            params![description, new_content, tags, now, old.id],
        )?;
        log_change(
            conn,
            &old.id,
            "update",
            old.content.as_deref(),
            new_content.as_deref(),
            &args.source,
        )?;
        Ok(())
    })?;
    upsert_index(app, &conn, &old.id)
        .with_context(|| format!("update index for {}; run `mem reindex` if needed", old.id))?;

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
    let new_id = unique_memory_id(&conn, &slugify(&args.new_name))?;
    let now = now();
    let content = strip_secrets(&args.content)?;
    let confidence = confidence_for_source(&args.source);
    let protected = args.source == "manual";

    with_db_transaction(&conn, |conn| {
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
            conn,
            &old.id,
            "supersede",
            old.content.as_deref(),
            Some(&content),
            &args.source,
        )?;
        Ok(())
    })?;
    upsert_index(app, &conn, &new_id)
        .with_context(|| format!("update index for {new_id}; run `mem reindex` if needed"))?;
    reindex(app).context("rebuild index after supersede; run `mem reindex` if needed")?;

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
        with_db_transaction(&conn, |conn| {
            conn.execute("DELETE FROM memories WHERE id = ?1", params![old.id])?;
            log_change(
                conn,
                &old.id,
                "delete",
                old.content.as_deref(),
                None,
                &args.source,
            )?;
            Ok(())
        })?;
        reindex(app).context("rebuild index after delete; run `mem reindex` if needed")?;
        println!(
            "{}",
            json!({"status": "deleted", "mode": "hard", "id": old.id})
        );
    } else {
        let now = now();
        with_db_transaction(&conn, |conn| {
            conn.execute(
                "UPDATE memories SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                params![now, old.id],
            )?;
            log_change(
                conn,
                &old.id,
                "delete",
                old.content.as_deref(),
                None,
                &args.source,
            )?;
            Ok(())
        })?;
        reindex(app).context("rebuild index after delete; run `mem reindex` if needed")?;
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
    println!("{}", serde_json::to_string_pretty(&stats_report(&conn)?)?);
    Ok(())
}

fn cmd_audit(app: &App, args: AuditArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let report = audit_report(&conn, app, args.fix)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn stats_report(conn: &Connection) -> Result<Value> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE valid_until IS NULL",
        [],
        |r| r.get(0),
    )?;
    let by_type = grouped_count(conn, "type")?;
    let by_scope = grouped_count(conn, "scope")?;
    let by_confidence = grouped_count(conn, "confidence")?;
    let top_accessed = query_json_rows(
        conn,
        "SELECT name, access_count, last_accessed_at FROM memories WHERE valid_until IS NULL ORDER BY access_count DESC LIMIT 10",
    )?;
    Ok(json!({
        "total_active": total,
        "by_type": by_type,
        "by_scope": by_scope,
        "by_confidence": by_confidence,
        "top_accessed": top_accessed
    }))
}

fn audit_report(conn: &Connection, app: &App, fix: bool) -> Result<Value> {
    let broken = query_json_rows(
        conn,
        "SELECT name, superseded_by FROM memories
         WHERE superseded_by IS NOT NULL
         AND superseded_by NOT IN (SELECT id FROM memories)",
    )?;
    let expired = query_json_rows(
        conn,
        "SELECT name, expires_at FROM memories
         WHERE expires_at IS NOT NULL AND datetime(expires_at) < datetime('now') AND valid_until IS NULL",
    )?;
    let stale_low_access = query_json_rows(
        conn,
        "SELECT name, created_at, access_count FROM memories
         WHERE access_count = 0 AND datetime(created_at) < datetime('now', '-30 day') AND valid_until IS NULL",
    )?;
    let low_confidence_high_access = query_json_rows(
        conn,
        "SELECT name, confidence, access_count, last_accessed_at FROM memories
         WHERE confidence = 'low' AND access_count >= 3 AND valid_until IS NULL
         ORDER BY access_count DESC",
    )?;
    let cleanup_candidates = query_json_rows(
        conn,
        "SELECT name, confidence, created_at, access_count FROM memories
         WHERE access_count = 0
         AND confidence IN ('low', 'medium')
         AND datetime(created_at) < datetime('now', '-60 day')
         AND valid_until IS NULL
         ORDER BY created_at ASC",
    )?;

    let mut fixed_expired = 0;
    let mut fixed_broken_links = 0;
    if fix {
        let now = now();
        let expired_memories = active_expired_memories(conn)?;
        with_db_transaction(conn, |conn| {
            for memory in &expired_memories {
                conn.execute(
                    "UPDATE memories SET valid_until = ?1, updated_at = ?1 WHERE id = ?2",
                    params![now, memory.id],
                )?;
                log_change(
                    conn,
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
            Ok(())
        })?;
        reindex(app).context("rebuild index after audit --fix; run `mem reindex` if needed")?;
    }

    Ok(json!({
        "broken_superseded_links": broken,
        "expired_active_memories": expired,
        "stale_low_access": stale_low_access,
        "low_confidence_high_access": low_confidence_high_access,
        "cleanup_candidates": cleanup_candidates,
        "fixed": fix,
        "fixed_expired": fixed_expired,
        "fixed_broken_links": fixed_broken_links
    }))
}

fn cmd_gc(app: &App, args: GcArgs) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let cutoff = (Utc::now() - Duration::days(args.days)).to_rfc3339();
    let changed = with_db_transaction(&conn, |conn| {
        let gc_memories = gc_candidate_memories(conn, &cutoff)?;
        for memory in &gc_memories {
            log_change(
                conn,
                &memory.id,
                "gc",
                memory.content.as_deref(),
                None,
                "gc",
            )?;
        }
        let changed = conn.execute(
            "DELETE FROM memories WHERE valid_until IS NOT NULL AND datetime(valid_until) < datetime(?1)",
            params![cutoff],
        )?;
        Ok(changed)
    })?;
    reindex(app).context("rebuild index after gc; run `mem reindex` if needed")?;
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

fn save_args_from_import_value(value: Value, source: &str) -> Result<SaveArgs> {
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
        source: source.to_string(),
        confidence: None,
        expires_at: value
            .get("expires_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        why: None,
        force: false,
    })
}

fn result_status(result: &Value) -> String {
    result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn increment_count(counts: &mut serde_json::Map<String, Value>, status: &str) {
    let current = counts.get(status).and_then(Value::as_u64).unwrap_or(0);
    counts.insert(status.to_string(), json!(current + 1));
}

fn cmd_import(app: &App, args: ImportArgs) -> Result<()> {
    app.init()?;
    let text =
        fs::read_to_string(&args.file).with_context(|| format!("read {}", args.file.display()))?;
    let mut results = Vec::new();
    let mut counts = serde_json::Map::new();

    if args.file.extension().and_then(|s| s.to_str()) == Some("json") {
        let values: Vec<Value> = serde_json::from_str(&text).context("parse json import")?;
        for (index, value) in values.into_iter().enumerate() {
            let import_result = save_args_from_import_value(value, &args.source)
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
        let status = result_status(&result);
        increment_count(&mut counts, &status);
        results.push(json!({
            "index": 0,
            "status": status,
            "result": result
        }));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "import_complete",
            "total": results.len(),
            "counts": Value::Object(counts),
            "results": results
        }))?
    );
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
    let mut changed_index_ids = Vec::new();

    with_db_transaction(&conn, |conn| {
        for mut memory in incoming {
            if let Some(content) = memory.content.take() {
                memory.content = Some(strip_secrets(&content)?);
            }

            if let Some(existing) = memory_by_name(conn, &memory.name)? {
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
                    update_memory_from_merge(conn, &existing, &memory)?;
                    changed_index_ids.push(existing.id.clone());
                    trusted_updates += 1;
                    continue;
                }

                let context = serde_json::to_string(&json!({
                    "kind": "merge_conflict",
                    "source_db": args.db.display().to_string(),
                    "local": {
                        "id": &existing.id,
                        "name": &existing.name,
                        "source": &existing.source,
                        "priority": existing_priority,
                        "content": &existing.content
                    },
                    "incoming": {
                        "id": &memory.id,
                        "name": &memory.name,
                        "type": &memory.r#type,
                        "description": &memory.description,
                        "content": &memory.content,
                        "tags": &memory.tags,
                        "scope": &memory.scope,
                        "source": &memory.source,
                        "confidence": &memory.confidence,
                        "priority": incoming_priority,
                        "version": memory.version
                    }
                }))?;
                add_ambiguity_record(
                    conn,
                    &format!("merge:{}", memory.name),
                    &[existing.id.clone(), memory.id.clone()],
                    Some(&context),
                )?;
                conflicts += 1;
                continue;
            }

            let original_id = memory.id.clone();
            memory.id = unique_memory_id(conn, &memory.id)?;
            if memory.id != original_id {
                regenerated_ids += 1;
            }
            insert_memory_record(conn, &memory)?;
            log_change(
                conn,
                &memory.id,
                "merge",
                None,
                memory.content.as_deref(),
                "merge",
            )?;
            changed_index_ids.push(memory.id.clone());
            imported += 1;
        }
        Ok(())
    })?;

    for id in &changed_index_ids {
        upsert_index(app, &conn, id)
            .with_context(|| format!("update index for {id}; run `mem reindex` if needed"))?;
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

fn cmd_retro(app: &App, command: RetroCommand) -> Result<()> {
    app.init()?;
    let conn = app.conn()?;
    let (kind, limit) = match command {
        RetroCommand::Daily(args) => ("daily", args.limit),
        RetroCommand::Weekly(args) => ("weekly", args.limit),
    };
    let limit = limit.clamp(1, 500);
    let recent_history = query_json_rows(
        &conn,
        &format!(
            "SELECT id, memory_id, action, old_content, new_content, source, created_at
             FROM changelog
             ORDER BY created_at DESC
             LIMIT {limit}"
        ),
    )?;
    let pending_ambiguities = ambiguity_rows(&conn, true)?;
    let active_memories = query_json_rows(
        &conn,
        &format!(
            "SELECT id, type, name, tags, scope, source, confidence, version, access_count, updated_at
             FROM memories
             WHERE valid_until IS NULL
             ORDER BY updated_at DESC
             LIMIT {limit}"
        ),
    )?;
    let instructions = match kind {
        "daily" => vec![
            "Use platform-provided conversation context; repo readers are optional adapters.",
            "Compare today's conversation facts against active_memories.",
            "Persist durable new facts with source=daily_retro.",
            "Use update/supersede/delete with --expected-version when changing existing memory.",
            "Record unresolved conflicts with mem ambiguity add.",
        ],
        _ => vec![
            "Review memory quality from changelog, audit, and pending ambiguities.",
            "Merge duplicates, resolve ambiguities, and identify skill/profile candidates.",
            "Calibrate low-confidence high-access memories after review.",
            "Use audit --fix only for deterministic repairs.",
        ],
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "retro_bundle",
            "kind": kind,
            "generated_at": now(),
            "instructions": instructions,
            "stats": stats_report(&conn)?,
            "audit": audit_report(&conn, app, false)?,
            "pending_ambiguities": pending_ambiguities,
            "recent_history": recent_history,
            "active_memories": active_memories
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
            let rows = ambiguity_rows(&conn, args.pending)?;
            println!("{}", serde_json::to_string_pretty(&rows)?);
        }
        AmbiguityCommand::Resolve(args) => {
            let now = now();
            let ambiguity = ambiguity_by_id(&conn, args.id)?
                .ok_or_else(|| anyhow!("ambiguity not found: {}", args.id))?;
            let raw_memory_ids = ambiguity
                .get("memory_ids")
                .and_then(Value::as_str)
                .unwrap_or("[]");
            let memory_ids = parse_string_array(raw_memory_ids)?;
            let mut soft_deleted = Vec::new();
            let mut skipped_protected = Vec::new();
            let keep_id = match args.keep.as_deref() {
                Some(reference) => Some(resolve_memory_ref(&conn, reference)?),
                None => None,
            };
            if args.soft_delete_others {
                let keep_id = keep_id
                    .as_deref()
                    .ok_or_else(|| anyhow!("--soft-delete-others requires --keep"))?;
                for memory_id in memory_ids.iter().filter(|id| id.as_str() != Some(keep_id)) {
                    let Some(memory) = memory_by_id(&conn, memory_id)? else {
                        continue;
                    };
                    if memory.protected {
                        skipped_protected.push(memory.id);
                        continue;
                    }
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
                        "ambiguity_resolve",
                    )?;
                    soft_deleted.push(memory.id);
                }
                if !soft_deleted.is_empty() {
                    reindex(app)?;
                }
            }
            let resolution = json!({
                "status": "resolved",
                "note": args.note,
                "keep": keep_id,
                "soft_deleted": soft_deleted,
                "skipped_protected": skipped_protected
            })
            .to_string();
            conn.execute(
                "UPDATE ambiguities SET resolution = ?1, resolved_at = ?2 WHERE id = ?3",
                params![resolution, now, args.id],
            )?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "status": "resolved",
                    "id": args.id,
                    "resolution": serde_json::from_str::<Value>(&resolution)?
                }))?
            );
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
        } else if (ch == '_' || ch == '-' || ch.is_whitespace() || ch == '/')
            && !slug.ends_with('_')
        {
            slug.push('_');
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

fn memory_has_tag(tags: &str, wanted: &str) -> bool {
    parse_string_array(tags)
        .map(|tags| tags.iter().any(|tag| tag == wanted))
        .unwrap_or(false)
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

fn resolve_memory_ref(conn: &Connection, reference: &str) -> Result<String> {
    if let Some(memory) = memory_by_id(conn, reference)? {
        return Ok(memory.id);
    }
    if let Some(memory) = memory_by_name(conn, reference)? {
        return Ok(memory.id);
    }
    bail!("memory not found: {reference}")
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

fn gc_candidate_memories(conn: &Connection, cutoff: &str) -> Result<Vec<Memory>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM memories
         WHERE valid_until IS NOT NULL
         AND datetime(valid_until) < datetime(?1)",
    )?;
    let rows = stmt.query_map(params![cutoff], row_to_memory)?;
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

fn update_memory_from_merge(conn: &Connection, existing: &Memory, incoming: &Memory) -> Result<()> {
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
    let ids = search_index(app, content, false, false, 25)?;
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

fn literal_query_text(input: &str) -> String {
    let mut output = String::new();
    let mut last_was_space = false;
    for ch in input.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            output.push(ch);
            last_was_space = false;
        } else if !last_was_space {
            output.push(' ');
            last_was_space = true;
        }
    }
    output.trim().to_string()
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

fn ambiguity_rows(conn: &Connection, pending_only: bool) -> Result<Vec<Value>> {
    let sql = if pending_only {
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         WHERE resolution = 'pending'
         ORDER BY created_at DESC"
    } else {
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         ORDER BY created_at DESC"
    };
    let mut rows = query_json_rows(conn, sql)?;
    for row in &mut rows {
        parse_json_string_field(row, "memory_ids");
        parse_json_string_field(row, "context");
        parse_json_string_field(row, "resolution");
    }
    Ok(rows)
}

fn parse_json_string_field(row: &mut Value, field: &str) {
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

fn ambiguity_by_id(conn: &Connection, id: i64) -> Result<Option<Value>> {
    conn.query_row(
        "SELECT id, query, memory_ids, context, resolution, created_at, resolved_at
         FROM ambiguities
         WHERE id = ?1",
        params![id],
        |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "query": row.get::<_, String>(1)?,
                "memory_ids": row.get::<_, String>(2)?,
                "context": row.get::<_, Option<String>>(3)?,
                "resolution": row.get::<_, Option<String>>(4)?,
                "created_at": row.get::<_, String>(5)?,
                "resolved_at": row.get::<_, Option<String>>(6)?,
            }))
        },
    )
    .optional()
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

fn search_index(
    app: &App,
    query: &str,
    fuzzy: bool,
    raw_query: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let index = ensure_index(&app.index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let query_text = if raw_query {
        query.trim().to_string()
    } else {
        literal_query_text(query)
    };
    let boxed_query: Box<dyn tantivy::query::Query> = if query_text.is_empty() {
        Box::new(AllQuery)
    } else if fuzzy {
        let terms = [fields.name, fields.description, fields.content, fields.tags]
            .into_iter()
            .map(|field| {
                (
                    Occur::Should,
                    Box::new(FuzzyTermQuery::new(
                        Term::from_field_text(field, &query_text),
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
        Box::new(parser.parse_query(&query_text)?)
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
