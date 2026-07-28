use std::fmt;

use anyhow::Error;
use serde_json::{json, Map, Value};

pub(crate) const INDEX_STALE_AFTER_WRITE: &str = "index_stale_after_write";

#[derive(Debug)]
pub(crate) struct StructuredCommandError {
    code: &'static str,
    message: String,
    details: Value,
}

impl StructuredCommandError {
    pub(crate) fn code(&self) -> &'static str {
        self.code
    }

    pub(crate) fn details(&self) -> &Value {
        &self.details
    }
}

impl fmt::Display for StructuredCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for StructuredCommandError {}

pub(crate) fn committed_index_error(
    operation: &str,
    operation_details: Value,
    source: Error,
) -> Error {
    let mut details = match operation_details {
        Value::Object(details) => details,
        _ => Map::new(),
    };
    details.insert("operation".to_string(), json!(operation));
    details.insert("durable_write_committed".to_string(), json!(true));
    details.insert("index_stale".to_string(), json!(true));
    details.insert("recovery".to_string(), json!("mem reindex"));

    StructuredCommandError {
        code: INDEX_STALE_AFTER_WRITE,
        message: format!(
            "durable {operation} committed, but the search index update failed; run `mem reindex`: {source:#}"
        ),
        details: Value::Object(details),
    }
    .into()
}
