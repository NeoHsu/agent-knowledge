use std::fs;
use std::ops::Bound;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Utc;
use tantivy::collector::TopDocs;
use tantivy::query::{
    AllQuery, BooleanQuery, BoostQuery, FuzzyTermQuery, Occur, Query, QueryParser, RangeQuery,
    TermQuery,
};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value as TantivyValue, FAST,
    INDEXED, STORED, STRING,
};
use tantivy::{doc, Index, IndexWriter, TantivyDocument, Term};

use crate::error;
use crate::index::{SearchFilters, SearchLifecycle};
use crate::search_tokenizer;

pub(crate) const INDEX_SCHEMA_VERSION: i64 = 4;
const INDEX_VERSION_MARKER: &str = ".mnemark-index-version";

// Tantivy requires at least 15 MB. Interleaved 10k import measurements showed
// that 20 MB preserves throughput while substantially reducing peak RSS.
const INDEX_WRITER_MEMORY_BUDGET_BYTES: usize = 20_000_000;

#[derive(Debug)]
pub(crate) struct IndexCompatibilityError {
    marker_path: PathBuf,
    expected: i64,
    found: IndexVersionFound,
}

#[derive(Debug)]
enum IndexVersionFound {
    Missing,
    Invalid(String),
    Different(i64),
}

impl std::fmt::Display for IndexCompatibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.found {
            IndexVersionFound::Missing => write!(
                f,
                "index schema version mismatch: expected {}, found missing marker at {}",
                self.expected,
                self.marker_path.display()
            ),
            IndexVersionFound::Invalid(value) => write!(
                f,
                "index schema version mismatch: expected {}, found invalid marker {:?} at {}",
                self.expected,
                value,
                self.marker_path.display()
            ),
            IndexVersionFound::Different(version) => write!(
                f,
                "index schema version mismatch: expected {}, found {} at {}",
                self.expected,
                version,
                self.marker_path.display()
            ),
        }
    }
}

impl std::error::Error for IndexCompatibilityError {}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub id: String,
    pub score: f64,
}

#[derive(Debug, Clone)]
pub struct IndexedMemory {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: String,
    pub exact_tags: Vec<String>,
    pub scope: String,
    pub r#type: String,
    pub valid: bool,
    pub expires_at: i64,
}

pub(crate) struct IndexFields {
    id: Field,
    name: Field,
    description: Field,
    content: Field,
    tags: Field,
    exact_tag: Field,
    scope: Field,
    r#type: Field,
    valid: Field,
    expires_at: Field,
}

pub fn rebuild(index_path: &Path, memories: &[IndexedMemory]) -> Result<()> {
    if validate_index_directory(index_path)? {
        fs::remove_dir_all(index_path)?;
    }
    fs::create_dir_all(index_path)?;
    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(index_path, schema)?;
    register_tokenizers(&index)?;
    let mut writer = index.writer(INDEX_WRITER_MEMORY_BUDGET_BYTES)?;
    for memory in memories {
        add_memory_doc(&mut writer, &fields, memory)?;
    }
    writer.commit()?;
    write_index_version(index_path)?;
    Ok(())
}

pub fn upsert(index_path: &Path, memory: &IndexedMemory) -> Result<()> {
    let index = ensure_index(index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let mut writer = index.writer(INDEX_WRITER_MEMORY_BUDGET_BYTES)?;
    upsert_with_writer(&mut writer, &fields, memory)?;
    writer.commit()?;
    Ok(())
}

/// Upsert a single memory using a shared `IndexWriter`. The caller is
/// responsible for calling `writer.commit()` when done.
pub(crate) fn upsert_with_writer(
    writer: &mut IndexWriter,
    fields: &IndexFields,
    memory: &IndexedMemory,
) -> Result<()> {
    writer.delete_term(Term::from_field_text(fields.id, &memory.id));
    add_memory_doc(writer, fields, memory)?;
    Ok(())
}

/// Upsert bounded batches with one shared `IndexWriter` and one commit.
pub fn upsert_batches(
    index_path: &Path,
    batches: impl IntoIterator<Item = Result<Vec<IndexedMemory>>>,
) -> Result<()> {
    let index = ensure_index(index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let mut writer = index.writer(INDEX_WRITER_MEMORY_BUDGET_BYTES)?;
    for batch in batches {
        for memory in batch? {
            upsert_with_writer(&mut writer, &fields, &memory)?;
        }
    }
    writer.commit()?;
    Ok(())
}

pub fn search_hits(
    index_path: &Path,
    query: &str,
    fuzzy: bool,
    raw_query: bool,
    limit: usize,
    filters: SearchFilters<'_>,
) -> Result<Vec<SearchHit>> {
    let index = open_existing_index(index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let reader = index.reader()?;
    let searcher = reader.searcher();
    let query_text = if raw_query {
        query.trim().to_string()
    } else {
        literal_query_text(query)
    };

    let text_clause: Option<Box<dyn Query>> = if query_text.is_empty() {
        None
    } else if fuzzy {
        build_fuzzy_query(&index, &query_text, &fields)?
    } else {
        let mut parser = QueryParser::for_index(&index, default_search_fields(&fields));
        apply_field_boosts(&mut parser, &fields);
        Some(Box::new(parser.parse_query(&query_text)?))
    };
    let type_clause = filters.memory_type.map(|memory_type| {
        Box::new(TermQuery::new(
            Term::from_field_text(fields.r#type, memory_type),
            IndexRecordOption::Basic,
        )) as Box<dyn Query>
    });
    let scope_clause = filters.scopes.and_then(|scopes| {
        if scopes.is_empty() {
            return None;
        }
        let terms = scopes
            .iter()
            .map(|scope| {
                (
                    Occur::Should,
                    Box::new(TermQuery::new(
                        Term::from_field_text(fields.scope, scope),
                        IndexRecordOption::Basic,
                    )) as Box<dyn Query>,
                )
            })
            .collect();
        Some(Box::new(BooleanQuery::new(terms)) as Box<dyn Query>)
    });
    let tag_clause = filters.tag.map(|tag| {
        Box::new(TermQuery::new(
            Term::from_field_text(fields.exact_tag, tag),
            IndexRecordOption::Basic,
        )) as Box<dyn Query>
    });
    let valid_clause = match filters.lifecycle {
        SearchLifecycle::Active | SearchLifecycle::Expired => Some(Box::new(TermQuery::new(
            Term::from_field_text(fields.valid, "true"),
            IndexRecordOption::Basic,
        )) as Box<dyn Query>),
        SearchLifecycle::IncludeSuperseded => None,
    };
    let now = Utc::now().timestamp();
    let expiration_clause = match filters.lifecycle {
        SearchLifecycle::Expired => Box::new(RangeQuery::new(
            Bound::Unbounded,
            Bound::Excluded(Term::from_field_i64(fields.expires_at, now)),
        )) as Box<dyn Query>,
        SearchLifecycle::Active | SearchLifecycle::IncludeSuperseded => Box::new(RangeQuery::new(
            Bound::Included(Term::from_field_i64(fields.expires_at, now)),
            Bound::Unbounded,
        ))
            as Box<dyn Query>,
    };

    let mut must_clauses: Vec<(Occur, Box<dyn Query>)> = Vec::new();
    for clause in [
        text_clause,
        type_clause,
        scope_clause,
        tag_clause,
        valid_clause,
        Some(expiration_clause),
    ]
    .into_iter()
    .flatten()
    {
        must_clauses.push((Occur::Must, clause));
    }
    let boxed_query: Box<dyn Query> = if must_clauses.is_empty() {
        Box::new(AllQuery)
    } else {
        Box::new(BooleanQuery::new(must_clauses))
    };

    let docs = searcher.search(&boxed_query, &TopDocs::with_limit(limit).order_by_score())?;
    let max_score = docs
        .first()
        .map(|(score, _)| *score)
        .unwrap_or(1.0)
        .max(0.000_001);
    let mut hits = Vec::new();
    for (score, address) in docs {
        let retrieved = searcher.doc::<TantivyDocument>(address)?;
        if let Some(value) = retrieved
            .get_first(fields.id)
            .and_then(|value| value.as_str())
        {
            hits.push(SearchHit {
                id: value.to_string(),
                score: f64::from(score / max_score),
            });
        }
    }
    Ok(hits)
}

pub fn ensure(index_path: &Path) -> Result<()> {
    ensure_index(index_path).map(|_| ())
}

pub fn validate_existing(index_path: &Path) -> Result<()> {
    open_existing_index(index_path).map(|_| ())
}

pub(crate) fn is_compatibility_error(err: &anyhow::Error) -> bool {
    err.downcast_ref::<IndexCompatibilityError>().is_some()
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
    let exact_tag = builder.add_text_field("exact_tag", STRING);
    let scope = builder.add_text_field("scope", STRING);
    let r#type = builder.add_text_field("type", STRING);
    let valid = builder.add_text_field("valid", STRING);
    let expires_at = builder.add_i64_field("expires_at", INDEXED | FAST);
    let schema = builder.build();
    let fields = IndexFields {
        id,
        name,
        description,
        content,
        tags,
        exact_tag,
        scope,
        r#type,
        valid,
        expires_at,
    };
    (schema, fields)
}

fn ensure_index(path: &Path) -> Result<Index> {
    validate_index_directory(path)?;
    fs::create_dir_all(path)?;
    let index = match Index::open_in_dir(path) {
        Ok(index) => {
            read_index_version(path)?;
            index
        }
        Err(_) => {
            let (schema, _) = build_schema();
            let index = Index::create_in_dir(path, schema).context("create Tantivy index")?;
            write_index_version(path)?;
            index
        }
    };
    register_tokenizers(&index)?;
    Ok(index)
}

fn open_existing_index(path: &Path) -> Result<Index> {
    if !validate_index_directory(path)? {
        return Err(IndexCompatibilityError {
            marker_path: index_version_marker_path(path),
            expected: INDEX_SCHEMA_VERSION,
            found: IndexVersionFound::Missing,
        }
        .into());
    }
    let index = Index::open_in_dir(path).map_err(|error| IndexCompatibilityError {
        marker_path: index_version_marker_path(path),
        expected: INDEX_SCHEMA_VERSION,
        found: IndexVersionFound::Invalid(format!("unreadable Tantivy index: {error}")),
    })?;
    read_index_version(path)?;
    register_tokenizers(&index)?;
    Ok(index)
}

fn validate_index_directory(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(error::safety_violation(format!(
                "refusing unsafe search index path: {}",
                path.display()
            )))
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn index_version_marker_path(index_path: &Path) -> PathBuf {
    index_path.join(INDEX_VERSION_MARKER)
}

fn read_index_version(index_path: &Path) -> Result<i64, IndexCompatibilityError> {
    let marker_path = index_version_marker_path(index_path);
    let contents = fs::read_to_string(&marker_path).map_err(|err| IndexCompatibilityError {
        marker_path: marker_path.clone(),
        expected: INDEX_SCHEMA_VERSION,
        found: if err.kind() == std::io::ErrorKind::NotFound {
            IndexVersionFound::Missing
        } else {
            IndexVersionFound::Invalid(err.to_string())
        },
    })?;
    let trimmed = contents.trim();
    let version = trimmed
        .parse::<i64>()
        .map_err(|_| IndexCompatibilityError {
            marker_path: marker_path.clone(),
            expected: INDEX_SCHEMA_VERSION,
            found: IndexVersionFound::Invalid(trimmed.to_string()),
        })?;
    if version == INDEX_SCHEMA_VERSION {
        Ok(version)
    } else {
        Err(IndexCompatibilityError {
            marker_path,
            expected: INDEX_SCHEMA_VERSION,
            found: IndexVersionFound::Different(version),
        })
    }
}

fn write_index_version(index_path: &Path) -> Result<()> {
    fs::write(
        index_version_marker_path(index_path),
        format!("{INDEX_SCHEMA_VERSION}\n"),
    )
    .context("write index schema version marker")
}

fn register_tokenizers(index: &Index) -> Result<()> {
    search_tokenizer::register(index)
}

fn add_memory_doc(
    writer: &mut IndexWriter,
    fields: &IndexFields,
    memory: &IndexedMemory,
) -> Result<()> {
    let mut document = doc!(
        fields.id => memory.id.clone(),
        fields.name => memory.name.clone(),
        fields.description => memory.description.clone().unwrap_or_default(),
        fields.content => memory.content.clone().unwrap_or_default(),
        fields.tags => memory.tags.clone(),
        fields.scope => memory.scope.clone(),
        fields.r#type => memory.r#type.clone(),
        fields.valid => memory.valid.to_string(),
        fields.expires_at => memory.expires_at,
    );
    for tag in &memory.exact_tags {
        document.add_text(fields.exact_tag, tag);
    }
    writer.add_document(document)?;
    Ok(())
}

fn default_search_fields(fields: &IndexFields) -> Vec<Field> {
    vec![fields.name, fields.description, fields.content, fields.tags]
}

fn boosted_search_fields(fields: &IndexFields) -> [(Field, f32); 4] {
    [
        (fields.name, 4.0),
        (fields.tags, 3.0),
        (fields.description, 2.0),
        (fields.content, 1.0),
    ]
}

fn apply_field_boosts(parser: &mut QueryParser, fields: &IndexFields) {
    for (field, boost) in boosted_search_fields(fields) {
        parser.set_field_boost(field, boost);
    }
}

fn build_fuzzy_query(
    index: &Index,
    query_text: &str,
    fields: &IndexFields,
) -> Result<Option<Box<dyn Query>>> {
    // Fuzzy terms must be produced by the same analyzer that indexed the
    // document. Splitting on whitespace bypasses Lindera and turns an entire
    // Chinese phrase into one term that can never match the indexed tokens.
    let mut analyzer = index
        .tokenizers()
        .get("multilingual")
        .context("multilingual tokenizer is not registered")?;
    let mut stream = analyzer.token_stream(query_text);
    let mut tokens = Vec::new();
    while stream.advance() {
        let token = stream.token().text.trim();
        if !token.is_empty() {
            tokens.push(token.to_string());
        }
    }

    let token_clauses = tokens
        .iter()
        .map(|token| {
            let field_clauses = boosted_search_fields(fields)
                .into_iter()
                .map(|(field, boost)| {
                    let query = Box::new(FuzzyTermQuery::new(
                        Term::from_field_text(field, token),
                        1,
                        true,
                    ));
                    (
                        Occur::Should,
                        Box::new(BoostQuery::new(query, boost)) as Box<dyn Query>,
                    )
                })
                .collect::<Vec<_>>();
            (
                Occur::Must,
                Box::new(BooleanQuery::new(field_clauses)) as Box<dyn Query>,
            )
        })
        .collect::<Vec<_>>();

    Ok(if token_clauses.is_empty() {
        None
    } else {
        Some(Box::new(BooleanQuery::new(token_clauses)))
    })
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
        exact_tag: schema
            .get_field("exact_tag")
            .context("index missing exact_tag field")?,
        scope: schema
            .get_field("scope")
            .context("index missing scope field")?,
        r#type: schema
            .get_field("type")
            .context("index missing type field")?,
        valid: schema
            .get_field("valid")
            .context("index missing valid field")?,
        expires_at: schema
            .get_field("expires_at")
            .context("index missing expires_at field")?,
    })
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
