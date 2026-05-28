use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, BooleanQuery, FuzzyTermQuery, Occur, QueryParser};
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, Value as TantivyValue,
    STORED, STRING,
};
use tantivy::{doc, Index, IndexWriter, TantivyDocument, Term};

use crate::search_tokenizer;

#[derive(Debug, Clone)]
pub struct IndexedMemory {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub content: Option<String>,
    pub tags: String,
    pub scope: String,
    pub r#type: String,
}

pub(crate) struct IndexFields {
    id: Field,
    name: Field,
    description: Field,
    content: Field,
    tags: Field,
    scope: Field,
    r#type: Field,
}

pub fn rebuild(index_path: &Path, memories: &[IndexedMemory]) -> Result<()> {
    if index_path.exists() {
        fs::remove_dir_all(index_path)?;
    }
    fs::create_dir_all(index_path)?;
    let (schema, fields) = build_schema();
    let index = Index::create_in_dir(index_path, schema)?;
    register_tokenizers(&index)?;
    let mut writer = index.writer(50_000_000)?;
    for memory in memories {
        add_memory_doc(&mut writer, &fields, memory)?;
    }
    writer.commit()?;
    Ok(())
}

pub fn upsert(index_path: &Path, memory: &IndexedMemory) -> Result<()> {
    let index = ensure_index(index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let mut writer = index.writer(50_000_000)?;
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

/// Upsert a batch of memories with a single `IndexWriter` and one commit.
#[allow(dead_code)]
pub fn upsert_batch(index_path: &Path, memories: &[IndexedMemory]) -> Result<()> {
    if memories.is_empty() {
        return Ok(());
    }
    let index = ensure_index(index_path)?;
    let fields = fields_from_schema(index.schema())?;
    let mut writer = index.writer(50_000_000)?;
    for memory in memories {
        upsert_with_writer(&mut writer, &fields, memory)?;
    }
    writer.commit()?;
    Ok(())
}

pub fn search(
    index_path: &Path,
    query: &str,
    fuzzy: bool,
    raw_query: bool,
    limit: usize,
) -> Result<Vec<String>> {
    let index = ensure_index(index_path)?;
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
    let docs = searcher.search(&boxed_query, &TopDocs::with_limit(limit).order_by_score())?;
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

pub fn ensure(index_path: &Path) -> Result<()> {
    ensure_index(index_path).map(|_| ())
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
    search_tokenizer::register(index)
}

fn add_memory_doc(
    writer: &mut IndexWriter,
    fields: &IndexFields,
    memory: &IndexedMemory,
) -> Result<()> {
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
